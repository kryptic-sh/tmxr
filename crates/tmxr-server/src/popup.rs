//! `display-popup`: a command in a box over the panes, taking the client's
//! keys until it exits.
//!
//! A popup belongs to its client's overlay, so it goes when the overlay does
//! (closed, replaced, the client detached); dropping it ends its program. Its
//! PTY's events carry an id from the pane id space and are routed here when
//! no pane has that id.

use crossterm::event::KeyEvent;
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
    /// tmux's `-E` count: 0 keeps the popup open after its command exits
    /// (a key then closes it), 1 closes it then, 2 only when it succeeded.
    pub close_on_exit: usize,
    pub exited: bool,
    /// `extended-keys always`, for encoding keys as for a pane.
    pub extended_keys: bool,
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

    pub fn key(&mut self, ev: &KeyEvent) -> OverlayAction {
        if self.exited {
            return OverlayAction::Close;
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
        let replies = p.emu.process(&bytes);
        if !replies.is_empty() {
            let _ = p.pty.write(&replies);
        }
        self.mark_client_dirty(client);
    }

    /// The popup's program exited: close the popup, or keep it to be read.
    pub fn popup_exited(&mut self, id: PaneId, spawn: u64, code: Option<u32>) {
        let Some((client, p)) = self.popup_mut(id, spawn) else {
            return;
        };
        p.exited = true;
        if p.closes_on(code)
            && let Some(att) = self.clients.get_mut(&client).and_then(|c| c.att.as_mut())
        {
            att.overlay = None;
        }
        self.mark_client_dirty(client);
    }
}
