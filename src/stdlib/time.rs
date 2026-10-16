//! Dates and times. Timestamps are Unix seconds (a number, possibly with a
//! fraction), and every calendar computation is in UTC. The `date` module is
//! pure; only `now`, which reads the clock, needs the `time` capability.

use super::{Library, checked_str, num, text};
use crate::error::{Error, Result};
use crate::value::{Map, Value};

const DAY: i64 = 86_400;
/// Years outside this range are refused, so the arithmetic can't overflow.
const YEARS: std::ops::RangeInclusive<i64> = -9999..=9999;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `days` after 1970-01-01, as (year, month, day).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if month <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        month,
        day,
    )
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A broken-down UTC time.
struct Civil {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    millis: i64,
}

impl Civil {
    fn from_timestamp(ts: f64) -> Result<Civil> {
        let limit = (days_from_civil(10_000, 1, 1) * DAY) as f64;
        if !ts.is_finite() || ts.abs() >= limit {
            return Err(Error::value_error(format!(
                "{ts} is not a timestamp between the years -9999 and 9999"
            )));
        }
        let total_ms = (ts * 1000.0).floor() as i64;
        let secs = total_ms.div_euclid(1000);
        let days = secs.div_euclid(DAY);
        let rem = secs.rem_euclid(DAY);
        let (year, month, day) = civil_from_days(days);
        Ok(Civil {
            year,
            month,
            day,
            hour: rem / 3600,
            minute: rem / 60 % 60,
            second: rem % 60,
            millis: total_ms.rem_euclid(1000),
        })
    }

    fn timestamp(&self) -> Value {
        let secs = days_from_civil(self.year, self.month, self.day) * DAY
            + self.hour * 3600
            + self.minute * 60
            + self.second;
        if self.millis == 0 {
            Value::Int(secs)
        } else {
            Value::num(secs as f64 + self.millis as f64 / 1000.0)
        }
    }

    fn days(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// 1 (Monday) to 7 (Sunday).
    fn weekday(&self) -> i64 {
        (self.days() + 3).rem_euclid(7) + 1
    }

    fn yday(&self) -> i64 {
        self.days() - days_from_civil(self.year, 1, 1) + 1
    }
}

fn format(ts: f64, fmt: &str) -> Result<String> {
    use std::fmt::Write;
    let t = Civil::from_timestamp(ts)?;
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let _ = match chars.next() {
            Some('Y') => write!(out, "{:04}", t.year),
            Some('y') => write!(out, "{:02}", t.year.rem_euclid(100)),
            Some('m') => write!(out, "{:02}", t.month),
            Some('d') => write!(out, "{:02}", t.day),
            Some('e') => write!(out, "{:2}", t.day),
            Some('H') => write!(out, "{:02}", t.hour),
            Some('I') => write!(out, "{:02}", (t.hour + 11) % 12 + 1),
            Some('p') => write!(out, "{}", if t.hour < 12 { "AM" } else { "PM" }),
            Some('M') => write!(out, "{:02}", t.minute),
            Some('S') => write!(out, "{:02}", t.second),
            Some('f') => write!(out, "{:03}", t.millis),
            Some('j') => write!(out, "{:03}", t.yday()),
            Some('u') => write!(out, "{}", t.weekday()),
            Some('w') => write!(out, "{}", t.weekday() % 7),
            Some('a') => write!(out, "{}", &WEEKDAYS[t.weekday() as usize - 1][..3]),
            Some('A') => write!(out, "{}", WEEKDAYS[t.weekday() as usize - 1]),
            Some('b') => write!(out, "{}", &MONTHS[t.month as usize - 1][..3]),
            Some('B') => write!(out, "{}", MONTHS[t.month as usize - 1]),
            Some('F') => write!(out, "{:04}-{:02}-{:02}", t.year, t.month, t.day),
            Some('T') => write!(out, "{:02}:{:02}:{:02}", t.hour, t.minute, t.second),
            Some('s') => write!(out, "{}", ts.floor() as i64),
            Some('z') => write!(out, "+0000"),
            Some('Z') => write!(out, "UTC"),
            Some('%') => write!(out, "%"),
            Some(other) => {
                return Err(Error::value_error(format!(
                    "unknown date format `%{other}`"
                )));
            }
            None => return Err(Error::value_error("a date format can't end with `%`")),
        };
    }
    Ok(out)
}

/// Reads a date by a format; see [`format`] for the directives.
struct Reader<'a> {
    text: &'a str,
    pos: usize,
}

impl Reader<'_> {
    fn fail(&self) -> Error {
        Error::value_error(format!(
            "{:?} doesn't match the date format at {:?}",
            self.text,
            &self.text[self.pos..]
        ))
    }

    fn number(&mut self, min: usize, max: usize, signed: bool) -> Result<i64> {
        let rest = &self.text[self.pos..];
        let sign_len = usize::from(signed && rest.starts_with(['-', '+']));
        let digits = rest[sign_len..]
            .bytes()
            .take(max)
            .take_while(u8::is_ascii_digit)
            .count();
        if digits < min {
            return Err(self.fail());
        }
        let end = sign_len + digits;
        let n = rest[..end].parse().map_err(|_| self.fail())?;
        self.pos += end;
        Ok(n)
    }

    /// One of `names`, in full or by its first three letters, ignoring case.
    fn name(&mut self, names: &[&str]) -> Result<i64> {
        let rest = self.text[self.pos..].to_ascii_lowercase();
        for (i, name) in names.iter().enumerate() {
            let name = name.to_ascii_lowercase();
            for len in [name.len(), 3.min(name.len())] {
                if rest.starts_with(&name[..len]) {
                    self.pos += len;
                    return Ok(i as i64 + 1);
                }
            }
        }
        Err(self.fail())
    }

    fn literal(&mut self, s: &str) -> Result<()> {
        if self.text[self.pos..].starts_with(s) {
            self.pos += s.len();
            Ok(())
        } else {
            Err(self.fail())
        }
    }

    /// `Z`, `UTC`, `+HH`, `+HHMM` or `+HH:MM`, as seconds east of UTC.
    fn offset(&mut self) -> Result<i64> {
        let rest = &self.text[self.pos..];
        if let Some(len) = ["UTC", "Z", "z"]
            .iter()
            .find_map(|z| rest.starts_with(z).then_some(z.len()))
        {
            self.pos += len;
            return Ok(0);
        }
        let sign = match rest.chars().next() {
            Some('+') => 1,
            Some('-') => -1,
            _ => return Err(self.fail()),
        };
        self.pos += 1;
        let hours = self.number(2, 2, false)?;
        if self.text[self.pos..].starts_with(':') {
            self.pos += 1;
        }
        let minutes = if self.text[self.pos..].starts_with(|c: char| c.is_ascii_digit()) {
            self.number(2, 2, false)?
        } else {
            0
        };
        Ok(sign * (hours * 3600 + minutes * 60))
    }
}

/// Parses `text` by `fmt`, or as ISO 8601 when there's no format.
fn parse(text: &str, fmt: Option<&str>) -> Result<Value> {
    let mut r = Reader { text, pos: 0 };
    let mut t = Civil {
        year: 1970,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        millis: 0,
    };
    let mut offset = 0;
    let mut pm = None;
    let mut unix = None;
    match fmt {
        None => {
            // YYYY-MM-DD, then optionally [T ]HH:MM[:SS[.fff]] and an offset.
            t.year = r.number(4, 4, true)?;
            r.literal("-")?;
            t.month = r.number(2, 2, false)?;
            r.literal("-")?;
            t.day = r.number(2, 2, false)?;
            if r.text[r.pos..].starts_with(['T', 't', ' ']) {
                r.pos += 1;
                t.hour = r.number(2, 2, false)?;
                r.literal(":")?;
                t.minute = r.number(2, 2, false)?;
                if r.text[r.pos..].starts_with(':') {
                    r.pos += 1;
                    t.second = r.number(2, 2, false)?;
                    if r.text[r.pos..].starts_with(['.', ',']) {
                        r.pos += 1;
                        let start = r.pos;
                        let digits = r.number(1, 9, false)?;
                        let len = r.pos - start;
                        t.millis = digits * 1000 / 10i64.pow(len as u32);
                    }
                }
                if r.pos < text.len() {
                    offset = r.offset()?;
                }
            }
        }
        Some(fmt) => {
            let mut chars = fmt.chars();
            while let Some(c) = chars.next() {
                if c.is_whitespace() {
                    let rest = &r.text[r.pos..];
                    r.pos += rest.len() - rest.trim_start().len();
                    continue;
                }
                if c != '%' {
                    let mut buf = [0; 4];
                    r.literal(c.encode_utf8(&mut buf))?;
                    continue;
                }
                match chars.next() {
                    Some('Y') => t.year = r.number(1, 4, true)?,
                    Some('y') => {
                        // POSIX: 69–99 are 1969–1999, 00–68 are 2000–2068.
                        let yy = r.number(2, 2, false)?;
                        t.year = if yy < 69 { 2000 + yy } else { 1900 + yy };
                    }
                    Some('m') => t.month = r.number(1, 2, false)?,
                    Some('d' | 'e') => {
                        let rest = &r.text[r.pos..];
                        r.pos += rest.len() - rest.trim_start().len();
                        t.day = r.number(1, 2, false)?;
                    }
                    Some('H') => t.hour = r.number(1, 2, false)?,
                    Some('I') => t.hour = r.number(1, 2, false)? % 12,
                    Some('M') => t.minute = r.number(1, 2, false)?,
                    Some('S') => t.second = r.number(1, 2, false)?,
                    Some('f') => t.millis = r.number(3, 3, false)?,
                    Some('p') => pm = Some(r.name(&["AM", "PM"])? == 2),
                    Some('b' | 'B') => t.month = r.name(&MONTHS)?,
                    Some('a' | 'A') => {
                        r.name(&WEEKDAYS)?;
                    }
                    Some('F') => {
                        t.year = r.number(1, 4, true)?;
                        r.literal("-")?;
                        t.month = r.number(1, 2, false)?;
                        r.literal("-")?;
                        t.day = r.number(1, 2, false)?;
                    }
                    Some('T') => {
                        t.hour = r.number(1, 2, false)?;
                        r.literal(":")?;
                        t.minute = r.number(1, 2, false)?;
                        r.literal(":")?;
                        t.second = r.number(1, 2, false)?;
                    }
                    Some('s') => unix = Some(r.number(1, 15, true)?),
                    Some('z' | 'Z') => offset = r.offset()?,
                    Some('%') => r.literal("%")?,
                    Some(other) => {
                        return Err(Error::value_error(format!(
                            "unknown date format `%{other}`"
                        )));
                    }
                    None => return Err(Error::value_error("a date format can't end with `%`")),
                }
            }
        }
    }
    if r.pos != text.len() {
        return Err(r.fail());
    }
    if let Some(secs) = unix {
        return Ok(Value::Int(secs));
    }
    if pm == Some(true) {
        t.hour += 12;
    }
    let valid = YEARS.contains(&t.year)
        && (1..=12).contains(&t.month)
        && (1..=days_in_month(t.year, t.month)).contains(&t.day)
        && (0..24).contains(&t.hour)
        && (0..60).contains(&t.minute)
        && (0..61).contains(&t.second);
    if !valid {
        return Err(Error::value_error(format!("{text:?} is not a valid date")));
    }
    Ok(match t.timestamp() {
        Value::Int(secs) => Value::Int(secs - offset),
        ts => Value::num(num(&ts) - offset as f64),
    })
}

pub(super) fn install(lib: &mut Library) {
    let mut date = Library::new();
    date.def("format", "timestamp: num", |vm, args| {
        checked_str(vm, format(num(&args[0]), "%Y-%m-%dT%H:%M:%SZ")?)
    });
    date.def("format", "timestamp: num, format: str", |vm, args| {
        let fmt = text(&args[1]);
        vm.meter().tick(fmt.len() as u64 / 8)?;
        vm.meter().str_len(fmt.len().saturating_mul(10))?;
        checked_str(vm, format(num(&args[0]), fmt)?)
    });
    date.def("parse", "text: str", |_, args| parse(text(&args[0]), None));
    date.def("parse", "text: str, format: str", |vm, args| {
        vm.meter().tick(text(&args[1]).len() as u64 / 8)?;
        parse(text(&args[0]), Some(text(&args[1])))
    });
    date.def("parts", "timestamp: num", |_, args| {
        let t = Civil::from_timestamp(num(&args[0]))?;
        let mut map = Map::default();
        for (key, value) in [
            ("year", t.year),
            ("month", t.month),
            ("day", t.day),
            ("hour", t.hour),
            ("minute", t.minute),
            ("second", t.second),
            ("millis", t.millis),
            ("weekday", t.weekday()),
            ("yday", t.yday()),
        ] {
            map.insert(key.into(), Value::Int(value));
        }
        Ok(Value::map(map))
    });
    lib.module("date", date);
}

pub(super) fn install_time(lib: &mut Library) {
    lib.def("now", "", |_, _| {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::value_error("the system clock is set before 1970"))?;
        let ms = since.as_millis() as i64;
        Ok(if ms % 1000 == 0 {
            Value::Int(ms / 1000)
        } else {
            Value::num(ms as f64 / 1000.0)
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips() {
        for days in [-1_000_000, -719_468, -1, 0, 1, 11_016, 19_000, 2_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[test]
    fn parse_and_format() {
        let ts = parse("2024-02-29T13:45:30.250+02:00", None).unwrap();
        assert_eq!(num(&ts), 1_709_207_130.25);
        assert_eq!(
            format(num(&ts), "%A %d %B %Y, %I:%M:%S.%f %p").unwrap(),
            "Thursday 29 February 2024, 11:45:30.250 AM"
        );
        let ts = parse("Mar 5 99 4:07 pm", Some("%b %d %y %I:%M %p")).unwrap();
        assert_eq!(format(num(&ts), "%F %T").unwrap(), "1999-03-05 16:07:00");
        assert!(parse("2023-02-29", None).is_err());
        assert!(parse("2024-01-01 junk", None).is_err());
    }

    proptest::proptest! {
        #[test]
        fn never_panics(text in ".{0,24}", fmt in "(%[a-zA-Z%]|[ :/-]|.){0,8}", ts in proptest::num::f64::ANY) {
            let _ = parse(&text, None);
            let _ = parse(&text, Some(&fmt));
            let _ = format(ts, &fmt);
        }
    }
}
