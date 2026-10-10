//! `server-access`: which other users may use the server, and how.
//!
//! The endpoint's `socket-access` decides who can open it at all. Every
//! connection is then checked here, by the user its process runs as: the
//! owner (and root or SYSTEM) always gets in, other users only when
//! `server-access` named them, read-only or not. A uid identifies a user on
//! Unix, a SID string on Windows.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// A user as the platform identifies one: a uid on Unix, a SID on Windows.
pub type UserId = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The name `server-access -a` was given, for `-l`.
    pub name: String,
    pub write: bool,
}

/// The users besides the owner who may connect, by id.
#[derive(Debug, Default)]
pub struct Acl(BTreeMap<UserId, Entry>);

/// The list as the accept thread and the server share it.
pub type SharedAcl = Arc<Mutex<Acl>>;

/// Lock the shared list; it is plain data, valid whatever a panicking
/// holder left behind.
pub fn lock(acl: &SharedAcl) -> MutexGuard<'_, Acl> {
    acl.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Acl {
    pub fn allow(&mut self, id: UserId, name: String, write: bool) {
        self.0.insert(id, Entry { name, write });
    }

    /// Make an allowed user read-only or writable; `false` if not allowed.
    pub fn set_write(&mut self, id: &str, write: bool) -> bool {
        self.0.get_mut(id).map(|e| e.write = write).is_some()
    }

    /// Take a user off the list; `false` if not on it.
    pub fn deny(&mut self, id: &str) -> bool {
        self.0.remove(id).is_some()
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.0.get(id)
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.0.values()
    }

    /// Whether `peer` may connect to `owner`'s server, and whether
    /// read-only: `None` refuses it.
    pub fn admit(&self, owner: &str, peer: &str) -> Option<bool> {
        if peer == owner || is_privileged(peer) {
            return Some(false);
        }
        self.0.get(peer).map(|e| !e.write)
    }
}

/// root on Unix, SYSTEM on Windows: allowed in, as tmux lets root in.
fn is_privileged(id: &str) -> bool {
    if cfg!(windows) {
        id == "S-1-5-18"
    } else {
        id == "0"
    }
}

pub use sys::{current_user, lookup_user, peer_user};

#[cfg(unix)]
mod sys {
    use std::ffi::CString;
    use std::io;

    use interprocess::local_socket::Stream;
    use interprocess::local_socket::traits::StreamCommon as _;

    use super::UserId;

    /// The user this process runs as (its effective uid).
    pub fn current_user() -> io::Result<UserId> {
        // SAFETY: geteuid has no preconditions and cannot fail.
        Ok(unsafe { libc::geteuid() }.to_string())
    }

    /// The user the process at the other end of `stream` runs as.
    pub fn peer_user(stream: &Stream) -> io::Result<UserId> {
        stream
            .peer_creds()?
            .euid()
            .map(|uid| uid.to_string())
            .ok_or_else(|| io::Error::other("the peer's user is unknown"))
    }

    /// The uid of the user called `name`.
    pub fn lookup_user(name: &str) -> io::Result<UserId> {
        /// The most `getpwnam_r` is given for the entry's strings.
        const BUF_LIMIT: usize = 1 << 20;
        let cname =
            CString::new(name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let mut buf = vec![0u8; 1024];
        loop {
            // SAFETY: passwd is plain data, valid when zeroed.
            let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
            let mut found: *mut libc::passwd = std::ptr::null_mut();
            // SAFETY: cname is NUL-terminated; pwd and found are live locals;
            // buf is writable for buf.len() bytes and outlives the call, and
            // pwd's strings (which point into it) are not used.
            let rc = unsafe {
                libc::getpwnam_r(
                    cname.as_ptr(),
                    &raw mut pwd,
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                    &raw mut found,
                )
            };
            if rc == libc::ERANGE && buf.len() < BUF_LIMIT {
                buf.resize(buf.len() * 2, 0);
                continue;
            }
            if rc != 0 {
                return Err(io::Error::from_raw_os_error(rc));
            }
            if found.is_null() {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no such user"));
            }
            return Ok(pwd.pw_uid.to_string());
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::io;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

    use interprocess::local_socket::Stream;
    use interprocess::local_socket::traits::StreamCommon as _;
    use windows_sys::Win32::Foundation::{HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{
        GetTokenInformation, LookupAccountNameW, PSID, SID_NAME_USE, SidTypeUser, TOKEN_QUERY,
        TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::UserId;

    /// The user this process runs as.
    pub fn current_user() -> io::Result<UserId> {
        // SAFETY: GetCurrentProcess has no preconditions; its pseudo-handle
        // needs no closing.
        token_user(unsafe { GetCurrentProcess() })
    }

    /// The user the process at the other end of `stream` runs as.
    pub fn peer_user(stream: &Stream) -> io::Result<UserId> {
        let pid = stream
            .peer_creds()?
            .pid()
            .ok_or_else(|| io::Error::other("the peer's process is unknown"))?;
        // SAFETY: OpenProcess takes no pointers; a null result is an error.
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: raw is a process handle OpenProcess just gave us to own.
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        token_user(process.as_raw_handle())
    }

    /// The SID of the user `process` (a handle with query access) runs as.
    fn token_user(process: HANDLE) -> io::Result<UserId> {
        let mut raw: HANDLE = std::ptr::null_mut();
        // SAFETY: process is a valid handle; raw is a live out-pointer.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut raw) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcessToken succeeded, so raw is a token we own.
        let token = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut len = 0u32;
        // SAFETY: a null buffer of length 0 only asks for the size needed;
        // the call fails by design and len receives the size.
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                std::ptr::null_mut(),
                0,
                &raw mut len,
            )
        };
        // u64s: aligned for TOKEN_USER, which the SID it points to follows.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        // SAFETY: buf is writable for at least len bytes.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buf.as_mut_ptr().cast(),
                len,
                &raw mut len,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: on success buf starts with a TOKEN_USER (u64 alignment
        // covers its pointer field), whose SID points into buf, alive here.
        let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };
        sid_string(user.User.Sid)
    }

    /// The `S-1-5-…` form of `sid`.
    fn sid_string(sid: PSID) -> io::Result<String> {
        let mut out: *mut u16 = std::ptr::null_mut();
        // SAFETY: sid is a valid SID (from a token or LookupAccountNameW) and
        // out a live out-pointer.
        if unsafe { ConvertSidToStringSidW(sid, &raw mut out) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: on success out is a NUL-terminated wide string; it is read
        // up to the NUL, then freed with LocalFree as the API asks.
        let text = unsafe {
            let len = (0..).take_while(|&i| *out.add(i) != 0).count();
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(out, len));
            LocalFree(out.cast());
            text
        };
        Ok(text)
    }

    /// The SID of the user account called `name` (`user` or `DOMAIN\user`).
    pub fn lookup_user(name: &str) -> io::Result<UserId> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let (mut sid_len, mut dom_len) = (0u32, 0u32);
        let mut kind: SID_NAME_USE = 0;
        // SAFETY: wide is NUL-terminated; null buffers with zero lengths only
        // ask for the sizes needed, written to the live locals.
        unsafe {
            LookupAccountNameW(
                std::ptr::null(),
                wide.as_ptr(),
                std::ptr::null_mut(),
                &raw mut sid_len,
                std::ptr::null_mut(),
                &raw mut dom_len,
                &raw mut kind,
            )
        };
        if sid_len == 0 {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such user"));
        }
        // u64s: aligned for the SID's fields.
        let mut sid = vec![0u64; (sid_len as usize).div_ceil(8)];
        let mut domain = vec![0u16; dom_len as usize];
        // SAFETY: the buffers have the sizes the first call asked for.
        if unsafe {
            LookupAccountNameW(
                std::ptr::null(),
                wide.as_ptr(),
                sid.as_mut_ptr().cast(),
                &raw mut sid_len,
                domain.as_mut_ptr(),
                &raw mut dom_len,
                &raw mut kind,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if kind != SidTypeUser {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a user account",
            ));
        }
        sid_string(sid.as_mut_ptr().cast())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn the_owner_and_listed_users_get_in() {
        let mut acl = Acl::default();
        assert_eq!(acl.admit("owner", "owner"), Some(false));
        assert_eq!(acl.admit("owner", "other"), None);
        acl.allow("other".into(), "Other".into(), false);
        assert_eq!(acl.admit("owner", "other"), Some(true), "read-only");
        assert!(acl.set_write("other", true));
        assert_eq!(acl.admit("owner", "other"), Some(false));
        assert!(acl.deny("other"));
        assert_eq!(acl.admit("owner", "other"), None);
        assert!(!acl.deny("other"));
        assert!(!acl.set_write("other", true));
        let root = if cfg!(windows) { "S-1-5-18" } else { "0" };
        assert_eq!(acl.admit("owner", root), Some(false));
    }

    #[test]
    fn this_user_is_found_by_name() {
        let me = current_user().unwrap();
        let name = if cfg!(windows) {
            std::env::var("USERNAME")
        } else {
            std::env::var("USER").or_else(|_| std::env::var("LOGNAME"))
        };
        // Not every CI environment names its user.
        if let Ok(name) = name {
            assert_eq!(lookup_user(&name).unwrap(), me, "{name}");
        }
        assert_eq!(
            lookup_user("no-such-user-tmxr").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}
