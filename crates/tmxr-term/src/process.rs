//! What is running in a pane, and where.
//!
//! | Platform | Foreground process                                   | Directory                 | Arguments |
//! | -------- | ---------------------------------------------------- | ------------------------- | --------- |
//! | Linux    | the terminal's foreground process group leader       | `/proc/<pid>/cwd`         | `/proc/<pid>/cmdline` |
//! | macOS    | the terminal's foreground process group leader       | `proc_pidinfo` vnode path | `sysctl` `KERN_PROCARGS2` |
//! | Windows  | the newest settled direct child of the pane's process, else the process itself | the process's PEB (`CurrentDirectory`) | `NtQueryInformationProcess` command line, split by `CommandLineToArgvW` |
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

/// The pane's foreground process's command line, program first, as it was
/// started. `None` when it cannot be read, or an argument is not UTF-8 (it
/// could not be restored as it was).
pub fn foreground_args(pty: &Pty) -> Option<Vec<String>> {
    process_args(foreground_pid(pty)?)
}

/// Working directory of the pane's foreground process. `None` when it
/// cannot be read; callers fall back to what the shell reported (OSC 7) or
/// the pane's start directory.
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
fn process_args(pid: u32) -> Option<Vec<String>> {
    parse_cmdline(&std::fs::read(format!("/proc/{pid}/cmdline")).ok()?)
}

/// `/proc/<pid>/cmdline`: each argument followed by a NUL. Empty for a
/// process with no command line (a zombie, a kernel thread).
#[cfg(any(target_os = "linux", test))]
fn parse_cmdline(raw: &[u8]) -> Option<Vec<String>> {
    let body = raw.strip_suffix(&[0])?;
    body.split(|b| *b == 0)
        .map(|a| String::from_utf8(a.to_vec()).ok())
        .collect()
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
fn process_args(pid: u32) -> Option<Vec<String>> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    // SAFETY: `mib` names three valid integers; a null output buffer asks
    // only for the size, written to `size`.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    // SAFETY: `buf` is valid for `size` bytes, which sysctl writes at most,
    // updating `size` to the count written.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(size);
    parse_procargs2(&buf)
}

/// `KERN_PROCARGS2`: `argc` as a native-endian `int`, the executable's path,
/// NUL padding, then `argc` NUL-terminated arguments (the environment
/// follows, and is ignored).
#[cfg(any(target_os = "macos", test))]
fn parse_procargs2(buf: &[u8]) -> Option<Vec<String>> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let argc = usize::try_from(argc).ok()?;
    let rest = &buf[4..];
    let rest = &rest[rest.iter().position(|b| *b == 0)?..];
    let rest = &rest[rest.iter().position(|b| *b != 0)?..];
    let args: Vec<String> = rest
        .split(|b| *b == 0)
        .take(argc)
        .map(|a| String::from_utf8(a.to_vec()).ok())
        .collect::<Option<_>>()?;
    (args.len() == argc).then_some(args)
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

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn process_args(_pid: u32) -> Option<Vec<String>> {
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
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut now = zero;
        let created = created_100ns(pid)?;
        // SAFETY: `now` is a live, writable FILETIME.
        unsafe { GetSystemTimeAsFileTime(&mut now) };
        Some(ticks(now).saturating_sub(created))
    }

    /// When `pid` was created, in 100 ns units since 1601; `None` if the
    /// process cannot be opened (gone, or not ours to inspect).
    pub fn created_100ns(pid: u32) -> Option<u64> {
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
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: every out-pointer is a live, writable FILETIME and `h` is a
        // process handle opened with query rights.
        let ok =
            unsafe { GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) } != 0;
        // SAFETY: `h` is valid and closed exactly once.
        unsafe { CloseHandle(h) };
        ok.then(|| ticks(created))
    }

    /// The command line `pid` was started with, as one string.
    pub fn command_line(pid: u32) -> Option<Vec<u16>> {
        use windows_sys::Wdk::System::Threading::{
            NtQueryInformationProcess, ProcessCommandLineInformation,
        };
        use windows_sys::Win32::Foundation::UNICODE_STRING;
        // SAFETY: OpenProcess takes no pointers; the handle is checked and
        // closed below.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if h.is_null() {
            return None;
        }
        let mut needed = 0u32;
        // SAFETY: a null buffer of length 0 only asks for the size, which is
        // written to `needed`; `h` has the query rights this class needs.
        unsafe {
            NtQueryInformationProcess(
                h,
                ProcessCommandLineInformation,
                std::ptr::null_mut(),
                0,
                &mut needed,
            )
        };
        // u64 words, so the UNICODE_STRING at the start is aligned.
        let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(1)];
        let len = u32::try_from(buf.len() * 8).unwrap_or(0);
        // SAFETY: `buf` is valid and writable for `len` bytes, and `needed`
        // is a live u32.
        let status = unsafe {
            NtQueryInformationProcess(
                h,
                ProcessCommandLineInformation,
                buf.as_mut_ptr().cast(),
                len,
                &mut needed,
            )
        };
        // SAFETY: `h` is valid and closed exactly once.
        unsafe { CloseHandle(h) };
        if status < 0 || (needed as usize) < std::mem::size_of::<UNICODE_STRING>() {
            return None;
        }
        // SAFETY: the call succeeded and wrote at least a UNICODE_STRING at
        // the start of `buf`, which is 8-aligned.
        let us = unsafe { &*buf.as_ptr().cast::<UNICODE_STRING>() };
        let start = buf.as_ptr() as usize;
        let end = start + buf.len() * 8;
        let text = us.Buffer as usize;
        let bytes = usize::from(us.Length);
        // The text lives in `buf`, after the header; refuse anything else.
        let inside = text >= start && text.checked_add(bytes).is_some_and(|e| e <= end);
        if us.Buffer.is_null() || !text.is_multiple_of(2) || !inside {
            return None;
        }
        // SAFETY: just checked that [text, text + bytes) lies inside `buf`
        // and is u16-aligned.
        Some(unsafe { std::slice::from_raw_parts(us.Buffer, bytes / 2) }.to_vec())
    }

    /// The current directory of `pid`, read from its process parameters
    /// the way psutil and Process Explorer do: the PEB's `ProcessParameters`
    /// pointer, then that block's `CurrentDirectory.DosPath`. `None` when
    /// the process cannot be opened for reading (another user's, elevated)
    /// or is gone.
    pub fn current_dir(pid: u32) -> Option<std::path::PathBuf> {
        use windows_sys::Win32::System::Threading::{PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
        // SAFETY: OpenProcess takes no pointers; the handle is checked and
        // closed below.
        let h = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if h.is_null() {
            return None;
        }
        let dir = read_current_dir(h);
        // SAFETY: `h` is valid and closed exactly once.
        unsafe { CloseHandle(h) };
        dir
    }

    /// Offset of `CurrentDirectory` (a `CURDIR`, whose first field is the
    /// `DosPath` UNICODE_STRING) in a 64-bit `RTL_USER_PROCESS_PARAMETERS`.
    /// windows-sys hides it in `Reserved2`; the layout is the one ReactOS
    /// and phnt publish and psutil reads.
    #[cfg(target_pointer_width = "64")]
    const CURRENT_DIRECTORY: usize = 0x38;

    #[cfg(target_pointer_width = "64")]
    fn read_current_dir(h: windows_sys::Win32::Foundation::HANDLE) -> Option<std::path::PathBuf> {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Wdk::System::Threading::{
            NtQueryInformationProcess, ProcessBasicInformation,
        };
        use windows_sys::Win32::System::Threading::{PEB, PROCESS_BASIC_INFORMATION};
        // SAFETY: PROCESS_BASIC_INFORMATION is integers and a raw pointer,
        // for which all-zero is a valid value.
        let mut info: PROCESS_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32;
        let mut written = 0u32;
        // SAFETY: `info` is a live PROCESS_BASIC_INFORMATION of `size`
        // bytes, the struct this class fills; `h` has query rights.
        let status = unsafe {
            NtQueryInformationProcess(
                h,
                ProcessBasicInformation,
                (&raw mut info).cast(),
                size,
                &mut written,
            )
        };
        if status < 0 || info.PebBaseAddress.is_null() {
            return None;
        }
        // UNICODE_STRING: Length (bytes) and MaximumLength as u16, then the
        // buffer pointer, aligned to the pointer size.
        let (len, buffer) = if let Some(peb32) = wow64_peb(h) {
            // A 32-bit program: its 64-bit PEB's parameters are the WOW64
            // layer's, not its own, and name a different directory.
            let params = read_u32(h, peb32 + PEB32_PROCESS_PARAMETERS)?;
            let mut header = [0u8; 8];
            read(h, params as usize + CURRENT_DIRECTORY_32, &mut header)?;
            let buffer = u32::from_ne_bytes(header[4..].try_into().ok()?);
            (u16::from_ne_bytes([header[0], header[1]]), buffer as usize)
        } else {
            let peb = info.PebBaseAddress as usize;
            let params = read_usize(h, peb + std::mem::offset_of!(PEB, ProcessParameters))?;
            let mut header = [0u8; 16];
            read(h, params + CURRENT_DIRECTORY, &mut header)?;
            let buffer = usize::from_ne_bytes(header[8..].try_into().ok()?);
            (u16::from_ne_bytes([header[0], header[1]]), buffer)
        };
        let len = usize::from(len);
        if len == 0 || buffer == 0 || !len.is_multiple_of(2) {
            return None;
        }
        let mut bytes = vec![0u8; len];
        read(h, buffer, &mut bytes)?;
        let wide: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_ne_bytes(*c))
            .collect();
        let mut dir = std::ffi::OsString::from_wide(&wide)
            .to_string_lossy()
            .into_owned();
        // `C:\work\` → `C:\work`, but a drive root keeps its slash.
        if dir.ends_with('\\') && !dir.ends_with(":\\") {
            dir.pop();
        }
        Some(dir.into())
    }

    /// Only 64-bit tmxr is built; a 32-bit build would need the 32-bit
    /// offsets, and reports nothing rather than reading the wrong ones.
    #[cfg(not(target_pointer_width = "64"))]
    fn read_current_dir(_h: windows_sys::Win32::Foundation::HANDLE) -> Option<std::path::PathBuf> {
        None
    }

    /// In a 32-bit `PEB` (a WOW64 program's): the `ProcessParameters` pointer.
    #[cfg(target_pointer_width = "64")]
    const PEB32_PROCESS_PARAMETERS: usize = 0x10;

    /// In a 32-bit `RTL_USER_PROCESS_PARAMETERS`: `CurrentDirectory.DosPath`
    /// (the same phnt / psutil layout as [`CURRENT_DIRECTORY`]).
    #[cfg(target_pointer_width = "64")]
    const CURRENT_DIRECTORY_32: usize = 0x24;

    /// The 32-bit PEB's address when the process is a WOW64 (32-bit) one.
    #[cfg(target_pointer_width = "64")]
    fn wow64_peb(h: windows_sys::Win32::Foundation::HANDLE) -> Option<usize> {
        use windows_sys::Wdk::System::Threading::{
            NtQueryInformationProcess, ProcessWow64Information,
        };
        let mut peb32 = 0usize;
        let mut written = 0u32;
        // SAFETY: this class writes one pointer-sized value, the 32-bit
        // PEB's address or 0, into `peb32`; `h` has query rights.
        let status = unsafe {
            NtQueryInformationProcess(
                h,
                ProcessWow64Information,
                (&raw mut peb32).cast(),
                std::mem::size_of::<usize>() as u32,
                &mut written,
            )
        };
        (status >= 0 && peb32 != 0).then_some(peb32)
    }

    #[cfg(target_pointer_width = "64")]
    fn read_u32(h: windows_sys::Win32::Foundation::HANDLE, addr: usize) -> Option<u32> {
        let mut b = [0u8; 4];
        read(h, addr, &mut b)?;
        Some(u32::from_ne_bytes(b))
    }

    #[cfg(target_pointer_width = "64")]
    fn read_usize(h: windows_sys::Win32::Foundation::HANDLE, addr: usize) -> Option<usize> {
        let mut b = [0u8; std::mem::size_of::<usize>()];
        read(h, addr, &mut b)?;
        Some(usize::from_ne_bytes(b))
    }

    /// Copy `buf.len()` bytes at `addr` in the process into `buf`, all or
    /// nothing.
    #[cfg(target_pointer_width = "64")]
    fn read(h: windows_sys::Win32::Foundation::HANDLE, addr: usize, buf: &mut [u8]) -> Option<()> {
        use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
        let mut got = 0usize;
        // SAFETY: `buf` is writable for `buf.len()` bytes, which is all
        // ReadProcessMemory writes; `addr` is in the other process, which the
        // call validates (it fails rather than faulting); `h` has VM-read
        // rights.
        let ok = unsafe {
            ReadProcessMemory(
                h,
                addr as *const core::ffi::c_void,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut got,
            )
        } != 0;
        (ok && got == buf.len()).then_some(())
    }

    /// Split a command line into arguments as a C program's startup code
    /// would (`CommandLineToArgvW`).
    pub fn split_command_line(line: &[u16]) -> Option<Vec<String>> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
        if line.iter().all(|c| *c == u16::from(b' ')) {
            // An empty line would give the calling program's own path.
            return None;
        }
        let mut z = line.to_vec();
        z.push(0);
        let mut n = 0i32;
        // SAFETY: `z` is NUL-terminated; `n` is a live i32.
        let argv = unsafe { CommandLineToArgvW(z.as_ptr(), &mut n) };
        if argv.is_null() {
            return None;
        }
        let count = usize::try_from(n).unwrap_or(0);
        let args = (0..count)
            .map(|i| {
                // SAFETY: CommandLineToArgvW returned `n` pointers, each to a
                // NUL-terminated string, all alive until the LocalFree below.
                unsafe {
                    let p = *argv.add(i);
                    let len = (0..).take_while(|&j| *p.add(j) != 0).count();
                    String::from_utf16(std::slice::from_raw_parts(p, len)).ok()
                }
            })
            .collect();
        // SAFETY: `argv` came from CommandLineToArgvW, which documents
        // LocalFree as how to release it; freed exactly once.
        unsafe { LocalFree(argv.cast()) };
        args
    }
}

#[cfg(windows)]
fn process_args(pid: u32) -> Option<Vec<String>> {
    win::split_command_line(&win::command_line(pid)?)
}

/// Minimum age of a child process before it counts as the pane's foreground
/// (0.5 s in 100 ns units).
#[cfg(windows)]
const SETTLE_100NS: u64 = 5_000_000;

/// The newest direct child of `pid` that has been running for at least
/// [`SETTLE_100NS`] (the snapshot lists processes in creation order).
///
/// Windows keeps a process's parent pid after the parent exits, and pids
/// are reused, so a process whose parent pid matches may be an orphan of an
/// older process that had `pid`: only one created after `pid` was is its
/// child.
#[cfg(windows)]
fn newest_child(pid: u32) -> Option<u32> {
    let processes: Vec<(u32, u32)> = win::snapshot()
        .into_iter()
        .map(|(child, parent, _)| (child, parent))
        .collect();
    pick_child(pid, &processes, win::created_100ns, win::age_100ns)
}

/// [`newest_child`]'s choice among `processes` (`(pid, parent pid)`, oldest
/// first), given each process's creation time and age in 100 ns units.
#[cfg(windows)]
fn pick_child(
    pid: u32,
    processes: &[(u32, u32)],
    created: impl Fn(u32) -> Option<u64>,
    age: impl Fn(u32) -> Option<u64>,
) -> Option<u32> {
    let born = created(pid)?;
    processes
        .iter()
        .filter(|(child, parent)| *parent == pid && *child != pid)
        .map(|(child, _)| *child)
        .rev()
        .find(|child| {
            created(*child).is_some_and(|c| c >= born)
                && age(*child).is_some_and(|a| a >= SETTLE_100NS)
        })
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
fn process_cwd(pid: u32) -> Option<PathBuf> {
    win::current_dir(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Emulator, PtyEvent, SpawnSpec};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    /// A command that stays alive long enough to be inspected, the name the
    /// inspection should report, and the arguments after the program.
    fn long_running() -> (Vec<String>, &'static str, &'static [&'static str]) {
        #[cfg(unix)]
        {
            (vec!["sleep".into(), "30".into()], "sleep", &["30"])
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
                &["-n", "30", "127.0.0.1"],
            )
        }
    }

    #[test]
    fn cmdline_and_procargs2_parse_into_arguments() {
        assert_eq!(
            parse_cmdline(b"less\0-R\0\0a b.log\0"),
            Some(vec![
                "less".into(),
                "-R".into(),
                String::new(),
                "a b.log".into()
            ])
        );
        assert_eq!(parse_cmdline(b""), None);
        assert_eq!(parse_cmdline(b"bad\xff\0"), None);

        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/bin/less\0\0\0\0less\0x.log\0HOME=/home/u\0");
        assert_eq!(
            parse_procargs2(&buf),
            Some(vec!["less".into(), "x.log".into()])
        );
        // Fewer arguments than argc says: refused, not truncated.
        let mut short = 3i32.to_ne_bytes().to_vec();
        short.extend_from_slice(b"/bin/x\0x\0");
        assert_eq!(parse_procargs2(&short), None);
    }

    /// A process left with a reused pid's old children is not their parent:
    /// only processes created after it count.
    #[cfg(windows)]
    #[test]
    fn an_orphan_of_a_reused_pid_is_not_a_child() {
        let settled = SETTLE_100NS;
        // Pane process 10, created at 1000. Process 20 claims it as parent
        // but was created at 500, by an older process that had pid 10;
        // process 30 is a real child.
        let created = |pid| match pid {
            10 => Some(1000),
            20 => Some(500),
            30 => Some(2000),
            _ => None,
        };
        let age = |_| Some(settled);
        assert_eq!(pick_child(10, &[(10, 1), (30, 10)], created, age), Some(30));
        assert_eq!(pick_child(10, &[(10, 1), (20, 10)], created, age), None);
        // The orphan is newest in the list: the real child is still found.
        assert_eq!(
            pick_child(10, &[(30, 10), (20, 10)], created, age),
            Some(30)
        );
        // A child too young to have settled is passed over.
        let young = |pid| Some(if pid == 30 { 0 } else { settled });
        assert_eq!(pick_child(10, &[(30, 10)], created, young), None);
    }

    /// A 32-bit program's directory comes from its own (32-bit) PEB: the
    /// 64-bit one's names another directory.
    #[cfg(windows)]
    #[test]
    fn a_32_bit_programs_directory_is_read() {
        let dir = std::env::temp_dir();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: Arc<dyn Fn(PtyEvent) + Send + Sync> =
            Arc::new(move |e| drop(tx.lock().unwrap().send(e)));
        let mut pty = crate::Pty::spawn(
            &SpawnSpec {
                argv: vec![
                    r"C:\Windows\SysWOW64\cmd.exe".into(),
                    "/d".into(),
                    "/k".into(),
                ],
                cwd: Some(dir.clone()),
                rows: 24,
                cols: 80,
                ..SpawnSpec::default()
            },
            sink,
        )
        .unwrap();
        let mut emu = Emulator::new(24, 80, 0);
        let want = dir.canonicalize().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut got = None;
        while Instant::now() < deadline {
            while let Ok(ev) = rx.try_recv() {
                if let PtyEvent::Output(b) = ev {
                    let r = emu.process(&b);
                    if !r.is_empty() {
                        pty.write(&r).unwrap();
                    }
                }
            }
            got = current_dir(&pty).map(|p| p.canonicalize().unwrap());
            if got.as_ref() == Some(&want) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(got, Some(want));
        // Cleanup only; dropping the pty closes the terminal either way.
        let _ = pty.kill();
    }

    #[test]
    fn foreground_command_names_the_running_program() {
        let (argv, want, want_args) = long_running();
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
        let (mut seen, mut args) = (None, None);
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
            // While Linux is in the middle of exec'ing the program, its
            // name is already new but its command line is still empty: wait
            // for both.
            args = foreground_args(&pty);
            if seen
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case(want))
                && args.is_some()
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
        let args = args.expect("foreground arguments");
        assert_eq!(&args[1..], want_args, "{args:?}");
        #[cfg(any(target_os = "linux", windows))]
        assert_eq!(
            current_dir(&pty).map(|p| p.canonicalize().unwrap()),
            Some(dir.canonicalize().unwrap())
        );
        // Cleanup only; dropping the pty closes the terminal either way.
        let _ = pty.kill();
    }
}
