//! tmux-compatible command language for tmxr.
//!
//! - [`tokenize`]: split a command line into commands and words, with tmux's
//!   quoting rules.
//! - [`table`]: the command table and argument checking.
//! - [`args`]: getopt-style flag parsing.
//! - [`keys`]: tmux key names.
//! - [`bind_key`]: what a key table binds, keys or mouse keys.
//! - [`format`]: `#{…}` format expansion and `#[…]` style runs.
//! - [`template`]: answers put into a command template's `%%` and `%1`.
//!
//! See `docs/plan/10-config-and-commands.md`.

pub mod args;
pub mod bind_key;
pub mod format;
pub mod keys;
pub mod localtime;
pub mod table;
pub mod template;
pub mod tokenize;

pub use args::Args;
pub use bind_key::{BindKey, MouseAction, MouseKey, MouseLocation};
pub use keys::Key;
pub use table::{
    CommandError, CommandSpec, Parsed, lookup, parse, print_command_list, reads_stdin,
    split_arguments,
};
pub use template::{fill_template, template_replace};
pub use tokenize::{escape_arg, join_args, tokenize};
