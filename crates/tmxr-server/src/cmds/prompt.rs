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
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(Overlay::prompt(prompt, initial, template));
            }
        }
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
        "choose-tree" | "choose-buffer" | "find-window" => {
            let c = attached_client(srv, ctx).ok_or("no current client")?;
            let ov = match p.name() {
                "choose-buffer" if srv.buffers.is_empty() => return Err("no buffers".into()),
                "choose-buffer" => Overlay::buffer_picker(srv),
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
