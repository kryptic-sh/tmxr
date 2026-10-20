//! Prompts, messages and the other client-facing commands.

use std::fmt::Write as _;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, Res, expand_for, join_args};
use crate::model::{ClientId, PaneId};
use crate::overlay::Overlay;
use crate::server::Server;
use crate::target;
use crate::vars::Vars;

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
            // -t names the client; else the caller, or (from a command
            // client) the best attached one, as tmux's.
            let c = match a.value('t') {
                Some(spec) => super::session::target_client(srv, spec)?,
                None => super::display_client(srv, ctx).ok_or("no current client")?,
            };
            // Formats (-F's template, -I's inputs) are expanded for the
            // caller's pane, else that of the client the prompt shows on,
            // as tmux's command target is the current session from a shell.
            let pane = ctx.pane.or_else(|| {
                let session = srv.clients.get(&c)?.att.as_ref()?.session;
                srv.active_pane_of_session(session)
            });
            // -F: the template is a format, expanded first.
            let template = pos.first().map(|t| {
                if a.has('F') {
                    expand_for(srv, ctx, pane, t)
                } else {
                    t.clone()
                }
            });
            // -p's prompts and -I's inputs, by comma (one each with -l),
            // each prompt followed by a space; without -p the template's
            // command in brackets, or ":" with none, as tmux's.
            let prompts: Vec<String> = match a.value('p') {
                Some(p) if a.has('l') => vec![format!("{p} ")],
                Some(p) => p.split(',').map(|p| format!("{p} ")).collect(),
                None => vec![
                    template
                        .as_ref()
                        .map_or_else(|| ":".to_owned(), |t| format!("({t}) ")),
                ],
            };
            let inputs: Vec<String> = match a.value('I') {
                Some(i) if a.has('l') => vec![expand_for(srv, ctx, pane, i)],
                Some(i) => i
                    .split(',')
                    .map(|i| expand_for(srv, ctx, pane, i))
                    .collect(),
                None => Vec::new(),
            };
            let mut queue = prompts
                .into_iter()
                .enumerate()
                .map(|(n, p)| (p, inputs.get(n).cloned().unwrap_or_default()));
            let (prompt, initial) = queue.next().expect("one prompt at least");
            let mut overlay = if a.has('k') {
                Overlay::key_prompt(prompt, template.unwrap_or_else(|| "%%".into()))
            } else {
                Overlay::prompt(prompt, initial, template)
            };
            if let Overlay::Prompt(p) = &mut overlay {
                p.pending = queue.collect();
                p.single = a.has('1');
                p.numeric = a.has('N') && !p.single;
                p.incremental = a.has('i');
                if p.incremental {
                    p.last = p.input.drain(..).collect();
                    p.cursor = 0;
                }
                a.value('T').unwrap_or("command").clone_into(&mut p.kind);
                p.history = srv.prompt_history.get(&p.kind).cloned().unwrap_or_default();
            }
            // An incremental prompt runs its template once as it opens, with
            // the input `=`, as tmux's.
            let opened = match &overlay {
                Overlay::Prompt(p) if p.incremental => Some(p.incremental_command('=')),
                _ => None,
            };
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(overlay);
            }
            if let Some(cmd) = opened {
                srv.run_bind(c, &cmd, None);
            }
            srv.mark_client_dirty(c);
        }
        "display-menu" => {
            let (c, pane) = shown_on(srv, ctx, a)?;

            let title = a
                .value('T')
                .map(|t| expand_for(srv, ctx, pane, t))
                .unwrap_or_default();
            let start = a.value('C').and_then(|n| n.parse().ok()).unwrap_or(0);
            let mut menu =
                crate::menu::Menu::parse(title, pos, start, &|w| expand_for(srv, ctx, pane, w))?;
            menu.look = crate::overlay::BoxLook::from_args(
                a,
                "menu-border-lines",
                srv.client_window_options(c).menu_border_lines,
            )?;
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
            // tmux's: on an open popup, display-popup changes it (its title,
            // empty without -T, styles, border and closing); over a menu it
            // does nothing.
            let title = a
                .value('T')
                .map(|t| expand_for(srv, ctx, pane, t))
                .unwrap_or_default();
            let look = crate::overlay::BoxLook::from_args(
                a,
                "popup-border-lines",
                srv.client_window_options(c).popup_border_lines,
            )?;
            let border = !a.has('B') && look.lines.is_some();
            let close_on_exit = a.count('E').min(2);
            let closing = a.has('N') || a.has('E') || a.has('k');
            match srv
                .clients
                .get_mut(&c)
                .and_then(|c| c.att.as_mut())
                .and_then(|att| att.overlay.as_mut())
            {
                Some(Overlay::Popup(p)) => {
                    p.title = title;
                    if look.style.is_some() {
                        p.look.style = look.style;
                    }
                    if look.border_style.is_some() {
                        p.look.border_style = look.border_style;
                    }
                    if a.has('B') || a.has('b') {
                        p.look.lines = look.lines;
                        p.set_border(border);
                    }
                    if closing {
                        p.close_on_exit = close_on_exit;
                        p.any_key = a.has('k');
                    }
                    srv.mark_client_dirty(c);
                    return Ok(true);
                }
                Some(Overlay::Menu(_)) => return Ok(true),
                _ => {}
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
                close_on_exit,
                any_key: a.has('k'),
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
            // -t names the client to ask; else the caller, or (from a
            // command client) the best attached one, as tmux's.
            let c = match a.value('t') {
                Some(spec) => super::session::target_client(srv, spec)?,
                None => super::display_client(srv, ctx).ok_or("no current client")?,
            };
            let cmd = if pos.len() == 1 {
                pos[0].clone()
            } else {
                join_args(pos)
            };
            // -c: the key that confirms, one printable ASCII character, as
            // tmux's.
            let key = match a.value('c') {
                None => 'y',
                Some(c) => match c.as_bytes() {
                    [b @ b' '..=b'~'] => char::from(*b),
                    _ => return Err("invalid confirm key".into()),
                },
            };
            let prompt = a.value('p').map_or_else(
                || {
                    format!(
                        "Confirm '{}'? ({key}/n)",
                        cmd.split_whitespace().next().unwrap_or("")
                    )
                },
                |p| expand_for(srv, ctx, None, p),
            );
            // -b (do not wait for the answer) is how tmxr's prompt always
            // works: the commands after it run at once.
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(Overlay::confirm(prompt, cmd, key, a.has('y')));
            }
            srv.mark_client_dirty(c);
        }
        "display-message" => {
            // A -t that names nothing is an error, as tmux's; with no -t and
            // no pane to be current, the formats expand without one.
            let target = match a.value('t') {
                Some(t) => Some(target::pane(srv, ctx, Some(t))?),
                None => target::pane(srv, ctx, None).ok(),
            };
            if a.value('F').is_some() && !pos.is_empty() {
                return Err("only one of -F or argument must be given".into());
            }
            let delay = a
                .value('d')
                .map(|d| {
                    d.parse::<u64>()
                        .map_err(|_| format!("delay {d} is invalid"))
                })
                .transpose()?;
            let fmt = pos.first().map(String::as_str).or_else(|| a.value('F')).unwrap_or(
                "[#S] #I:#P [#{pane_width}x#{pane_height}] \"#{pane_title}\" #{pane_current_command} #{pane_current_path}",
            );
            // The target's session, which a linked window has several of.
            let mut vars = Vars::for_pane(srv, target.map(|(_, _, p)| p), ctx.client);
            if let Some((sid, _, _)) = target {
                vars.session = Some(sid);
            }
            // -I: the client's input shown in an empty pane, and nothing
            // else, as tmux's window_pane_start_input.
            if a.has('I') {
                let Some((_, _, pane)) = target else {
                    return Ok(true);
                };
                if let Some(bytes) = super::take_stdin(srv, ctx) {
                    srv.feed_empty_pane(pane, &bytes)?;
                } else if srv.panes.get(&pane).is_some_and(|p| !p.pty.is_empty()) {
                    return Err("pane is not empty".into());
                }
                return Ok(true);
            }
            // -a: every variable with a value, as tmux's format_each.
            if a.has('a') {
                for name in crate::vars::NAMES {
                    if let Some(value) = tmxr_command::format::Context::get(&vars, name) {
                        let _ = writeln!(out.stdout, "{name}={value}");
                    }
                }
                return Ok(true);
            }
            // -l: the text as given, not expanded; else with its strftime
            // sequences filled in first, as tmux's.
            let msg = if a.has('l') {
                fmt.to_owned()
            } else {
                super::expand_by(srv, ctx, &vars, fmt, tmxr_command::format::expand_time)
            };
            // -c: the client the message shows on, else the command's or,
            // from a shell, the best one, as tmux's cmd_find_current_client;
            // with none, as tmux's, it shows nowhere.
            let client = match a.value('c') {
                Some(c) => Some(super::session::target_client(srv, c)?),
                None => super::display_client(srv, ctx),
            };
            match client {
                _ if a.has('p') => {
                    out.stdout.push_str(&msg);
                    out.stdout.push('\n');
                }
                Some(c) => srv.show_message_for(c, msg, delay, a.has('N'), true),
                None => {}
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
            // -f sets the client's flags (-F is the same, as tmux's), then
            // the client redraws.
            for list in [a.value('F'), a.value('f')] {
                let flags = super::session::client_flags(list)?;
                super::session::set_client_flags(srv, c, &flags);
            }
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
                    att.rows.saturating_sub(srv.status_rows(att.session)),
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
                    stderr: a.has('E'),
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
                session: srv
                    .session_of_pane(pane)
                    .or_else(|| target::current_session(srv, ctx)),
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
                    session: target::current_session(srv, &ctx),
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
            // -t: its session's, window's and pane's own options too, as
            // tmux's tree shows the target's, each set back at its scope.
            let mut local = Vec::new();
            if let Some(t) = a.value('t') {
                let (sid, wid, pid) = target::pane(srv, ctx, Some(t))?;
                for (flag, id) in [
                    ("", format!("${sid}")),
                    ("-w ", format!("@{wid}")),
                    ("-p ", format!("%{pid}")),
                ] {
                    let listed = super::run_line(srv, ctx, &format!("show-options {flag}-t {id}"));
                    for line in listed.stdout.lines() {
                        local.push((
                            format!("{flag}-t {id} {line}"),
                            format!("set-option {flag}-t {id} {line}"),
                        ));
                    }
                }
            }
            let ov = Overlay::customize_picker(srv, a.value('f').unwrap_or(""), local);
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(ov);
            }
            srv.mark_client_dirty(c);
        }
        "choose-tree" | "choose-buffer" | "choose-client" | "find-window" => {
            let c = super::display_client(srv, ctx).ok_or("no current client")?;
            // The current session and window the tree starts from: -t's
            // (tmux's target pane for the mode), else the client's.
            let current = match a.value('t') {
                Some(t) => {
                    let (sid, idx, _) = target::window(srv, ctx, Some(t))?;
                    Some((sid, idx))
                }
                None => srv
                    .clients
                    .get(&c)
                    .and_then(|cl| cl.att.as_ref())
                    .and_then(|at| srv.sessions.get(&at.session))
                    .map(|s| (s.id, s.current)),
            };
            let choose = crate::overlay::Choose {
                format: a.value('F'),
                filter: a.value('f'),
                sort: a.value('O'),
                reverse: a.has('r'),
                client: Some(c),
                key_format: a.value('K'),
            };
            let mut ov = match p.name() {
                "choose-buffer" if srv.buffers.is_empty() => return Err("no buffers".into()),
                "choose-buffer" => Overlay::buffer_picker(srv, &choose),
                "choose-client" => Overlay::client_picker(srv, &choose),
                "find-window" => {
                    let found = window_matcher(srv, a, &pos[0])?;
                    Overlay::found_windows(srv, current, found)
                }
                _ if a.has('w') => Overlay::window_picker(srv, current, &choose),
                _ => Overlay::session_picker(srv, current.map(|c| c.0), &choose),
            };
            // A template runs for the chosen item, as tmux's.
            // find-window's argument is its match, and its -N matches names.
            if let Overlay::Picker(picker) = &mut ov
                && p.name() != "find-window"
            {
                picker.template = pos.first().cloned();
                // -N: no preview; -y: no question before a kill.
                picker.no_preview = a.has('N');
                picker.no_confirm = a.has('y');
            }
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
    let c = super::target_client(srv, ctx, a)?;
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
    // The window starts below a status line at the top.
    let (ox, oy, top) = srv
        .clients
        .get(&c)
        .and_then(|c| c.att.as_ref())
        .map_or((0, 0, 0), |a| {
            let (ox, oy) = crate::render::window_offset(srv, a);
            (ox, oy, srv.window_top(a.session))
        });
    Places {
        pane: pane.and_then(|p| srv.panes.get(&p)).map(|p| {
            ratatui::layout::Rect::new(
                p.rect.x.saturating_sub(ox),
                top + p.rect.y.saturating_sub(oy),
                p.rect.w,
                p.rect.h,
            )
        }),
        mouse: ctx.mouse.map(|m| match m.location {
            tmxr_command::MouseLocation::Pane | tmxr_command::MouseLocation::Border => {
                (m.col.saturating_sub(ox), top + m.row.saturating_sub(oy))
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
        status_lines: srv.client_status_rows(c),
        status_top: srv.client_session(c).is_some_and(|s| srv.status_at_top(s)),
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
    /// The rows the client's status line takes.
    status_lines: u16,
    /// Whether that line is the client's top row.
    status_top: bool,
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
    let lines = i64::from(places.status_lines);
    let mut vars: Vec<(&str, i64)> = vec![
        ("popup_width", w),
        ("popup_height", h),
        ("popup_centre_x", ((sx - 1) / 2 - w / 2).max(0)),
    ];
    // As tmux's cmd_display_menu_get_pos: below a status line at the top,
    // above one at the bottom.
    vars.push((
        "popup_status_line_y",
        if places.status_top {
            lines + h
        } else {
            sy - lines
        },
    ));
    let n = (sy - 1) / 2 + h / 2;
    vars.push(("popup_centre_y", if n >= sy { sy - h } else { n }));
    if let Some(x) = places.window {
        vars.push(("popup_window_status_line_x", i64::from(x)));
        let y = if places.status_top { 1 + h } else { sy - lines };
        vars.push(("popup_window_status_line_y", y));
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
