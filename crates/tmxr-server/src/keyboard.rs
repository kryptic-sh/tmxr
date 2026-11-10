//! Which keyboard protocol each attached client's terminal is asked for:
//! its tier, decided here from what the terminal said of itself
//! (`ClientMsg::Keyboard`), its `TERM` and the options, and kept with the
//! client until one of them changes.
//!
//! The kitty tier's flags are [`crate::kitty`]'s. The xterm tier is tmux's
//! `extkeys` ([`crate::features`]): modifyOtherKeys level 2 for the whole
//! attach while `extended-keys` is not `off` ([`extended_keys`]), told to
//! the client as the mouse mode is, when it changes.

use tmxr_command::BindKey;
use tmxr_config::Array;
use tmxr_proto::KeyboardReport;
use tmxr_term::ExtendedKeys;
use ttyr::event::{KeyCode, KeyModifiers};

use crate::model::ClientId;
use crate::server::Server;

/// A client terminal's keyboard tier: how it is asked to report keys
/// beyond the bytes a VT sends. Each tier sends every key the one before it
/// does, so they order from least to most.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Keyboard {
    /// Bytes as a VT sends them: ASCII control bytes and escape sequences,
    /// typed text as UTF-8.
    #[default]
    Legacy,
    /// xterm's modifyOtherKeys: tmux's `extkeys` feature.
    Xterm,
    /// The kitty keyboard protocol.
    Kitty,
}

/// The tier of a terminal that said `report` and has `TERM` `term`: the
/// kitty protocol where it speaks it and `kitty-keys` (`kitty_keys`) is on,
/// as it wins where a terminal has both; else modifyOtherKeys where the
/// client can ask for it and the terminal has `extkeys`, by its name or by
/// `terminal-features` (`features`); else legacy.
pub fn tier(report: &KeyboardReport, term: &str, kitty_keys: bool, features: &Array) -> Keyboard {
    if report.kitty && kitty_keys {
        return Keyboard::Kitty;
    }
    let extkeys = report.modify_other_keys
        && crate::features::extkeys(
            term,
            report.version.as_deref(),
            report.secondary,
            features.values(),
        );
    if extkeys {
        Keyboard::Xterm
    } else {
        Keyboard::Legacy
    }
}

/// The least tier whose terminal can send `key`, so a bind on it can fire
/// there:
///
/// - `Legacy`: what a VT's bytes carry. A character with Alt (`ESC` in
///   front) and Ctrl only where a C0 byte names it: a lower-case ASCII
///   letter but `i` and `m` (Tab's and Enter's bytes), and C-Space and
///   C-\ C-] C-^ C-_; no Shift kept apart (C-S-h). Enter, Tab, BTab,
///   BSpace and Escape with Alt alone. The cursor and editing keys and F1
///   to F12 with any of Ctrl, Alt and Shift, in xterm's `CSI 1 ; mods`
///   forms. xterm's Meta modifier (bit 8) in those forms is `M-`, as tmux
///   reads it ([`tmxr_command::keys::xterm_meta_as_alt`]), so it fires the
///   same binds as Alt.
/// - `Xterm`: modifyOtherKeys's `CSI 27 ; mods ; key ~` adds Ctrl, Alt (or
///   xterm's Meta, `M-` again) and Shift on every other character and on
///   Enter, Tab, BSpace and Escape (C-S-h, C-i apart from Tab, C-4 apart
///   from C-\).
/// - `Kitty`: the rest. Super, Hyper and the kitty protocol's own Meta
///   (`Meta-`), the function keys above F12, the keypad apart from the main
///   keys (tmxr asks no terminal for application keypad mode), the lock,
///   media and modifier keys. tmux's C-H (Ctrl with an upper-case letter
///   and no Shift) and `S-Tab` are here too though no terminal tmxr reads
///   sends them: ttyr reads `CSI 27 ; 5 ; 72 ~` (Ctrl+h with Caps Lock on)
///   as Ctrl+h with Caps Lock, as kitty reports it, and Shift+Tab is BTab.
///
/// A mouse key is sent by every tier.
pub fn needs(key: &BindKey) -> Keyboard {
    let BindKey::Key(key) = key else {
        return Keyboard::Legacy;
    };
    let mods = key.mods;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let shift = mods.contains(KeyModifiers::SHIFT);
    let kitty_only = KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META;
    if key.keypad || mods.intersects(kitty_only) {
        return Keyboard::Kitty;
    }
    let legacy = match key.code {
        KeyCode::Char(c) if ctrl && c.is_uppercase() && !shift => return Keyboard::Kitty,
        KeyCode::Char(c) => {
            !shift
                && (!ctrl
                    || (c.is_ascii_lowercase() && !matches!(c, 'i' | 'm'))
                    || matches!(c, ' ' | '\\' | ']' | '^' | '_'))
        }
        KeyCode::Tab if shift => return Keyboard::Kitty,
        KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab | KeyCode::Backspace | KeyCode::Esc => {
            !ctrl && !shift
        }
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Home
        | KeyCode::End
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Insert
        | KeyCode::Delete
        | KeyCode::F(1..=12) => true,
        _ => return Keyboard::Kitty,
    };
    if legacy {
        Keyboard::Legacy
    } else {
        Keyboard::Xterm
    }
}

/// The comment `list-keys` ends a bind's line with where `client`'s tier
/// cannot send what `needs` says the key needs.
pub fn needs_comment(need: Keyboard, client: Keyboard) -> Option<&'static str> {
    if need <= client {
        return None;
    }
    match need {
        Keyboard::Kitty => Some("# needs kitty keys"),
        Keyboard::Xterm => Some("# needs extended keys"),
        Keyboard::Legacy => None,
    }
}

/// Whether client `id`'s terminal should have modifyOtherKeys on now: in
/// the xterm tier, while `extended-keys` is not `off`, as tmux's
/// `tty_update_features` sends `Eneks`.
pub fn extended_keys(srv: &Server, id: ClientId) -> bool {
    let xterm = srv
        .clients
        .get(&id)
        .and_then(|c| c.att.as_ref())
        .is_some_and(|att| att.keyboard == Keyboard::Xterm);
    xterm && srv.extended_keys() != ExtendedKeys::Off
}

impl Server {
    /// Decide client `id`'s tier afresh, from what its terminal last said
    /// and the options now.
    pub fn decide_keyboard(&mut self, id: ClientId) {
        let term = self
            .client_terminal(id)
            .map(|t| t.term.clone())
            .unwrap_or_default();
        let (kitty_keys, features) = (self.cfg.kitty_keys, &self.cfg.terminal_features);
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        att.keyboard = tier(&att.keyboard_report, &term, kitty_keys, features);
    }

    /// Decide every attached client's tier afresh, as `kitty-keys` or
    /// `terminal-features` changes; what a client is told follows with its
    /// next frame.
    pub fn decide_keyboards(&mut self) {
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        for id in ids {
            self.decide_keyboard(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::testing::{attached, redraw, report as told, say_keyboard};
    use tmxr_proto::ServerMsg;

    fn report(kitty: bool, version: Option<&str>) -> KeyboardReport {
        KeyboardReport {
            kitty,
            version: version.map(str::to_owned),
            secondary: None,
            modify_other_keys: true,
        }
    }

    fn need(name: &str) -> Keyboard {
        needs(&name.parse::<BindKey>().unwrap())
    }

    #[test]
    fn each_tier_sends_the_keys_its_terminal_can() {
        // Every default bind is sendable from a legacy terminal.
        let (tables, _) = crate::keys::KeyTables::from_config(&tmxr_config::defaults());
        for (table, key, _) in tables.list(None) {
            assert_eq!(needs(&key), Keyboard::Legacy, "{table} {key}");
        }
        for name in [
            "a",
            "A",
            "%",
            "M-a",
            "M-A",
            "M-!",
            "C-a",
            "C-h",
            "C-j",
            "C-z",
            "C-M-x",
            "C-Space",
            "C-\\",
            "C-]",
            "C-^",
            "C-_",
            "Enter",
            "M-Enter",
            "Tab",
            "BTab",
            "M-BTab",
            "BSpace",
            "M-BSpace",
            "Escape",
            "M-Escape",
            "Up",
            "C-Up",
            "C-M-S-Up",
            "S-Home",
            "M-PPage",
            "C-DC",
            "F1",
            "C-F12",
            "S-F5",
            "MouseDown1Pane",
            "C-WheelUpPane",
        ] {
            assert_eq!(need(name), Keyboard::Legacy, "{name}");
        }
        // Shift kept apart, Ctrl on a key whose byte is another's, and the
        // modified Enter / Tab / BSpace / Escape: modifyOtherKeys's.
        for name in [
            "C-S-h", "C-S-a", "C-i", "C-m", "C-[", "C-4", "C-2", "C-@", "C-1", "C-!", "C-é",
            "C-Enter", "S-Enter", "C-Tab", "C-BTab", "C-BSpace", "C-Escape", "C-M-S-h", "C-S-1",
        ] {
            assert_eq!(need(name), Keyboard::Xterm, "{name}");
        }
        // Super, Hyper, and the keys only the kitty protocol reports.
        for name in [
            "Super-a",
            "Hyper-a",
            "Meta-a",
            "Meta-Up",
            "Meta-F1",
            "C-Meta-x",
            "S-Tab",
            "C-Super-h",
            "Super-Up",
            "F13",
            "F35",
            "KP1",
            "KPEnter",
            "KPLeft",
            "KPBegin",
            "CapsLock",
            "PrintScreen",
            "Pause",
            "Menu",
            "MediaPlay",
            "LeftShift",
            "C-H",
            "C-A",
        ] {
            assert_eq!(need(name), Keyboard::Kitty, "{name}");
        }
    }

    /// The keys xterm's Meta sends (bit 8, read as `M-` outside the kitty
    /// protocol) need no more than the xterm tier; kitty's own Meta, under
    /// the protocol, needs kitty.
    #[test]
    fn xterm_meta_needs_the_xterm_tier_at_most_and_kittys_needs_kitty() {
        let need_of = |bytes: &[u8], kitty: bool| {
            let mut decoder = ttyr::event::Decoder::new();
            decoder.set_kitty_keyboard(kitty);
            decoder.feed(bytes);
            let Some(ttyr::event::Event::Key(ev)) = decoder.flush_event() else {
                panic!("{bytes:?}: not a key");
            };
            let ev = tmxr_command::keys::xterm_meta_as_alt(ev, kitty);
            needs(&BindKey::Key(tmxr_command::Key::from_event(&ev)))
        };
        for (bytes, kitty, want) in [
            // M-h, C-M-h, M-Enter, M-Up, M-F5.
            (&b"\x1b[27;9;104~"[..], false, Keyboard::Legacy),
            (b"\x1b[27;13;104~", false, Keyboard::Legacy),
            (b"\x1b[27;9;13~", false, Keyboard::Legacy),
            (b"\x1b[1;9A", false, Keyboard::Legacy),
            (b"\x1b[15;9~", false, Keyboard::Legacy),
            // C-M-S-h and C-M-Enter: modifyOtherKeys's.
            (b"\x1b[27;14;104~", false, Keyboard::Xterm),
            (b"\x1b[27;13;13~", false, Keyboard::Xterm),
            // Meta-x and Meta-Up.
            (b"\x1b[120;33u", true, Keyboard::Kitty),
            (b"\x1b[1;33A", true, Keyboard::Kitty),
        ] {
            assert_eq!(need_of(bytes, kitty), want, "{bytes:?}");
        }
    }

    #[test]
    fn a_bind_is_marked_only_for_a_tier_below_its_need() {
        use Keyboard::{Kitty, Legacy, Xterm};
        for (need, client, want) in [
            (Legacy, Legacy, None),
            (Legacy, Kitty, None),
            (Xterm, Legacy, Some("# needs extended keys")),
            (Xterm, Xterm, None),
            (Xterm, Kitty, None),
            (Kitty, Legacy, Some("# needs kitty keys")),
            (Kitty, Xterm, Some("# needs kitty keys")),
            (Kitty, Kitty, None),
        ] {
            assert_eq!(needs_comment(need, client), want, "{need:?} on {client:?}");
        }
    }

    /// `list-keys` run by an attached client itself (from a bind or the
    /// prompt) marks for that client's tier, as it changes.
    #[test]
    fn list_keys_marks_for_the_attached_client_that_runs_it() {
        let (mut srv, _rx) = attached(true);
        let ctx = crate::cmds::Ctx {
            client: Some(1),
            ..crate::cmds::Ctx::default()
        };
        let run = |srv: &mut Server, line: &str| crate::cmds::run_line(srv, &ctx, line).stdout;
        run(&mut srv, "bind -T t C-S-h display a");
        run(&mut srv, "bind -T t F13 display b");
        let marked = |srv: &mut Server| run(srv, "list-keys -T t");
        assert_eq!(
            marked(&mut srv),
            "bind-key -T t F13   display-message b # needs kitty keys\n\
             bind-key -T t C-S-h display-message a # needs extended keys\n"
        );
        say_keyboard(&mut srv, told(false, Some("XTerm(388)")));
        assert_eq!(
            marked(&mut srv),
            "bind-key -T t F13   display-message b # needs kitty keys\n\
             bind-key -T t C-S-h display-message a\n"
        );
        say_keyboard(&mut srv, told(true, None));
        assert_eq!(
            marked(&mut srv),
            "bind-key -T t F13   display-message b\n\
             bind-key -T t C-S-h display-message a\n"
        );
    }

    fn defaults() -> Array {
        tmxr_config::defaults().terminal_features
    }

    #[test]
    fn kitty_wins_over_an_xterm_name_while_kitty_keys_is_on() {
        let both = report(true, Some("XTerm(390)"));
        assert_eq!(tier(&both, "xterm", true, &defaults()), Keyboard::Kitty);
        // `kitty-keys off`: the terminal falls to the tier it has besides.
        assert_eq!(tier(&both, "xterm", false, &defaults()), Keyboard::Xterm);
        let kitty = report(true, Some("kitty(0.39.1)"));
        assert_eq!(
            tier(&kitty, "xterm-kitty", true, &defaults()),
            Keyboard::Kitty
        );
        assert_eq!(
            tier(&kitty, "xterm-kitty", false, &defaults()),
            Keyboard::Legacy
        );
    }

    #[test]
    fn an_xterm_name_or_terminal_features_gives_the_xterm_tier() {
        let xterm = report(false, Some("XTerm(390)"));
        assert_eq!(tier(&xterm, "xterm", true, &defaults()), Keyboard::Xterm);
        let unnamed = report(false, None);
        assert_eq!(tier(&unnamed, "foo", true, &defaults()), Keyboard::Legacy);
        let mut features = defaults();
        features.assign_split("foo*:extkeys", tmxr_config::TERMINAL_FEATURES_SEPARATORS);
        assert_eq!(
            tier(&unnamed, "foo-256color", true, &features),
            Keyboard::Xterm
        );
        assert_eq!(tier(&unnamed, "bar", true, &features), Keyboard::Legacy);
    }

    #[test]
    fn a_client_that_cannot_ask_for_modify_other_keys_stays_legacy() {
        let mut features = defaults();
        features.assign_split("*:extkeys", tmxr_config::TERMINAL_FEATURES_SEPARATORS);
        let windows = KeyboardReport {
            modify_other_keys: false,
            ..report(false, Some("XTerm(390)"))
        };
        assert_eq!(tier(&windows, "xterm", true, &features), Keyboard::Legacy);
    }

    /// Whether the server has told the client to turn modifyOtherKeys on
    /// or off so far, in order.
    fn extended_keys_told(rx: &std::sync::mpsc::Receiver<ServerMsg>) -> Vec<bool> {
        rx.try_iter()
            .filter_map(|msg| match msg {
                ServerMsg::ExtendedKeys(on) => Some(on),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_xterm_terminal_is_told_modify_other_keys_under_extended_keys() {
        let (mut srv, rx) = attached(true);
        srv.cfg.extended_keys = "on".into();
        redraw(&mut srv);
        // Nothing until it says, nor for a legacy terminal, nor for one
        // that speaks both protocols, which gets kitty's.
        for report in [told(false, None), told(true, Some("XTerm(390)"))] {
            say_keyboard(&mut srv, report.clone());
            redraw(&mut srv);
            assert_eq!(extended_keys_told(&rx), Vec::<bool>::new(), "{report:?}");
        }
        // XTerm: on at once, then as `extended-keys` goes off and back.
        say_keyboard(&mut srv, told(false, Some("XTerm(390)")));
        assert_eq!(extended_keys_told(&rx), [true]);
        redraw(&mut srv);
        assert_eq!(extended_keys_told(&rx), Vec::<bool>::new());
        srv.cfg.extended_keys = "off".into();
        redraw(&mut srv);
        assert_eq!(extended_keys_told(&rx), [false]);
        srv.cfg.extended_keys = "always".into();
        redraw(&mut srv);
        assert_eq!(extended_keys_told(&rx), [true]);
        // Asked again (after a suspend or a lock) and still XTerm: on again
        // at once, as the client turned it off when its answers changed.
        say_keyboard(&mut srv, told(false, Some("XTerm(391)")));
        assert_eq!(extended_keys_told(&rx), [true]);
        // `kitty-keys off` hands a kitty terminal with an XTerm name to
        // the xterm tier.
        say_keyboard(&mut srv, told(true, Some("XTerm(390)")));
        redraw(&mut srv);
        assert_eq!(extended_keys_told(&rx), Vec::<bool>::new());
        srv.cfg.kitty_keys = false;
        srv.decide_keyboards();
        redraw(&mut srv);
        assert_eq!(extended_keys_told(&rx), [true]);
    }

    #[test]
    fn terminal_features_give_a_terminal_by_term_the_xterm_tier() {
        let (mut srv, rx) = attached(true);
        srv.cfg.extended_keys = "on".into();
        say_keyboard(&mut srv, told(false, None));
        redraw(&mut srv);
        assert_eq!(extended_keys_told(&rx), Vec::<bool>::new());
        srv.cfg.terminal_features.assign_split(
            "xterm-256*:extkeys",
            tmxr_config::TERMINAL_FEATURES_SEPARATORS,
        );
        srv.decide_keyboards();
        redraw(&mut srv);
        assert_eq!(extended_keys_told(&rx), [true]);
        // A client that cannot ask its terminal (the Windows console) is
        // never told.
        let windows = KeyboardReport {
            modify_other_keys: false,
            ..told(false, Some("XTerm(390)"))
        };
        say_keyboard(&mut srv, windows);
        redraw(&mut srv);
        assert_eq!(extended_keys_told(&rx), Vec::<bool>::new());
    }

    /// The keyboard messages the server has told the client so far, in
    /// order: kitty flags and modifyOtherKeys on or off.
    fn keyboard_told(rx: &std::sync::mpsc::Receiver<ServerMsg>) -> Vec<ServerMsg> {
        rx.try_iter()
            .filter(|msg| {
                matches!(
                    msg,
                    ServerMsg::KeyboardFlags(_) | ServerMsg::ExtendedKeys(_)
                )
            })
            .collect()
    }

    /// A terminal with both protocols moving between the tiers, as
    /// `kitty-keys` goes off and on, is told to turn one mode off before the
    /// other on, so it never has both: at once, as it says what it speaks,
    /// and as each frame tells it a change.
    #[test]
    fn a_mode_goes_off_before_the_other_comes_on() {
        let (mut srv, rx) = attached(false);
        srv.cfg.extended_keys = "on".into();
        say_keyboard(&mut srv, told(true, Some("XTerm(390)")));
        assert_eq!(
            keyboard_told(&rx),
            [ServerMsg::KeyboardFlags(0), ServerMsg::ExtendedKeys(true)]
        );
        let tmxr_flags = tmxr_term::encode::TMXR_KITTY_FLAGS;
        srv.cfg.kitty_keys = true;
        srv.decide_keyboards();
        redraw(&mut srv);
        assert_eq!(
            keyboard_told(&rx),
            [
                ServerMsg::ExtendedKeys(false),
                ServerMsg::KeyboardFlags(tmxr_flags)
            ]
        );
        srv.cfg.kitty_keys = false;
        srv.decide_keyboards();
        redraw(&mut srv);
        assert_eq!(
            keyboard_told(&rx),
            [ServerMsg::KeyboardFlags(0), ServerMsg::ExtendedKeys(true)]
        );
    }
}
