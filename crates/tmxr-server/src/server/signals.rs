//! SIGTERM and SIGHUP (logout, shutdown, `kill`): end the server the normal
//! way, which saves the sessions, instead of dying where it stands.

use std::sync::atomic::{AtomicBool, Ordering};

static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_signal(_: libc::c_int) {
    // Only an atomic store, which is async-signal-safe.
    EXIT_REQUESTED.store(true, Ordering::SeqCst);
}

/// Route SIGTERM and SIGHUP to [`exit_requested`]. The detached Windows
/// server has no console, so no shutdown or close event reaches it; there is
/// nothing to install there.
pub fn install() {
    #[cfg(unix)]
    for sig in [libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: `on_signal` only stores to an atomic, which a signal
        // handler may do; `signal` itself has no other preconditions.
        unsafe {
            libc::signal(sig, on_signal as libc::sighandler_t);
        }
    }
}

/// Whether a signal asked the server to exit.
pub fn exit_requested() -> bool {
    EXIT_REQUESTED.load(Ordering::SeqCst)
}
