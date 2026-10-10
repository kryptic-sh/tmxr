//! Prompts, messages and the other client-facing commands.

use std::fmt::Write as _;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, Res, attached_client, expand_for, join_args};
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
            // tmux's -b/-H/-s/-S styles, -O and -x/-y placement are accepted and
            // not followed: the menu is centred in tmxr's own colours.
            let (c, pane) = shown_on(srv, ctx, a)?;
            let words: Vec<String> = pos.iter().map(|w| expand_for(srv, ctx, pane, w)).collect();
            let title = a
                .value('T')
                .map(|t| expand_for(srv, ctx, pane, t))
                .unwrap_or_default();
            let start = a.value('C').and_then(|n| n.parse().ok()).unwrap_or(0);
            let menu = crate::menu::Menu::parse(title, &words, start)?;
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(Overlay::Menu(Box::new(menu)));
            }
            // Shown now, not at the next repaint: from a command client
            // nothing else marks the attached client.
            srv.mark_client_dirty(c);
        }
        "display-popup" => {
            // tmux's -b/-s/-S styles, -k and -N are accepted and not followed.
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
            let (cols, rows) = srv.clients[&c]
                .att
                .as_ref()
                .map(|att| (att.cols, att.rows))
                .ok_or("no current client")?;
            let rect = popup_rect(a, cols, rows)?;
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
            if a.count('e') > 1 {
                return Err("display-popup: -e may be given once".into());
            }
            let env: Vec<(String, String)> = a
                .value('e')
                .map(|e| {
                    e.split_once('=')
                        .map(|(k, v)| (k.to_owned(), v.to_owned()))
                        .ok_or_else(|| format!("display-popup: -e {e}: not NAME=VALUE"))
                })
                .transpose()?
                .into_iter()
                .collect();
            let title = a
                .value('T')
                .map(|t| expand_for(srv, ctx, pane, t))
                .unwrap_or_default();
            let border = !a.has('B');
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
            if let Some(c) = attached_client(srv, ctx)
                && let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut())
            {
                att.full_redraw = true;
                att.dirty = true;
            }
        }
        "run-shell" => {
            let line = expand_for(srv, ctx, None, &pos[0]);
            crate::server::run_shell(srv, attached_client(srv, ctx), line, a.has('b'));
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
                // tmux blocks the client until the shell command finishes
                // unless -b; tmxr always runs it in the background, so the
                // chosen command runs after this command list has returned.
                crate::server::if_shell(srv, ctx, cond, then, otherwise);
            }
        }
        "choose-tree" | "choose-buffer" | "choose-client" | "find-window" => {
            let c = attached_client(srv, ctx).ok_or("no current client")?;
            let ov = match p.name() {
                "choose-buffer" if srv.buffers.is_empty() => return Err("no buffers".into()),
                "choose-buffer" => Overlay::buffer_picker(srv),
                "choose-client" => Overlay::client_picker(srv),
                "find-window" => Overlay::window_picker(srv, c, &pos[0]),
                _ if a.has('w') => Overlay::window_picker(srv, c, ""),
                _ => Overlay::session_picker(srv, c),
            };
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(ov);
            }
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
    let inner = super::run_string(srv, ctx, line);
    out.stdout.push_str(&inner.stdout);
    out.attach = inner.attach.or(out.attach);
    if inner.status == 0 {
        Ok(())
    } else {
        Err(inner.stderr)
    }
}

/// The client an overlay command shows on (`-c`, else the best one) and the
/// pane its formats and directory come from (`-t`, else the caller's).
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
        .or(ctx.pane);
    Ok((c, pane))
}

/// A popup's box in a `cols` x `rows` client: `-w` / `-h` in cells or
/// percent (half the client by default), at `-x` / `-y` or centred (`C`).
fn popup_rect(
    a: &tmxr_command::Args,
    cols: u16,
    rows: u16,
) -> Result<ratatui::layout::Rect, String> {
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
    let at = |flag: char, len: u16, total: u16| -> Result<u16, String> {
        match a.value(flag) {
            None | Some("C") => Ok(total.saturating_sub(len) / 2),
            Some(v) => v
                .parse::<u16>()
                .map(|n| n.min(total.saturating_sub(len)))
                .map_err(|_| format!("-{flag} {v}: tmxr takes a number or C")),
        }
    };
    let (w, h) = (size('w', cols)?, size('h', rows)?);
    Ok(ratatui::layout::Rect::new(
        at('x', w, cols)?,
        at('y', h, rows)?,
        w,
        h,
    ))
}
