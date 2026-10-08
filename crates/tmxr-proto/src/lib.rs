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
pub const PROTOCOL_VERSION: u32 = 2;

/// First message a client sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    /// `CARGO_PKG_VERSION` of the sender, for error messages only.
    pub version: String,
    /// The client's working directory: the default `-c` for new sessions.
    pub cwd: String,
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
}

/// Messages from a client to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello(Hello),
    /// A command line in tmux's language, already split into arguments
    /// (`["split-window", "-h"]`). Empty means the default command.
    Command(Vec<String>),
    /// An input event from the attached client's terminal (keys, mouse,
    /// paste, focus, resize), decoded by crossterm on the client.
    Input(crossterm::event::Event),
    /// The client wants to detach (sent on terminal hangup).
    Detach,
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
    /// the reason and exit.
    Detached { reason: String },
    /// Turn the attached client's mouse capture on or off (the `mouse`
    /// option); with it off, the terminal's own selection works unshifted.
    Mouse(bool),
}
