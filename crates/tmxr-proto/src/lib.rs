//! Client/server wire protocol for tmxr.
//!
//! Every message is a little-endian `u32` length followed by a `postcard`
//! body ([`codec`]). A connection starts with a [`Hello`] each way, then the
//! client sends one [`ClientMsg::Command`]; an attach command turns the
//! connection into a long-lived attached client, anything else is answered
//! with one [`ServerMsg::CommandResult`]. See `docs/plan/03-protocol.md`.

pub mod codec;
pub mod socket;

use serde::{Deserialize, Serialize};

pub use codec::{ProtoError, read_msg, write_msg};

/// Bumped on any change to a type in this module. Client and server refuse to
/// talk across a mismatch rather than mis-decode each other.
pub const PROTOCOL_VERSION: u32 = 15;

/// How long a client waits for a server it started to accept connections.
pub const SPAWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The most bytes of standard input one [`ClientMsg::Stdin`] carries, and
/// of a reply's output one [`ServerMsg::Output`] carries, so input or a
/// reply of any size stays under [`codec::MAX_FRAME`].
pub const STDIN_PIECE: usize = 1 << 20;

/// First message a client sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    /// `CARGO_PKG_VERSION` of the sender, for error messages only.
    pub version: String,
    /// The client's working directory: the default `-c` for new sessions.
    pub cwd: String,
    /// The client's process id (`#{client_pid}`).
    pub pid: u32,
    /// Environment variables the server may copy into new panes
    /// (`update-environment`), plus `TMXR_PANE` for command clients.
    pub env: Vec<(String, String)>,
    /// Present when the client runs in a terminal and can attach.
    pub terminal: Option<TerminalInfo>,
}

/// The attaching client's terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInfo {
    pub cols: u16,
    pub rows: u16,
    /// `$TERM` of the outer terminal.
    pub term: String,
    /// The client can suspend itself (`suspend-client`): Unix job control.
    pub job_control: bool,
    /// The terminal's device (`#{client_tty}`), empty where it has none.
    pub tty: String,
    /// The client reads its terminal's answers to colour queries (OSC 4,
    /// 10, 11) while attached, so the server may send it those queries.
    pub colour_answers: bool,
}

/// What the attached client's terminal said of its keyboard when asked
/// ([`ClientMsg::Keyboard`]). The server decides from it, with its options,
/// which keyboard protocol the terminal is asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardReport {
    /// It speaks the kitty keyboard protocol: it answered the protocol's
    /// query before DA1.
    pub kitty: bool,
    /// Its name and version, as it answered XTVERSION (`XTerm(390)`,
    /// `foot(1.20.2)`), as tmux reads it to tell its features.
    pub version: Option<String>,
    /// Its terminal type, the first parameter of its secondary device
    /// attributes (DA2: 77 for mintty), as tmux also reads it.
    pub secondary: Option<u16>,
    /// The client can turn xterm's modifyOtherKeys on in its terminal and
    /// read the keys it then sends (`ServerMsg::ExtendedKeys`).
    pub modify_other_keys: bool,
}

/// Messages from a client to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello(Hello),
    /// A command line in tmux's language, already split into arguments
    /// (`["split-window", "-h"]`). Empty means the default command.
    Command(Vec<String>),
    /// Input events from the attached client's terminal (keys, mouse,
    /// paste, focus, resize, its colour reports) as ttyr's events: those
    /// the client read together, in order, as tmux takes one read of its
    /// terminal.
    Input(Vec<ttyr::event::Event>),
    /// The default foreground and background colours its terminal said it
    /// has when asked (OSC 10 / 11) at attach, as RGB, each `None` if it did
    /// not say; for programs' colour queries.
    TerminalColours {
        fg: Option<u32>,
        bg: Option<u32>,
    },
    /// What the attached client's terminal said of its keyboard when
    /// asked: sent after attaching, and again when asked after a suspend
    /// or a lock, if it changed. Until it comes the server takes the
    /// terminal as one that speaks neither the kitty protocol nor
    /// modifyOtherKeys.
    Keyboard(KeyboardReport),
    /// The client wants to detach (sent on terminal hangup).
    Detach,
    /// The client is back from `suspend-client` or a lock; its terminal was
    /// handed to another program meanwhile and needs a full redraw.
    Resumed,
    /// The lock command could not run or did not exit cleanly: why. The
    /// client has its terminal back, as after [`ClientMsg::Resumed`].
    LockFailed(String),
    /// The client's standard input, sent before a [`ClientMsg::Command`]
    /// that reads it (`load-buffer -`), in pieces of at most
    /// [`STDIN_PIECE`] bytes, which the server joins.
    Stdin(Vec<u8>),
}

/// Messages from the server to a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMsg {
    Hello {
        protocol: u32,
        version: String,
        pid: u32,
    },
    /// The client is now attached; terminal output follows.
    Attached,
    /// Bytes for the attached client's terminal.
    Output(Vec<u8>),
    /// Reply to a non-attaching command; the connection closes after it.
    CommandResult {
        status: i32,
        stdout: String,
        stderr: String,
    },
    /// The attached client is detached; it should restore its terminal, print
    /// the reason and exit, first sending its parent process SIGHUP when
    /// `hangup` is set (`detach-client -P`). With `exec` (`detach-client -E`)
    /// it prints nothing and runs that argv in its place, as tmux's
    /// MSG_EXEC.
    Detached {
        reason: String,
        hangup: bool,
        exec: Option<Vec<String>>,
    },
    /// Turn the attached client's mouse capture on or off (the `mouse`
    /// option); with it off, the terminal's own selection works unshifted.
    Mouse(bool),
    /// Ask the attached client's terminal for focus reports or stop (the
    /// `focus-events` option), as tmux's tty_start_tty asks.
    FocusEvents(bool),
    /// The kitty keyboard flags the attached client's terminal should
    /// report keys under now: tmxr's own and those of the program its keys
    /// go to, or none under `kitty-keys off`. Sent only to a client that
    /// said its terminal speaks the protocol ([`KeyboardReport::kitty`]), at once
    /// when it says so and on each change after; it asks its terminal for
    /// them in place of those it asked for before, and for none until told.
    KeyboardFlags(u8),
    /// Turn xterm's modifyOtherKeys (level 2, `CSI > 4 ; 2 m`) on in the
    /// attached client's terminal, or off (`CSI > 4 m`), as tmux's `Eneks`
    /// and `Dseks`: on where its terminal has tmux's `extkeys` feature,
    /// does not get the kitty protocol, and `extended-keys` is not `off`.
    /// Sent only to a client that said it can
    /// ([`KeyboardReport::modify_other_keys`]); one mode for the whole
    /// attach, which the client turns off on detach and around a suspend
    /// or a lock, and on again after.
    ExtendedKeys(bool),
    /// Stop the client process (`suspend-client`), as `C-z` stops a program.
    Suspend,
    /// Lock the client (`lock-client`): hand the terminal to this shell
    /// command (`lock-command`) and take it back when the command exits.
    Lock { command: String },
}
