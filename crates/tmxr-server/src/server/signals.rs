//! Ending the server the normal way, which saves the sessions, instead of
//! dying where it stands, when the system asks it to stop: SIGTERM and SIGHUP
//! on Unix (logout, shutdown, `kill`), the end of the Windows session (logoff,
//! shutdown) on Windows.

use std::sync::atomic::{AtomicBool, Ordering};

static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);
/// Set once the exit save has been made, for a handler that must not return
/// before it.
static SAVED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_signal(_: libc::c_int) {
    // Only an atomic store, which is async-signal-safe.
    EXIT_REQUESTED.store(true, Ordering::SeqCst);
}

/// Route SIGTERM and SIGHUP (Unix), or the end of the Windows session, to
/// [`exit_requested`].
pub fn install() {
    #[cfg(unix)]
    for sig in [libc::SIGTERM, libc::SIGHUP] {
        // `signal` takes the handler as an address.
        let handler: extern "C" fn(libc::c_int) = on_signal;
        // SAFETY: `on_signal` only stores to an atomic, which a signal
        // handler may do; `signal` itself has no other preconditions.
        unsafe {
            libc::signal(sig, handler as libc::sighandler_t);
        }
    }
    #[cfg(windows)]
    session_end::install();
}

/// Whether the system asked the server to exit.
pub fn exit_requested() -> bool {
    EXIT_REQUESTED.load(Ordering::SeqCst)
}

/// The server has saved on its way out.
pub fn mark_saved() {
    SAVED.store(true, Ordering::SeqCst);
}

/// The detached Windows server has no console, so no console control event
/// reaches it. Windows tells a process the session is ending through
/// `WM_QUERYENDSESSION` / `WM_ENDSESSION`, sent to its top-level windows, so
/// the server keeps a hidden one on its own thread.
#[cfg(windows)]
mod session_end {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW,
        TranslateMessage, WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW, WS_OVERLAPPED,
    };

    /// The window class; the title adds the server's pid, so a test can find
    /// the window (`FindWindowW`).
    pub const CLASS: &str = "tmxr-server";

    /// How long the session's end waits for the save: Windows ends the
    /// process once `WM_ENDSESSION` returns.
    const SAVE_WAIT: Duration = Duration::from_secs(5);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn install() {
        let spawned = std::thread::Builder::new()
            .name("tmxr-session-end".into())
            .spawn(run);
        if let Err(e) = spawned {
            tracing::warn!("no session-end window, logoff will not save: {e}");
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            // Do not hold the session up.
            WM_QUERYENDSESSION => 1,
            WM_ENDSESSION => {
                if wp != 0 {
                    super::EXIT_REQUESTED.store(true, Ordering::SeqCst);
                    let deadline = Instant::now() + SAVE_WAIT;
                    while !super::SAVED.load(Ordering::SeqCst) && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                0
            }
            // SAFETY: the arguments are the ones this window procedure was
            // called with.
            _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
        }
    }

    fn run() {
        let class = wide(CLASS);
        let title = wide(&format!("{CLASS} {}", std::process::id()));
        // SAFETY: a null name asks for this executable's module handle.
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            // SAFETY: WNDCLASSW is integers, handles and pointers, for which
            // zero is valid (none, null).
            ..unsafe { std::mem::zeroed() }
        };
        // SAFETY: `wc` is fully initialised and `class` outlives the class
        // registration, which lasts as long as this thread's loop.
        if unsafe { RegisterClassW(&wc) } == 0 {
            tracing::warn!("no session-end window class, logoff will not save");
            return;
        }
        // A top-level window, never shown: message-only windows do not get
        // the session-end broadcast.
        // SAFETY: the class was registered above; the strings are
        // NUL-terminated and live through the call.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            tracing::warn!("no session-end window, logoff will not save");
            return;
        }
        // SAFETY: MSG is plain data, zero is valid.
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        // SAFETY: `msg` is a live MSG; GetMessageW fills it, and returns 0
        // on WM_QUIT and -1 on error, both ending the loop.
        while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
            // SAFETY: `msg` was just filled by GetMessageW.
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
