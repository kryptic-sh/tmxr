//! The local wall-clock time, for clock mode. The standard library has no
//! time zones, so this asks the platform: `localtime_r` on Unix,
//! `GetLocalTime` on Windows.

/// The local `(hour, minute)`, or `None` if the platform cannot say.
pub fn hour_minute() -> Option<(u8, u8)> {
    imp()
}

#[cfg(unix)]
fn imp() -> Option<(u8, u8)> {
    // SAFETY: `time` with a null pointer only returns the current time.
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    // SAFETY: `tm` is integers and (on some platforms) a nullable pointer,
    // for all of which zero is a valid value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `localtime_r` reads `now` and writes only into `tm`; both
    // outlive the call.
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return None;
    }
    Some((
        u8::try_from(tm.tm_hour).ok()?,
        u8::try_from(tm.tm_min).ok()?,
    ))
}

#[cfg(windows)]
fn imp() -> Option<(u8, u8)> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut st = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    // SAFETY: `GetLocalTime` writes the time into `st`, which outlives the
    // call.
    unsafe { GetLocalTime(&mut st) };
    Some((u8::try_from(st.wHour).ok()?, u8::try_from(st.wMinute).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn utc_minute_of_day() -> u64 {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        secs / 60 % (24 * 60)
    }

    #[test]
    fn local_time_is_utc_shifted_by_a_real_zone_offset() {
        // Every time zone's offset is a whole number of quarter hours, so
        // local minus UTC must be too; swapped or garbage fields would not be.
        for _ in 0..3 {
            let before = utc_minute_of_day();
            let (h, m) = hour_minute().expect("local time");
            if utc_minute_of_day() != before {
                continue; // crossed a minute boundary; try again
            }
            assert!(h < 24 && m < 60, "{h}:{m}");
            let local = u64::from(h) * 60 + u64::from(m);
            let offset = (local + 24 * 60 - before) % (24 * 60);
            assert_eq!(offset % 15, 0, "local {h}:{m}, utc minute {before}");
            return;
        }
        panic!("minute boundary every time");
    }
}
