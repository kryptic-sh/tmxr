//! Which keyboard protocol each attached client's terminal is asked for:
//! its tier, decided here from what the terminal said of itself
//! (`ClientMsg::Keyboard`), its `TERM` and the options, and kept with the
//! client until one of them changes.
//!
//! The kitty tier's flags are [`crate::kitty`]'s. The xterm tier is tmux's
//! `extkeys` ([`crate::features`]): modifyOtherKeys level 2 for the whole
//! attach while `extended-keys` is not `off` ([`extended_keys`]), told to
//! the client as the mouse mode is, when it changes.

use tmxr_config::Array;
use tmxr_proto::KeyboardReport;
use tmxr_term::ExtendedKeys;

use crate::model::ClientId;
use crate::server::Server;

/// A client terminal's keyboard tier: how it is asked to report keys
/// beyond the bytes a VT sends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
