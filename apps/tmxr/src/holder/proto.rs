//! The holder proof of concept's messages, one enum per direction, framed
//! with `tmxr_proto::codec`. Throwaway: no version, no capabilities. The
//! integration test `tests/holder_poc.rs` includes this file as it is.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Server to holder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToHolder {
    /// Start the program. Only the first one that succeeds counts: a holder
    /// runs one program in this proof of concept.
    Spawn {
        argv: Vec<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        rows: u16,
        cols: u16,
    },
    /// Bytes for the program.
    Input(Vec<u8>),
    Resize {
        rows: u16,
        cols: u16,
    },
    /// Send the ring's output from this number, or from the oldest byte kept
    /// if that is newer, then the program's output as it comes.
    TakeFrom(u64),
    /// End the program and exit.
    Close,
}

/// Holder to server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FromHolder {
    /// Sent once the program runs: on `Spawn`, then on every connection.
    /// `ring_start` is the oldest output byte kept, `next` the number the
    /// next byte written will have.
    Hello {
        pid: Option<u32>,
        ring_start: u64,
        next: u64,
    },
    /// Output bytes, the first of them numbered `number`.
    Output { number: u64, bytes: Vec<u8> },
    /// The program exited, after all its output was sent.
    Exited {
        code: Option<u32>,
        signal: Option<i32>,
    },
    /// A request failed (a `Spawn` that could not start its program); the
    /// holder carries on.
    Error(String),
}
