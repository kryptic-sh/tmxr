//! What is running in a pane, and where.
//!
//! | Platform | Foreground process                                   | Directory                 |
//! | -------- | ---------------------------------------------------- | ------------------------- |
//! | Linux    | the terminal's foreground process group leader       | `/proc/<pid>/cwd`         |
//! | macOS    | the terminal's foreground process group leader       | `proc_pidinfo` vnode path |
//! | Windows  | the newest settled direct child of the pane's process, else the process itself | not available (`None`) |
//!
//! Windows has no foreground process group; the pane's program is usually a
//! shell and whatever it runs is its child, so the child is what the user sees.
//! Grandchildren are ignored on purpose: an editor's language servers must not
//! be mistaken for the editor. Children younger than [`SETTLE_100NS`] are
//! ignored too: shells spawn prompt helpers (starship, git) for every prompt,
//! and those must not be mistaken for what the user is running.

use std::path::PathBuf;

use crate::Pty;

/// Name of the process in the pane's foreground, as tmux shows it
/// (`#{pane_current_command}`): the executable's base name.
pub fn foreground_command(pty: &Pty) -> Option<String> {
    #[cfg(windows)]
    {
        foreground_command_windows(pty)
    }
    #[cfg(not(windows))]
    {
        process_name(foreground_pid(pty)?)
    }
}

/// Working directory of the pane's foreground process. `None` where the
/// platform offers no way to read it (Windows); callers fall back to what the
/// shell reported (OSC 7) or the pane's start directory.
pub fn current_dir(pty: &Pty) -> Option<PathBuf> {
    let pid = foreground_pid(pty)?;
    process_cwd(pid)
}

#[cfg(unix)]
fn foreground_pid(pty: &Pty) -> Option<u32> {
    pty.foreground_pgrp().or_else(|| pty.pid())
}

#[cfg(windows)]
fn foreground_pid(pty: &Pty) -> Option<u32> {
    let root = pty.pid()?;
    Some(newest_child(root).unwrap_or(root))
}

/// On Windows the chosen child can exit between the two snapshots; fall
/// back to the pane's own process rather than reporting nothing.
#[cfg(windows)]
pub fn foreground_command_windows(pty: &Pty) -> Option<String> {
    let root = pty.pid()?;
    newest_child(root)
        .and_then(process_name)
        .or_else(|| process_name(root))
}

#[cfg(target_os = "linux")]
fn process_name(pid: u32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim_end().to_owned())
}

#[cfg(target_os = "linux")]
fn process_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(target_os = "macos")]
fn process_name(pid: u32) -> Option<String> {
    let mut buf = [0u8; 256];
    let pid = libc::c_int::try_from(pid).ok()?;
    // SAFETY: `buf` is valid for `buf.len()` bytes and proc_name writes at
    // most that many, returning the length written.
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

#[cfg(target_os = "macos")]
fn process_cwd(pid: u32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut info = std::mem::MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    // SAFETY: `info` is valid for `size` bytes; proc_pidinfo fills at most
    // that much and returns the byte count written (≤ 0 on failure).
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if n != size {
        return None;
    }
    // SAFETY: proc_pidinfo wrote the whole struct (n == size), and it was
    // zero-initialised before that, so every byte is initialised.
    let info = unsafe { info.assume_init() };
    let raw: Vec<u8> = info
        .pvi_cdir
        .vip_path
        .iter()
        .flatten()
        .map(|c| *c as u8)
        .take_while(|b| *b != 0)
        .collect();
    if raw.is_empty() {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&raw)))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn process_name(_pid: u32) -> Option<String> {
    None
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn process_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// `(pid, parent pid, exe name)` for every process on the system.
    pub fn snapshot() -> Vec<(u32, u32, String)> {
        let mut out = Vec::new();
        // SAFETY: a process snapshot takes no pointers; the handle is checked
        // and closed below.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return out;
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..PROCESSENTRY32W::default()
        };
        // SAFETY: `entry` is a properly sized PROCESSENTRY32W with dwSize set,
        // as Process32FirstW/NextW require; `snap` is a live snapshot handle.
        let mut ok = unsafe { Process32FirstW(snap, &mut entry) } != 0;
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            out.push((
                entry.th32ProcessID,
                entry.th32ParentProcessID,
                String::from_utf16_lossy(&entry.szExeFile[..len]),
            ));
            // SAFETY: as above.
            ok = unsafe { Process32NextW(snap, &mut entry) } != 0;
        }
        // SAFETY: `snap` is a valid handle owned here and closed exactly once.
        unsafe { CloseHandle(snap) };
        out
    }

    fn ticks(t: FILETIME) -> u64 {
        (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
    }

    /// How long `pid` has been running, in 100 ns units; `None` if the
    /// process cannot be opened (gone, or not ours to inspect).
    pub fn age_100ns(pid: u32) -> Option<u64> {
        // SAFETY: OpenProcess takes no pointers; the handle is checked and
        // closed below.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if h.is_null() {
            return None;
        }
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user, mut now) =
            (zero, zero, zero, zero, zero);
        // SAFETY: every out-pointer is a live, writable FILETIME and `h` is a
        // process handle opened with query rights.
        let ok =
            unsafe { GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) } != 0;
        // SAFETY: `h` is valid and closed exactly once.
        unsafe { CloseHandle(h) };
        // SAFETY: `now` is a live, writable FILETIME.
        unsafe { GetSystemTimeAsFileTime(&mut now) };
        ok.then(|| ticks(now).saturating_sub(ticks(created)))
    }
}

/// Minimum age of a child process before it counts as the pane's foreground
/// (0.5 s in 100 ns units).
#[cfg(windows)]
const SETTLE_100NS: u64 = 5_000_000;

/// The newest direct child of `pid` that has been running for at least
/// [`SETTLE_100NS`] (the snapshot lists processes in creation order).
#[cfg(windows)]
fn newest_child(pid: u32) -> Option<u32> {
    win::snapshot()
        .into_iter()
        .filter(|(child, parent, _)| *parent == pid && *child != pid)
        .map(|(child, _, _)| child)
        .rev()
        .find(|child| win::age_100ns(*child).is_some_and(|age| age >= SETTLE_100NS))
}

#[cfg(windows)]
fn process_name(pid: u32) -> Option<String> {
    let exe = win::snapshot()
        .into_iter()
        .find(|(p, _, _)| *p == pid)
        .map(|(_, _, exe)| exe)?;
    let stem = exe
        .len()
        .checked_sub(4)
        .filter(|&i| exe.is_char_boundary(i) && exe[i..].eq_ignore_ascii_case(".exe"))
        .map_or(exe.as_str(), |i| &exe[..i]);
    Some(stem.to_owned())
}

#[cfg(windows)]
fn process_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Emulator, PtyEvent, SpawnSpec};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    /// A command that stays alive long enough to be inspected, and the name
    /// the inspection should report.
    fn long_running() -> (Vec<String>, &'static str) {
        #[cfg(unix)]
        {
            (vec!["sleep".into(), "30".into()], "sleep")
        }
        #[cfg(windows)]
        {
            // The pane's program is cmd; ping is its newest child.
            (
                vec![
                    "cmd.exe".into(),
                    "/d".into(),
                    "/c".into(),
                    "ping -n 30 127.0.0.1 >NUL".into(),
                ],
                "PING",
            )
        }
    }

    #[test]
    fn foreground_command_names_the_running_program() {
        let (argv, want) = long_running();
        let dir = std::env::temp_dir();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: Arc<dyn Fn(PtyEvent) + Send + Sync> =
            Arc::new(move |e| drop(tx.lock().unwrap().send(e)));
        let mut pty = crate::Pty::spawn(
            &SpawnSpec {
                argv,
                cwd: Some(dir.clone()),
                rows: 24,
                cols: 80,
                ..SpawnSpec::default()
            },
            sink,
        )
        .unwrap();
        let mut emu = Emulator::new(24, 80, 0);
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut seen = None;
        while Instant::now() < deadline {
            while let Ok(ev) = rx.try_recv() {
                if let PtyEvent::Output(b) = ev {
                    let r = emu.process(&b);
                    if !r.is_empty() {
                        pty.write(&r).unwrap();
                    }
                }
            }
            seen = foreground_command(&pty);
            if seen
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case(want))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let got = seen.unwrap_or_default();
        assert!(
            got.eq_ignore_ascii_case(want),
            "foreground command {got:?}, want {want:?}"
        );
        #[cfg(target_os = "linux")]
        assert_eq!(
            current_dir(&pty).map(|p| p.canonicalize().unwrap()),
            Some(dir.canonicalize().unwrap())
        );
        // Cleanup only; dropping the pty closes the terminal either way.
        let _ = pty.kill();
    }
}
