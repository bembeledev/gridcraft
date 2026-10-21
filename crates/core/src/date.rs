//! Date serial numbers.
//!
//! The 1900 system counts days from 1899-12-31 (serial 1 = 1900-01-01) and, like Lotus 1-2-3 and
//! Excel, treats 1900 as a leap year: serial 60 is the fictitious 1900-02-29. The 1904 system
//! counts from 1904-01-01 (serial 0). Times are fractions of a day.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DateSystem {
    #[default]
    D1900,
    D1904,
}

/// A calendar date and time of day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Milliseconds (rounded).
    pub milli: u32,
    /// 0 = Sunday.
    pub weekday: u32,
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    // `NaiveDate` counts from 0001-01-01 (num_days_from_ce 1); 1970-01-01 is 719_163.
    match chrono::NaiveDate::from_ymd_opt(y, m, d) {
        Some(date) => i64::from(chrono::Datelike::num_days_from_ce(&date)) - 719_163,
        None => days_from_civil_fallback(y, m, d),
    }
}

/// [`days_from_civil`] for dates outside `NaiveDate`'s range (Howard Hinnant's algorithm).
fn days_from_civil_fallback(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Milliseconds since the Unix epoch (UTC) from the system clock; `None` when the clock is unavailable.
/// On wasm32 `std::time::SystemTime::now` traps, so the browser clock is read through `Date.now()`.
#[allow(clippy::disallowed_methods)] // the std clock is read only off wasm
pub fn unix_now_ms() -> Option<f64> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?;
        Some(d.as_secs_f64() * 1000.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        Some(js_sys::Date::now())
    }
}

/// Date serial (1900 system, with fraction) for a Unix time in milliseconds.
pub fn serial_from_unix_ms(ms: f64) -> f64 {
    25569.0 + ms / 86_400_000.0
}

/// Converts a 1900-system serial (as the clock supplies it) to the serial of the same instant in `sys`.
pub fn serial_in_system(sys: DateSystem, serial_1900: f64) -> f64 {
    match sys {
        DateSystem::D1900 => serial_1900,
        DateSystem::D1904 => serial_1900 - 1462.0,
    }
}

/// Proleptic Gregorian `(year, month, day)` for a day count `z` since 1970-01-01 (the inverse of
/// [`days_from_civil`]).
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    // `NaiveDate` counts from 0001-01-01 (num_days_from_ce 1); 1970-01-01 is 719_163.
    match z.checked_add(719_163).and_then(|n| i32::try_from(n).ok()).and_then(chrono::NaiveDate::from_num_days_from_ce_opt) {
        Some(d) => (chrono::Datelike::year(&d), chrono::Datelike::month(&d), chrono::Datelike::day(&d)),
        None => civil_from_days_fallback(z),
    }
}

/// [`civil_from_days`] for values outside `NaiveDate`'s range (Howard Hinnant's algorithm).
fn civil_from_days_fallback(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

const UNIX_1899_12_30: i64 = -25569; // days_from_civil(1899, 12, 30)
const UNIX_1904_01_01: i64 = -24107;

/// Whether `y` is a Gregorian leap year. Works for every `i32`, including years outside the range
/// chrono can represent, by falling back to the Gregorian rule directly.
pub fn is_leap(y: i32) -> bool {
    // A year is a leap year iff 29 February exists in it; chrono validates that in range.
    match chrono::NaiveDate::from_ymd_opt(y, 2, 29) {
        Some(_) => true,
        // Out of chrono's range: the pure Gregorian rule (divisible by 4, except centuries not by 400).
        None => y % 4 == 0 && (y % 100 != 0 || y % 400 == 0),
    }
}

/// Number of days in month `m` of year `y` (1–31), or `0` for an invalid month number. Works for
/// every `i32` year, including years outside chrono's range, via [`days_from_civil`]'s fallback.
pub fn days_in_month(y: i32, m: u32) -> u32 {
    // The length of a month is the day before the first of the next month. An out-of-range month
    // keeps the old `0`.
    if !(1..=12).contains(&m) {
        return 0;
    }
    // December rolls into next January; `checked_add` avoids an overflow panic at `i32::MAX`.
    let (ny, nm) = if m == 12 {
        match y.checked_add(1) {
            Some(ny) => (ny, 1),
            None => return 31, // i32::MAX December has 31 days; the Gregorian rule is month-only here.
        }
    } else {
        (y, m + 1)
    };
    u32::try_from(days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)).unwrap_or(0)
}

/// Days in a month of a date system's calendar: like [`days_in_month`], except that February
/// 1900 has 29 days in the 1900 system (serial 60 is the fictitious 1900-02-29).
pub fn days_in_month_in(sys: DateSystem, y: i32, m: u32) -> u32 {
    if sys == DateSystem::D1900 && y == 1900 && m == 2 { 29 } else { days_in_month(y, m) }
}

/// Last serial that is a valid date in a date system: 9999-12-31 (2958465 in the 1900 system, 2957003 in the 1904 system).
pub fn max_serial(sys: DateSystem) -> i64 {
    match sys {
        DateSystem::D1900 => 2_958_465,
        DateSystem::D1904 => 2_958_465 - 1462,
    }
}

/// Serial for a date. Month and day overflow roll over like `DATE()` (month 13 = January next
/// year, day 0 = last day of the previous month), in the date system's own calendar: in the 1900
/// system February 1900 has 29 days, so `(1900, 2, 29)` and `(1900, 3, 0)` are serial 60.
/// `None` before the epoch or after 9999-12-31.
pub fn serial_from_ymd(sys: DateSystem, year: i64, month: i64, day: i64) -> Option<f64> {
    // DATE(): years 0–1899 are added to 1900.
    let y = if (0..1900).contains(&year) { year + 1900 } else { year };
    let m0 = month.checked_sub(1)?;
    let y = y.checked_add(m0.div_euclid(12))?;
    let m = m0.rem_euclid(12) + 1;
    if !(1900..=9999).contains(&y) && sys == DateSystem::D1900 || !(1904..=9999).contains(&y) && sys == DateSystem::D1904 {
        return None;
    }
    let first = days_from_civil(y as i32, m as u32, 1);
    // Serial of the first of the month; the day (and any overflow) counts on from there.
    let first = match sys {
        // Up to February 1900 the serial is one lower than the day count from 1899-12-30: the
        // fictitious 1900-02-29 (serial 60) comes before 1 March.
        DateSystem::D1900 => first - UNIX_1899_12_30 - i64::from(y == 1900 && m <= 2),
        DateSystem::D1904 => first - UNIX_1904_01_01,
    };
    let serial = first.checked_add(day)?.checked_sub(1)?;
    if !(0..=max_serial(sys)).contains(&serial) {
        return None;
    }
    Some(serial as f64)
}

/// Date/time from a serial. `None` for negative or too-large serials.
pub fn datetime_from_serial(sys: DateSystem, serial: f64) -> Option<DateTime> {
    if !serial.is_finite() || !(0.0..max_serial(sys) as f64 + 1.0).contains(&serial) {
        return None;
    }
    let mut day = serial.floor() as i64;
    let mut frac_ms = ((serial - serial.floor()) * 86_400_000.0).round() as i64;
    if frac_ms >= 86_400_000 {
        frac_ms -= 86_400_000;
        day += 1;
    }
    let (y, m, d, weekday) = match sys {
        DateSystem::D1900 => {
            // Excel's weekdays continue the fictitious calendar: serial 1 is a Sunday.
            let wd = ((day + 6) % 7) as u32;
            if day == 60 {
                (1900, 2, 29, wd)
            } else if day == 0 {
                (1900, 1, 0, wd)
            } else {
                let real = if day < 60 { day + 1 } else { day };
                let (y, m, d) = civil_from_days(real + UNIX_1899_12_30);
                (y, m, d, wd)
            }
        }
        DateSystem::D1904 => {
            let unix = day + UNIX_1904_01_01;
            let (y, m, d) = civil_from_days(unix);
            (y, m, d, ((unix + 4).rem_euclid(7)) as u32)
        }
    };
    let secs = frac_ms / 1000;
    Some(DateTime {
        year: y,
        month: m,
        day: d,
        hour: (secs / 3600) as u32,
        minute: ((secs / 60) % 60) as u32,
        second: (secs % 60) as u32,
        milli: (frac_ms % 1000) as u32,
        weekday,
    })
}

/// Fraction of a day for a time.
pub fn time_fraction(h: f64, m: f64, s: f64) -> f64 {
    (h * 3600.0 + m * 60.0 + s) / 86400.0
}

pub const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
pub const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_and_month_length_outside_chrono_range() {
        // In range, the ordinary Gregorian rule holds.
        assert!(is_leap(2000));
        assert!(!is_leap(1900));
        assert!(is_leap(2024));
        assert!(!is_leap(2023));
        // Outside chrono's NaiveDate range the Gregorian rule is still applied.
        assert!(is_leap(400_000)); // divisible by 400
        assert!(!is_leap(300_100)); // century not divisible by 400
        assert!(is_leap(i32::MAX - 3)); // 2147483644 is divisible by 4, not a century
        assert!(!is_leap(i32::MAX)); // 2147483647 is odd
        // days_in_month keeps calendar lengths out of range and 0 for invalid months.
        assert_eq!(days_in_month(400_000, 2), 29);
        assert_eq!(days_in_month(300_100, 2), 28);
        assert_eq!(days_in_month(400_000, 1), 31);
        assert_eq!(days_in_month(400_000, 4), 30);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2023, 0), 0);
        assert_eq!(days_in_month(2023, 13), 0);
        // December at i32::MAX does not overflow and still reports 31 days.
        assert_eq!(days_in_month(i32::MAX, 12), 31);
        assert_eq!(days_in_month(i32::MAX, 2), 28); // 2147483647 is not a leap year
    }

    #[test]
    fn serial_from_unix_time() {
        assert_eq!(serial_from_unix_ms(0.0), 25569.0);
        // 2026-10-10T12:00:00Z
        assert_eq!(serial_from_unix_ms(1_791_633_600_000.0), 46305.5);
        assert!(unix_now_ms().is_some_and(|ms| serial_from_unix_ms(ms) > 46305.0));
    }

    #[test]
    fn known_serials() {
        let s = DateSystem::D1900;
        assert_eq!(serial_from_ymd(s, 1900, 1, 1), Some(1.0));
        assert_eq!(serial_from_ymd(s, 1900, 2, 28), Some(59.0));
        assert_eq!(serial_from_ymd(s, 1900, 3, 1), Some(61.0));
        assert_eq!(serial_from_ymd(s, 2000, 1, 1), Some(36526.0));
        assert_eq!(serial_from_ymd(s, 2026, 10, 7), Some(46302.0));
        assert_eq!(serial_from_ymd(s, 9999, 12, 31), Some(2958465.0));
        assert_eq!(serial_from_ymd(DateSystem::D1904, 9999, 12, 31), Some(2957003.0));
        assert_eq!(serial_from_ymd(DateSystem::D1904, 9999, 12, 32), None);
        assert!(datetime_from_serial(DateSystem::D1904, 2957003.5).is_some());
        assert!(datetime_from_serial(DateSystem::D1904, 2957004.0).is_none());
        assert_eq!(serial_from_ymd(s, 2020, 13, 1), serial_from_ymd(s, 2021, 1, 1));
        assert_eq!(serial_from_ymd(s, 2020, 3, 0), serial_from_ymd(s, 2020, 2, 29));
        assert_eq!(serial_from_ymd(s, 120, 1, 1), serial_from_ymd(s, 2020, 1, 1));
        assert_eq!(serial_from_ymd(DateSystem::D1904, 1904, 1, 1), Some(0.0));
        assert_eq!(serial_from_ymd(DateSystem::D1904, 2000, 1, 1), Some(35064.0));
    }

    #[test]
    fn fictitious_1900_02_29() {
        // Excel's 1900 calendar has a 29 February 1900 (serial 60); day and month overflow count
        // through it.
        let s = DateSystem::D1900;
        assert_eq!(serial_from_ymd(s, 1900, 2, 29), Some(60.0));
        assert_eq!(serial_from_ymd(s, 1900, 3, 0), Some(60.0));
        assert_eq!(serial_from_ymd(s, 1900, 2, 30), Some(61.0));
        assert_eq!(serial_from_ymd(s, 1900, 1, 60), Some(60.0));
        assert_eq!(serial_from_ymd(s, 1900, 1, 61), Some(61.0));
        assert_eq!(serial_from_ymd(s, 1900, 1, 400), Some(400.0));
        assert_eq!(serial_from_ymd(s, 1900, 4, -30), Some(61.0));
        assert_eq!(serial_from_ymd(s, 1900, 4, -31), Some(60.0));
        assert_eq!(serial_from_ymd(s, 1900, 3, -1), Some(59.0));
        assert_eq!(serial_from_ymd(s, 1901, -10, 29), Some(60.0));
        assert_eq!(serial_from_ymd(s, 1900, 1, 0), Some(0.0));
        assert_eq!(serial_from_ymd(s, 1900, 1, -1), None);
        assert_eq!(serial_from_ymd(s, 1900, 0, 1), None);
        // Later years keep the real calendar.
        assert_eq!(serial_from_ymd(s, 1901, 2, 29), serial_from_ymd(s, 1901, 3, 1));
        assert_eq!(serial_from_ymd(s, 1904, 2, 29), Some(1521.0));
        // Every serial up to 1900-12-31 comes back from its own year, month and day.
        for serial in 0..=366 {
            let d = datetime_from_serial(s, serial as f64).unwrap();
            assert_eq!(serial_from_ymd(s, d.year as i64, d.month as i64, d.day as i64), Some(serial as f64), "{d:?}");
        }
        assert_eq!(days_in_month_in(s, 1900, 2), 29);
        assert_eq!(days_in_month_in(s, 1901, 2), 28);
        assert_eq!(days_in_month_in(DateSystem::D1904, 1900, 2), 28);
        // The 1904 system has no fictitious day: 1904 is a real leap year.
        let s = DateSystem::D1904;
        assert_eq!(serial_from_ymd(s, 1904, 2, 29), Some(59.0));
        assert_eq!(serial_from_ymd(s, 1904, 3, 0), Some(59.0));
        assert_eq!(serial_from_ymd(s, 1904, 3, 1), Some(60.0));
        assert_eq!(serial_from_ymd(s, 1904, 1, 0), None);
        assert_eq!(serial_from_ymd(s, 1900, 2, 29), None);
    }

    #[test]
    fn hostile_month_and_day() {
        let s = DateSystem::D1900;
        assert_eq!(serial_from_ymd(s, 2000, i64::MIN, 1), None);
        assert_eq!(serial_from_ymd(s, 2000, i64::MAX, 1), None);
        assert_eq!(serial_from_ymd(s, 2000, 1, i64::MAX), None);
        assert_eq!(serial_from_ymd(s, 2000, 1, i64::MIN), None);
        assert_eq!(serial_from_ymd(s, i64::MAX, 1, 1), None);
    }

    #[test]
    fn roundtrip() {
        let s = DateSystem::D1900;
        for serial in [1.0, 59.0, 60.0, 61.0, 36526.0, 46302.0, 2958465.0] {
            let d = datetime_from_serial(s, serial).unwrap();
            assert_eq!(serial_from_ymd(s, d.year as i64, d.month as i64, d.day as i64), Some(serial));
        }
        let d = datetime_from_serial(s, 60.0).unwrap();
        assert_eq!((d.year, d.month, d.day), (1900, 2, 29));
        let d = datetime_from_serial(s, 46302.75).unwrap();
        assert_eq!((d.hour, d.minute, d.weekday), (18, 0, 3)); // 2026-10-07 is a Wednesday
        let d = datetime_from_serial(s, 1.0).unwrap();
        assert_eq!(d.weekday, 0); // 1900-01-01 was a Sunday in Excel's calendar
    }
}
