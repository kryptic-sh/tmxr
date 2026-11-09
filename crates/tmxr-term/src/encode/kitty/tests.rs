//! The spec's own examples, and the cases kitty's test suite
//! (`kitty_tests/keys.py`, `test_encode_key_event`) checks its encoder
//! with, under each flag and the combinations it names.

use ttyr::event::{
    KeyCode, KeyEncoding, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, ModifierKeyCode,
};

use crate::encode::{ExtendedKeys, ExtendedKeysFormat, InputModes, encode_key};

const NONE: KeyModifiers = KeyModifiers::NONE;
const S: KeyModifiers = KeyModifiers::SHIFT;
const A: KeyModifiers = KeyModifiers::ALT;
const C: KeyModifiers = KeyModifiers::CONTROL;
const RELEASE: KeyEventKind = KeyEventKind::Release;
const REPEAT: KeyEventKind = KeyEventKind::Repeat;

/// `key` for a program that pushed `flags`, escaped for reading.
fn enc(flags: u8, key: &KeyEvent) -> String {
    let modes = InputModes {
        kitty_flags: flags,
        ..InputModes::default()
    };
    let bytes = encode_key(key, modes, ExtendedKeys::On, ExtendedKeysFormat::Xterm);
    bytes.escape_ascii().to_string()
}

fn key(code: KeyCode, m: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, m)
}

fn ch(c: char, m: KeyModifiers) -> KeyEvent {
    key(KeyCode::Char(c), m)
}

/// The spec's "Text as code points" examples: Shift+a with its text `A`,
/// and the text `å` that came with no key (the OS took Alt to make it).
#[test]
fn the_specs_associated_text_examples() {
    let shift_a = ch('a', S).with_text(Some("A".into()));
    assert_eq!(enc(8 | 16, &shift_a), r"\x1b[97;2;65u");
    let alt_a = key(KeyCode::Null, NONE).with_text(Some("å".into()));
    assert_eq!(enc(8 | 16, &alt_a), r"\x1b[0;;229u");
    // Without flag 8 text is typed, key or none.
    assert_eq!(enc(1 | 16, &alt_a), "\\xc3\\xa5");
    assert_eq!(enc(1, &shift_a), "A");
}

/// The spec's "Event types" forms: a release and a repeat carry the event
/// type after the modifiers, `1` standing for none.
#[test]
fn the_specs_event_type_examples() {
    let ctrl_a = ch('a', C);
    assert_eq!(enc(1 | 2, &ctrl_a), r"\x1b[97;5u");
    assert_eq!(
        enc(1 | 2, &ctrl_a.clone().with_kind(REPEAT)),
        r"\x1b[97;5:2u"
    );
    assert_eq!(enc(1 | 2, &ctrl_a.with_kind(RELEASE)), r"\x1b[97;5:3u");
    let esc = key(KeyCode::Esc, NONE);
    assert_eq!(enc(1 | 2, &esc.clone().with_kind(RELEASE)), r"\x1b[27;1:3u");
    // Without flag 2 a repeat is a press and a release is nothing.
    assert_eq!(enc(1, &esc.clone().with_kind(REPEAT)), r"\x1b[27u");
    assert_eq!(enc(1, &esc.with_kind(RELEASE)), "");
}

/// The spec's "Key codes": the unshifted key always, the shifted key only
/// with Shift, and an empty shifted field before a lone base-layout key
/// (Ctrl+С on a Cyrillic layout is Ctrl+c on PC-101's).
#[test]
fn the_specs_alternate_key_examples() {
    let ctrl_shift_a = ch('a', C | S).with_shifted(Some('A'));
    assert_eq!(enc(1, &ctrl_shift_a), r"\x1b[97;6u", "never 65");
    assert_eq!(enc(1 | 4, &ctrl_shift_a), r"\x1b[97:65;6u");
    let ctrl_es = ch('с', C).with_base_layout(Some(KeyCode::Char('c')));
    assert_eq!(enc(1 | 4, &ctrl_es), r"\x1b[1089::99;5u");
    assert_eq!(enc(1, &ctrl_es), r"\x1b[1089;5u");
    let both = ch('с', C | S)
        .with_shifted(Some('С'))
        .with_base_layout(Some(KeyCode::Char('c')));
    assert_eq!(enc(1 | 4, &both), r"\x1b[1089:1057:99;6u");
    // The shifted key without Shift held is not sent.
    let a = ch('a', C).with_shifted(Some('A'));
    assert_eq!(enc(1 | 4, &a), r"\x1b[97;5u");
}

/// The spec's legacy-text example row for `i`, Ctrl+Shift, which flag 1
/// already sends as `CSI 105;6u`; and its Enter, Tab and Backspace, which
/// flag 1 leaves as they are unless modified.
#[test]
fn disambiguate_as_kittys_tests() {
    assert_eq!(enc(1, &ch('a', NONE)), "a");
    assert_eq!(enc(1, &key(KeyCode::Esc, NONE)), r"\x1b[27u");
    assert_eq!(enc(1, &key(KeyCode::Enter, NONE)), r"\r");
    assert_eq!(enc(1, &key(KeyCode::Enter, S)), r"\x1b[13;2u");
    assert_eq!(enc(1, &key(KeyCode::Tab, NONE)), r"\t");
    assert_eq!(enc(1, &key(KeyCode::Backspace, NONE)), r"\x7f");
    assert_eq!(enc(1, &key(KeyCode::Tab, S)), r"\x1b[9;2u");
    assert_eq!(enc(1, &key(KeyCode::BackTab, S)), r"\x1b[9;2u");
    for (m, n) in [(C, 5), (A, 3), (C | S, 6), (A | S, 4)] {
        assert_eq!(enc(1, &ch('a', m)), format!(r"\x1b[97;{n}u"), "{m:?}");
    }
    assert_eq!(enc(1, &ch(' ', C)), r"\x1b[32;5u");
    assert_eq!(enc(1, &ch('i', C | S)), r"\x1b[105;6u");
    let keypad = KeyEventState::KEYPAD;
    let kp_page_up = key(KeyCode::PageUp, NONE).with_state(keypad);
    assert_eq!(enc(1, &kp_page_up), r"\x1b[57421u");
    let kp_0 = key(KeyCode::Char('0'), C).with_state(keypad);
    assert_eq!(enc(1, &kp_0), r"\x1b[57399;5u");
    assert_eq!(enc(1, &key(KeyCode::Up, NONE)), r"\x1b[A");
    assert_eq!(enc(1, &key(KeyCode::Up, C)), r"\x1b[1;5A");
    // A keypad key that types text is its text.
    assert_eq!(
        enc(1, &key(KeyCode::Char('0'), NONE).with_state(keypad)),
        "0"
    );
}

/// Flag 2: kitty's `tq` cases, and Enter, Tab and Backspace, which keep
/// their bytes and are not released without flag 8.
#[test]
fn event_types_as_kittys_tests() {
    // Under flag 2 alone a key without text: a press as a legacy one, the
    // rest as escape codes.
    let a = key(KeyCode::Char('a'), C);
    assert_eq!(enc(2, &a), r"\x01");
    assert_eq!(enc(2, &a.clone().with_kind(REPEAT)), r"\x1b[97;5:2u");
    assert_eq!(enc(2, &a.with_kind(RELEASE)), r"\x1b[97;5:3u");
    // A key that types text is typed, repeat and all, and its release is
    // reported. This is real kitty, whose terminal sends a key with text
    // as that text on press and repeat (`encode_glfw_key_event`'s
    // SEND_TEXT_TO_CHILD) and only the release through its encoder; kitty's
    // `tq` cases pass no text, so there `a` repeats as `CSI 97;1:2u`.
    let text_a = ch('a', NONE);
    assert_eq!(enc(2, &text_a), "a");
    assert_eq!(enc(2, &text_a.clone().with_kind(REPEAT)), "a");
    assert_eq!(enc(2, &text_a.with_kind(RELEASE)), r"\x1b[97;1:3u");
    let shift_a = ch('a', S).with_shifted(Some('A'));
    assert_eq!(enc(2, &shift_a.clone().with_kind(RELEASE)), r"\x1b[97;2:3u");
    let bs = key(KeyCode::Backspace, NONE);
    assert_eq!(enc(1 | 2, &bs), r"\x7f");
    assert_eq!(enc(1 | 2, &bs.with_kind(RELEASE)), "");
    let locks = KeyEventState::NUM_LOCK | KeyEventState::CAPS_LOCK;
    let enter = key(KeyCode::Enter, NONE).with_state(locks);
    assert_eq!(enc(1 | 2, &enter), r"\r");
    assert_eq!(enc(1 | 2, &enter.with_kind(RELEASE)), "");
    // Escape keeps its byte under flag 2 alone; its release is nothing.
    let esc = key(KeyCode::Esc, NONE);
    assert_eq!(enc(2, &esc), r"\x1b");
    assert_eq!(enc(2, &esc.with_kind(RELEASE)), "");
    // A modified Enter is an escape code, released as one.
    let ctrl_enter = key(KeyCode::Enter, C).with_kind(RELEASE);
    assert_eq!(enc(1 | 2, &ctrl_enter), r"\x1b[13;5:3u");
    // A cursor key's 1 is written once anything follows it.
    let up = key(KeyCode::Up, NONE).with_kind(RELEASE);
    assert_eq!(enc(1 | 2, &up), r"\x1b[1;1:3A");
}

/// Flag 4: kitty's `aq` cases, under flag 8 where a plain key is an
/// escape code (with flag 4 alone it would be typed, as the spec has it).
#[test]
fn alternate_keys_as_kittys_tests() {
    assert_eq!(enc(4 | 8, &ch('a', NONE)), r"\x1b[97u");
    assert_eq!(
        enc(4 | 8, &ch('a', NONE).with_shifted(Some('A'))),
        r"\x1b[97u"
    );
    assert_eq!(
        enc(4 | 8, &ch('a', S).with_shifted(Some('A'))),
        r"\x1b[97:65;2u"
    );
    assert_eq!(
        enc(
            4 | 8,
            &ch('a', NONE).with_base_layout(Some(KeyCode::Char('A')))
        ),
        r"\x1b[97::65u"
    );
    let both = ch('a', S)
        .with_shifted(Some('A'))
        .with_base_layout(Some(KeyCode::Char('b')));
    assert_eq!(enc(4 | 8, &both), r"\x1b[97:65:98;2u");
    // Flag 4 alone changes no key's form: Shift+a is typed.
    assert_eq!(enc(4, &ch('a', S).with_shifted(Some('A'))), "A");
    // Functional keys have no alternates.
    assert_eq!(enc(1 | 4, &key(KeyCode::Up, S)), r"\x1b[1;2A");
}

/// Flag 8: kitty's `kq` cases; every key an escape code, the modifier keys
/// among them.
#[test]
fn all_keys_as_kittys_tests() {
    assert_eq!(enc(8, &ch('a', NONE)), r"\x1b[97u");
    assert_eq!(enc(8, &ch('a', NONE).with_kind(REPEAT)), r"\x1b[97u");
    assert_eq!(enc(8, &ch('a', C)), r"\x1b[97;5u");
    assert_eq!(enc(8, &key(KeyCode::Up, NONE)), r"\x1b[A");
    let left_shift = key(KeyCode::Modifier(ModifierKeyCode::LeftShift), NONE);
    assert_eq!(enc(8, &left_shift), r"\x1b[57441u");
    assert_eq!(enc(1, &left_shift), "", "only flag 8 reports it");
    assert_eq!(enc(8, &key(KeyCode::Enter, NONE)), r"\x1b[13u");
    assert_eq!(enc(8, &key(KeyCode::Enter, C)), r"\x1b[13;5u");
    assert_eq!(enc(8, &key(KeyCode::Tab, NONE)), r"\x1b[9u");
    assert_eq!(enc(8, &key(KeyCode::Backspace, NONE)), r"\x1b[127u");
    // With flag 2, Enter's release too, and a letter's.
    let enter = key(KeyCode::Enter, NONE).with_kind(RELEASE);
    assert_eq!(enc(2 | 8, &enter), r"\x1b[13;1:3u");
    assert_eq!(enc(2 | 8, &ch('w', NONE)), r"\x1b[119u");
    assert_eq!(
        enc(2 | 8, &ch('w', NONE).with_kind(REPEAT)),
        r"\x1b[119;1:2u"
    );
    // The lock modifiers are reported on every key.
    let caps = ch('a', NONE).with_state(KeyEventState::CAPS_LOCK);
    assert_eq!(enc(8, &caps), r"\x1b[97;65u");
    assert_eq!(enc(1, &caps), "A", "typed, without the lock, under flag 1");
}

/// Flag 16: kitty's `eq` cases, under flag 8 as the spec requires; text
/// the client's terminal did not report is the character it typed, where
/// known.
#[test]
fn associated_text_as_kittys_tests() {
    let a = ch('a', NONE).with_text(Some("a".into()));
    assert_eq!(enc(8 | 16, &a), r"\x1b[97;;97u");
    let shift_a = ch('a', S).with_text(Some("A".into()));
    assert_eq!(enc(8 | 16, &shift_a), r"\x1b[97;2;65u");
    let ab = ch('a', S).with_text(Some("AB".into()));
    assert_eq!(enc(8 | 16, &ab), r"\x1b[97;2;65:66u");
    // From a terminal that reported no text: the character typed.
    assert_eq!(enc(8 | 16, &ch('a', NONE)), r"\x1b[97;;97u");
    let legacy_shift_a = ch('a', S).with_shifted(Some('A'));
    assert_eq!(enc(8 | 16, &legacy_shift_a), r"\x1b[97;2;65u");
    // No text for a chord, nor for a key that types none.
    assert_eq!(enc(8 | 16, &ch('a', C)), r"\x1b[97;5u");
    assert_eq!(enc(8 | 16, &key(KeyCode::Enter, NONE)), r"\x1b[13u");
    // Control characters are never text.
    let control = ch('a', NONE).with_text(Some("\x01".into()));
    assert_eq!(enc(8 | 16, &control), r"\x1b[97u");
    // Flag 16 means nothing without flag 8.
    assert_eq!(enc(1 | 16, &ch('a', C)), r"\x1b[97;5u");
    assert_eq!(enc(1 | 16, &ch('a', NONE)), "a");
    // With every flag: the key, its alternates, the event and the text.
    let all = ch('a', S)
        .with_shifted(Some('A'))
        .with_text(Some("A".into()))
        .with_kind(RELEASE);
    assert_eq!(enc(31, &all), r"\x1b[97:65;2:3;65u");
}

/// A legacy client's keys reach a flag-8 pane from what it reported: its
/// characters (an upper-case letter as the key with Shift and its shifted
/// key), its control bytes as Ctrl chords, its `ESC` prefix as Alt.
#[test]
fn a_legacy_clients_keys_under_every_flag() {
    let legacy = |code, m| key(code, m).with_encoding(KeyEncoding::Legacy);
    let upper_a = legacy(KeyCode::Char('a'), S).with_shifted(Some('A'));
    assert_eq!(enc(8, &upper_a), r"\x1b[97;2u");
    assert_eq!(enc(4 | 8, &upper_a), r"\x1b[97:65;2u");
    assert_eq!(enc(31, &upper_a), r"\x1b[97:65;2;65u");
    assert_eq!(enc(8, &legacy(KeyCode::Char('a'), C)), r"\x1b[97;5u");
    assert_eq!(enc(8, &legacy(KeyCode::Char('x'), A)), r"\x1b[120;3u");
    assert_eq!(enc(8, &legacy(KeyCode::Enter, NONE)), r"\x1b[13u");
    // A legacy terminal's 0x1c, read as Ctrl+4, is Ctrl+\ to tmux.
    assert_eq!(enc(8, &legacy(KeyCode::Char('4'), C)), r"\x1b[92;5u");
    // F13 and up have numbers of their own.
    assert_eq!(enc(8, &key(KeyCode::F(13), NONE)), r"\x1b[57376u");
    assert_eq!(enc(1, &key(KeyCode::F(3), NONE)), r"\x1b[13~");
    assert_eq!(enc(1, &key(KeyCode::F(1), NONE)), r"\x1b[P");
}

/// A chord keeps its modifiers whatever text the terminal sent with it: an
/// Alt chord is not typed as its text, nor loses Alt to it.
#[test]
fn a_chord_with_text_keeps_its_modifiers() {
    let alt_a = ch('a', A).with_text(Some("å".into()));
    assert_eq!(enc(1, &alt_a), r"\x1b[97;3u");
    assert_eq!(enc(8 | 16, &alt_a), r"\x1b[97;3u");
    // Text with no chord is still the text.
    let shift_a = ch('a', S).with_text(Some("A".into()));
    assert_eq!(enc(1, &shift_a), "A");
}

/// Under flag 2 alone an ASCII chord keeps its legacy form where kitty's
/// encode_printable_ascii_key_legacy gives one: Shift takes the shifted
/// key but for a letter with Ctrl, Ctrl maps through the spec's table, and
/// Ctrl+Shift is legacy only on Space; a key off ASCII takes its
/// base-layout key's form for Ctrl or Alt.
#[test]
fn flag_2_alone_keeps_kittys_legacy_chords() {
    assert_eq!(enc(2, &ch(' ', C | S)), r"\x00");
    assert_eq!(enc(2, &ch(' ', A | S)), r"\x1b ");
    let shifted = |c, s, m| ch(c, m).with_shifted(Some(s));
    assert_eq!(enc(2, &shifted('2', '@', C | S)), r"\x00");
    assert_eq!(enc(2, &shifted('6', '^', C | S)), r"\x1e");
    // `#` is not in the table, so Ctrl leaves it.
    assert_eq!(enc(2, &shifted('3', '#', C | S)), "#");
    assert_eq!(enc(2, &shifted('a', 'A', C | S)), r"\x1b[97;6u");
    assert_eq!(enc(2, &shifted('a', 'A', A | S)), r"\x1bA");
    assert_eq!(enc(2, &shifted('1', '!', A | S)), r"\x1b!");
    // Kitty's legacy-letter cases with Ctrl: `2` NUL, `8` DEL, `9` itself.
    assert_eq!(enc(2, &ch('2', C)), r"\x00");
    assert_eq!(enc(2, &ch('8', C)), r"\x7f");
    assert_eq!(enc(2, &ch('9', C)), "9");
    assert_eq!(enc(2, &ch('a', C | A)), r"\x1b\x01");
    // Ctrl+С on a Cyrillic layout is Ctrl+c's byte; without its
    // base-layout key it has no legacy form.
    let es = ch('с', C).with_base_layout(Some(KeyCode::Char('c')));
    assert_eq!(enc(2, &es), r"\x03");
    assert_eq!(enc(2, &ch('с', C)), r"\x1b[1089;5u");
    // Super has no legacy form.
    assert_eq!(enc(2, &ch('a', KeyModifiers::SUPER)), r"\x1b[97;9u");
}
