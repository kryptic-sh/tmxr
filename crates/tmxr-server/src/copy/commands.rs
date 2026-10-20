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
    enter_from(srv, pane, pane, page_up);
}

/// A snapshot of `source`'s screen and history, sized to `pane`'s view.
fn snapshot_for(srv: &mut Server, pane: PaneId, source: PaneId) -> Option<CopyMode> {
    let (rows, cols) = srv.panes.get(&pane)?.emu.screen().size();
    let mut cm = CopyMode::new(srv.panes.get_mut(&source)?.emu.screen_mut());
    if source != pane {
        cm.source = Some(source);
        cm.resize(cols, rows);
    }
    Some(cm)
}

/// Enter copy mode in `pane` showing `source`'s screen and history
/// (`copy-mode -s`, as tmux's window_copy_init with a source pane); no-op
/// if already in it.
pub fn enter_from(srv: &mut Server, pane: PaneId, source: PaneId, page_up: bool) {
    let entered = srv.panes.get(&pane).is_some_and(|p| p.copy.is_none());
    if entered {
        let mut cm = snapshot_for(srv, pane, source);
        if let Some(p) = srv.panes.get_mut(&pane) {
            if let Some(cm) = cm.as_mut() {
                cm.inherited_search.clone_from(&p.last_search);
            }
            p.copy = cm;
        }
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

/// A copying command's arguments, as tmux's `CP` flags and positionals:
/// `-P` adds no buffer, `-C` sets no clipboard; then copy-pipe's command,
/// and the prefix of the buffer's name.
struct CopyArgs<'a> {
    no_buffer: bool,
    no_clipboard: bool,
    command: Option<&'a str>,
    prefix: Option<&'a str>,
}

/// The copy commands that take flags (`-C`, `-P`), as tmux 3.6's
/// window_copy_cmd_table gives them.
const TAKE_FLAGS: &[&str] = &[
    "copy-end-of-line",
    "copy-end-of-line-and-cancel",
    "copy-pipe-end-of-line",
    "copy-pipe-end-of-line-and-cancel",
    "copy-line",
    "copy-line-and-cancel",
    "copy-pipe-line",
    "copy-pipe-line-and-cancel",
    "copy-pipe-no-clear",
    "copy-pipe",
    "copy-pipe-and-cancel",
    "copy-selection-no-clear",
    "copy-selection",
    "copy-selection-and-cancel",
    "next-prompt",
    "previous-prompt",
];

/// Whether tmux would skip copy command `name` given `args`: it parses them
/// against the command's own flags and drops the command when they do not
/// parse, so one that takes none is skipped when its first argument looks
/// like a flag (`-li`, an incremental search up's text, as `command-prompt
/// -i` writes it after `C-r`), unless `--` comes first.
pub fn refuses_args(name: &str, args: &[String]) -> bool {
    !TAKE_FLAGS.contains(&name)
        && args
            .first()
            .is_some_and(|a| a.len() > 1 && a.starts_with('-') && a != "--")
}

impl<'a> CopyArgs<'a> {
    fn parse(args: &'a [String], pipe: bool) -> Self {
        let mut copy = Self {
            no_buffer: false,
            no_clipboard: false,
            command: None,
            prefix: None,
        };
        let mut rest = args.iter().map(String::as_str);
        let mut positional = Vec::new();
        for arg in rest.by_ref() {
            if arg == "--" {
                break;
            }
            match arg
                .strip_prefix('-')
                .filter(|f| !f.is_empty() && f.chars().all(|c| c == 'P' || c == 'C'))
            {
                Some(flags) => {
                    copy.no_buffer |= flags.contains('P');
                    copy.no_clipboard |= flags.contains('C');
                }
                None => {
                    positional.push(arg);
                    break;
                }
            }
        }
        positional.extend(rest);
        let mut positional = positional.into_iter();
        if pipe {
            copy.command = positional.next();
        }
        copy.prefix = positional.next();
        copy
    }
}

/// `send-keys -X <name>` in a copy-mode pane; a search it makes is kept
/// with the pane, as tmux's window_copy_search keeps `wp->searchstr`.
pub fn command(
    srv: &mut Server,
    ctx: &Ctx,
    pane: PaneId,
    name: &str,
    args: &[String],
) -> Result<(), String> {
    let result = run_command(srv, ctx, pane, name, args);
    if let Some(p) = srv.panes.get_mut(&pane)
        && let Some(search) = p.copy.as_ref().and_then(|cm| cm.search.clone())
    {
        p.last_search = Some(search);
    }
    result
}

fn run_command(
    srv: &mut Server,
    ctx: &Ctx,
    pane: PaneId,
    name: &str,
    args: &[String],
) -> Result<(), String> {
    if name == "refresh-from-pane" {
        // A fresh snapshot of the live pane (or the one `copy-mode -s`
        // showed, while it lasts), keeping where the user was.
        let p = srv.panes.get_mut(&pane).ok_or("no such pane")?;
        let old = p.copy.take().ok_or("not in a mode")?;
        let source = old
            .source
            .filter(|s| srv.panes.contains_key(s))
            .unwrap_or(pane);
        let mut fresh = snapshot_for(srv, pane, source).ok_or("no such pane")?;
        fresh.carry_over(&old);
        let p = srv.panes.get_mut(&pane).ok_or("no such pane")?;
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
        // added to its end (a new buffer when there is none); then the
        // selection is cleared, or copy mode left.
        "append-selection" | "append-selection-and-cancel" => {
            if let Some(text) = cm.selection_text() {
                srv.copy_to_clipboard(pane, &text);
                match srv.buffers.front_mut() {
                    Some(top) => top.data.push_str(&text),
                    None => srv.add_buffer(text, None),
                }
            }
            if name == "append-selection" {
                if let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) {
                    cm.apply("clear-selection", None);
                }
            } else {
                leave(srv, pane);
            }
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
        | "copy-pipe-line-and-cancel"
        | "copy-selection-no-clear"
        | "copy-pipe-no-clear" => {
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
                let sel_end = cm.sel_end.take();
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
                cm.sel_end = sel_end;
                text
            } else if name.contains("end-of-line") {
                // From the cursor to the end of its line, leaving the
                // cursor and any selection as they were.
                let (cursor, anchor, kind) = ((cm.cy, cm.cx), cm.anchor, cm.kind);
                let sel_end = cm.sel_end.take();
                cm.anchor = Some(cursor);
                cm.kind = SelKind::Char;
                cm.apply("end-of-line", None);
                let text = cm.selection_text();
                ((cm.cy, cm.cx), cm.anchor, cm.kind) = (cursor, anchor, kind);
                cm.sel_end = sel_end;
                text
            } else {
                cm.selection_text()
            };
            let cancel = name.ends_with("-and-cancel");
            if let Some(mut text) = text {
                if name.contains("no-newlines") {
                    text = text.replace(['\r', '\n'], "");
                }
                let copy = CopyArgs::parse(args, name.starts_with("copy-pipe"));
                let expand = |s: &str| crate::cmds::expand_for(srv, ctx, Some(pane), s);
                // copy-pipe also hands the text to a command: its argument,
                // else the copy-command option.
                let pipe = name.starts_with("copy-pipe").then(|| {
                    copy.command
                        .map_or_else(|| srv.cfg.copy_command.clone(), expand)
                });
                let prefix = copy.prefix.map(expand);
                if let Some(cmd) = pipe.filter(|c| !c.is_empty()) {
                    crate::server::pipe_to_shell(srv, pane, cmd, text.clone());
                }
                if !copy.no_clipboard {
                    srv.copy_to_clipboard(pane, &text);
                }
                if !copy.no_buffer {
                    srv.add_buffer_prefixed(text, prefix.as_deref().unwrap_or("buffer"));
                }
            }
            if cancel {
                leave(srv, pane);
            } else if !name.ends_with("-no-clear")
                && let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut())
            {
                cm.apply("clear-selection", None);
            }
        }
        // The selection to a command (its argument, else copy-command), with
        // no buffer or clipboard; then the selection cleared, kept, or copy
        // mode left, as tmux's window_copy_cmd_pipe*.
        "pipe" | "pipe-no-clear" | "pipe-and-cancel" => {
            if let Some(text) = cm.selection_text() {
                let cmd = args.first().map_or_else(
                    || srv.cfg.copy_command.clone(),
                    |c| crate::cmds::expand_for(srv, ctx, Some(pane), c),
                );
                if !cmd.is_empty() {
                    crate::server::pipe_to_shell(srv, pane, cmd, text);
                }
            }
            if name == "pipe-and-cancel" {
                leave(srv, pane);
            } else if name == "pipe"
                && let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut())
            {
                cm.apply("clear-selection", None);
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
