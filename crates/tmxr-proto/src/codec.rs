//! Length-prefixed `postcard` frames over any `Read` / `Write`.

use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Largest frame either side accepts. A length above this is a protocol error
/// before anything is allocated, so a corrupt or hostile peer cannot make the
/// reader allocate an arbitrary amount.
pub const MAX_FRAME: usize = 16 << 20;

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    #[error("frame of {0} bytes exceeds the {MAX_FRAME}-byte limit")]
    TooLarge(usize),
    #[error("malformed message: {0}")]
    Decode(#[from] postcard::Error),
}

/// Encode `msg` and write it as one frame.
pub fn write_msg<W: Write, T: Serialize>(w: &mut W, msg: &T) -> Result<(), ProtoError> {
    let body = postcard::to_stdvec(msg)?;
    if body.len() > MAX_FRAME {
        return Err(ProtoError::TooLarge(body.len()));
    }
    let len = u32::try_from(body.len()).map_err(|_| ProtoError::TooLarge(body.len()))?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&body);
    w.write_all(&frame)?;
    w.flush()?;
    Ok(())
}

/// Read one frame and decode it. `Ok(None)` is a clean end of stream between
/// frames; an end of stream inside a frame is an error.
pub fn read_msg<R: Read, T: DeserializeOwned>(r: &mut R) -> Result<Option<T>, ProtoError> {
    let mut len = [0u8; 4];
    let mut filled = 0;
    while filled < len.len() {
        match r.read(&mut len[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(ProtoError::TooLarge(len));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some(postcard::from_bytes(&body)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientMsg, Hello, PROTOCOL_VERSION, ServerMsg, TerminalInfo};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    fn sample_client() -> Vec<ClientMsg> {
        vec![
            ClientMsg::Hello(Hello {
                protocol: PROTOCOL_VERSION,
                version: "0.1.0".into(),
                cwd: "/home/u".into(),
                env: vec![("TERM".into(), "xterm-256color".into())],
                terminal: Some(TerminalInfo {
                    cols: 80,
                    rows: 24,
                    term: "xterm-256color".into(),
                    job_control: true,
                }),
            }),
            ClientMsg::Command(vec!["split-window".into(), "-h".into()]),
            ClientMsg::Input(Event::Key(KeyEvent::new(
                KeyCode::Char('h'),
                KeyModifiers::CONTROL,
            ))),
            ClientMsg::Input(Event::Resize(120, 40)),
            ClientMsg::Detach,
            ClientMsg::Resumed,
            ClientMsg::LockFailed("lock: not found".into()),
        ]
    }

    fn sample_server() -> Vec<ServerMsg> {
        vec![
            ServerMsg::Hello {
                protocol: PROTOCOL_VERSION,
                version: "0.1.0".into(),
                pid: 42,
            },
            ServerMsg::Attached,
            ServerMsg::Output(b"\x1b[H\x1b[2Jhello".to_vec()),
            ServerMsg::CommandResult {
                status: 1,
                stdout: String::new(),
                stderr: "can't find session: x".into(),
            },
            ServerMsg::Detached {
                reason: "detached".into(),
            },
            ServerMsg::Mouse(true),
            ServerMsg::Suspend,
            ServerMsg::Lock {
                command: "lock -np".into(),
            },
        ]
    }

    #[test]
    fn every_message_round_trips_through_one_stream() {
        let mut buf = Vec::new();
        for m in sample_client() {
            write_msg(&mut buf, &m).unwrap();
        }
        let mut r = buf.as_slice();
        for want in sample_client() {
            let got: ClientMsg = read_msg(&mut r).unwrap().unwrap();
            assert_eq!(got, want);
        }
        assert!(read_msg::<_, ClientMsg>(&mut r).unwrap().is_none());

        let mut buf = Vec::new();
        for m in sample_server() {
            write_msg(&mut buf, &m).unwrap();
        }
        let mut r = buf.as_slice();
        for want in sample_server() {
            let got: ServerMsg = read_msg(&mut r).unwrap().unwrap();
            assert_eq!(got, want);
        }
    }

    /// Golden bytes: a change to any wire type changes these, which must come
    /// with a `PROTOCOL_VERSION` bump (and an update of this test).
    #[test]
    fn wire_format_is_pinned() {
        let mut buf = Vec::new();
        for m in sample_client() {
            write_msg(&mut buf, &m).unwrap();
        }
        for m in sample_server() {
            write_msg(&mut buf, &m).unwrap();
        }
        assert_eq!(PROTOCOL_VERSION, 4, "update the golden digest below too");
        assert_eq!(
            fnv1a(&buf),
            GOLDEN,
            "wire format changed: bump PROTOCOL_VERSION"
        );
    }

    const GOLDEN: u64 = 9_609_753_476_290_932_931;

    /// FNV-1a 64-bit (http://www.isthe.com/chongo/tech/comp/fnv/), used only to
    /// pin the golden bytes in a readable constant.
    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    #[test]
    fn fnv1a_matches_reference_vectors() {
        // From the FNV reference test suite.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn oversized_length_is_rejected_before_allocating() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&u32::MAX.to_le_bytes());
        let err = read_msg::<_, ServerMsg>(&mut frame.as_slice()).unwrap_err();
        assert!(matches!(err, ProtoError::TooLarge(_)), "{err}");
    }

    #[test]
    fn truncated_frame_is_an_error_not_eof() {
        let mut buf = Vec::new();
        write_msg(&mut buf, &ServerMsg::Attached).unwrap();
        buf.pop();
        let mut r = buf.as_slice();
        assert!(read_msg::<_, ServerMsg>(&mut r).is_err());
        let mut r: &[u8] = &[1, 0];
        assert!(read_msg::<_, ServerMsg>(&mut r).is_err());
    }
}
