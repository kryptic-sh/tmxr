//! Prompts, messages and the other client-facing commands.

use std::fmt::Write as _;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, Res, attached_client, expand_for, join_args};
use crate::model::{ClientId, PaneId};
use crate::overlay::Overlay;
use crate::server::Server;
use crate::target;

pub(super) fn run(
    srv: &mut Server,
    ctx: &Ctx,
    p: &Parsed,
    out: &mut Outcome,
) -> Result<bool, String> {
    let a = &p.args;
    let pos = a.positional();
    match p.name() {
        "command-prompt" => {
            let c = attached_client(srv, ctx).ok_or("no current client")?;
            let initial = a
                .value('I')
                .map(|i| expand_for(srv, ctx, None, i))
                .unwrap_or_default();
            let prompt = a
                .value('p')
                .map_or_else(|| ":".to_owned(), |p| format!("{p} "));
            let template = pos.first().cloned();
            let mut overlay = if a.has('k') {
                Overlay::key_prompt(prompt, template.unwrap_or_else(|| "%%".into()))
            } else {
                Overlay::prompt(prompt, initial, template)
            };
            if let Overlay::Prompt(p) = &mut overlay {
                p.incremental = a.has('i');
                a.value('T').unwrap_or("command").clone_into(&mut p.kind);
                p.history = srv.prompt_history.get(&p.kind).cloned().unwrap_or_default();
            }
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(overlay);
            }
        }
        "display-menu" => {
            // tmux's -O is accepted and not followed.
            let (c, pane) = shown_on(srv, ctx, a)?;

            let title = a
                .value('T')
                .map(|t| expand_for(srv, ctx, pane, t))
                .unwrap_or_default();
            let start = a.value('C').and_then(|n| n.parse().ok()).unwrap_or(0);
            let mut menu =
                crate::menu::Menu::parse(title, pos, start, &|w| expand_for(srv, ctx, pane, w))?;
            menu.look = crate::overlay::BoxLook::from_args(a)?;
            (menu.pane, menu.mouse) = (pane, ctx.mouse);
            // tmux's: a menu the mouse opened (or -M) takes the mouse and
            // starts with nothing highlighted; -O keeps it open.
            menu.mouse_mode = ctx.mouse.is_some() || a.has('M');
            menu.stay_open = a.has('O');
            if menu.mouse_mode {
                menu.selected = None;
            }
            let (cols, rows) = client_size(srv, c)?;
            let places = places(srv, ctx, c, pane);
            menu.at = Some(place(a, (cols, rows), menu.size(), places, |v| {
                expand_for(srv, ctx, pane, v)
            })?);
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(Overlay::Menu(Box::new(menu)));
            }
            // Shown now, not at the next repaint: from a command client
            // nothing else marks the attached client.
            srv.mark_client_dirty(c);
        }
        "display-popup" => {
            // tmux's -k and -N are accepted and not followed.
            let (c, pane) = shown_on(srv, ctx, a)?;
            if a.has('C') {
                if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut())
                    && matches!(att.overlay, Some(Overlay::Popup(_)))
                {
                    att.overlay = None;
                }
                srv.mark_client_dirty(c);
                return Ok(true);
            }
            let (cols, rows) = client_size(srv, c)?;
            let places = places(srv, ctx, c, pane);
            let (w, h) = popup_size(a, (cols, rows))?;
            let (x, y) = place(a, (cols, rows), (w, h), places, |v| {
                expand_for(srv, ctx, pane, v)
            })?;
            let rect = ratatui::layout::Rect::new(x, y, w, h);
            let sid = pane
                .and_then(|p| srv.panes.get(&p))
                .and_then(|p| srv.session_of_window(p.window))
                .or_else(|| srv.clients[&c].att.as_ref().map(|att| att.session))
                .ok_or("no session")?;
            let cwd = match a.value('d') {
                Some(d) => std::path::PathBuf::from(expand_for(srv, ctx, pane, d)),
                None => pane
                    .and_then(|p| crate::vars::pane_current_path(srv, p))
                    .or_else(|| srv.sessions.get(&sid).map(|s| s.cwd.clone()))
                    .unwrap_or_else(crate::util::home_dir),
            };
            let env = super::env_flags(a)?;
            let title = a
                .value('T')
                .map(|t| expand_for(srv, ctx, pane, t))
                .unwrap_or_default();
            let look = crate::overlay::BoxLook::from_args(a)?;
            let border = !a.has('B') && look.lines.is_some();
            let inner = crate::popup::Popup::inner_of(rect, border);
            let id = srv.next_pane;
            let started = srv.start_pty(
                id,
                sid,
                pos,
                cwd,
                &env,
                inner.width.max(1),
                inner.height.max(1),
            )?;
            let popup = crate::popup::Popup {
                id,
                spawn: started.spawn,
                pty: started.pty,
                emu: tmxr_term::Emulator::new(inner.height.max(1), inner.width.max(1), 0),
                output: started.output,
                rect,
                border,
                title,
                look,
                close_on_exit: a.count('E'),
                exited: false,
                extended_keys: srv.cfg.extended_keys == "always",
            };
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(Overlay::Popup(Box::new(popup)));
            }
            srv.mark_client_dirty(c);
        }
        "show-prompt-history" => {
            for (kind, list) in &srv.prompt_history {
                if a.value('T').is_some_and(|t| t != kind) {
                    continue;
                }
                let _ = writeln!(out.stdout, "History for {kind}:\n");
                for (i, entry) in list.iter().enumerate() {
                    let _ = writeln!(out.stdout, "{}: {entry}", i + 1);
                }
                let _ = writeln!(out.stdout);
            }
        }
        "clear-prompt-history" => match a.value('T') {
            Some(kind) => {
                srv.prompt_history.remove(kind);
            }
            None => srv.prompt_history.clear(),
        },
        "confirm-before" => {
            let c = attached_client(srv, ctx).ok_or("no current client")?;
            let cmd = if pos.len() == 1 {
                pos[0].clone()
            } else {
                join_args(pos)
            };
            let prompt = a.value('p').map_or_else(
                || {
                    format!(
                        "Confirm '{}'? (y/n)",
                        cmd.split_whitespace().next().unwrap_or("")
                    )
                },
                |p| expand_for(srv, ctx, None, p),
            );
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(Overlay::confirm(prompt, cmd));
            }
        }
        "display-message" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t')).unwrap_or((0, 0, 0));
            let fmt = pos.first().map_or(
                "[#S] #I:#P [#{pane_width}x#{pane_height}] \"#{pane_title}\" #{pane_current_command} #{pane_current_path}",
                String::as_str,
            );
            let msg = expand_for(srv, ctx, Some(pid), fmt);
            match attached_client(srv, ctx) {
                Some(c) if !a.has('p') => srv.show_message(c, msg),
                _ => {
                    out.stdout.push_str(&msg);
                    out.stdout.push('\n');
                }
            }
        }
        "show-messages" => {
            for m in &srv.messages {
                out.stdout.push_str(m);
                out.stdout.push('\n');
            }
        }
        "refresh-client" => {
            // From a command client, the current client, as tmux's.
            let Some(c) = super::display_client(srv, ctx) else {
                return Ok(true);
            };
            // -L / -R / -U / -D pan a window larger than the client by the
            // adjustment (1 by default), from where the view is now; -c
            // goes back to following the cursor. As tmux 3.6's.
            if "cLRUD".chars().any(|f| a.has(f)) {
                let adjust = match pos.first() {
                    Some(n) => n
                        .parse::<u16>()
                        .ok()
                        .filter(|n| *n >= 1)
                        .ok_or_else(|| format!("adjustment {n} is invalid"))?,
                    None => 1,
                };
                let att = srv
                    .clients
                    .get(&c)
                    .and_then(|c| c.att.as_ref())
                    .ok_or("no client")?;
                let window = srv
                    .sessions
                    .get(&att.session)
                    .and_then(crate::model::Session::current_window)
                    .and_then(|w| srv.windows.get(&w))
                    .map(|w| (w.id, w.cols, w.rows));
                let (ox, oy) = crate::render::window_offset(srv, att);
                let view = (
                    att.cols,
                    att.rows.saturating_sub(crate::server::STATUS_ROWS),
                );
                let pan = match window {
                    _ if a.has('c') => None,
                    None => att.pan,
                    Some((wid, wcols, wrows)) => {
                        let (mut x, mut y) = match att.pan {
                            Some((w, x, y)) if w == wid => (x, y),
                            _ => (ox, oy),
                        };
                        if a.has('L') {
                            x = x.saturating_sub(adjust);
                        } else if a.has('R') {
                            x = x.saturating_add(adjust).min(wcols.saturating_sub(view.0));
                        } else if a.has('U') {
                            y = y.saturating_sub(adjust);
                        } else if a.has('D') {
                            y = y.saturating_add(adjust).min(wrows.saturating_sub(view.1));
                        }
                        Some((wid, x, y))
                    }
                };
                if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                    att.pan = pan;
                    att.dirty = true;
                }
                return Ok(true);
            }
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.full_redraw = true;
                att.dirty = true;
            }
        }
        "run-shell" => {
            use crate::jobs::{Job, Then, Work};
            let delay = match a.value('d') {
                Some(d) => d
                    .parse::<f64>()
                    .ok()
                    .and_then(|s| std::time::Duration::try_from_secs_f64(s).ok())
                    .ok_or_else(|| format!("run-shell: bad delay: {d}"))?,
                None => std::time::Duration::ZERO,
            };
            let pane = a
                .value('t')
                .map(|t| target::pane(srv, ctx, Some(t)).map(|(_, _, p)| p))
                .transpose()?
                .or(ctx.pane);
            let work = match pos.first() {
                Some(cmd) if a.has('C') => Work::Command(cmd.clone()),
                Some(cmd) => Work::Shell {
                    line: expand_for(srv, ctx, pane, cmd),
                    cwd: a
                        .value('c')
                        .map(|c| std::path::PathBuf::from(expand_for(srv, ctx, pane, c))),
                },
                None if a.has('d') => Work::Wait,
                None => return Err("run-shell: no command".into()),
            };
            let job = Job {
                delay,
                work,
                rest: Vec::new(),
            };
            if a.has('b') {
                // In the background: nothing waits, and its output is dropped.
                let quiet = Ctx {
                    client: None,
                    ..ctx.clone()
                };
                srv.start_job(
                    job,
                    Then {
                        ctx: quiet,
                        reply: None,
                    },
                );
            } else {
                out.job = Some(job);
            }
        }
        "if-shell" => {
            let mut ctx = ctx.clone();
            if a.value('t').is_some() {
                ctx.pane = Some(target::pane(srv, &ctx, a.value('t'))?.2);
            }
            let cond = expand_for(srv, &ctx, ctx.pane, &pos[0]);
            let (then, otherwise) = (pos[1].clone(), pos.get(2).cloned());
            if a.has('F') {
                let ok = !cond.is_empty() && cond != "0";
                if let Some(cmd) = if ok { Some(then) } else { otherwise } {
                    run_nested(srv, &ctx, &cmd, out)?;
                }
            } else {
                // The rest of the list waits for the choice, unless -b.
                let job = crate::jobs::Job {
                    delay: std::time::Duration::ZERO,
                    work: crate::jobs::Work::If {
                        line: cond,
                        then,
                        otherwise,
                    },
                    rest: Vec::new(),
                };
                if a.has('b') {
                    let then = crate::jobs::Then { ctx, reply: None };
                    srv.start_job(job, then);
                } else {
                    out.job = Some(job);
                }
            }
        }
        // tmux's -f is a format filter; tmxr opens the picker with it as the
        // query. -a, -F, -N, -t and -Z are accepted and not followed.
        // tmux's -Z zooms the pane the mode is in so it fills the window;
        // tmxr's picker covers the whole client already, so -Z changes
        // nothing.
        "customize-mode" => {
            let c = super::display_client(srv, ctx).ok_or("no current client")?;
            let ov = Overlay::customize_picker(srv, a.value('f').unwrap_or(""));
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(ov);
            }
            srv.mark_client_dirty(c);
        }
        "choose-tree" | "choose-buffer" | "choose-client" | "find-window" => {
            let c = super::display_client(srv, ctx).ok_or("no current client")?;
            let ov = match p.name() {
                "choose-buffer" if srv.buffers.is_empty() => return Err("no buffers".into()),
                "choose-buffer" => Overlay::buffer_picker(srv),
                "choose-client" => Overlay::client_picker(srv),
                "find-window" => {
                    let found = window_matcher(srv, a, &pos[0])?;
                    Overlay::found_windows(srv, c, found)
                }
                _ if a.has('w') => Overlay::window_picker(srv, c),
                _ => Overlay::session_picker(srv, c),
            };
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(ov);
            }
            srv.mark_client_dirty(c);
        }
        "list-commands" => {
            for c in tmxr_command::table::COMMANDS {
                let alias = c.alias.map(|a| format!(" ({a})")).unwrap_or_default();
                let _ = writeln!(out.stdout, "{}{alias} {}", c.name, c.usage);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Run a command line as part of the current command: its output joins
/// `out`, and its failure fails the current command.
fn run_nested(srv: &mut Server, ctx: &Ctx, line: &str, out: &mut Outcome) -> Res {
    let inner = super::run_line(srv, ctx, line);
    out.stdout.push_str(&inner.stdout);
    out.attach = inner.attach.or(out.attach);
    // A run-shell in it: the outer list waits for it too.
    out.job = inner.job;
    if inner.status == 0 {
        Ok(())
    } else {
        Err(inner.stderr)
    }
}

/// The client an overlay command shows on (`-c`, else the best one) and the
/// pane its formats and directory come from (`-t`, else the caller's, else
/// that client's current pane).
fn shown_on(
    srv: &Server,
    ctx: &Ctx,
    a: &tmxr_command::Args,
) -> Result<(crate::model::ClientId, Option<crate::model::PaneId>), String> {
    let c = match a.value('c') {
        Some(name) => srv
            .clients
            .values()
            .find(|cl| cl.att.is_some() && cl.id.to_string() == name)
            .map(|cl| cl.id)
            .ok_or_else(|| format!("can't find client: {name}"))?,
        None => super::display_client(srv, ctx).ok_or("no current client")?,
    };
    let pane = a
        .value('t')
        .map(|t| target::pane(srv, ctx, Some(t)).map(|(_, _, p)| p))
        .transpose()?
        .or(ctx.pane)
        // As tmux: else the shown-on client's current pane.
        .or_else(|| {
            let session = srv.clients.get(&c)?.att.as_ref()?.session;
            srv.active_pane_of_session(session)
        });
    Ok((c, pane))
}

/// Client `c`'s size.
fn client_size(srv: &Server, c: ClientId) -> Result<(u16, u16), String> {
    srv.clients
        .get(&c)
        .and_then(|c| c.att.as_ref())
        .map(|att| (att.cols, att.rows))
        .ok_or_else(|| "no current client".into())
}

/// What a menu or popup on client `c` for `pane` may be placed by.
fn places(srv: &Server, ctx: &Ctx, c: ClientId, pane: Option<PaneId>) -> Places {
    // Panes and pane clicks are in window cells; a box is placed in the
    // client's, which differ while the view is panned.
    let (ox, oy) = srv
        .clients
        .get(&c)
        .and_then(|c| c.att.as_ref())
        .map_or((0, 0), |a| crate::render::window_offset(srv, a));
    Places {
        pane: pane.and_then(|p| srv.panes.get(&p)).map(|p| {
            ratatui::layout::Rect::new(
                p.rect.x.saturating_sub(ox),
                p.rect.y.saturating_sub(oy),
                p.rect.w,
                p.rect.h,
            )
        }),
        mouse: ctx.mouse.map(|m| match m.location {
            tmxr_command::MouseLocation::Pane | tmxr_command::MouseLocation::Border => {
                (m.col.saturating_sub(ox), m.row.saturating_sub(oy))
            }
            _ => (m.col, m.row),
        }),
        window: srv
            .clients
            .get(&c)
            .and_then(|c| c.att.as_ref())
            .and_then(|att| {
                let current = srv.sessions.get(&att.session)?.current;
                let ranges = &att.status_ranges.windows;
                ranges.iter().find(|r| r.2 == current).map(|r| r.0)
            }),
    }
}

/// Where a popup or menu may be put besides numbers, as tmux's position
/// letters and `popup_*` variables name them.
#[derive(Debug, Clone, Copy, Default)]
struct Places {
    /// The target pane, in client cells (`P`).
    pane: Option<ratatui::layout::Rect>,
    /// The mouse, for a popup a mouse bind opened (`M`).
    mouse: Option<(u16, u16)>,
    /// Where the current window's name starts on the status line (`W`).
    window: Option<u16>,
}

/// A popup's size in a `cols` x `rows` client: `-w` / `-h` in cells or
/// percent, half the client by default.
fn popup_size(a: &tmxr_command::Args, (cols, rows): (u16, u16)) -> Result<(u16, u16), String> {
    let size = |flag: char, total: u16| -> Result<u16, String> {
        let n = match a.value(flag) {
            None => total / 2,
            Some(v) => match v.strip_suffix('%') {
                Some(pct) => {
                    let pct: u32 = pct.parse().map_err(|_| format!("bad size: {v}"))?;
                    u16::try_from(u32::from(total) * pct.min(100) / 100).unwrap_or(total)
                }
                None => v.parse().map_err(|_| format!("bad size: {v}"))?,
            },
        };
        Ok(n.clamp(1, total.max(1)))
    };
    Ok((size('w', cols)?, size('h', rows)?))
}

/// Where a `w` x `h` box goes in a `cols` x `rows` client, from `-x` and
/// `-y`, transcribed from tmux 3.6's `cmd_display_menu_get_pos`. Each is a
/// number or a format, which may use tmux's `popup_*` variables, or a
/// letter standing for one of them: `-x` `C` centre, `R` the pane's right
/// edge, `P` its left, `M` centred on the mouse, `W` the window's name on
/// the status line; `-y` `C`, `P` the pane's bottom, `M` the mouse, `S` the
/// status line, `W` the window's name. `-y` gives the box's bottom edge. A
/// variable this client lacks (no mouse event, say) is empty, and empty is
/// 0, as tmux's `strtol` makes it.
fn place(
    a: &tmxr_command::Args,
    (cols, rows): (u16, u16),
    (w, h): (u16, u16),
    places: Places,
    expand: impl Fn(&str) -> String,
) -> Result<(u16, u16), String> {
    let (sx, sy) = (i64::from(cols), i64::from(rows));
    let (w, h) = (i64::from(w), i64::from(h));
    let lines = i64::from(crate::server::STATUS_ROWS);
    let mut vars: Vec<(&str, i64)> = vec![
        ("popup_width", w),
        ("popup_height", h),
        ("popup_centre_x", ((sx - 1) / 2 - w / 2).max(0)),
        ("popup_status_line_y", sy - lines),
    ];
    let n = (sy - 1) / 2 + h / 2;
    vars.push(("popup_centre_y", if n >= sy { sy - h } else { n }));
    if let Some(x) = places.window {
        vars.push(("popup_window_status_line_x", i64::from(x)));
        vars.push(("popup_window_status_line_y", sy - lines));
    }
    if let Some((mx, my)) = places.mouse {
        let (mx, my) = (i64::from(mx), i64::from(my));
        vars.push(("popup_mouse_x", mx));
        vars.push(("popup_mouse_y", my));
        vars.push(("popup_mouse_centre_x", (mx - w / 2).max(0)));
        let n = my - h / 2;
        vars.push(("popup_mouse_centre_y", if n + h >= sy { sy - h } else { n }));
        vars.push(("popup_mouse_top", (my + h).min(sy - 1)));
        vars.push(("popup_mouse_bottom", (my - h).max(0)));
    }
    if let Some(r) = places.pane {
        let (x, y) = (i64::from(r.x), i64::from(r.y));
        let (pw, ph) = (i64::from(r.width), i64::from(r.height));
        vars.push(("popup_pane_top", if y + h >= sy { sy - h } else { y + h }));
        vars.push(("popup_pane_bottom", y + ph));
        vars.push(("popup_pane_left", x));
        vars.push(("popup_pane_right", (x + pw - w).max(0)));
    }
    let resolve = |flag: char, letters: &[(&str, &str)]| -> Result<i64, String> {
        let given = a.value(flag).unwrap_or("C");
        let format = letters
            .iter()
            .find(|(l, _)| *l == given)
            .map_or(given, |(_, f)| f);
        // tmxr's formats do not know the popup_* variables: those first.
        let mut text = format.to_owned();
        for (name, value) in &vars {
            text = text.replace(&format!("#{{{name}}}"), &value.to_string());
        }
        let text = expand(&text);
        let text = text.trim();
        if text.is_empty() {
            return Ok(0);
        }
        text.parse()
            .map_err(|_| format!("-{flag} {given}: not a number ({text})"))
    };
    let mut x = resolve(
        'x',
        &[
            ("C", "#{popup_centre_x}"),
            ("R", "#{popup_pane_right}"),
            ("P", "#{popup_pane_left}"),
            ("M", "#{popup_mouse_centre_x}"),
            ("W", "#{popup_window_status_line_x}"),
        ],
    )?;
    if x + w >= sx {
        x = sx - w;
    }
    let mut y = resolve(
        'y',
        &[
            ("C", "#{popup_centre_y}"),
            ("P", "#{popup_pane_bottom}"),
            ("M", "#{popup_mouse_top}"),
            ("S", "#{popup_status_line_y}"),
            ("W", "#{popup_window_status_line_y}"),
        ],
    )?;
    y = if y < h { 0 } else { y - h };
    if y + h >= sy {
        y = sy - h;
    }
    let cell = |n: i64| u16::try_from(n.max(0)).unwrap_or(u16::MAX);
    Ok((cell(x), cell(y)))
}

/// `find-window`'s test: `text` in a window's name (`-N`), a pane's title
/// (`-T`) or a pane's visible contents (`-C`), all three when none is
/// given, as tmux's `*text*` match; `-i` ignores case and `-r` makes `text`
/// a regular expression.
fn window_matcher<'s>(
    srv: &'s Server,
    a: &tmxr_command::Args,
    text: &str,
) -> Result<impl Fn(&crate::model::Window) -> bool + 's, String> {
    let all = !(a.has('N') || a.has('T') || a.has('C'));
    let (names, titles, contents) = (all || a.has('N'), all || a.has('T'), all || a.has('C'));
    let pattern = if a.has('r') {
        text.to_owned()
    } else {
        regex::escape(text)
    };
    let re = regex::RegexBuilder::new(&pattern)
        .case_insensitive(a.has('i'))
        .build()
        .map_err(|e| format!("find-window: {e}"))?;
    Ok(move |w: &crate::model::Window| {
        (names && re.is_match(&w.name))
            || w.panes().iter().filter_map(|p| srv.panes.get(p)).any(|p| {
                (titles && p.emu.title().is_some_and(|t| re.is_match(t)))
                    || (contents && re.is_match(&p.emu.screen().contents()))
            })
    })
}
