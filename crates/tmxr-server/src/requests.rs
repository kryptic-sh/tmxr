//! Palette entries a program asked for and had not set, asked of a
//! client's terminal as tmux's input_add_request does: the pane's later
//! replies wait behind each one, so the program reads its answers in the
//! order it asked, until the terminal answers or the request times out.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crossterm::event::{ColorSlot, TerminalColor};

use crate::model::{ClientId, PaneId};
use crate::server::Server;

/// How long a request waits for the terminal's answer, as tmux's
/// INPUT_REQUEST_TIMEOUT.
pub const TIMEOUT: Duration = Duration::from_secs(2);

/// How often a client's terminal is asked for its colours on a resize,
/// as tmux's TTY_REQUEST_LIMIT.
const ASK_COLOURS_EVERY: Duration = Duration::from_secs(30);

/// What a terminal is asked for its default colours with, as tmux's
/// tty_repeat_requests.
const ASK_COLOURS: &str = "\x1b]10;?\x1b\\\x1b]11;?\x1b\\";

/// What waits to be written to a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    /// Entry `index`, asked of `client`'s terminal, the `seq`th asked.
    Palette {
        client: ClientId,
        index: u8,
        bel: bool,
        seq: u64,
        at: Instant,
    },
    /// A reply held behind a request.
    Reply(Vec<u8>),
}

/// Each pane's requests and the replies behind them.
#[derive(Debug, Default)]
pub struct Requests {
    panes: BTreeMap<PaneId, VecDeque<Pending>>,
    seq: u64,
}

impl Requests {
    /// `bytes` for `pane`: handed back to write now, or held behind its
    /// requests.
    pub fn reply(&mut self, pane: PaneId, bytes: Vec<u8>) -> Option<Vec<u8>> {
        if bytes.is_empty() {
            return None;
        }
        match self.panes.get_mut(&pane) {
            Some(queue) => {
                queue.push_back(Pending::Reply(bytes));
                None
            }
            None => Some(bytes),
        }
    }

    /// `client`'s terminal was asked for entry `index` for `pane`.
    pub fn ask(&mut self, pane: PaneId, client: ClientId, index: u8, bel: bool, now: Instant) {
        self.seq += 1;
        self.panes
            .entry(pane)
            .or_default()
            .push_back(Pending::Palette {
                client,
                index,
                bel,
                seq: self.seq,
                at: now,
            });
    }

    /// `client`'s terminal answered entry `index` with `rgb`, as tmux's
    /// input_request_reply: the oldest request of it for that entry gets
    /// the answer, and those asked of it before go unanswered, as the
    /// terminal answers in order. The bytes each pane can now be given.
    pub fn answered(&mut self, client: ClientId, index: u8, rgb: u32) -> Vec<(PaneId, Vec<u8>)> {
        let asked = |p: &Pending| match *p {
            Pending::Palette { client: c, seq, .. } if c == client => Some(seq),
            _ => None,
        };
        let found = self
            .panes
            .values()
            .flatten()
            .filter_map(|p| match *p {
                Pending::Palette { index: i, .. } if i == index => asked(p),
                _ => None,
            })
            .min();
        let Some(found) = found else {
            return Vec::new();
        };
        for queue in self.panes.values_mut() {
            queue.retain_mut(|p| match asked(p) {
                Some(seq) if seq < found => false,
                Some(seq) if seq == found => {
                    let Pending::Palette { bel, .. } = *p else {
                        unreachable!("asked names requests only");
                    };
                    let end = if bel { "\x07" } else { "\x1b\\" };
                    *p = Pending::Reply(tmxr_term::palette::answer(index, rgb, end));
                    true
                }
                _ => true,
            });
        }
        self.release()
    }

    /// Drop the requests asked before `now - TIMEOUT`, as tmux's
    /// input_request_timer_callback; the bytes each pane can now be given.
    pub fn expire(&mut self, now: Instant) -> Vec<(PaneId, Vec<u8>)> {
        for queue in self.panes.values_mut() {
            queue.retain(|p| !matches!(p, Pending::Palette { at, .. } if now >= *at + TIMEOUT));
        }
        self.release()
    }

    /// When the oldest request times out.
    pub fn next_due(&self) -> Option<Instant> {
        self.panes
            .values()
            .flatten()
            .filter_map(|p| match p {
                Pending::Palette { at, .. } => Some(*at + TIMEOUT),
                Pending::Reply(_) => None,
            })
            .min()
    }

    /// The replies no request holds back any longer, each pane's joined in
    /// order; a pane left with nothing waiting is forgotten.
    fn release(&mut self) -> Vec<(PaneId, Vec<u8>)> {
        let mut out = Vec::new();
        self.panes.retain(|&pane, queue| {
            let mut bytes = Vec::new();
            while let Some(Pending::Reply(_)) = queue.front() {
                if let Some(Pending::Reply(b)) = queue.pop_front() {
                    bytes.extend(b);
                }
            }
            if !bytes.is_empty() {
                out.push((pane, bytes));
            }
            !queue.is_empty()
        });
        out
    }
}

impl Server {
    /// Write `bytes` to pane `pid`'s program, behind any request it waits
    /// on.
    pub fn reply_to_pane(&mut self, pid: PaneId, bytes: Vec<u8>) {
        if let Some(bytes) = self.colour_requests.reply(pid, bytes) {
            self.write_to_pane(pid, &bytes);
        }
    }

    /// Ask for palette entry `index` (its answer ended by a BEL where
    /// `bel`) of the terminal of the client on pane `pid`'s window with the
    /// latest input, as tmux's input_add_request; with no such client it
    /// goes unanswered.
    pub fn ask_palette(&mut self, pid: PaneId, index: u8, bel: bool) {
        let client = self
            .clients_on(pid)
            .filter(|c| Self::takes_colour_answers(c))
            .max_by_key(|c| c.att.as_ref().map(|a| a.last_input))
            .map(|c| c.id);
        let Some(client) = client else {
            return;
        };
        self.push_to_terminal(client, &format!("\x1b]4;{index};?\x1b\\"));
        self.colour_requests
            .ask(pid, client, index, bel, Instant::now());
    }

    /// Client `id`'s terminal reported one of its colours.
    pub fn terminal_colour(&mut self, id: ClientId, colour: TerminalColor) {
        let (r, g, b) = colour.rgb;
        let rgb = u32::from(r >> 8) << 16 | u32::from(g >> 8) << 8 | u32::from(b >> 8);
        match colour.slot {
            ColorSlot::Palette(index) => {
                let released = self.colour_requests.answered(id, index, rgb);
                self.write_released(released);
            }
            ColorSlot::Foreground | ColorSlot::Background => {
                if let Some(c) = self.clients.get_mut(&id) {
                    if colour.slot == ColorSlot::Foreground {
                        c.terminal_colours.0 = Some(rgb);
                    } else {
                        c.terminal_colours.1 = Some(rgb);
                    }
                }
            }
            // Not asked for.
            ColorSlot::Cursor => {}
        }
    }

    /// Ask client `id`'s terminal for its default colours again, as tmux's
    /// tty_repeat_requests: always where `force`, else only once
    /// [`ASK_COLOURS_EVERY`] has passed since it was last asked.
    pub fn ask_terminal_colours(&mut self, id: ClientId, force: bool) {
        let now = Instant::now();
        let Some(c) = self.clients.get_mut(&id) else {
            return;
        };
        if !Self::takes_colour_answers(c)
            || !force && c.colours_asked.is_some_and(|t| now < t + ASK_COLOURS_EVERY)
        {
            return;
        }
        c.colours_asked = Some(now);
        self.push_to_terminal(id, ASK_COLOURS);
    }

    /// Drop the palette requests no terminal answered in time.
    pub fn expire_colour_requests(&mut self) {
        let released = self.colour_requests.expire(Instant::now());
        self.write_released(released);
    }

    fn write_released(&mut self, released: Vec<(PaneId, Vec<u8>)>) {
        for (pid, bytes) in released {
            self.write_to_pane(pid, &bytes);
        }
    }

    fn write_to_pane(&mut self, pid: PaneId, bytes: &[u8]) {
        if let Some(p) = self.panes.get_mut(&pid) {
            let _ = p.pty.write(bytes);
        }
    }

    /// An attached client whose terminal's answers to colour queries come
    /// back to the server.
    fn takes_colour_answers(c: &crate::server::Client) -> bool {
        c.att
            .as_ref()
            .is_some_and(|a| a.term.is_some() && !a.locked)
            && c.hello
                .as_ref()
                .and_then(|h| h.terminal.as_ref())
                .is_some_and(|t| t.colour_answers)
    }

    /// Bytes for client `id`'s terminal, outside its frames.
    pub(crate) fn push_to_terminal(&mut self, id: ClientId, seq: &str) {
        if let Some(t) = self
            .clients
            .get_mut(&id)
            .and_then(|c| c.att.as_mut())
            .and_then(|a| a.term.as_mut())
        {
            t.backend_mut().push_raw(seq);
        }
        self.mark_client_dirty(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: u32 = 0xcd0000;

    #[test]
    fn replies_wait_behind_a_request_until_it_is_answered() {
        let (pane, client) = (1, 7);
        let mut r = Requests::default();
        assert_eq!(r.reply(pane, b"early".to_vec()), Some(b"early".to_vec()));
        r.ask(pane, client, 1, true, Instant::now());
        assert_eq!(r.reply(pane, b"DA1".to_vec()), None, "held behind it");
        // Another client's answer, or another entry's, is not this one's.
        assert!(r.answered(8, 1, RED).is_empty());
        assert!(r.answered(client, 2, RED).is_empty());
        assert_eq!(
            r.answered(client, 1, RED),
            [(pane, b"\x1b]4;1;rgb:cdcd/0000/0000\x07DA1".to_vec())]
        );
        assert_eq!(r.reply(pane, b"late".to_vec()), Some(b"late".to_vec()));
        assert_eq!(r.next_due(), None);
    }

    #[test]
    fn an_answer_skips_the_requests_asked_before_it() {
        let (pane, client) = (1, 7);
        let mut r = Requests::default();
        let now = Instant::now();
        r.ask(pane, client, 1, false, now);
        r.ask(pane, client, 2, false, now);
        // The terminal did not answer entry 1, and answered 2.
        assert_eq!(
            r.answered(client, 2, RED),
            [(pane, b"\x1b]4;2;rgb:cdcd/0000/0000\x1b\\".to_vec())]
        );
        assert_eq!(r.next_due(), None);
    }

    #[test]
    fn a_request_unanswered_in_time_lets_the_replies_go() {
        let (pane, client) = (1, 7);
        let mut r = Requests::default();
        let now = Instant::now();
        r.ask(pane, client, 1, true, now);
        assert_eq!(r.reply(pane, b"DA1".to_vec()), None);
        assert_eq!(r.next_due(), Some(now + TIMEOUT));
        assert!(r.expire(now + TIMEOUT / 2).is_empty());
        assert_eq!(r.expire(now + TIMEOUT), [(pane, b"DA1".to_vec())]);
        assert!(r.answered(client, 1, RED).is_empty(), "too late");
    }
}
