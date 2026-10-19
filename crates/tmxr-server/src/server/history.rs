//! The prompts' history kept in `history-file`, as tmux's: read when the
//! server starts, written when it exits, one `type:entry` line each.

use std::path::PathBuf;

use tracing::warn;

use super::Server;

/// The prompt types a history file holds, in the order tmux writes them
/// (`prompt_type_strings`).
const TYPES: [&str; 4] = ["command", "search", "target", "window-target"];

impl Server {
    /// Where `history-file` points, as tmux's
    /// status_prompt_find_history_file: an absolute path, or one under the
    /// home directory by `~/`; anything else, or empty, is no file.
    fn history_path(&self) -> Option<PathBuf> {
        let file = self.cfg.history_file.as_str();
        if let Some(rest) = file.strip_prefix("~/") {
            return Some(crate::util::home_dir().join(rest));
        }
        let path = PathBuf::from(file);
        path.is_absolute().then_some(path)
    }

    /// Read the history file into the prompts' history, as tmux's
    /// status_prompt_load_history: a line whose type tmux does not know is
    /// a command, whole. A missing file is no history yet.
    pub fn load_prompt_history(&mut self) {
        let Some(path) = self.history_path() else {
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                warn!("history-file {}: {e}", path.display());
                return;
            }
        };
        for line in text.lines() {
            let (kind, entry) = match line.split_once(':') {
                Some((kind, entry)) if TYPES.contains(&kind) => (kind, entry),
                _ => ("command", line),
            };
            self.remember_prompt(kind, entry.to_owned());
        }
    }

    /// Write the prompts' history to the history file, as tmux's
    /// status_prompt_save_history.
    pub fn save_prompt_history(&self) {
        let Some(path) = self.history_path() else {
            return;
        };
        let mut text = String::new();
        for kind in TYPES {
            for entry in self.prompt_history.get(kind).into_iter().flatten() {
                text.push_str(kind);
                text.push(':');
                text.push_str(entry);
                text.push('\n');
            }
        }
        if let Err(e) = std::fs::write(&path, text) {
            warn!("history-file {}: {e}", path.display());
        }
    }
}
