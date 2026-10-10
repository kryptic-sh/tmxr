//! `copy-command-line` (`prefix y`): copy the shell's command line, as
//! tmux-yank's `copy_line` does, but timed by the pane rather than by
//! sleeps.
//!
//! A shell that marks its prompts (OSC 133) has said where its input starts,
//! and the input runs from there to the end of the cursor's line. Otherwise
//! the shell is sent its beginning-of-line key; once the pane has gone quiet
//! the cursor marks the start. Then its end-of-line key, and the cursor marks
//! the end. Either way the text is copied as a copy-mode copy is.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::model::PaneId;
use crate::server::Server;

/// How long a pane must stay quiet after a key before the cursor is read.
const SETTLE: Duration = Duration::from_millis(60);
/// The longest a step waits, however busy the pane.
const STEP_LIMIT: Duration = Duration::from_secs(1);

/// A copy under way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyLine {
    pane: PaneId,
    /// Where the line starts, once the first key has settled.
    start: Option<(u16, u16)>,
    /// The cursor is read once the pane has been quiet until here.
    quiet_until: Instant,
    /// Or at the latest here.
    give_up: Instant,
    /// The keys for this shell: beginning and end of line.
    keys: (KeyEvent, KeyEvent),
}

impl CopyLine {
    /// When this step reads the cursor.
    pub fn due(&self) -> Instant {
        self.quiet_until.min(self.give_up)
    }
}

/// The input after the shell's last prompt mark: from the mark to the end of
/// the cursor's line, wrapped rows included. `None` without a usable mark.
fn marked_input(emu: &tmxr_term::Emulator) -> Option<String> {
    let (row, col) = emu.input_start()?;
    let screen = emu.screen();
    let (rows, cols) = screen.size();
    let mut end = screen.cursor_position().0.max(row);
    while end + 1 < rows && screen.row_wrapped(end) {
        end += 1;
    }
    Some(
        screen
            .contents_between(row, col, end, cols)
            .trim_end()
            .to_owned(),
    )
}

/// The line-editing keys `program` takes: Home and End for the Windows
/// shells, whose `C-a` selects everything (PowerShell) or does nothing
/// (cmd), and tmux-yank's `C-a` / `C-e` for the rest.
fn keys_for(program: &str) -> (KeyEvent, KeyEvent) {
    let name = program.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    if matches!(name, "pwsh" | "powershell" | "cmd") {
        (
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        )
    } else {
        (
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
        )
    }
}

impl Server {
    /// Start copying `pane`'s command line; one at a time, a new one
    /// replacing any under way.
    pub fn copy_command_line(&mut self, pane: PaneId) -> Result<(), String> {
        if let Some(text) = self.panes.get(&pane).and_then(|p| marked_input(&p.emu)) {
            self.copy_line = None;
            self.copy_text(text);
            return Ok(());
        }
        let program = crate::resurrect::foreground(self, pane).unwrap_or_default();
        let keys = keys_for(&program);
        let now = Instant::now();
        self.copy_line = Some(CopyLine {
            pane,
            start: None,
            quiet_until: now + SETTLE,
            give_up: now + STEP_LIMIT,
            keys,
        });
        self.key_to_one_pane(pane, &keys.0)
    }

    /// The pane printed: a copy waiting on it waits a little longer.
    pub fn copy_line_output(&mut self, pane: PaneId) {
        if let Some(c) = self.copy_line.as_mut().filter(|c| c.pane == pane) {
            c.quiet_until = Instant::now() + SETTLE;
        }
    }

    /// Take the next step of a copy whose pane has settled.
    pub fn advance_copy_line(&mut self) {
        let Some(mut c) = self.copy_line.filter(|c| c.due() <= Instant::now()) else {
            return;
        };
        let Some(cursor) = self
            .panes
            .get(&c.pane)
            .map(|p| p.emu.screen().cursor_position())
        else {
            self.copy_line = None;
            return;
        };
        match c.start {
            None => {
                let now = Instant::now();
                c.start = Some(cursor);
                c.quiet_until = now + SETTLE;
                c.give_up = now + STEP_LIMIT;
                self.copy_line = Some(c);
                if let Err(e) = self.key_to_one_pane(c.pane, &c.keys.1) {
                    self.log_message(format!("copy-command-line: {e}"));
                    self.copy_line = None;
                }
            }
            Some(start) => {
                self.copy_line = None;
                let text = self.panes.get(&c.pane).map(|p| {
                    p.emu
                        .screen()
                        .contents_between(start.0, start.1, cursor.0, cursor.1)
                });
                self.copy_text(text.unwrap_or_default().trim_end().to_owned());
            }
        }
    }

    /// Copy a command line as a copy-mode copy is: to the clipboard, a new
    /// buffer, and `copy-command` when set. Nothing for an empty line.
    fn copy_text(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        let pipe = self.cfg.copy_command.clone();
        if !pipe.is_empty() {
            crate::server::pipe_to_shell(self, pipe, text.clone());
        }
        self.set_clipboard(&text);
        self.add_buffer(text, None);
    }

    /// Send `key` to `pane` alone, whatever synchronize-panes says.
    fn key_to_one_pane(&mut self, pane: PaneId, key: &KeyEvent) -> Result<(), String> {
        let always = self.cfg.extended_keys == "always";
        let p = self.panes.get_mut(&pane).ok_or("no such pane")?;
        let bytes = tmxr_term::encode_key(key, p.emu.input_modes(), always);
        p.pty.write(&bytes).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prompt_mark_gives_the_input_without_keys() {
        let mut emu = tmxr_term::Emulator::new(4, 12, 100);
        assert_eq!(marked_input(&emu), None, "no mark");
        // A prompt, then input wrapping onto the next row, cursor moved back.
        emu.process(b"$ \x1b]133;B\x07echo wrapped-text\x1b[5D");
        assert_eq!(marked_input(&emu).as_deref(), Some("echo wrapped-text"));
        emu.process(b"\x1b]133;C\x07\r\n");
        assert_eq!(marked_input(&emu), None, "the command ran");
    }

    #[test]
    fn windows_shells_get_home_and_end() {
        for shell in ["pwsh", "PowerShell.exe", "cmd.exe", "CMD"] {
            assert_eq!(keys_for(shell).0.code, KeyCode::Home, "{shell}");
        }
        for shell in ["bash", "zsh", "fish", ""] {
            let (start, end) = keys_for(shell);
            assert_eq!(start.code, KeyCode::Char('a'), "{shell}");
            assert_eq!(end.code, KeyCode::Char('e'), "{shell}");
            assert_eq!(start.modifiers, KeyModifiers::CONTROL);
        }
    }
}
