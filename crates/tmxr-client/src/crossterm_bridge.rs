//! The temporary bridge from crossterm's events, which the client still
//! reads its terminal with, to ttyr's, which the rest of tmxr and the wire
//! use (plan 18, step 3).
//!
//! It goes when ttyr's reader replaces crossterm's: on Unix in the second
//! half of that step, on Windows once ttyr reads the Windows console.
//!
//! The conversion is field for field and keeps crossterm's meaning, so the
//! server sees exactly what it saw before: a character key is the character
//! crossterm reported (`A` with Shift for a legacy Shift+a, the shifted
//! character with Shift cleared under kitty flag 4), not ttyr's unshifted
//! key with the shifted one beside it. crossterm reports no shifted key,
//! base-layout key or associated text, so those are always `None`.

use crossterm::event as ct;
use ttyr::event::{
    ColorScheme, ColorSlot, Event, KeyCode, KeyEncoding, KeyEvent, KeyEventKind, KeyEventState,
    KeyModifiers, MediaKeyCode, ModifierKeyCode, MouseButton, MouseEvent, MouseEventKind, Report,
    TerminalColor,
};

/// crossterm's event `ev` as ttyr's. `kitty` is whether the terminal has
/// kitty keyboard flags on, which sets every key's [`KeyEncoding`]:
/// crossterm does not say which form each key came in, so it is `Kitty`
/// while the protocol is on and `Legacy` otherwise, per batch and not per
/// key.
pub fn ttyr_event(ev: ct::Event, kitty: bool) -> Event {
    match ev {
        ct::Event::FocusGained => Event::FocusGained,
        ct::Event::FocusLost => Event::FocusLost,
        ct::Event::Key(k) => Event::Key(key_event(k, kitty)),
        ct::Event::Mouse(m) => Event::Mouse(mouse_event(m)),
        ct::Event::Paste(text) => Event::Paste(text),
        ct::Event::Resize(cols, rows) => Event::Resize(cols, rows),
        ct::Event::ColorScheme(scheme) => Event::Report(Report::ColorScheme(match scheme {
            ct::ColorScheme::Dark => ColorScheme::Dark,
            ct::ColorScheme::Light => ColorScheme::Light,
        })),
        ct::Event::TerminalColor(colour) => Event::Report(Report::TerminalColor(TerminalColor {
            slot: match colour.slot {
                ct::ColorSlot::Foreground => ColorSlot::Foreground,
                ct::ColorSlot::Background => ColorSlot::Background,
                ct::ColorSlot::Cursor => ColorSlot::Cursor,
                ct::ColorSlot::Palette(n) => ColorSlot::Palette(n),
            },
            rgb: colour.rgb,
        })),
    }
}

fn key_event(k: ct::KeyEvent, kitty: bool) -> KeyEvent {
    let kind = match k.kind {
        ct::KeyEventKind::Press => KeyEventKind::Press,
        ct::KeyEventKind::Repeat => KeyEventKind::Repeat,
        ct::KeyEventKind::Release => KeyEventKind::Release,
    };
    let mut state = KeyEventState::NONE;
    for (from, to) in [
        (ct::KeyEventState::KEYPAD, KeyEventState::KEYPAD),
        (ct::KeyEventState::CAPS_LOCK, KeyEventState::CAPS_LOCK),
        (ct::KeyEventState::NUM_LOCK, KeyEventState::NUM_LOCK),
    ] {
        state.set(to, k.state.contains(from));
    }
    let encoding = if kitty {
        KeyEncoding::Kitty
    } else {
        KeyEncoding::Legacy
    };
    KeyEvent::new(key_code(k.code), modifiers(k.modifiers))
        .with_kind(kind)
        .with_state(state)
        .with_encoding(encoding)
}

fn modifiers(m: ct::KeyModifiers) -> KeyModifiers {
    let mut out = KeyModifiers::NONE;
    for (from, to) in [
        (ct::KeyModifiers::SHIFT, KeyModifiers::SHIFT),
        (ct::KeyModifiers::CONTROL, KeyModifiers::CONTROL),
        (ct::KeyModifiers::ALT, KeyModifiers::ALT),
        (ct::KeyModifiers::SUPER, KeyModifiers::SUPER),
        (ct::KeyModifiers::HYPER, KeyModifiers::HYPER),
        (ct::KeyModifiers::META, KeyModifiers::META),
    ] {
        out.set(to, m.contains(from));
    }
    out
}

fn key_code(code: ct::KeyCode) -> KeyCode {
    match code {
        ct::KeyCode::Backspace => KeyCode::Backspace,
        ct::KeyCode::Enter => KeyCode::Enter,
        ct::KeyCode::Left => KeyCode::Left,
        ct::KeyCode::Right => KeyCode::Right,
        ct::KeyCode::Up => KeyCode::Up,
        ct::KeyCode::Down => KeyCode::Down,
        ct::KeyCode::Home => KeyCode::Home,
        ct::KeyCode::End => KeyCode::End,
        ct::KeyCode::PageUp => KeyCode::PageUp,
        ct::KeyCode::PageDown => KeyCode::PageDown,
        ct::KeyCode::Tab => KeyCode::Tab,
        ct::KeyCode::BackTab => KeyCode::BackTab,
        ct::KeyCode::Delete => KeyCode::Delete,
        ct::KeyCode::Insert => KeyCode::Insert,
        ct::KeyCode::F(n) => KeyCode::F(n),
        ct::KeyCode::Char(c) => KeyCode::Char(c),
        ct::KeyCode::Null => KeyCode::Null,
        ct::KeyCode::Esc => KeyCode::Esc,
        ct::KeyCode::CapsLock => KeyCode::CapsLock,
        ct::KeyCode::ScrollLock => KeyCode::ScrollLock,
        ct::KeyCode::NumLock => KeyCode::NumLock,
        ct::KeyCode::PrintScreen => KeyCode::PrintScreen,
        ct::KeyCode::Pause => KeyCode::Pause,
        ct::KeyCode::Menu => KeyCode::Menu,
        ct::KeyCode::KeypadBegin => KeyCode::KeypadBegin,
        ct::KeyCode::Media(m) => KeyCode::Media(media_key(m)),
        ct::KeyCode::Modifier(m) => KeyCode::Modifier(modifier_key(m)),
    }
}

fn media_key(m: ct::MediaKeyCode) -> MediaKeyCode {
    match m {
        ct::MediaKeyCode::Play => MediaKeyCode::Play,
        ct::MediaKeyCode::Pause => MediaKeyCode::Pause,
        ct::MediaKeyCode::PlayPause => MediaKeyCode::PlayPause,
        ct::MediaKeyCode::Reverse => MediaKeyCode::Reverse,
        ct::MediaKeyCode::Stop => MediaKeyCode::Stop,
        ct::MediaKeyCode::FastForward => MediaKeyCode::FastForward,
        ct::MediaKeyCode::Rewind => MediaKeyCode::Rewind,
        ct::MediaKeyCode::TrackNext => MediaKeyCode::TrackNext,
        ct::MediaKeyCode::TrackPrevious => MediaKeyCode::TrackPrevious,
        ct::MediaKeyCode::Record => MediaKeyCode::Record,
        ct::MediaKeyCode::LowerVolume => MediaKeyCode::LowerVolume,
        ct::MediaKeyCode::RaiseVolume => MediaKeyCode::RaiseVolume,
        ct::MediaKeyCode::MuteVolume => MediaKeyCode::MuteVolume,
    }
}

fn modifier_key(m: ct::ModifierKeyCode) -> ModifierKeyCode {
    match m {
        ct::ModifierKeyCode::LeftShift => ModifierKeyCode::LeftShift,
        ct::ModifierKeyCode::LeftControl => ModifierKeyCode::LeftControl,
        ct::ModifierKeyCode::LeftAlt => ModifierKeyCode::LeftAlt,
        ct::ModifierKeyCode::LeftSuper => ModifierKeyCode::LeftSuper,
        ct::ModifierKeyCode::LeftHyper => ModifierKeyCode::LeftHyper,
        ct::ModifierKeyCode::LeftMeta => ModifierKeyCode::LeftMeta,
        ct::ModifierKeyCode::RightShift => ModifierKeyCode::RightShift,
        ct::ModifierKeyCode::RightControl => ModifierKeyCode::RightControl,
        ct::ModifierKeyCode::RightAlt => ModifierKeyCode::RightAlt,
        ct::ModifierKeyCode::RightSuper => ModifierKeyCode::RightSuper,
        ct::ModifierKeyCode::RightHyper => ModifierKeyCode::RightHyper,
        ct::ModifierKeyCode::RightMeta => ModifierKeyCode::RightMeta,
        ct::ModifierKeyCode::IsoLevel3Shift => ModifierKeyCode::IsoLevel3Shift,
        ct::ModifierKeyCode::IsoLevel5Shift => ModifierKeyCode::IsoLevel5Shift,
    }
}

fn mouse_event(m: ct::MouseEvent) -> MouseEvent {
    let button = |b: ct::MouseButton| match b {
        ct::MouseButton::Left => MouseButton::Left,
        ct::MouseButton::Right => MouseButton::Right,
        ct::MouseButton::Middle => MouseButton::Middle,
    };
    let kind = match m.kind {
        ct::MouseEventKind::Down(b) => MouseEventKind::Down(button(b)),
        ct::MouseEventKind::Up(b) => MouseEventKind::Up(button(b)),
        ct::MouseEventKind::Drag(b) => MouseEventKind::Drag(button(b)),
        ct::MouseEventKind::Moved => MouseEventKind::Moved,
        ct::MouseEventKind::ScrollDown => MouseEventKind::ScrollDown,
        ct::MouseEventKind::ScrollUp => MouseEventKind::ScrollUp,
        ct::MouseEventKind::ScrollLeft => MouseEventKind::ScrollLeft,
        ct::MouseEventKind::ScrollRight => MouseEventKind::ScrollRight,
    };
    MouseEvent {
        kind,
        column: m.column,
        row: m.row,
        modifiers: modifiers(m.modifiers),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same variant name on both sides, for each name given.
    macro_rules! pairs {
        ($from:ident => $to:ident: $($v:ident),+ $(,)?) => {
            [$(($from::$v, $to::$v)),+]
        };
    }

    #[test]
    fn every_key_code_crosses_as_itself() {
        use ct::{KeyCode as CtKey, MediaKeyCode as CtMedia, ModifierKeyCode as CtModifier};
        let mut keys = pairs!(CtKey => KeyCode:
            Backspace, Enter, Left, Right, Up, Down, Home, End, PageUp, PageDown, Tab,
            BackTab, Delete, Insert, Null, Esc, CapsLock, ScrollLock, NumLock, PrintScreen,
            Pause, Menu, KeypadBegin,
        )
        .to_vec();
        keys.extend([
            (CtKey::F(1), KeyCode::F(1)),
            (CtKey::F(24), KeyCode::F(24)),
            (CtKey::Char('a'), KeyCode::Char('a')),
            (CtKey::Char('É'), KeyCode::Char('É')),
        ]);
        keys.extend(
            pairs!(CtMedia => MediaKeyCode:
                Play, Pause, PlayPause, Reverse, Stop, FastForward, Rewind, TrackNext,
                TrackPrevious, Record, LowerVolume, RaiseVolume, MuteVolume,
            )
            .map(|(from, to)| (CtKey::Media(from), KeyCode::Media(to))),
        );
        keys.extend(
            pairs!(CtModifier => ModifierKeyCode:
                LeftShift, LeftControl, LeftAlt, LeftSuper, LeftHyper, LeftMeta, RightShift,
                RightControl, RightAlt, RightSuper, RightHyper, RightMeta, IsoLevel3Shift,
                IsoLevel5Shift,
            )
            .map(|(from, to)| (CtKey::Modifier(from), KeyCode::Modifier(to))),
        );
        for (from, to) in keys {
            let ev = ct::Event::Key(ct::KeyEvent::new(from, ct::KeyModifiers::NONE));
            let Event::Key(k) = ttyr_event(ev, false) else {
                panic!("not a key: {from:?}");
            };
            assert_eq!(k.key, to, "{from:?}");
        }
    }

    #[test]
    fn every_mouse_kind_crosses_as_itself() {
        use ct::{MouseButton as CtButton, MouseEventKind as CtKind};
        let buttons = pairs!(CtButton => MouseButton: Left, Right, Middle);
        let mut kinds = pairs!(CtKind => MouseEventKind:
            Moved, ScrollDown, ScrollUp, ScrollLeft, ScrollRight,
        )
        .to_vec();
        for (from, to) in buttons {
            kinds.extend([
                (CtKind::Down(from), MouseEventKind::Down(to)),
                (CtKind::Up(from), MouseEventKind::Up(to)),
                (CtKind::Drag(from), MouseEventKind::Drag(to)),
            ]);
        }
        for (from, to) in kinds {
            let ev = ct::Event::Mouse(ct::MouseEvent {
                kind: from,
                column: 0,
                row: 0,
                modifiers: ct::KeyModifiers::NONE,
            });
            let Event::Mouse(m) = ttyr_event(ev, false) else {
                panic!("not a mouse event: {from:?}");
            };
            assert_eq!(m.kind, to, "{from:?}");
        }
    }

    #[test]
    fn a_key_keeps_crossterms_meaning_field_for_field() {
        // A legacy Shift+a, as crossterm reports it: the capital with Shift.
        let ev = ct::KeyEvent::new_with_kind_and_state(
            ct::KeyCode::Char('A'),
            ct::KeyModifiers::SHIFT | ct::KeyModifiers::META,
            ct::KeyEventKind::Release,
            ct::KeyEventState::CAPS_LOCK | ct::KeyEventState::KEYPAD,
        );
        let want = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT | KeyModifiers::META)
            .with_kind(KeyEventKind::Release)
            .with_state(KeyEventState::CAPS_LOCK | KeyEventState::KEYPAD)
            .with_encoding(KeyEncoding::Legacy);
        assert_eq!(ttyr_event(ct::Event::Key(ev), false), Event::Key(want));
        let Event::Key(kitty) = ttyr_event(ct::Event::Key(ev), true) else {
            panic!("not a key");
        };
        assert_eq!(kitty.encoding, KeyEncoding::Kitty);
        // BackTab stays BackTab, with the Shift crossterm gives it.
        let back = ct::KeyEvent::new(ct::KeyCode::BackTab, ct::KeyModifiers::SHIFT);
        let Event::Key(back) = ttyr_event(ct::Event::Key(back), false) else {
            panic!("not a key");
        };
        assert_eq!(
            (back.key, back.modifiers),
            (KeyCode::BackTab, KeyModifiers::SHIFT)
        );
        // Every modifier crosses on its own.
        for (from, to) in [
            (ct::KeyModifiers::SHIFT, KeyModifiers::SHIFT),
            (ct::KeyModifiers::CONTROL, KeyModifiers::CONTROL),
            (ct::KeyModifiers::ALT, KeyModifiers::ALT),
            (ct::KeyModifiers::SUPER, KeyModifiers::SUPER),
            (ct::KeyModifiers::HYPER, KeyModifiers::HYPER),
            (ct::KeyModifiers::META, KeyModifiers::META),
        ] {
            assert_eq!(modifiers(from), to, "{from:?}");
        }
    }

    #[test]
    fn mouse_paste_focus_resize_and_reports_cross() {
        let mouse = ct::MouseEvent {
            kind: ct::MouseEventKind::Drag(ct::MouseButton::Right),
            column: 3,
            row: 7,
            modifiers: ct::KeyModifiers::ALT,
        };
        assert_eq!(
            ttyr_event(ct::Event::Mouse(mouse), false),
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Right),
                column: 3,
                row: 7,
                modifiers: KeyModifiers::ALT,
            })
        );
        for (from, to) in [
            (ct::Event::Paste("hi".into()), Event::Paste("hi".into())),
            (ct::Event::FocusGained, Event::FocusGained),
            (ct::Event::FocusLost, Event::FocusLost),
            (ct::Event::Resize(80, 24), Event::Resize(80, 24)),
        ] {
            assert_eq!(ttyr_event(from.clone(), false), to, "{from:?}");
        }
    }

    #[test]
    fn every_colour_scheme_and_slot_crosses_as_itself() {
        use ct::{ColorScheme as CtScheme, ColorSlot as CtSlot};
        for (from, to) in pairs!(CtScheme => ColorScheme: Light, Dark) {
            assert_eq!(
                ttyr_event(ct::Event::ColorScheme(from), false),
                Event::Report(Report::ColorScheme(to)),
                "{from:?}"
            );
        }
        let mut slots = pairs!(CtSlot => ColorSlot: Foreground, Background, Cursor).to_vec();
        slots.extend([
            (CtSlot::Palette(0), ColorSlot::Palette(0)),
            (CtSlot::Palette(255), ColorSlot::Palette(255)),
        ]);
        for (from, to) in slots {
            let colour = ct::TerminalColor {
                slot: from,
                rgb: (1, 0x8080, 0xffff),
            };
            assert_eq!(
                ttyr_event(ct::Event::TerminalColor(colour), false),
                Event::Report(Report::TerminalColor(TerminalColor {
                    slot: to,
                    rgb: (1, 0x8080, 0xffff),
                })),
                "{from:?}"
            );
        }
    }
}
