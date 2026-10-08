//! One tmxr pane's terminal.
//!
//! - [`pty`]: spawn a program on a pseudo-terminal (openpty / ConPTY) and
//!   stream its output and exit as [`pty::PtyEvent`]s.
//! - [`emulator`]: the per-pane `vt100` screen plus the side channels vt100
//!   leaves to its host (title, bell, OSC 52, OSC 7, replies to terminal
//!   queries, extended-key modes, tmux passthrough via [`dcs`]).
//! - [`encode`]: turn crossterm key and paste events into the bytes the
//!   program in the pane expects, given the modes it has set.
//!
//! See `docs/plan/04-panes-and-terminal.md`.

pub mod dcs;
pub mod emulator;
pub mod encode;
pub mod localtime;
pub mod process;
pub mod pty;

pub use emulator::Emulator;
pub use encode::{InputModes, encode_key, encode_paste};
pub use pty::{Pty, PtyEvent, SpawnSpec};
