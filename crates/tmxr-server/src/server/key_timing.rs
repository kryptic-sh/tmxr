//! `escape-time` and `assume-paste-time`: when a key is held, combined or
//! taken as pasted text, as tmux decides it.

use std::time::{Duration, Instant};

use ttyr::event::{Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::Server;
use super::input::not_typed;
use crate::model::{ClientId, SessionId};

impl Server {
    /// Whether a key `since` the last one is pasted text, as tmux's
    /// server_client_is_assume_paste: the first key within
    /// `assume-paste-time` of the one before starts assuming a paste and is
    /// still a key, each one after it is pasted, and a slower key ends it.
    pub(super) fn assume_paste(&mut self, session: SessionId, since: Option<Duration>) -> bool {
        let t = self.session_options(session).assume_paste_time;
        let Some(s) = self.sessions.get_mut(&session) else {
            return false;
        };
        if t == 0 {
            return false;
        }
        if since.is_some_and(|d| d < Duration::from_millis(t)) {
            std::mem::replace(&mut s.pasting, true)
        } else {
            s.pasting = false;
            false
        }
    }

    /// `escape-time`, as tmux's: a lone Escape is held that long, and a key
    /// pressed before it is up goes as Meta and that key; anything else
    /// sends the Escape first. Returns the event to handle, none while one
    /// is held. A key's release, which the kitty protocol and Windows
    /// send, and a modifier key itself, which kitty's flag 8 sends, change
    /// nothing; but the held Escape's own release, which a Windows console
    /// reports before the next key is pressed, waits with it: dropped
    /// where the Escape makes the next key Meta, sent after it where it
    /// goes alone, so a program that asked for releases sees it after the
    /// press.
    pub(super) fn escape_time(&mut self, id: ClientId, ev: TermEvent) -> Option<TermEvent> {
        let wait = Duration::from_millis(self.cfg.escape_time);
        let att = self.clients.get_mut(&id)?.att.as_mut()?;
        let press = match &ev {
            TermEvent::Key(k)
                if k.kind == KeyEventKind::Release
                    && k.key == KeyCode::Esc
                    && att.pending_escape.is_some() =>
            {
                att.pending_escape_release = Some(k.clone());
                return None;
            }
            TermEvent::Key(k) if not_typed(k) => return Some(ev),
            TermEvent::Key(k) => Some(k.clone()),
            _ => None,
        };
        let now = Instant::now();
        if let Some(deadline) = att.pending_escape.take() {
            let release = att.pending_escape_release.take();
            match press {
                Some(mut k) if now < deadline && !k.modifiers.contains(KeyModifiers::ALT) => {
                    k.modifiers |= KeyModifiers::ALT;
                    return Some(TermEvent::Key(k));
                }
                _ => self.send_escape(id, release),
            }
        }
        let lone = press.is_some_and(|k| k.key == KeyCode::Esc && k.modifiers.is_empty());
        if lone && !wait.is_zero() {
            let att = self.clients.get_mut(&id)?.att.as_mut()?;
            att.pending_escape = Some(now + wait);
            return None;
        }
        Some(ev)
    }

    /// A held Escape, gone alone: its press, then its `release` where the
    /// terminal reported one meanwhile.
    fn send_escape(&mut self, id: ClientId, release: Option<KeyEvent>) {
        self.client_key(id, escape_key(), false);
        if let Some(release) = release {
            self.client_release(id, &release);
        }
    }

    /// Escapes whose `escape-time` is up, sent as Escape.
    pub(super) fn flush_escapes(&mut self) {
        let now = Instant::now();
        let due: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| {
                c.att
                    .as_ref()
                    .and_then(|a| a.pending_escape)
                    .is_some_and(|d| d <= now)
            })
            .map(|c| c.id)
            .collect();
        for id in due {
            let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
                continue;
            };
            att.pending_escape = None;
            let release = att.pending_escape_release.take();
            self.send_escape(id, release);
        }
    }

    /// The soonest a held Escape is due, for the server loop to wake.
    pub(super) fn next_escape(&self) -> Option<Instant> {
        self.clients
            .values()
            .filter_map(|c| c.att.as_ref()?.pending_escape)
            .min()
    }
}

/// An Escape key press.
fn escape_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
}
