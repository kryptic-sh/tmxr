//! Copy mode as the server drives it: entering and leaving it in a pane, and
//! `send-keys -X` commands sent to a pane in it.

use crate::cmds::Ctx;
use crate::model::PaneId;
use crate::server::Server;

use super::{CopyMode, SelKind};

/// Give `pane`'s copy mode the options it reads: `word-separators` and
/// `wrap-search`.
fn take_options(srv: &mut Server, pane: PaneId) {
    let separators = srv.pane_session_options(Some(pane)).word_separators.clone();
    let wrap = srv.pane_options(pane).wrap_search;
    if let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) {
        cm.word_separators = separators;
        cm.wrap_search = wrap;
    }
}

/// Enter copy mode in `pane` (no-op if already in it).
pub fn enter(srv: &mut Server, pane: PaneId, page_up: bool) {
    let Some(p) = srv.panes.get_mut(&pane) else {
        return;
    };
    let entered = p.copy.is_none();
    if entered {
        p.copy = Some(CopyMode::new(p.emu.screen_mut()));
    }
    take_options(srv, pane);
    let Some(p) = srv.panes.get_mut(&pane) else {
        return;
    };
    if page_up && let Some(cm) = p.copy.as_mut() {
        cm.apply("page-up", None);
    }
    let w = p.window;
    srv.mark_window_dirty(w);
    if entered {
        srv.refit_scrollbar(pane);
    }
}

fn leave(srv: &mut Server, pane: PaneId) {
    if let Some(p) = srv.panes.get_mut(&pane) {
        p.copy = None;
        let w = p.window;
        srv.mark_window_dirty(w);
        srv.refit_scrollbar(pane);
    }
}

/// `send-keys -X <name>` in a copy-mode pane.
pub fn command(
    srv: &mut Server,
    _ctx: &Ctx,
    pane: PaneId,
    name: &str,
    args: &[String],
) -> Result<(), String> {
    if name == "refresh-from-pane" {
        // A fresh snapshot of the live pane, keeping where the user was.
        let p = srv.panes.get_mut(&pane).ok_or("no such pane")?;
        let old = p.copy.take().ok_or("not in a mode")?;
        let mut fresh = CopyMode::new(p.emu.screen_mut());
        fresh.carry_over(&old);
        p.copy = Some(fresh);
        let window = p.window;
        take_options(srv, pane);
        srv.mark_window_dirty(window);
        return Ok(());
    }
    take_options(srv, pane);
    let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) else {
        return Err("not in a mode".into());
    };
    match name {
        "cancel" => leave(srv, pane),
        // tmux's: the clipboard gets the selection, the newest buffer gets it
        // added to its end (a new buffer when there is none).
        "append-selection-and-cancel" => {
            if let Some(text) = cm.selection_text() {
                srv.set_clipboard(&text);
                match srv.buffers.front_mut() {
                    Some(top) => top.data.push_str(&text),
                    None => srv.add_buffer(text, None),
                }
            }
            leave(srv, pane);
        }
        "copy-selection"
        | "copy-selection-and-cancel"
        | "copy-selection-no-newlines-and-cancel"
        | "copy-pipe"
        | "copy-pipe-and-cancel"
        | "copy-end-of-line"
        | "copy-end-of-line-and-cancel"
        | "copy-pipe-end-of-line"
        | "copy-pipe-end-of-line-and-cancel"
        | "copy-line"
        | "copy-line-and-cancel"
        | "copy-pipe-line"
        | "copy-pipe-line-and-cancel" => {
            let whole_lines = matches!(
                name,
                "copy-line"
                    | "copy-line-and-cancel"
                    | "copy-pipe-line"
                    | "copy-pipe-line-and-cancel"
            );
            let text = if whole_lines {
                // The cursor's line, and the count's lines below it, whole,
                // leaving the cursor and any selection as they were, as
                // tmux's window_copy_do_copy_line.
                let (cursor, anchor, kind) = ((cm.cy, cm.cx), cm.anchor, cm.kind);
                let lines = std::mem::take(&mut cm.count).max(1);
                cm.cx = 0;
                cm.anchor = Some((cm.cy, 0));
                cm.kind = SelKind::Char;
                for _ in 1..lines {
                    cm.apply("cursor-down", None);
                }
                cm.apply("end-of-line", None);
                let text = cm.selection_text();
                ((cm.cy, cm.cx), cm.anchor, cm.kind) = (cursor, anchor, kind);
                text
            } else if name.contains("end-of-line") {
                // From the cursor to the end of its line, leaving the
                // cursor and any selection as they were.
                let (cursor, anchor, kind) = ((cm.cy, cm.cx), cm.anchor, cm.kind);
                cm.anchor = Some(cursor);
                cm.kind = SelKind::Char;
                cm.apply("end-of-line", None);
                let text = cm.selection_text();
                ((cm.cy, cm.cx), cm.anchor, cm.kind) = (cursor, anchor, kind);
                text
            } else {
                cm.selection_text()
            };
            let cancel = name.ends_with("-and-cancel");
            if let Some(mut text) = text {
                if name.contains("no-newlines") {
                    text = text.replace(['\r', '\n'], "");
                }
                // copy-pipe also hands the text to a command: its argument,
                // else the copy-command option.
                let pipe = name.starts_with("copy-pipe").then(|| {
                    args.first()
                        .cloned()
                        .unwrap_or_else(|| srv.cfg.copy_command.clone())
                });
                if let Some(cmd) = pipe.filter(|c| !c.is_empty()) {
                    crate::server::pipe_to_shell(srv, pane, cmd, text.clone());
                }
                srv.set_clipboard(&text);
                srv.add_buffer(text, None);
            }
            if cancel {
                leave(srv, pane);
            } else if let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) {
                cm.anchor = None;
            }
        }
        // A motion down that leaves copy mode once it is at the bottom, as
        // tmux's: the scrolls when the view reaches the bottom, the cursor
        // when it could not move there.
        "scroll-down-and-cancel"
        | "page-down-and-cancel"
        | "halfpage-down-and-cancel"
        | "cursor-down-and-cancel" => {
            let motion = name.trim_end_matches("-and-cancel");
            let before = cm.cy;
            cm.apply_counted(motion, None);
            let bottom = cm.position().0 == 0;
            let stuck = motion != "cursor-down" || cm.cy == before;
            if bottom && stuck {
                leave(srv, pane);
            }
        }
        other => {
            if !cm.apply_counted(other, args.first().map(String::as_str)) {
                return Err(format!("unknown copy-mode command: {other}"));
            }
            let scrolled_out = matches!(other, "scroll-down" | "page-down")
                && cm.scroll_exit
                && cm.anchor.is_none()
                && cm.position().0 == 0;
            if scrolled_out {
                leave(srv, pane);
            }
        }
    }
    if let Some(w) = srv.panes.get(&pane).map(|p| p.window) {
        srv.mark_window_dirty(w);
    }
    Ok(())
}
