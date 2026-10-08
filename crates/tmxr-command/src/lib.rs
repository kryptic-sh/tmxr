//! tmux-compatible command language for tmxr.
//!
//! - [`tokenize`]: split a command line into commands and words, with tmux's
//!   quoting rules.
//! - [`table`]: the command table and argument checking.
//! - [`args`]: getopt-style flag parsing.
//! - [`keys`]: tmux key names.
//! - [`format`]: `#{…}` format expansion and `#[…]` style runs.
//!
//! See `docs/plan/10-config-and-commands.md`.

pub mod args;
pub mod format;
pub mod keys;
pub mod table;
pub mod tokenize;

pub use args::Args;
pub use keys::Key;
pub use table::{CommandError, CommandSpec, Parsed, lookup, parse};
pub use tokenize::tokenize;
