//! The attached client's terminal: modes in, modes out, input pump.

mod guard;
mod input;
mod keys;
mod probe;
mod process;

pub use guard::{Guard, set_focus_events, set_mouse};
pub use input::{Input, Sender, forward, spawn_input};
pub use keys::Keys;
pub use process::{exec, hang_up_parent, run_lock, suspend, tty_name};
