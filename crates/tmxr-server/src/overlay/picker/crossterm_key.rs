//! The temporary bridge from ttyr's key events, which tmxr uses, to
//! crossterm's, which `handle_key` in the `hjkl-picker-tui` crate still
//! takes. It goes when hjkl's picker takes ttyr's keys, a version bump
//! coordinated with hjkl (plan 18).
//!
//! The key in crossterm's meaning: the character typed, as tmxr reads it
//! ([`Typed`]), an upper-case letter with Shift as crossterm's parser
//! reports one (`A` with Shift, where ttyr has `a` with Shift and `A`
//! shifted); the kind and state field for field. crossterm has no place for
//! the shifted key, base-layout key, associated text or encoding, so they
//! are dropped.

use crossterm::event as ct;
use tmxr_command::Typed;
use ttyr::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MediaKeyCode, ModifierKeyCode,
};

/// ttyr's key event `k` as crossterm's.
pub(super) fn crossterm_key(k: &KeyEvent) -> ct::KeyEvent {
    let kind = match k.kind {
        KeyEventKind::Press => ct::KeyEventKind::Press,
        KeyEventKind::Repeat => ct::KeyEventKind::Repeat,
        KeyEventKind::Release => ct::KeyEventKind::Release,
    };
    let typed = Typed::of(k);
    let mut typed_mods = typed.modifiers;
    if let KeyCode::Char(c) = typed.key
        && c.is_uppercase()
    {
        typed_mods |= KeyModifiers::SHIFT;
    }
    let mut modifiers = ct::KeyModifiers::NONE;
    for (from, to) in [
        (KeyModifiers::SHIFT, ct::KeyModifiers::SHIFT),
        (KeyModifiers::CONTROL, ct::KeyModifiers::CONTROL),
        (KeyModifiers::ALT, ct::KeyModifiers::ALT),
        (KeyModifiers::SUPER, ct::KeyModifiers::SUPER),
        (KeyModifiers::HYPER, ct::KeyModifiers::HYPER),
        (KeyModifiers::META, ct::KeyModifiers::META),
    ] {
        modifiers.set(to, typed_mods.contains(from));
    }
    let mut state = ct::KeyEventState::NONE;
    for (from, to) in [
        (KeyEventState::KEYPAD, ct::KeyEventState::KEYPAD),
        (KeyEventState::CAPS_LOCK, ct::KeyEventState::CAPS_LOCK),
        (KeyEventState::NUM_LOCK, ct::KeyEventState::NUM_LOCK),
    ] {
        state.set(to, k.state.contains(from));
    }
    ct::KeyEvent::new_with_kind_and_state(key_code(typed.key), modifiers, kind, state)
}

fn key_code(code: KeyCode) -> ct::KeyCode {
    match code {
        KeyCode::Backspace => ct::KeyCode::Backspace,
        KeyCode::Enter => ct::KeyCode::Enter,
        KeyCode::Left => ct::KeyCode::Left,
        KeyCode::Right => ct::KeyCode::Right,
        KeyCode::Up => ct::KeyCode::Up,
        KeyCode::Down => ct::KeyCode::Down,
        KeyCode::Home => ct::KeyCode::Home,
        KeyCode::End => ct::KeyCode::End,
        KeyCode::PageUp => ct::KeyCode::PageUp,
        KeyCode::PageDown => ct::KeyCode::PageDown,
        KeyCode::Tab => ct::KeyCode::Tab,
        KeyCode::BackTab => ct::KeyCode::BackTab,
        KeyCode::Delete => ct::KeyCode::Delete,
        KeyCode::Insert => ct::KeyCode::Insert,
        KeyCode::F(n) => ct::KeyCode::F(n),
        KeyCode::Char(c) => ct::KeyCode::Char(c),
        KeyCode::Null => ct::KeyCode::Null,
        KeyCode::Esc => ct::KeyCode::Esc,
        KeyCode::CapsLock => ct::KeyCode::CapsLock,
        KeyCode::ScrollLock => ct::KeyCode::ScrollLock,
        KeyCode::NumLock => ct::KeyCode::NumLock,
        KeyCode::PrintScreen => ct::KeyCode::PrintScreen,
        KeyCode::Pause => ct::KeyCode::Pause,
        KeyCode::Menu => ct::KeyCode::Menu,
        KeyCode::KeypadBegin => ct::KeyCode::KeypadBegin,
        KeyCode::Media(m) => ct::KeyCode::Media(media_key(m)),
        KeyCode::Modifier(m) => ct::KeyCode::Modifier(modifier_key(m)),
    }
}

fn media_key(m: MediaKeyCode) -> ct::MediaKeyCode {
    match m {
        MediaKeyCode::Play => ct::MediaKeyCode::Play,
        MediaKeyCode::Pause => ct::MediaKeyCode::Pause,
        MediaKeyCode::PlayPause => ct::MediaKeyCode::PlayPause,
        MediaKeyCode::Reverse => ct::MediaKeyCode::Reverse,
        MediaKeyCode::Stop => ct::MediaKeyCode::Stop,
        MediaKeyCode::FastForward => ct::MediaKeyCode::FastForward,
        MediaKeyCode::Rewind => ct::MediaKeyCode::Rewind,
        MediaKeyCode::TrackNext => ct::MediaKeyCode::TrackNext,
        MediaKeyCode::TrackPrevious => ct::MediaKeyCode::TrackPrevious,
        MediaKeyCode::Record => ct::MediaKeyCode::Record,
        MediaKeyCode::LowerVolume => ct::MediaKeyCode::LowerVolume,
        MediaKeyCode::RaiseVolume => ct::MediaKeyCode::RaiseVolume,
        MediaKeyCode::MuteVolume => ct::MediaKeyCode::MuteVolume,
    }
}

fn modifier_key(m: ModifierKeyCode) -> ct::ModifierKeyCode {
    match m {
        ModifierKeyCode::LeftShift => ct::ModifierKeyCode::LeftShift,
        ModifierKeyCode::LeftControl => ct::ModifierKeyCode::LeftControl,
        ModifierKeyCode::LeftAlt => ct::ModifierKeyCode::LeftAlt,
        ModifierKeyCode::LeftSuper => ct::ModifierKeyCode::LeftSuper,
        ModifierKeyCode::LeftHyper => ct::ModifierKeyCode::LeftHyper,
        ModifierKeyCode::LeftMeta => ct::ModifierKeyCode::LeftMeta,
        ModifierKeyCode::RightShift => ct::ModifierKeyCode::RightShift,
        ModifierKeyCode::RightControl => ct::ModifierKeyCode::RightControl,
        ModifierKeyCode::RightAlt => ct::ModifierKeyCode::RightAlt,
        ModifierKeyCode::RightSuper => ct::ModifierKeyCode::RightSuper,
        ModifierKeyCode::RightHyper => ct::ModifierKeyCode::RightHyper,
        ModifierKeyCode::RightMeta => ct::ModifierKeyCode::RightMeta,
        ModifierKeyCode::IsoLevel3Shift => ct::ModifierKeyCode::IsoLevel3Shift,
        ModifierKeyCode::IsoLevel5Shift => ct::ModifierKeyCode::IsoLevel5Shift,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ct::KeyModifiers as CtMods;

    /// Field by field: crossterm's own equality takes `A` for `a` with
    /// Shift, which would hide a case lost on the way.
    fn fields(
        k: ct::KeyEvent,
    ) -> (
        ct::KeyCode,
        ct::KeyModifiers,
        ct::KeyEventKind,
        ct::KeyEventState,
    ) {
        (k.code, k.modifiers, k.kind, k.state)
    }

    #[test]
    fn a_key_crosses_in_crossterms_meaning() {
        // ttyr's Shift+a is crossterm's `A` with Shift.
        let k = KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::SHIFT | KeyModifiers::HYPER,
        )
        .with_shifted(Some('A'))
        .with_kind(KeyEventKind::Repeat)
        .with_state(KeyEventState::NUM_LOCK);
        assert_eq!(
            fields(crossterm_key(&k)),
            (
                ct::KeyCode::Char('A'),
                ct::KeyModifiers::SHIFT | ct::KeyModifiers::HYPER,
                ct::KeyEventKind::Repeat,
                ct::KeyEventState::NUM_LOCK,
            )
        );
        let k = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(
            fields(crossterm_key(&k)),
            (
                ct::KeyCode::Char('a'),
                ct::KeyModifiers::CONTROL,
                ct::KeyEventKind::Press,
                ct::KeyEventState::NONE,
            )
        );
    }

    #[test]
    fn hjkl_and_shifted_symbols_are_the_characters_typed() {
        let alt_shift = KeyModifiers::ALT | KeyModifiers::SHIFT;
        let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        for (k, want) in [
            (
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                (ct::KeyCode::Char('j'), CtMods::NONE),
            ),
            (
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
                (ct::KeyCode::Char('k'), CtMods::CONTROL),
            ),
            // Kitty's Alt+Shift+1 reporting alternate keys is `!` with Alt.
            (
                KeyEvent::new(KeyCode::Char('1'), alt_shift).with_shifted(Some('!')),
                (ct::KeyCode::Char('!'), CtMods::ALT),
            ),
            // Ctrl+Shift+h stays the letter with Shift.
            (
                KeyEvent::new(KeyCode::Char('h'), ctrl_shift).with_shifted(Some('H')),
                (ct::KeyCode::Char('h'), CtMods::CONTROL | CtMods::SHIFT),
            ),
            // A shifted letter typed alone, and with Alt.
            (
                KeyEvent::new(KeyCode::Char('l'), KeyModifiers::SHIFT).with_shifted(Some('L')),
                (ct::KeyCode::Char('L'), CtMods::SHIFT),
            ),
            (
                KeyEvent::new(KeyCode::Char('l'), alt_shift).with_shifted(Some('L')),
                (ct::KeyCode::Char('L'), CtMods::ALT | CtMods::SHIFT),
            ),
        ] {
            let got = crossterm_key(&k);
            assert_eq!((got.code, got.modifiers), want, "{k:?}");
        }
    }

    /// The same variant name on both sides, for each name given.
    macro_rules! pairs {
        ($from:ident => $to:ident: $($v:ident),+ $(,)?) => {
            [$(($from::$v, $to::$v)),+]
        };
    }

    #[test]
    fn every_key_code_crosses_as_itself() {
        use ct::{KeyCode as CtKey, MediaKeyCode as CtMedia, ModifierKeyCode as CtModifier};
        let mut keys = pairs!(KeyCode => CtKey:
            Backspace, Enter, Left, Right, Up, Down, Home, End, PageUp, PageDown, Tab,
            BackTab, Delete, Insert, Null, Esc, CapsLock, ScrollLock, NumLock, PrintScreen,
            Pause, Menu, KeypadBegin,
        )
        .to_vec();
        keys.extend([
            (KeyCode::F(1), CtKey::F(1)),
            (KeyCode::F(24), CtKey::F(24)),
            (KeyCode::Char('a'), CtKey::Char('a')),
            (KeyCode::Char('É'), CtKey::Char('É')),
        ]);
        keys.extend(
            pairs!(MediaKeyCode => CtMedia:
                Play, Pause, PlayPause, Reverse, Stop, FastForward, Rewind, TrackNext,
                TrackPrevious, Record, LowerVolume, RaiseVolume, MuteVolume,
            )
            .map(|(from, to)| (KeyCode::Media(from), CtKey::Media(to))),
        );
        keys.extend(
            pairs!(ModifierKeyCode => CtModifier:
                LeftShift, LeftControl, LeftAlt, LeftSuper, LeftHyper, LeftMeta, RightShift,
                RightControl, RightAlt, RightSuper, RightHyper, RightMeta, IsoLevel3Shift,
                IsoLevel5Shift,
            )
            .map(|(from, to)| (KeyCode::Modifier(from), CtKey::Modifier(to))),
        );
        for (from, to) in keys {
            let k = KeyEvent::new(from, KeyModifiers::NONE);
            assert_eq!(crossterm_key(&k).code, to, "{from:?}");
        }
    }

    #[test]
    fn every_modifier_crosses_on_its_own() {
        for (from, to) in pairs!(KeyModifiers => CtMods: SHIFT, CONTROL, ALT, SUPER, HYPER, META) {
            let k = KeyEvent::new(KeyCode::Char('x'), from);
            assert_eq!(crossterm_key(&k).modifiers, to, "{from:?}");
        }
    }
}
