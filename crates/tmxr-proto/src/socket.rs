//! Where a server listens and who may connect.
//!
//! - Unix: a socket file named after the label inside a per-user directory
//!   `<base>/tmxr-<uid>`, where `<base>` is `$TMXR_TMPDIR`, `$XDG_RUNTIME_DIR`,
//!   `$TMPDIR` or `/tmp` (first set wins). The directory is created `0700` and
//!   refused unless it is a real directory owned by us with no group/other
//!   access; connecting peers must run as our uid.
//! - Windows: the named pipe `\\.\pipe\tmxr-<user>-<label>`, created with a
//!   DACL that grants access to its owner and SYSTEM only.

use std::io;
use std::path::{Path, PathBuf};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericFilePath, Listener, ListenerOptions, Stream, ToFsName};

/// Environment variable set in every pane: `<endpoint>,<server pid>,<session id>`.
pub const ENV_TMXR: &str = "TMXR";
/// Environment variable set in every pane: `%<pane id>`.
pub const ENV_TMXR_PANE: &str = "TMXR_PANE";
/// Label used when neither `-L` nor `-S` is given.
pub const DEFAULT_LABEL: &str = "default";

/// A server's address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    path: PathBuf,
}

impl Endpoint {
    /// Pick the endpoint the way tmux does: `-S` path, then `-L` label, then
    /// the server named in `$TMXR` (so commands run inside a pane reach that
    /// pane's server), then the default label.
    pub fn resolve(socket: Option<&Path>, label: Option<&str>) -> io::Result<Self> {
        if let Some(path) = socket {
            return Ok(Self {
                path: path.to_path_buf(),
            });
        }
        if let Some(label) = label {
            return Self::for_label(label);
        }
        if let Some(path) = std::env::var_os(ENV_TMXR)
            .and_then(|v| v.into_string().ok())
            .and_then(|v| parse_tmxr_env(&v).map(|(p, _, _)| p))
        {
            return Ok(Self { path });
        }
        Self::for_label(DEFAULT_LABEL)
    }

    /// The endpoint for a `-L` label.
    pub fn for_label(label: &str) -> io::Result<Self> {
        validate_label(label)?;
        Ok(Self {
            path: label_path(label)?,
        })
    }

    /// The socket path (a pipe path on Windows).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Connect to a server at this endpoint.
    pub fn connect(&self) -> io::Result<Stream> {
        Stream::connect(self.name()?)
    }

    /// Bind a listener at this endpoint, replacing a stale socket file left
    /// by a server that died. Fails with `AddrInUse` if a live server owns it.
    pub fn listen(&self) -> io::Result<Listener> {
        #[cfg(unix)]
        {
            if let Some(dir) = self.path.parent() {
                ensure_private_dir(dir)?;
            }
            match self.bind() {
                Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                    if self.connect().is_ok() {
                        return Err(e);
                    }
                    std::fs::remove_file(&self.path)?;
                    self.bind()
                }
                other => other,
            }
        }
        #[cfg(windows)]
        {
            self.bind()
        }
    }

    #[cfg(unix)]
    fn bind(&self) -> io::Result<Listener> {
        use std::os::unix::fs::PermissionsExt;
        let listener = ListenerOptions::new()
            .name(self.name()?)
            .reclaim_name(true)
            .create_sync()?;
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    #[cfg(windows)]
    fn bind(&self) -> io::Result<Listener> {
        use interprocess::os::windows::local_socket::ListenerOptionsExt;
        use interprocess::os::windows::security_descriptor::SecurityDescriptor;
        // Protected DACL: full access for the pipe's owner (the user running
        // the server) and SYSTEM; nobody else, inherited ACEs ignored.
        let sddl = widestring::U16CString::from_str("D:P(A;;GA;;;OW)(A;;GA;;;SY)")
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        ListenerOptions::new()
            .name(self.name()?)
            .security_descriptor(SecurityDescriptor::deserialize(&sddl)?)
            .create_sync()
    }

    fn name(&self) -> io::Result<interprocess::local_socket::Name<'static>> {
        self.path.clone().to_fs_name::<GenericFilePath>()
    }
}

/// Whether the peer on `stream` runs as the same user as this process.
///
/// Unix compares effective uids. On Windows the pipe's DACL already limits
/// who can open it, so every peer that got this far is accepted.
pub fn peer_is_trusted(stream: &Stream) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::traits::StreamCommon as _;
        let creds = stream.peer_creds()?;
        Ok(creds.euid() == Some(current_uid()))
    }
    #[cfg(windows)]
    {
        let _ = stream;
        Ok(true)
    }
}

/// Format the `$TMXR` value for a pane.
pub fn format_tmxr_env(endpoint: &Endpoint, server_pid: u32, session_id: u32) -> String {
    format!("{},{server_pid},{session_id}", endpoint.path.display())
}

/// Parse `$TMXR` into (endpoint path, server pid, session id). The path is
/// split off from the right, so a path containing commas still parses.
pub fn parse_tmxr_env(value: &str) -> Option<(PathBuf, u32, u32)> {
    let mut parts = value.rsplitn(3, ',');
    let session = parts.next()?.parse().ok()?;
    let pid = parts.next()?.parse().ok()?;
    let path = parts.next().filter(|p| !p.is_empty())?;
    Some((PathBuf::from(path), pid, session))
}

fn validate_label(label: &str) -> io::Result<()> {
    let ok = !label.is_empty()
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && label != "."
        && label != "..";
    if ok {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid socket label {label:?}: use letters, digits, '-', '_' or '.'"),
        ))
    }
}

#[cfg(unix)]
fn label_path(label: &str) -> io::Result<PathBuf> {
    let base = ["TMXR_TMPDIR", "XDG_RUNTIME_DIR", "TMPDIR"]
        .iter()
        .filter_map(std::env::var_os)
        .find(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    Ok(base.join(format!("tmxr-{}", current_uid())).join(label))
}

#[cfg(windows)]
fn label_path(label: &str) -> io::Result<PathBuf> {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".to_owned());
    // A backslash is the only character a pipe name cannot contain.
    let user = user.replace('\\', "_");
    Ok(PathBuf::from(format!(r"\\.\pipe\tmxr-{user}-{label}")))
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// Create `dir` with mode `0700` if missing, then insist it is a directory
/// (not a symlink) owned by us with no group or other permissions — the same
/// checks tmux applies to its socket directory.
#[cfg(unix)]
fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    let problem = if !meta.file_type().is_dir() {
        Some("is not a directory")
    } else if meta.uid() != current_uid() {
        Some("is owned by another user")
    } else if meta.mode() & 0o077 != 0 {
        Some("is accessible by other users")
    } else {
        None
    };
    match problem {
        Some(why) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("socket directory {} {why}", dir.display()),
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn tmxr_env_round_trips_including_commas_in_the_path() {
        let ep = Endpoint {
            path: PathBuf::from("/tmp/a,b/default"),
        };
        let v = format_tmxr_env(&ep, 99, 3);
        assert_eq!(
            parse_tmxr_env(&v),
            Some((PathBuf::from("/tmp/a,b/default"), 99, 3))
        );
        assert_eq!(parse_tmxr_env("nonsense"), None);
        assert_eq!(parse_tmxr_env(",1,2"), None);
    }

    #[test]
    fn labels_are_restricted() {
        assert!(validate_label("default").is_ok());
        assert!(validate_label("work-2.x_y").is_ok());
        for bad in ["", ".", "..", "a/b", r"a\b", "a b"] {
            assert!(validate_label(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn explicit_socket_wins_over_label() {
        let ep = Endpoint::resolve(Some(Path::new("/x/y")), Some("lbl")).unwrap();
        assert_eq!(ep.path(), Path::new("/x/y"));
    }

    fn unique_endpoint(dir: &Path) -> Endpoint {
        let label = format!(
            "test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        )
        .replace(['(', ')'], "");
        #[cfg(unix)]
        let path = dir.join("tmxr-test").join(label);
        #[cfg(windows)]
        let path = {
            let _ = dir;
            PathBuf::from(format!(r"\\.\pipe\tmxr-test-{label}"))
        };
        Endpoint { path }
    }

    #[test]
    fn listen_accept_connect_round_trip() {
        use interprocess::local_socket::traits::Listener as _;
        let tmp = tempfile::tempdir().unwrap();
        let ep = unique_endpoint(tmp.path());
        let listener = ep.listen().unwrap();
        let server = std::thread::spawn(move || {
            let mut s = listener.accept().unwrap();
            assert!(peer_is_trusted(&s).unwrap());
            let mut buf = [0u8; 4];
            s.read_exact(&mut buf).unwrap();
            s.write_all(&buf).unwrap();
        });
        let mut c = ep.connect().unwrap();
        c.write_all(b"ping").unwrap();
        let mut buf = [0u8; 4];
        c.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
        server.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stale_socket_file_is_replaced_but_live_one_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        let ep = unique_endpoint(tmp.path());
        let _live = ep.listen().unwrap();
        assert_eq!(ep.listen().unwrap_err().kind(), io::ErrorKind::AddrInUse);

        // A crashed server leaves its socket file behind with nobody listening.
        let stale = Endpoint {
            path: ep.path().with_file_name("stale"),
        };
        drop(std::os::unix::net::UnixListener::bind(stale.path()).unwrap());
        assert!(stale.path().exists());
        stale.listen().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn group_readable_socket_dir_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("open");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = ensure_private_dir(&dir).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        ensure_private_dir(&dir).unwrap();
    }
}
