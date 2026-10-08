//! Prompts, messages and the other client-facing commands.

use std::fmt::Write as _;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, attached_client, expand_for, join_args};
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
        "choose-tree" => {
            let c = attached_client(srv, ctx).ok_or("no current client")?;
            let ov = if a.has('w') {
                Overlay::window_picker(srv, c)
            } else {
                Overlay::session_picker(srv, c)
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
