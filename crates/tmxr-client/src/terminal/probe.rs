//! Asking the terminal what it speaks, and reading its answers.

use std::io::{self, Write};
#[cfg(unix)]
use std::time::{Duration, Instant};

use ttyr::event::Event;
// Read in the answers to the probe, which only Unix asks.
#[cfg(any(unix, test))]
use ttyr::event::{ColorSlot, ModeSetting, Report};

use super::Keys;

/// What the terminal said when asked: whether it takes synchronized
/// updates (DEC mode 2026), its default foreground and background colours
/// (OSC 10 / 11) as RGB, whether it speaks the kitty keyboard protocol, and
/// its name and version (XTVERSION).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Probe {
    pub(super) sync: bool,
    pub(super) fg: Option<u32>,
    pub(super) bg: Option<u32>,
    pub(super) kitty: bool,
    /// Kept for plan 18 step 5, whose xterm tier goes by the terminal's
    /// name; nothing reads it yet.
    version: Option<String>,
}

// Only Unix asks; built for its tests everywhere.
#[cfg(any(unix, test))]
impl Probe {
    /// The answers among `read`, the events read while the terminal
    /// answered, up to (not including) its answer to DA1; and the rest of
    /// `read` in the order they came, input for the server (keys typed, a
    /// resize, a report the probe did not ask for). `complete` is whether
    /// DA1's answer came: if not, the terminal answered too slowly (over
    /// SSH, say), and `last`, what it said when last asked, if it was, is
    /// kept rather than taken from what part of its answers came in time.
    fn read(read: Vec<Event>, complete: bool, last: Option<&Self>) -> (Self, Vec<Event>) {
        let mut probe = Self::default();
        let mut others = Vec::new();
        for event in read {
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
                event => others.push(event),
            }
        }
        match last {
            Some(last) if !complete => (last.clone(), others),
            _ => (probe, others),
        }
    }
}

/// Synchronized output's DEC private mode.
#[cfg(any(unix, test))]
const SYNC_MODE: u16 = 2026;

/// How long the probe waits for the terminal's answers.
#[cfg(unix)]
const PROBE_WAIT: Duration = Duration::from_secs(1);

impl Keys {
    /// Ask the terminal what [`Probe`] holds and read its answers, as tmux
    /// asks at attach. Returns them with the events read meanwhile (keys
    /// typed, a resize, a report not asked for), in the order they came,
    /// which are input like any other.
    #[cfg(unix)]
    pub(super) fn probe(&mut self, out: &mut impl Write) -> io::Result<(Probe, Vec<Event>)> {
        use ttyr::terminal::{
            RequestKeyboardEnhancementFlags, RequestMode, RequestPrimaryDeviceAttributes,
            RequestTerminalColor, RequestTerminalVersion,
        };
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
            RequestTerminalVersion,
            RequestPrimaryDeviceAttributes,
        )?;
        // Read here rather than with ttyr's `collect_answers`, which keeps
        // the reports apart from the other events and so loses which came
        // first, a key or a report the probe did not ask for.
        let deadline = Instant::now() + PROBE_WAIT;
        let mut read = Vec::new();
        let mut complete = false;
        while self
            .reader
            .poll(deadline.saturating_duration_since(Instant::now()))?
        {
            match self.reader.read()? {
                Event::Report(Report::PrimaryDeviceAttributes(_)) => {
                    complete = true;
                    break;
                }
                Event::Wake => {}
                event => read.push(event),
            }
        }
        let (probe, events) = Probe::read(read, complete, self.last.as_ref());
        self.last = Some(probe.clone());
        Ok((probe, events))
    }

    /// The Windows console hands over input records, so it is not asked;
    /// whether it speaks the kitty protocol is crossterm's answer.
    #[cfg(windows)]
    pub(super) fn probe(&mut self, _out: &mut impl Write) -> io::Result<(Probe, Vec<Event>)> {
        let kitty = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
        Ok((
            Probe {
                kitty,
                ..Probe::default()
            },
            Vec::new(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::keys::tests::key;
    use ttyr::event::{
        ColorScheme, KeyCode, KeyModifiers, KeyboardEnhancementFlags, TerminalColor,
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

    /// What a foot-like terminal answers, all of it.
    fn foot() -> Probe {
        Probe {
            sync: true,
            fg: Some(0xc0caf5),
            bg: Some(0x1a1b26),
            kitty: true,
            version: Some("foot(1.20.2)".into()),
        }
    }

    #[test]
    fn the_probe_takes_its_answers_and_hands_on_the_rest_in_order() {
        let scheme = Event::Report(Report::ColorScheme(ColorScheme::Dark));
        let x = key(KeyCode::Char('x'), KeyModifiers::NONE);
        let (probe, others) = Probe::read(
            vec![
                sync(ModeSetting::Reset),
                x.clone(),
                colour(ColorSlot::Foreground, (0xc0c0, 0xcaca, 0xf5f5)),
                scheme.clone(),
                colour(ColorSlot::Background, (0x1a1a, 0x1b1b, 0x2626)),
                Event::Resize(80, 24),
                kitty(),
                Event::Report(Report::TerminalVersion("foot(1.20.2)".into())),
            ],
            true,
            None,
        );
        assert_eq!(probe, foot());
        // What the probe did not ask for, in the order it came.
        assert_eq!(others, vec![x, scheme, Event::Resize(80, 24)]);
    }

    #[test]
    fn a_probe_answered_too_late_keeps_what_the_terminal_said_before() {
        // Asked again (after a suspend or a lock) over a slow link: only
        // DECRPM came in time, and nothing is taken from it.
        let late = vec![sync(ModeSetting::NotRecognized)];
        assert_eq!(
            Probe::read(late.clone(), false, Some(&foot())),
            (foot(), Vec::new())
        );
        // Answered in time, the new answers stand.
        assert_eq!(
            Probe::read(late.clone(), true, Some(&foot())).0,
            Probe::default()
        );
        // Asked for the first time, what came in time is all there is.
        let (first, _) = Probe::read(vec![kitty()], false, None);
        assert!(first.kitty && !first.sync);
    }

    #[test]
    fn the_probe_says_no_to_what_the_terminal_did_not_answer() {
        // A terminal that ignores every query but DA1.
        assert_eq!(
            Probe::read(Vec::new(), true, None),
            (Probe::default(), Vec::new())
        );
        // Synchronized output not recognized, or permanently reset.
        for setting in [ModeSetting::NotRecognized, ModeSetting::PermanentlyReset] {
            assert!(
                !Probe::read(vec![sync(setting)], true, None).0.sync,
                "{setting:?}"
            );
        }
        for setting in [
            ModeSetting::Set,
            ModeSetting::Reset,
            ModeSetting::PermanentlySet,
        ] {
            assert!(
                Probe::read(vec![sync(setting)], true, None).0.sync,
                "{setting:?}"
            );
        }
    }
}
