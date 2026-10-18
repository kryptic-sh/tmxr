//! Local wall-clock time, for clock mode and time formats. The standard
//! library has no time zones, so this asks the platform: `localtime_r` on
//! Unix, `FileTimeToSystemTime` and `SystemTimeToTzSpecificLocalTime` on
//! Windows.

use std::time::{SystemTime, UNIX_EPOCH};

/// A moment in local time, broken down as C's `struct tm` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    /// 1 (January) to 12.
    pub month: u8,
    /// 1 to 31.
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// 0 (Sunday) to 6.
    pub weekday: u8,
    /// 0 (1 January) to 365.
    pub yearday: u16,
    /// Seconds since the epoch.
    pub epoch: i64,
}

/// The local time now, or `None` if the platform cannot say.
pub fn now() -> Option<LocalTime> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    at(i64::try_from(secs).ok()?)
}

/// The local time at `epoch` seconds since the epoch, or `None` if the
/// platform cannot say.
pub fn at(epoch: i64) -> Option<LocalTime> {
    imp(epoch)
}

/// The local `(hour, minute, second)`, or `None` if the platform cannot
/// say.
pub fn hour_minute_second() -> Option<(u8, u8, u8)> {
    now().map(|t| (t.hour, t.minute, t.second))
}

#[cfg(unix)]
fn imp(epoch: i64) -> Option<LocalTime> {
    let secs = libc::time_t::try_from(epoch).ok()?;
    // SAFETY: `tm` is integers and (on some platforms) a nullable pointer,
    // for all of which zero is a valid value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `localtime_r` reads `secs` and writes only into `tm`; both
    // outlive the call.
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return None;
    }
    Some(LocalTime {
        year: tm.tm_year.checked_add(1900)?,
        month: u8::try_from(tm.tm_mon + 1).ok()?,
        day: u8::try_from(tm.tm_mday).ok()?,
        hour: u8::try_from(tm.tm_hour).ok()?,
        minute: u8::try_from(tm.tm_min).ok()?,
        second: u8::try_from(tm.tm_sec).ok()?,
        weekday: u8::try_from(tm.tm_wday).ok()?,
        yearday: u16::try_from(tm.tm_yday).ok()?,
        epoch,
    })
}

#[cfg(windows)]
fn imp(epoch: i64) -> Option<LocalTime> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // A FILETIME counts 100-nanosecond ticks from 1601-01-01 (UTC), which is
    // this many seconds before the Unix epoch.
    const EPOCH_1601: i64 = 11_644_473_600;
    let ticks = u64::try_from(epoch.checked_add(EPOCH_1601)?.checked_mul(10_000_000)?).ok()?;
    let file = FILETIME {
        dwLowDateTime: (ticks & 0xffff_ffff) as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let blank = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    let (mut utc, mut local) = (blank, blank);
    // SAFETY: both read `file` / `utc` and write only into the SYSTEMTIME
    // given, all of which outlive the calls; a null zone means the current
    // one.
    let ok = unsafe {
        FileTimeToSystemTime(&file, &mut utc) != 0
            && SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) != 0
    };
    if !ok {
        return None;
    }
    let month = u8::try_from(local.wMonth).ok()?;
    let day = u8::try_from(local.wDay).ok()?;
    let year = i32::from(local.wYear);
    Some(LocalTime {
        year,
        month,
        day,
        hour: u8::try_from(local.wHour).ok()?,
        minute: u8::try_from(local.wMinute).ok()?,
        second: u8::try_from(local.wSecond).ok()?,
        weekday: u8::try_from(local.wDayOfWeek).ok()?,
        yearday: day_of_year(year, month, day),
        epoch,
    })
}

/// Days since 1 January, 0 for that day, in the Gregorian calendar.
#[cfg(any(windows, test))]
fn day_of_year(year: i32, month: u8, day: u8) -> u16 {
    const BEFORE: [u16; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let month = usize::from(month.clamp(1, 12));
    BEFORE[month - 1] + u16::from(leap && month > 2) + u16::from(day) - 1
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let (h, m, s) = hour_minute_second().expect("local time");
            assert!(s < 61, "second {s}");
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

    /// The weekday of a Gregorian date, 0 for Sunday, by Sakamoto's method.
    fn weekday_of(year: i32, month: u8, day: u8) -> u8 {
        const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let y = if month < 3 { year - 1 } else { year };
        let w = (y + y / 4 - y / 100 + y / 400 + T[usize::from(month) - 1] + i32::from(day)) % 7;
        u8::try_from(w).unwrap()
    }

    #[test]
    fn a_moment_breaks_down_into_a_consistent_date() {
        // Known dates check the helpers; the platform's answer for a fixed
        // moment, in whatever zone the test runs in, must agree with them.
        assert_eq!(weekday_of(2023, 11, 14), 2, "a Tuesday");
        assert_eq!(weekday_of(2000, 2, 29), 2, "a Tuesday");
        assert_eq!(day_of_year(2023, 1, 1), 0);
        assert_eq!(day_of_year(2023, 12, 31), 364);
        assert_eq!(day_of_year(2024, 12, 31), 365, "a leap year");
        assert_eq!(day_of_year(2000, 3, 1), 60, "2000 was a leap year");
        assert_eq!(day_of_year(1900, 3, 1), 59, "1900 was not");
        for epoch in [0, 951_782_400, 1_700_000_000, 1_735_689_599] {
            let t = at(epoch).expect("local time");
            assert_eq!(t.epoch, epoch);
            assert_eq!(t.weekday, weekday_of(t.year, t.month, t.day), "{t:?}");
            assert_eq!(t.yearday, day_of_year(t.year, t.month, t.day), "{t:?}");
        }
    }
}
