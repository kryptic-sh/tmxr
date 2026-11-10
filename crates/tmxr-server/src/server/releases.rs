//! Key releases, and the modifier keys a terminal under kitty flag 8
//! reports: key events that are not a key typed, so they run no bind, end
//! no key table and are no escape-time or paste key. A release goes to the
//! program its press reached, and nowhere when the press ran a bind or
//! went to tmxr itself.
//!
//! A client's terminal reports releases while it is asked for kitty flag
//! 2, which it is while its keys go to a program that pushed it
//! (`crate::kitty::flags`); and a Windows console's key records (from the
//! console, or Windows Terminal's win32-input-mode) come with their
//! releases whatever was asked, `KeyEncoding::Windows` on each key. A
//! press that reaches a program then is held until its release comes. A
//! press from a terminal that reports none (a legacy or xterm-tier one, or
//! a kitty one not asked for flag 2) is released at once, so a program that asked for releases never sees a key
//! stuck down; and when the terminal stops reporting them, every key still
//! held is released. A key `send-keys` sends has no release.

use tmxr_term::encode::KITTY_REPORT_EVENTS;
use ttyr::event::{KeyEncoding, KeyEvent, KeyEventKind, KeyEventState};

use super::Server;
use crate::kitty::KeyTarget;
use crate::model::{ClientId, PaneId};
use crate::overlay::Overlay;

/// A key a client's terminal pressed and has not yet released, and the
/// programs it reached: its press's, and any its repeats reached after
/// focus moved.
#[derive(Debug, Clone)]
pub struct Held {
    press: KeyEvent,
    to: Vec<Reached>,
}

/// The programs a key reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reached {
    /// These panes: the one keys went to and its synchronized siblings.
    Panes(Vec<PaneId>),
    /// The popup with this id.
    Popup(PaneId),
}

/// Whether `a` and `b` are events of one key: the same key code, both on
/// the keypad or neither (the keypad's Enter is `KeyCode::Enter` with
/// [`KeyEventState::KEYPAD`]).
fn same_key(a: &KeyEvent, b: &KeyEvent) -> bool {
    a.key == b.key
        && a.state.contains(KeyEventState::KEYPAD) == b.state.contains(KeyEventState::KEYPAD)
}

impl Server {
    /// Whether client `id`'s terminal reports key releases now: it speaks
    /// the kitty protocol and was last told to ask for flag 2.
    pub(super) fn reports_releases(&self, id: ClientId) -> bool {
        crate::kitty::speaks(self, id)
            && self
                .clients
                .get(&id)
                .and_then(|c| c.att.as_ref())
                .and_then(|a| a.keyboard_flags)
                .is_some_and(|f| f & KITTY_REPORT_EVENTS != 0)
    }

    /// Whether the release of key `ev` from client `id`'s terminal will
    /// come: it is a Windows console key record, whose release comes
    /// whatever was asked, or the terminal reports releases now
    /// ([`Server::reports_releases`]).
    pub(super) fn release_comes(&self, id: ClientId, ev: &KeyEvent) -> bool {
        ev.encoding == KeyEncoding::Windows || self.reports_releases(id)
    }

    /// Key `ev` from client `id`'s terminal reached `to`: held until its
    /// release comes ([`Server::release_comes`]), or else released there
    /// at once. A repeat of a held key that reached another program adds
    /// it, so the release reaches every program that saw it.
    pub(super) fn key_reached(&mut self, id: ClientId, ev: &KeyEvent, to: Reached) {
        if !self.release_comes(id, ev) {
            self.key_to(id, &to, &ev.clone().with_kind(KeyEventKind::Release));
            return;
        }
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        match att.held.iter_mut().find(|h| same_key(&h.press, ev)) {
            Some(held) if !held.to.contains(&to) => held.to.push(to),
            Some(_) => {}
            None => att.held.push(Held {
                press: ev.clone(),
                to: vec![to],
            }),
        }
    }

    /// A press from client `id`'s terminal of a key it still holds: its
    /// release never came (it was pressed and released while the terminal
    /// was not yet reporting releases, or another key matched it), so the
    /// held key is released before the new press goes anywhere.
    pub(super) fn pressed_again(&mut self, id: ClientId, ev: &KeyEvent) {
        if ev.kind != KeyEventKind::Press {
            return;
        }
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        let Some(i) = att.held.iter().position(|h| same_key(&h.press, ev)) else {
            return;
        };
        let held = att.held.remove(i);
        let release = held.press.with_kind(KeyEventKind::Release);
        for to in &held.to {
            self.key_to(id, to, &release);
        }
    }

    /// A release from client `id`'s terminal: to the programs its press
    /// reached, if it reached any.
    pub(super) fn client_release(&mut self, id: ClientId, ev: &KeyEvent) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        let Some(i) = att.held.iter().position(|h| same_key(&h.press, ev)) else {
            return;
        };
        let held = att.held.remove(i);
        for to in &held.to {
            self.key_to(id, to, ev);
        }
    }

    /// A modifier key itself, pressed or repeating (kitty flag 8): to the
    /// program keys go to now, if any, and no further.
    pub(super) fn client_modifier_key(&mut self, id: ClientId, ev: &KeyEvent) {
        match crate::kitty::keys_to(self, id) {
            Some(KeyTarget::Pane(pane)) => self.client_key_to_pane(id, pane, ev, false),
            Some(KeyTarget::Popup(popup)) => {
                self.key_to(id, &Reached::Popup(popup), ev);
                self.key_reached(id, ev, Reached::Popup(popup));
            }
            None => {}
        }
    }

    /// Release every key client `id`'s terminal holds, as its terminal
    /// will report no more releases (it stopped being asked for them, or
    /// it is going).
    pub(crate) fn release_held(&mut self, id: ClientId) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        for held in std::mem::take(&mut att.held) {
            let release = held.press.with_kind(KeyEventKind::Release);
            for to in &held.to {
                self.key_to(id, to, &release);
            }
        }
    }

    /// Send key event `ev` to what `to` names, if it is still there: each
    /// pane, or client `id`'s popup while its program runs.
    fn key_to(&mut self, id: ClientId, to: &Reached, ev: &KeyEvent) {
        match to {
            Reached::Panes(panes) => {
                for &pane in panes {
                    self.write_key(pane, ev);
                }
            }
            Reached::Popup(popup) => {
                if let Some(Overlay::Popup(p)) = self
                    .clients
                    .get_mut(&id)
                    .and_then(|c| c.att.as_mut())
                    .and_then(|a| a.overlay.as_mut())
                    && p.id == *popup
                    && !p.exited
                {
                    p.send_key(ev);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tmxr_term::encode::TMXR_KITTY_FLAGS;
    use ttyr::event::{Event, KeyCode, KeyModifiers, ModifierKeyCode};

    use super::*;
    use crate::server::testing::{attached, redraw, report, say_keyboard};

    fn press(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn held(srv: &Server) -> Vec<KeyCode> {
        let att = srv.clients[&1].att.as_ref().unwrap();
        att.held.iter().map(|h| h.press.key).collect()
    }

    /// Client 1 was last told to ask its kitty terminal for `flags`.
    fn told(srv: &mut Server, flags: u8) {
        srv.clients
            .get_mut(&1)
            .unwrap()
            .att
            .as_mut()
            .unwrap()
            .keyboard_flags = Some(flags);
    }

    #[test]
    fn a_press_is_held_only_while_the_terminal_reports_releases() {
        let (mut srv, _rx) = attached(true);
        let nowhere = || Reached::Panes(Vec::new());
        // A legacy terminal: released at once.
        srv.key_reached(1, &press('a'), nowhere());
        assert_eq!(held(&srv), Vec::new());
        // A kitty terminal not asked for flag 2: the same.
        say_keyboard(&mut srv, report(true, None));
        srv.key_reached(1, &press('a'), nowhere());
        assert_eq!(held(&srv), Vec::new());
        // Asked for it: held, once however often it repeats, until its
        // own release.
        told(&mut srv, TMXR_KITTY_FLAGS | KITTY_REPORT_EVENTS);
        srv.key_reached(1, &press('a'), nowhere());
        let repeat = press('a').with_kind(KeyEventKind::Repeat);
        srv.key_reached(1, &repeat, nowhere());
        srv.key_reached(1, &press('b'), nowhere());
        assert_eq!(held(&srv), vec![KeyCode::Char('a'), KeyCode::Char('b')]);
        let release = |c| press(c).with_kind(KeyEventKind::Release);
        srv.client_release(1, &release('c'));
        srv.client_release(1, &release('a'));
        assert_eq!(held(&srv), vec![KeyCode::Char('b')]);
        // Asked for flag 2 no more (no program wants it): all released.
        redraw(&mut srv);
        assert_eq!(held(&srv), Vec::new());
    }

    /// A release or a modifier key's press from the terminal is no key
    /// typed: a held key table stays held, an Escape waiting for
    /// `escape-time` keeps waiting, and a held key's release takes it.
    #[test]
    fn a_release_or_a_modifier_key_is_no_key_typed() {
        let (mut srv, _rx) = attached(true);
        say_keyboard(&mut srv, report(true, None));
        told(&mut srv, TMXR_KITTY_FLAGS | KITTY_REPORT_EVENTS);
        srv.key_reached(1, &press('a'), Reached::Panes(Vec::new()));
        let soon = std::time::Instant::now() + std::time::Duration::from_secs(60);
        {
            let att = srv.clients.get_mut(&1).unwrap().att.as_mut().unwrap();
            att.table = "prefix".into();
            att.pending_escape = Some(soon);
        }
        let shift = KeyEvent::new(
            KeyCode::Modifier(ModifierKeyCode::LeftShift),
            KeyModifiers::SHIFT,
        );
        srv.inputs(
            1,
            vec![
                Event::Key(press('a').with_kind(KeyEventKind::Release)),
                Event::Key(press('z').with_kind(KeyEventKind::Release)),
                Event::Key(shift.clone()),
                Event::Key(shift.with_kind(KeyEventKind::Release)),
            ],
        );
        let att = srv.clients[&1].att.as_ref().unwrap();
        assert_eq!(att.table, "prefix");
        assert_eq!(att.pending_escape, Some(soon));
        assert_eq!(held(&srv), Vec::new());
        // The waiting Escape's own release waits with it, and goes after
        // it once its time is up, the Escape a key the held key table
        // takes.
        let release =
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE).with_kind(KeyEventKind::Release);
        srv.inputs(1, vec![Event::Key(release.clone())]);
        let att = srv.clients[&1].att.as_ref().unwrap();
        assert_eq!(att.pending_escape, Some(soon));
        assert_eq!(att.pending_escape_release, Some(release));
        escape_pending(&mut srv, std::time::Instant::now());
        srv.flush_escapes();
        let att = srv.clients[&1].att.as_ref().unwrap();
        assert_eq!(
            (att.pending_escape, &att.pending_escape_release),
            (None, &None)
        );
        assert_eq!(att.table, "root");
    }

    #[test]
    fn a_repeat_that_reaches_another_program_adds_it() {
        let (mut srv, _rx) = attached(true);
        say_keyboard(&mut srv, report(true, None));
        told(&mut srv, TMXR_KITTY_FLAGS | KITTY_REPORT_EVENTS);
        let repeat = press('a').with_kind(KeyEventKind::Repeat);
        srv.key_reached(1, &press('a'), Reached::Panes(vec![1]));
        srv.key_reached(1, &repeat, Reached::Panes(vec![1]));
        srv.key_reached(1, &repeat, Reached::Panes(vec![2, 3]));
        srv.key_reached(1, &repeat, Reached::Panes(vec![2, 3]));
        let att = srv.clients[&1].att.as_ref().unwrap();
        assert_eq!(
            att.held[0].to,
            vec![Reached::Panes(vec![1]), Reached::Panes(vec![2, 3])]
        );
    }

    /// A press of a key still held (its release lost while flag 2 was
    /// being switched on) releases the held one first; a repeat does not.
    #[test]
    fn a_new_press_of_a_held_key_releases_it_first() {
        let (mut srv, _rx) = attached(true);
        say_keyboard(&mut srv, report(true, None));
        told(&mut srv, TMXR_KITTY_FLAGS | KITTY_REPORT_EVENTS);
        srv.key_reached(1, &press('a'), Reached::Panes(vec![1]));
        srv.key_reached(1, &press('b'), Reached::Panes(vec![1]));
        let repeat = press('a').with_kind(KeyEventKind::Repeat);
        srv.inputs(1, vec![Event::Key(repeat)]);
        assert_eq!(held(&srv), vec![KeyCode::Char('a'), KeyCode::Char('b')]);
        // Session 0 does not exist, so the new press reaches nothing and
        // is not held again: only the release of the old one went out.
        srv.inputs(1, vec![Event::Key(press('a'))]);
        assert_eq!(held(&srv), vec![KeyCode::Char('b')]);
    }

    /// An Escape's release from any client, asked for releases or not,
    /// leaves `escape-time` alone: it waits with the Escape.
    #[test]
    fn an_escape_release_from_a_client_not_asked_for_releases_waits() {
        let (mut srv, _rx) = attached(true);
        let soon = std::time::Instant::now() + std::time::Duration::from_secs(60);
        srv.clients
            .get_mut(&1)
            .unwrap()
            .att
            .as_mut()
            .unwrap()
            .pending_escape = Some(soon);
        let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        srv.inputs(1, vec![Event::Key(escape.with_kind(KeyEventKind::Release))]);
        let att = srv.clients[&1].att.as_ref().unwrap();
        assert_eq!(att.pending_escape, Some(soon));
    }

    /// A Windows console's key record comes with its release whatever the
    /// client was asked for, so its press is held for that release, not
    /// released at once, from a client of any tier; another key from the
    /// same client is released at once.
    #[test]
    fn a_console_key_is_held_for_its_own_release() {
        let (mut srv, _rx) = attached(true);
        let console = |c| press(c).with_encoding(KeyEncoding::Windows);
        srv.key_reached(1, &console('a'), Reached::Panes(Vec::new()));
        srv.key_reached(1, &press('b'), Reached::Panes(Vec::new()));
        assert_eq!(held(&srv), vec![KeyCode::Char('a')]);
        srv.client_release(1, &console('a').with_kind(KeyEventKind::Release));
        assert_eq!(held(&srv), Vec::new());
    }

    /// Client 1 has an Escape waiting for `escape-time` until `deadline`.
    fn escape_pending(srv: &mut Server, deadline: std::time::Instant) {
        let att = srv.clients.get_mut(&1).unwrap().att.as_mut().unwrap();
        att.pending_escape = Some(deadline);
    }

    fn console_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE).with_encoding(KeyEncoding::Windows)
    }

    /// A console key record's Escape is released before the next key is
    /// pressed, as a user types Escape then x: the release waits with the
    /// Escape, and x within `escape-time` is still M-x, the Escape and its
    /// release going nowhere.
    #[test]
    fn an_escape_released_before_the_next_key_still_makes_it_meta() {
        let (mut srv, _rx) = attached(true);
        escape_pending(
            &mut srv,
            std::time::Instant::now() + std::time::Duration::from_secs(60),
        );
        let release = console_key(KeyCode::Esc).with_kind(KeyEventKind::Release);
        assert_eq!(srv.escape_time(1, Event::Key(release)), None);
        let x = console_key(KeyCode::Char('x'));
        let mut meta_x = x.clone();
        meta_x.modifiers = KeyModifiers::ALT;
        assert_eq!(srv.escape_time(1, Event::Key(x)), Some(Event::Key(meta_x)));
        let att = srv.clients[&1].att.as_ref().unwrap();
        assert_eq!(att.pending_escape, None);
        assert_eq!(att.pending_escape_release, None);
    }

    /// An Escape whose `escape-time` runs out with its release already in
    /// goes as Escape, its release after it.
    #[test]
    fn an_escape_released_and_timed_out_goes_with_its_release() {
        let (mut srv, _rx) = attached(true);
        escape_pending(&mut srv, std::time::Instant::now());
        let release = console_key(KeyCode::Esc).with_kind(KeyEventKind::Release);
        assert_eq!(srv.escape_time(1, Event::Key(release.clone())), None);
        assert_eq!(
            srv.clients[&1].att.as_ref().unwrap().pending_escape_release,
            Some(release)
        );
        srv.flush_escapes();
        let att = srv.clients[&1].att.as_ref().unwrap();
        assert_eq!(att.pending_escape, None);
        assert_eq!(att.pending_escape_release, None);
    }

    /// Enter and the keypad's Enter are one `KeyCode`, the keypad in the
    /// state: each is held, and released, as its own key.
    #[test]
    fn a_keypad_key_and_its_main_key_are_held_apart() {
        let (mut srv, _rx) = attached(true);
        say_keyboard(&mut srv, report(true, None));
        told(&mut srv, TMXR_KITTY_FLAGS | KITTY_REPORT_EVENTS);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let kp_enter = enter.clone().with_state(KeyEventState::KEYPAD);
        srv.key_reached(1, &enter, Reached::Panes(vec![1]));
        srv.key_reached(1, &kp_enter, Reached::Panes(vec![2]));
        // A press of the keypad's Enter releases nothing held.
        srv.pressed_again(1, &kp_enter.clone());
        srv.key_reached(1, &kp_enter, Reached::Panes(vec![2]));
        let att = srv.clients[&1].att.as_ref().unwrap();
        let keys: Vec<_> = att.held.iter().map(|h| h.to.clone()).collect();
        assert_eq!(
            keys,
            vec![vec![Reached::Panes(vec![1])], vec![Reached::Panes(vec![2])]]
        );
        // The keypad's release takes its own, leaving Enter held.
        srv.client_release(1, &kp_enter.with_kind(KeyEventKind::Release));
        let att = srv.clients[&1].att.as_ref().unwrap();
        assert_eq!(att.held.len(), 1);
        assert_eq!(att.held[0].to, vec![Reached::Panes(vec![1])]);
    }
}
