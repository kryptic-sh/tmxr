//! One tmxr pane's terminal.
//!
//! - [`child`]: a `Command` for other child processes that opens no console
//!   window on Windows.
//! - [`env`]: this process's environment as text, without panicking on a
//!   variable that is not valid Unicode.
//! - [`pty`]: spawn a program on a pseudo-terminal (openpty / ConPTY) and
//!   stream its output and exit as [`pty::PtyEvent`]s.
//! - [`modes`]: split a program's switches to the alternate screen from
//!   its output, to make them as tmux does or drop them.
//! - [`emulator`]: the per-pane `vt100` screen plus the side channels vt100
//!   leaves to its host (title, bell, OSC 52, OSC 7, replies to terminal
//!   queries, extended-key modes, tmux passthrough via [`dcs`]).
//! - [`encode`]: turn crossterm key and paste events into the bytes the
//!   program in the pane expects, given the modes it has set.
//!
//! See `docs/plan/04-panes-and-terminal.md`.

pub mod child;
pub mod dcs;
pub mod emulator;
pub mod encode;
pub mod env;
pub mod modes;
pub mod process;
pub mod pty;

pub use emulator::{Emulator, screen_size};
pub use encode::{ExtendedKeys, InputModes, conpty_key, encode_key, encode_paste};
pub use pty::{Pty, PtyEvent, SpawnSpec};
