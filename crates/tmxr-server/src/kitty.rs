//! The kitty keyboard flags each attached client's terminal is asked for.
//!
//! tmxr asks for its own flags ([`TMXR_KITTY_FLAGS`]) and for those the
//! program the client's keys go to pushed, as far as the encoder acts on
//! them ([`KITTY_FLAGS_ENCODED`]); none under `kitty-keys off`. [`flags`] is
//! the one place that decides, read for each client as its frame is drawn,
//! and the client is told when the answer changes, as the mouse mode is
//! ([`crate::mouse::capture`]): whatever moves focus or changes a program's
//! flags also marks the client for a redraw. A client whose terminal does
//! not speak the protocol ([`speaks`]) is never told any.

use tmxr_proto::Keyboard;
use tmxr_term::encode::{KITTY_FLAGS_ENCODED, TMXR_KITTY_FLAGS};

use crate::model::ClientId;
use crate::overlay::Overlay;
use crate::server::Server;

/// Whether attached client `id` said its terminal speaks the kitty
/// keyboard protocol (`ClientMsg::Keyboard`): only then is it told which
/// flags to ask for.
pub fn speaks(srv: &Server, id: ClientId) -> bool {
    srv.clients
        .get(&id)
        .and_then(|c| c.att.as_ref())
        .is_some_and(|att| att.keyboard == Keyboard::Kitty)
}

/// The kitty keyboard flags client `id`'s terminal should report keys
/// under now.
pub fn flags(srv: &Server, id: ClientId) -> u8 {
    let program = srv
        .clients
        .get(&id)
        .and_then(|c| c.att.as_ref())
        .and_then(|att| {
            program_flags(att.overlay.as_ref(), || {
                // A mode in the pane reads keys itself.
                let pane = srv
                    .active_pane_of_session(att.session)
                    .filter(|&p| !crate::mouse::in_mode(srv, p))?;
                let flags = srv.panes.get(&pane)?.emu.input_modes().kitty_flags;
                // So does a held key table, and a read-only client's keys
                // reach no program. Asked only of flags that would add to
                // tmxr's own, as the table's name is built afresh.
                let adds = flags & KITTY_FLAGS_ENCODED & !TMXR_KITTY_FLAGS != 0;
                let held = adds
                    && (att.table != srv.default_key_table(att.session)
                        || srv.client_read_only(id));
                (!held).then_some(flags)
            })
        });
    wanted(srv.cfg.kitty_keys, program, KITTY_FLAGS_ENCODED)
}

/// The flags of the program keys go to past `overlay`: a popup's own
/// program while it runs, or with no overlay that takes keys, what `pane`
/// says of the active pane. `None` where tmxr reads the keys itself (a
/// prompt, menu, picker or other overlay, or a popup whose program has
/// exited).
fn program_flags(overlay: Option<&Overlay>, pane: impl FnOnce() -> Option<u8>) -> Option<u8> {
    match overlay {
        Some(Overlay::Popup(p)) => (!p.exited).then(|| p.emu.input_modes().kitty_flags),
        // display-panes -N lets keys go on as if it were not there.
        None | Some(Overlay::Panes { keys: false, .. }) => pane(),
        Some(_) => None,
    }
}

/// The flags to ask for: none with `kitty-keys` off (`on` false), else
/// tmxr's own and those of `program` the encoder acts on (`encoded`).
fn wanted(on: bool, program: Option<u8>, encoded: u8) -> u8 {
    if !on {
        return 0;
    }
    TMXR_KITTY_FLAGS | (program.unwrap_or(0) & encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every flag, as an encoder that acts on them all will take them.
    const ALL_FLAGS: u8 = 0b1_1111;
    const RELEASES: u8 = 2;
    const ALL_KEYS: u8 = 8;

    #[test]
    fn a_client_is_asked_for_tmxrs_flags_and_its_programs() {
        assert_eq!(wanted(true, None, ALL_FLAGS), TMXR_KITTY_FLAGS);
        assert_eq!(wanted(true, Some(0), ALL_FLAGS), TMXR_KITTY_FLAGS);
        assert_eq!(
            wanted(true, Some(RELEASES | ALL_KEYS), ALL_FLAGS),
            TMXR_KITTY_FLAGS | RELEASES | ALL_KEYS
        );
        // Only the flags the encoder acts on.
        assert_eq!(
            wanted(true, Some(ALL_FLAGS), KITTY_FLAGS_ENCODED),
            TMXR_KITTY_FLAGS | KITTY_FLAGS_ENCODED
        );
        assert_eq!(
            wanted(true, Some(ALL_KEYS), ALL_FLAGS & !ALL_KEYS),
            TMXR_KITTY_FLAGS
        );
        // `kitty-keys off`: none at all, tmxr's own included.
        assert_eq!(wanted(false, None, ALL_FLAGS), 0);
        assert_eq!(wanted(false, Some(ALL_FLAGS), ALL_FLAGS), 0);
    }

    #[test]
    fn only_keys_that_reach_a_program_take_its_flags() {
        let pane = || Some(ALL_KEYS);
        assert_eq!(program_flags(None, pane), Some(ALL_KEYS));
        assert_eq!(program_flags(None, || None), None);
        let passing = Overlay::Panes {
            labels: Vec::new(),
            until: None,
            template: String::new(),
            keys: false,
        };
        assert_eq!(program_flags(Some(&passing), pane), Some(ALL_KEYS));
        for overlay in [
            Overlay::Panes {
                labels: Vec::new(),
                until: None,
                template: String::new(),
                keys: true,
            },
            Overlay::prompt(":".into(), String::new(), None),
            Overlay::confirm("kill?".into(), "kill-pane".into(), 'y', false),
            Overlay::text(vec!["list".into()]),
        ] {
            assert_eq!(program_flags(Some(&overlay), pane), None);
        }
    }
}
