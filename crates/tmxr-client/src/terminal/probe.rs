//! Asking the terminal what it speaks, and reading its answers.

use std::io::{self, Write};
use std::time::Duration;

use ttyr::event::{ColorSlot, Event, ModeSetting, Report};
use ttyr::reader::Answers;

use super::Keys;

/// What the terminal said when asked: whether it takes synchronized
/// updates (DEC mode 2026), its default foreground and background colours
/// (OSC 10 / 11) as RGB, whether it speaks the kitty keyboard protocol, its
/// name and version (XTVERSION), and its terminal type (DA2's first
/// parameter). The server tells modifyOtherKeys by the last two, as tmux.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Probe {
    pub(super) sync: bool,
    pub(super) fg: Option<u32>,
    pub(super) bg: Option<u32>,
    pub(super) kitty: bool,
    pub(super) version: Option<String>,
    pub(super) secondary: Option<u16>,
}

impl Probe {
    /// The answers among `answers`, the events read while the terminal
    /// answered; and the rest in the order they came, input for the server
    /// (keys typed, a resize, a report the probe did not ask for), without
    /// the answer to DA1, which only ends the probe, or a wake. Where DA1's
    /// answer did not come, the terminal answered too slowly (over SSH,
    /// say), and `last`, what it said when last asked, if it was, is kept
    /// rather than taken from what part of its answers came in time.
    fn read(answers: Answers, last: Option<&Self>) -> (Self, Vec<Event>) {
        let mut probe = Self::default();
        let mut others = Vec::new();
        for event in answers.events {
            match event {
                Event::Report(Report::Mode {
                    mode: SYNC_MODE,
                    private: true,
                    setting,
                }) => {
                    probe.sync = matches!(
                        setting,
                        ModeSetting::Set | ModeSetting::Reset | ModeSetting::PermanentlySet
                    );
                }
                Event::Report(Report::TerminalColor(colour))
                    if colour.slot == ColorSlot::Foreground =>
                {
                    probe.fg = Some(tmxr_command::colour::from_rgb16(colour.rgb));
                }
                Event::Report(Report::TerminalColor(colour))
                    if colour.slot == ColorSlot::Background =>
                {
                    probe.bg = Some(tmxr_command::colour::from_rgb16(colour.rgb));
                }
                Event::Report(Report::KeyboardEnhancementFlags(_)) => probe.kitty = true,
                Event::Report(Report::TerminalVersion(version)) => probe.version = Some(version),
                Event::Report(Report::SecondaryDeviceAttributes(params)) => {
                    probe.secondary = params.first().copied();
                }
                Event::Report(Report::PrimaryDeviceAttributes(_)) | Event::Wake => {}
                Event::Unknown(bytes) => match unread_colour(&bytes) {
                    Some((ColorSlot::Foreground, rgb)) => probe.fg = Some(rgb),
                    Some((_, rgb)) => probe.bg = Some(rgb),
                    None => others.push(Event::Unknown(bytes)),
                },
                event => others.push(event),
            }
        }
        match last {
            Some(last) if !answers.complete => (last.clone(), others),
            _ => (probe, others),
        }
    }
}

/// An OSC 10 or 11 answer `ttyr` does not read (it reads Xlib's numeric
/// forms), read as tmux reads it (`parse_x11`), so the forms 0.15.0 took
/// still are: a name, `cmyk:`, `cmy:`, `r,g,b`. The slot and its RGB value.
fn unread_colour(bytes: &[u8]) -> Option<(ColorSlot, u32)> {
    let (slot, rest) = if let Some(rest) = bytes.strip_prefix(b"\x1b]10;") {
        (ColorSlot::Foreground, rest)
    } else {
        (ColorSlot::Background, bytes.strip_prefix(b"\x1b]11;")?)
    };
    let spec = rest
        .strip_suffix(b"\x07")
        .or_else(|| rest.strip_suffix(b"\x1b\\"))?;
    let rgb = tmxr_command::colour::parse_x11(std::str::from_utf8(spec).ok()?)?;
    Some((slot, rgb))
}

/// Synchronized output's DEC private mode.
const SYNC_MODE: u16 = 2026;

/// How long the probe waits for the terminal's answers.
pub const PROBE_WAIT: Duration = Duration::from_secs(1);

impl Keys {
    /// Ask the terminal what [`Probe`] holds and read its answers, as tmux
    /// asks at attach. Returns them with the events read meanwhile (keys
    /// typed, a resize, a report not asked for), in the order they came,
    /// which are input like any other. A Windows console handing over key
    /// records ([`Keys::reads_vt`]) is not asked: how a terminal's answers
    /// reach a program reading records through ConPTY is not known, and as
    /// keys they would be typed into a pane. It says nothing of itself, as
    /// a terminal that answers nothing.
    pub(super) fn probe(&mut self, out: &mut impl Write) -> io::Result<(Probe, Vec<Event>)> {
        use ttyr::terminal::{
            RequestKeyboardEnhancementFlags, RequestMode, RequestPrimaryDeviceAttributes,
            RequestSecondaryDeviceAttributes, RequestTerminalColor, RequestTerminalVersion,
        };
        if !self.reads_vt() {
            return Ok((Probe::default(), Vec::new()));
        }
        // DA1 last, as every terminal answers it, and in order.
        ttyr::execute!(
            out,
            RequestMode {
                mode: SYNC_MODE,
                private: true,
            },
            RequestTerminalColor(ColorSlot::Foreground),
            RequestTerminalColor(ColorSlot::Background),
            RequestKeyboardEnhancementFlags,
            RequestSecondaryDeviceAttributes,
            RequestTerminalVersion,
            RequestPrimaryDeviceAttributes,
        )?;
        let mut answers = self.reader.collect_answers(PROBE_WAIT)?;
        self.read_meta(&mut answers.events);
        let (probe, events) = Probe::read(answers, self.last.as_ref());
        self.last = Some(probe.clone());
        Ok((probe, events))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::keys::tests::key;
    use ttyr::event::{
        ColorScheme, Decoder, KeyCode, KeyModifiers, KeyboardEnhancementFlags, TerminalColor,
    };

    fn colour(slot: ColorSlot, rgb: (u16, u16, u16)) -> Event {
        Event::Report(Report::TerminalColor(TerminalColor { slot, rgb }))
    }

    fn sync(setting: ModeSetting) -> Event {
        Event::Report(Report::Mode {
            mode: SYNC_MODE,
            private: true,
            setting,
        })
    }

    fn kitty() -> Event {
        Event::Report(Report::KeyboardEnhancementFlags(
            KeyboardEnhancementFlags::empty(),
        ))
    }

    fn da1() -> Event {
        Event::Report(Report::PrimaryDeviceAttributes(vec![62]))
    }

    /// What `collect_answers` returns: `events`, and whether DA1's answer
    /// (the last of them, then) came.
    fn answers(events: Vec<Event>, complete: bool) -> Answers {
        Answers { events, complete }
    }

    /// The events `bytes` decode to, as the terminal's reader reads them.
    pub(super) fn decode(bytes: &[u8]) -> Vec<Event> {
        let mut decoder = Decoder::new();
        decoder.feed(bytes);
        let mut events: Vec<_> = std::iter::from_fn(|| decoder.next_event()).collect();
        events.extend(std::iter::from_fn(|| decoder.flush_event()));
        events
    }

    /// What a foot-like terminal answers, all of it.
    fn foot() -> Probe {
        Probe {
            sync: true,
            fg: Some(0xc0caf5),
            bg: Some(0x1a1b26),
            kitty: true,
            version: Some("foot(1.20.2)".into()),
            secondary: Some(1),
        }
    }

    #[test]
    fn the_probe_takes_its_answers_and_hands_on_the_rest_in_order() {
        let scheme = Event::Report(Report::ColorScheme(ColorScheme::Dark));
        let x = key(KeyCode::Char('x'), KeyModifiers::NONE);
        let (probe, others) = Probe::read(
            answers(
                vec![
                    sync(ModeSetting::Reset),
                    x.clone(),
                    colour(ColorSlot::Foreground, (0xc0c0, 0xcaca, 0xf5f5)),
                    scheme.clone(),
                    Event::Wake,
                    colour(ColorSlot::Background, (0x1a1a, 0x1b1b, 0x2626)),
                    Event::Resize(80, 24),
                    kitty(),
                    Event::Report(Report::SecondaryDeviceAttributes(vec![1, 12002, 0])),
                    Event::Report(Report::TerminalVersion("foot(1.20.2)".into())),
                    da1(),
                ],
                true,
            ),
            None,
        );
        assert_eq!(probe, foot());
        // What the probe did not ask for, in the order it came; not DA1's
        // answer, which only ends the probe, nor a wake.
        assert_eq!(others, vec![x, scheme, Event::Resize(80, 24)]);
    }

    #[test]
    fn a_probe_answered_too_late_keeps_what_the_terminal_said_before() {
        // Asked again (after a suspend or a lock) over a slow link: only
        // DECRPM came in time, and nothing is taken from it.
        let late = vec![sync(ModeSetting::NotRecognized)];
        assert_eq!(
            Probe::read(answers(late.clone(), false), Some(&foot())),
            (foot(), Vec::new())
        );
        // Answered in time, the new answers stand.
        let mut in_time = late.clone();
        in_time.push(da1());
        assert_eq!(
            Probe::read(answers(in_time, true), Some(&foot())).0,
            Probe::default()
        );
        // Asked for the first time, what came in time is all there is.
        let (first, _) = Probe::read(answers(vec![kitty()], false), None);
        assert!(first.kitty && !first.sync);
    }

    #[test]
    fn the_probe_says_no_to_what_the_terminal_did_not_answer() {
        // A terminal that ignores every query but DA1.
        assert_eq!(
            Probe::read(answers(vec![da1()], true), None),
            (Probe::default(), Vec::new())
        );
        let sync_answer = |setting| answers(vec![sync(setting), da1()], true);
        // Synchronized output not recognized, or permanently reset.
        for setting in [ModeSetting::NotRecognized, ModeSetting::PermanentlyReset] {
            assert!(
                !Probe::read(sync_answer(setting), None).0.sync,
                "{setting:?}"
            );
        }
        for setting in [
            ModeSetting::Set,
            ModeSetting::Reset,
            ModeSetting::PermanentlySet,
        ] {
            assert!(
                Probe::read(sync_answer(setting), None).0.sync,
                "{setting:?}"
            );
        }
    }

    /// A colour answered in any of the forms tmux reads is the colour tmux
    /// makes of it (`parse_x11` mirrors tmux's parser): each channel's high
    /// byte, whatever form and width the terminal answered in.
    #[test]
    fn colour_answers_in_each_form_are_the_colour_tmux_reads() {
        for spec in [
            "#c0caf5",
            "rgb:c0/ca/f5",
            "rgb:c0c0/caca/f5f5",
            "rgb:c0ff/ca00/f5aa",
            "#c0ffca00f5aa",
        ] {
            let want = tmxr_command::colour::parse_x11(spec);
            assert_eq!(want, Some(0xc0caf5), "{spec}");
            let bytes = format!("\x1b]10;{spec}\x1b\\\x1b]11;{spec}\x07\x1b[?62c");
            let (probe, others) = Probe::read(answers(decode(bytes.as_bytes()), true), None);
            assert_eq!((probe.fg, probe.bg), (want, want), "{spec}");
            assert_eq!(others, Vec::new(), "{spec}");
        }
    }

    /// XTVERSION's and DA2's answers as xterm writes them give the name
    /// and type the server tells modifyOtherKeys by.
    #[test]
    fn the_terminals_name_and_type_are_kept() {
        let bytes = b"\x1b[>41;390;0c\x1bP>|XTerm(390)\x1b\\\x1b[?64;1;2c";
        let (probe, others) = Probe::read(answers(decode(bytes), true), None);
        assert_eq!(probe.version.as_deref(), Some("XTerm(390)"));
        assert_eq!(probe.secondary, Some(41));
        assert!(!probe.kitty);
        assert_eq!(others, Vec::new());
    }

    #[test]
    fn colour_answers_ttyr_does_not_read_are_read_as_tmux_reads_them() {
        let bytes = b"\x1b]10;red\x07\x1b]11;cmyk:0/1/0/0\x1b\\\x1b[?62c";
        let events = decode(bytes);
        // ttyr leaves both unread, for tmxr.
        assert!(
            matches!(&events[..2], [Event::Unknown(_), Event::Unknown(_)]),
            "{events:?}"
        );
        let (probe, others) = Probe::read(answers(events, true), None);
        assert_eq!(probe.fg, tmxr_command::colour::parse_x11("red"));
        assert_eq!(probe.fg, Some(0xff0000));
        assert_eq!(probe.bg, Some(0xff00ff));
        assert_eq!(others, Vec::new());
        // Unknown bytes that are no colour answer are handed on.
        let other = Event::Unknown(b"\x1b]11;nonsense\x07".to_vec());
        let (probe, others) = Probe::read(answers(vec![other.clone(), da1()], true), None);
        assert_eq!((probe.bg, others), (None, vec![other]));
    }
}

/// The probe through `ttyr`'s reader, over a socket pair in place of a
/// terminal: its deadline, an answer cut short, and what it hands on. Unix
/// only: a socket pair stands in for a terminal only there; on Windows the
/// reader reads the console.
#[cfg(all(test, unix))]
pub(super) mod reader_tests {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::sync::{Mutex, MutexGuard, PoisonError};
    use std::time::Instant;

    use super::tests::decode;
    use super::*;
    use ttyr::event::KeyModifiers;

    /// A process has one `ttyr` reader open at a time, so these tests take
    /// turns.
    static READER: Mutex<()> = Mutex::new(());

    /// What the terminal answers in full: the kitty protocol, its
    /// foreground colour, DA1.
    const ANSWERS: &[u8] = b"\x1b[?0u\x1b]10;rgb:c0c0/caca/f5f5\x1b\\\x1b[?62c";

    /// This test's turn with the reader, held until after its [`Keys`] are
    /// dropped.
    pub(in crate::terminal) fn turn() -> MutexGuard<'static, ()> {
        READER.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keys reading one end of a socket pair, the other end the terminal's.
    pub(in crate::terminal) fn keys() -> (Keys, UnixStream) {
        let (ours, terminal) = UnixStream::pair().unwrap();
        let tty = OwnedFd::from(ours);
        let reader = ttyr::reader::Reader::from_fd(tty.try_clone().unwrap()).unwrap();
        let keys = Keys {
            reader,
            tty,
            last: None,
            kitty_keyboard: false,
        };
        (keys, terminal)
    }

    fn probe(keys: &mut Keys) -> (Probe, Vec<Event>) {
        let mut asked = Vec::new();
        let read = keys.probe(&mut asked).unwrap();
        // DA1 asked last, as the terminal answers it after the rest.
        assert!(asked.ends_with(b"\x1b[c"), "{asked:?}");
        read
    }

    #[test]
    fn keys_and_reports_typed_among_the_answers_are_handed_on_in_order() {
        let _turn = turn();
        let (mut keys, mut terminal) = keys();
        let scheme = b"\x1b[?997;1n";
        let mut bytes = b"x".to_vec();
        bytes.extend_from_slice(b"\x1b[?0u");
        bytes.extend_from_slice(scheme);
        bytes.extend_from_slice(b"y\x1b]10;rgb:c0c0/caca/f5f5\x1b\\\x1b[?62cz");
        terminal.write_all(&bytes).unwrap();
        let (probe, others) = probe(&mut keys);
        assert!(probe.kitty);
        assert_eq!(probe.fg, Some(0xc0caf5));
        let mut want = decode(b"x");
        want.extend(decode(scheme));
        want.extend(decode(b"y"));
        assert_eq!(others, want);
        // What came after DA1 is the input's, read next.
        assert_eq!(keys.reader.read().unwrap(), decode(b"z")[0]);
    }

    #[test]
    fn a_wake_during_the_probe_is_not_handed_on() {
        let _turn = turn();
        let (mut keys, mut terminal) = keys();
        // As the input thread's pause wakes the reader before lending it:
        // read, so it comes ahead of the answers.
        keys.reader.waker().wake().unwrap();
        assert!(keys.reader.poll(Duration::ZERO).unwrap());
        terminal.write_all(b"x").unwrap();
        terminal.write_all(ANSWERS).unwrap();
        let (probe, others) = probe(&mut keys);
        assert!(probe.kitty);
        assert_eq!(others, decode(b"x"));
    }

    #[test]
    fn a_probe_cut_short_keeps_the_answers_from_before() {
        let _turn = turn();
        let (mut keys, mut terminal) = keys();
        terminal.write_all(ANSWERS).unwrap();
        let (first, _) = probe(&mut keys);
        assert!(first.kitty);
        // Asked again, only DECRPM and a key come before the deadline.
        terminal.write_all(b"\x1b[?2026;2$yx").unwrap();
        let start = Instant::now();
        let (again, others) = probe(&mut keys);
        assert!(start.elapsed() >= PROBE_WAIT, "{:?}", start.elapsed());
        assert_eq!(again, first);
        assert_eq!(others, decode(b"x"));
    }

    /// The modifiers of the next key the terminal sends after `bytes`.
    fn next_key_mods(keys: &mut Keys, terminal: &mut UnixStream, bytes: &[u8]) -> KeyModifiers {
        terminal.write_all(bytes).unwrap();
        loop {
            for event in keys.read_batch().unwrap() {
                if let Event::Key(key) = event {
                    return key.modifiers;
                }
            }
        }
    }

    /// Modifier bit 8 (`CSI 1 ; 9 A`) is xterm's Meta, read as tmux reads
    /// it, Alt, until the reader is told the kitty protocol is on, and
    /// Super while it is; the terminal's answer to the flags query says
    /// which too, for the keys after it.
    #[test]
    fn bit_8_reads_as_the_reader_is_told_and_the_flags_answer_says() {
        let _turn = turn();
        let (mut keys, mut terminal) = keys();
        let meta_up = b"\x1b[1;9A";
        let mut next = |keys: &mut Keys, bytes: &[u8]| next_key_mods(keys, &mut terminal, bytes);
        assert_eq!(next(&mut keys, meta_up), KeyModifiers::ALT);
        keys.set_kitty_keyboard(true);
        assert_eq!(next(&mut keys, meta_up), KeyModifiers::SUPER);
        keys.set_kitty_keyboard(false);
        assert_eq!(next(&mut keys, meta_up), KeyModifiers::ALT);
        let mut answered = b"\x1b[?1u".to_vec();
        answered.extend_from_slice(meta_up);
        assert_eq!(next(&mut keys, &answered), KeyModifiers::SUPER);
        assert!(keys.kitty_keyboard);
        let mut answered = b"\x1b[?0u".to_vec();
        answered.extend_from_slice(meta_up);
        assert_eq!(next(&mut keys, &answered), KeyModifiers::ALT);
        assert!(!keys.kitty_keyboard);
        // kitty's own Meta (bit 32) read with the answer that turned the
        // protocol on: Meta, not folded into Alt.
        assert_eq!(next(&mut keys, b"\x1b[?1u\x1b[120;33u"), KeyModifiers::META);
        assert!(keys.kitty_keyboard);
    }
}
