//! Copy mode and paste buffer commands.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, expand_for};
use crate::server::{Buffer, Server};
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
            if a.has('M') {
                return crate::mouse::copy_mode_drag(srv, ctx).map(|()| true);
            }
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            crate::copy::enter(srv, pid, a.has('u'));
            if a.has('e')
                && let Some(cm) = srv.panes.get_mut(&pid).and_then(|p| p.copy.as_mut())
            {
                cm.scroll_exit = true;
            }
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
                    let base = find_buffer(srv, name);
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
            let b = find_buffer(srv, a.value('b')).ok_or("no buffers")?;
            out.stdout.push_str(&b.data);
        }
        "save-buffer" => {
            let data = &find_buffer(srv, a.value('b')).ok_or("no buffers")?.data;
            if pos[0] == "-" {
                out.stdout.push_str(data);
            } else {
                let path = resolve(ctx, &pos[0]);
                std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .append(a.has('a'))
                    .truncate(!a.has('a'))
                    .open(&path)
                    .and_then(|mut f| f.write_all(data.as_bytes()))
                    .map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
        "load-buffer" => {
            if pos[0] == "-" {
                return Err("load-buffer: reading standard input is not supported".into());
            }
            let path = resolve(ctx, &pos[0]);
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let data = String::from_utf8_lossy(&bytes).into_owned();
            if a.has('w') {
                srv.set_clipboard(&data);
            }
            srv.add_buffer(data, a.value('b').map(str::to_owned));
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

/// The buffer named `name`, else the newest.
fn find_buffer<'a>(srv: &'a Server, name: Option<&str>) -> Option<&'a Buffer> {
    match name {
        Some(n) => srv.buffers.iter().find(|b| b.name == n),
        None => srv.buffers.front(),
    }
}

/// A `save-buffer` / `load-buffer` path: relative paths are relative to the
/// command client's directory, or the home directory for a key bind.
fn resolve(ctx: &Ctx, path: &str) -> PathBuf {
    ctx.cwd
        .clone()
        .unwrap_or_else(crate::util::home_dir)
        .join(path)
}
