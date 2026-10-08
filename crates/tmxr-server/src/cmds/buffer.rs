//! Copy mode and paste buffer commands.

use std::fmt::Write as _;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, expand_for};
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
        "copy-mode" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            crate::copy::enter(srv, pid, a.has('u'));
        }
        "paste-buffer" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            let idx = match a.value('b') {
                Some(n) => srv
                    .buffers
                    .iter()
                    .position(|b| b.name == n)
                    .ok_or_else(|| format!("no buffer {n}"))?,
                None if srv.buffers.is_empty() => return Ok(true),
                None => 0,
            };
            let data = srv.buffers[idx].data.clone();
            srv.paste_to_pane(pid, &data);
            if a.has('d') {
                srv.buffers.remove(idx);
            }
        }
        "set-buffer" => {
            // Unlike tmux, the data is format-expanded, so a bind can copy
            // `#{pane_current_path}` (tmux-yank's prefix-Y) without a script.
            let data = expand_for(srv, ctx, None, &pos[0]);
            let data = match (a.has('a'), a.value('b')) {
                (true, name) => {
                    let base = match name {
                        Some(n) => srv.buffers.iter().find(|b| b.name == n),
                        None => srv.buffers.front(),
                    };
                    format!(
                        "{}{data}",
                        base.map(|b| b.data.as_str()).unwrap_or_default()
                    )
                }
                _ => data,
            };
            if a.has('w') {
                srv.set_clipboard(&data);
            }
            srv.add_buffer(data, a.value('b').map(str::to_owned));
        }
        "show-buffer" => {
            let b = match a.value('b') {
                Some(n) => srv.buffers.iter().find(|b| b.name == n),
                None => srv.buffers.front(),
            };
            out.stdout.push_str(&b.ok_or("no buffers")?.data);
        }
        "list-buffers" => {
            for b in &srv.buffers {
                let preview: String = b
                    .data
                    .chars()
                    .take(50)
                    .map(|c| if c == '\n' { ' ' } else { c })
                    .collect();
                let _ = writeln!(
                    out.stdout,
                    "{}: {} bytes: \"{preview}\"",
                    b.name,
                    b.data.len()
                );
            }
        }
        "delete-buffer" => match a.value('b') {
            Some(n) => srv.buffers.retain(|b| b.name != n),
            None => {
                srv.buffers.pop_front();
            }
        },

        _ => return Ok(false),
    }
    Ok(true)
}
