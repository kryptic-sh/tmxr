//! Copy mode and paste buffer commands.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, expand_for, list_item};
use crate::server::{Buffer, Server};
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
        "copy-mode" => {
            if a.has('M') {
                return crate::mouse::copy_mode_drag(srv, ctx).map(|()| true);
            }
            if a.has('S') {
                return crate::mouse::slider_drag(srv, ctx, a.has('e')).map(|()| true);
            }
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            // -q: out of copy mode (and clock mode), as tmux's.
            if a.has('q') {
                if let Some(p) = srv.panes.get_mut(&pid) {
                    p.copy = None;
                    p.clock = false;
                    let window = p.window;
                    srv.mark_window_dirty(window);
                    srv.refit_scrollbar(pid);
                }
                return Ok(true);
            }
            let entering = srv.panes.get(&pid).is_some_and(|p| p.copy.is_none());
            crate::copy::enter(srv, pid, a.has('u'));
            if let Some(cm) = srv.panes.get_mut(&pid).and_then(|p| p.copy.as_mut()) {
                if a.has('e') {
                    cm.scroll_exit = true;
                }
                // tmux reads -H as the mode starts.
                if a.has('H') && entering {
                    cm.hide_position = true;
                }
            }
            // -d: a page down, which with -e may leave copy mode again.
            if a.has('d') {
                crate::copy::command(srv, ctx, pid, "page-down", &[])?;
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
            // tmux 3.6's cmd_paste_buffer_exec: each newline sent as the
            // separator (a carriage return, or a newline with -r), bracketed
            // with -p if the program asked for it, to the pane alone.
            let newline = if a.has('r') { "\n" } else { "\r" };
            let sep = a.value('s').unwrap_or(newline);
            let text = srv.buffers[idx].data.replace('\n', sep);
            if let Some(p) = srv.panes.get_mut(&pid) {
                let bytes = if a.has('p') {
                    tmxr_term::encode_paste(&text, p.emu.input_modes())
                } else {
                    text.into_bytes()
                };
                let _ = p.pty.write(&bytes);
            }
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
            let vars = Vars::for_pane(srv, ctx.pane, ctx.client);
            for b in &srv.buffers {
                // tmux's buffer_sample: the start, with newlines shown as spaces.
                let sample = || -> String {
                    b.data
                        .chars()
                        .take(50)
                        .map(|c| if c == '\n' { ' ' } else { c })
                        .collect()
                };
                let item = |name: &str| match name {
                    "buffer_name" => Some(b.name.clone()),
                    "buffer_size" => Some(b.data.len().to_string()),
                    "buffer_sample" => Some(sample()),
                    _ => tmxr_command::format::Context::get(&vars, name),
                };
                let line = list_item(a, &item, || {
                    format!("{}: {} bytes: \"{}\"", b.name, b.data.len(), sample())
                });
                if let Some(line) = line {
                    let _ = writeln!(out.stdout, "{line}");
                }
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
