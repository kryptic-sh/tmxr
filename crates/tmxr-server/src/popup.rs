//! `display-popup`: a command in a box over the panes, taking the client's
//! keys until it exits.
//!
//! A popup belongs to its client's overlay, so it goes when the overlay does
//! (closed, replaced, the client detached); dropping it ends its program. Its
//! PTY's events carry an id from the pane id space and are routed here when
//! no pane has that id.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use tmxr_term::{Emulator, Pty, encode_key, encode_paste};

use crate::model::{ClientId, PaneId};
use crate::output::OutputHandle;
use crate::overlay::{Overlay, OverlayAction};
use crate::server::Server;

pub struct Popup {
    pub id: PaneId,
    /// Which start of `id` the PTY is, as for a pane.
    pub spawn: u64,
    pub pty: Pty,
    pub emu: Emulator,
    pub output: OutputHandle,
    /// The box, borders included, in client cells.
    pub rect: Rect,
    pub border: bool,
    pub title: String,
    /// `-s`, `-S` and `-b`; `border` says whether there is one at all.
    pub look: crate::overlay::BoxLook,
    /// tmux's `-E` count: 0 keeps the popup open after its command exits
    /// (Escape or C-c then closes it), 1 closes it then, 2 only when it
    /// succeeded.
    pub close_on_exit: usize,
    /// `-k`: any key closes the popup once its command has exited.
    pub any_key: bool,
    pub exited: bool,
    /// The `extended-keys` option, for encoding keys as for a pane.
    pub extended_keys: tmxr_term::ExtendedKeys,
    /// A paste buffer being edited in this popup's program.
    pub edit: Option<BufferEdit>,
}

/// A paste buffer written to a file for an editor, as tmux's popup_editor:
/// read back into the buffer when the editor succeeds. The file goes with
/// it.
pub struct BufferEdit {
    pub name: String,
    pub path: std::path::PathBuf,
    /// The picker's command, run again once the editor is done, as tmux's
    /// buffer mode stays under the editor.
    pub reopen: Option<String>,
}

impl Drop for BufferEdit {
    fn drop(&mut self) {
        // The file is ours and only for this edit; one already gone is fine.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// What a new popup is: its box and look, how it closes, and the program
/// it runs where.
pub struct PopupSpec {
    pub rect: Rect,
    pub border: bool,
    pub title: String,
    pub look: crate::overlay::BoxLook,
    pub close_on_exit: usize,
    pub any_key: bool,
    pub session: crate::model::SessionId,
    pub argv: Vec<String>,
    pub cwd: std::path::PathBuf,
    pub env: Vec<(String, String)>,
    pub edit: Option<BufferEdit>,
}

impl Popup {
    /// Where the command's screen goes: inside the border, if any.
    pub fn inner(&self) -> Rect {
        Self::inner_of(self.rect, self.border)
    }

    /// The screen area of a popup whose box is `rect`.
    pub fn inner_of(rect: Rect, border: bool) -> Rect {
        if border {
            Rect::new(
                rect.x + 1,
                rect.y + 1,
                rect.width.saturating_sub(2),
                rect.height.saturating_sub(2),
            )
        } else {
            rect
        }
    }

    /// Keep the box inside a `cols` x `rows` client, as tmux does when the
    /// client is resized: moved in, and shrunk only if it no longer fits.
    pub fn fit(&mut self, cols: u16, rows: u16) {
        let w = self.rect.width.min(cols).max(1);
        let h = self.rect.height.min(rows).max(1);
        let x = self.rect.x.min(cols.saturating_sub(w));
        let y = self.rect.y.min(rows.saturating_sub(h));
        let before = self.inner();
        self.rect = Rect::new(x, y, w, h);
        self.refit(before);
    }

    /// Draw a border or not (`display-popup -b` / `-B` on an open popup),
    /// the command's screen resized to what is inside.
    pub fn set_border(&mut self, border: bool) {
        let before = self.inner();
        self.border = border;
        self.refit(before);
    }

    /// Resize the command's screen if its area changed from `before`.
    fn refit(&mut self, before: Rect) {
        let inner = self.inner();
        if (inner.width, inner.height) != (before.width, before.height) {
            let (iw, ih) = (inner.width.max(1), inner.height.max(1));
            self.emu.resize(ih, iw);
            if let Err(e) = self.pty.resize(ih, iw) {
                tracing::debug!(popup = self.id, error = %e, "pty resize failed");
            }
        }
    }

    /// A key, as tmux 3.6's `popup_key_cb`: Escape or C-c closes a popup
    /// whose command has exited, or one its command's exit would not close
    /// (no `-E`); with `-k` any key closes it once the command has exited.
    /// The rest go to the command while it runs.
    pub fn key(&mut self, ev: &KeyEvent) -> OverlayAction {
        let escape = (ev.code == KeyCode::Esc && ev.modifiers.is_empty())
            || (ev.code == KeyCode::Char('c') && ev.modifiers == KeyModifiers::CONTROL);
        if escape && (self.exited || self.close_on_exit == 0) {
            return OverlayAction::Close;
        }
        if self.exited {
            return if self.any_key {
                OverlayAction::Close
            } else {
                OverlayAction::Keep
            };
        }
        let bytes = encode_key(ev, self.emu.input_modes(), self.extended_keys);
        if !bytes.is_empty() {
            let _ = self.pty.write(&bytes);
        }
        OverlayAction::Keep
    }

    pub fn paste(&mut self, text: &str) {
        if !self.exited {
            let _ = self.pty.write(&encode_paste(text, self.emu.input_modes()));
        }
    }

    /// Whether the popup closes now its command exited with `code`.
    fn closes_on(&self, code: Option<u32>) -> bool {
        match self.close_on_exit {
            0 => false,
            1 => true,
            _ => code == Some(0),
        }
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        if !self.exited {
            let _ = self.pty.kill();
        }
    }
}

impl Server {
    /// The client showing popup `id` (start `spawn`), and the popup.
    fn popup_mut(&mut self, id: PaneId, spawn: u64) -> Option<(ClientId, &mut Popup)> {
        self.clients.values_mut().find_map(|c| {
            let cid = c.id;
            match c.att.as_mut()?.overlay.as_mut()? {
                Overlay::Popup(p) if p.id == id && p.spawn == spawn => Some((cid, &mut **p)),
                _ => None,
            }
        })
    }

    /// The popup's program printed: feed its screen.
    pub fn popup_output(&mut self, id: PaneId, spawn: u64) {
        let Some((client, p)) = self.popup_mut(id, spawn) else {
            return;
        };
        let bytes = p.output.0.take();
        p.emu.set_extended_keys(p.extended_keys);
        let replies = p.emu.process(&bytes);
        if !replies.is_empty() {
            let _ = p.pty.write(&replies);
        }
        self.mark_client_dirty(client);
    }

    /// The popup's program exited: close the popup, or keep it to be read;
    /// a buffer it edited takes the file back if it succeeded.
    pub fn popup_exited(&mut self, id: PaneId, spawn: u64, code: Option<u32>) {
        let Some((client, p)) = self.popup_mut(id, spawn) else {
            return;
        };
        p.exited = true;
        let edit = p.edit.take();
        if p.closes_on(code)
            && let Some(att) = self.clients.get_mut(&client).and_then(|c| c.att.as_mut())
        {
            att.overlay = None;
        }
        if let Some(edit) = edit {
            if code == Some(0) {
                self.finish_buffer_edit(&edit);
            }
            if let Some(cmd) = &edit.reopen {
                self.run_bind(client, cmd, None);
            }
        }
        self.mark_client_dirty(client);
    }

    /// Open a popup on `client` running `spec`'s program.
    pub fn open_popup(&mut self, client: ClientId, spec: PopupSpec) -> Result<(), String> {
        let inner = Popup::inner_of(spec.rect, spec.border);
        let id = self.next_pane;
        let started = self.start_pty(
            id,
            spec.session,
            &spec.argv,
            spec.cwd,
            &spec.env,
            inner.width.max(1),
            inner.height.max(1),
        )?;
        let popup = Popup {
            id,
            spawn: started.spawn,
            pty: started.pty,
            emu: self.new_emulator(inner.height.max(1), inner.width.max(1), 0),
            output: started.output,
            rect: spec.rect,
            border: spec.border,
            title: spec.title,
            look: spec.look,
            close_on_exit: spec.close_on_exit,
            any_key: spec.any_key,
            exited: false,
            extended_keys: self.extended_keys(),
            edit: spec.edit,
        };
        if let Some(att) = self.clients.get_mut(&client).and_then(|c| c.att.as_mut()) {
            att.overlay = Some(Overlay::Popup(Box::new(popup)));
        }
        self.mark_client_dirty(client);
        Ok(())
    }

    /// `choose-buffer`'s `e`, as tmux's window_buffer_start_edit and
    /// popup_editor: buffer `name` written to a file, which `editor` opens
    /// in a popup nine tenths of the client's size, closing when it exits.
    pub fn edit_buffer(
        &mut self,
        client: ClientId,
        name: &str,
        reopen: Option<String>,
    ) -> Result<(), String> {
        static EDITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let data = self
            .buffers
            .iter()
            .find(|b| b.name == name)
            .map(|b| b.data.clone())
            .ok_or_else(|| format!("no buffer {name}"))?;
        let att = self
            .clients
            .get(&client)
            .and_then(|c| c.att.as_ref())
            .ok_or("no current client")?;
        let (cols, rows, session) = (att.cols, att.rows, att.session);
        let n = EDITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("tmxr.{}.{n}", std::process::id()));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, data.as_bytes()))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let edit = BufferEdit {
            name: name.to_owned(),
            path,
            reopen,
        };
        let (w, h) = (cols * 9 / 10, rows * 9 / 10);
        let lines = self.client_window_options(client).popup_border_lines;
        let spec = PopupSpec {
            rect: Rect::new(cols / 2 - w / 2, rows / 2 - h / 2, w, h),
            border: crate::overlay::Lines::of(lines).is_some(),
            title: String::new(),
            look: crate::overlay::BoxLook {
                lines: crate::overlay::Lines::of(lines),
                ..crate::overlay::BoxLook::default()
            },
            close_on_exit: 1,
            any_key: false,
            session,
            argv: vec![format!("{} {}", self.cfg.editor, edit.path.display())],
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            edit: Some(edit),
        };
        self.open_popup(client, spec)
    }

    /// An edited buffer's file read back into it, as tmux's
    /// window_buffer_finish_edit: an empty file changes nothing, and a
    /// newline the editor added at the end goes if the buffer had none.
    fn finish_buffer_edit(&mut self, edit: &BufferEdit) {
        let Ok(bytes) = std::fs::read(&edit.path) else {
            return;
        };
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        let Some(buffer) = self.buffers.iter_mut().find(|b| b.name == edit.name) else {
            return;
        };
        if !buffer.data.is_empty() && !buffer.data.ends_with('\n') && text.ends_with('\n') {
            text.pop();
        }
        if !text.is_empty() {
            buffer.data = text;
        }
    }
}
