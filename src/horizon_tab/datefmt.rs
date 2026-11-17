//! Java `SimpleDateFormat` patterns (`dd.MM.yyyy`, `[HH:mm:ss / h:mm a]`), which TAB's
//! `placeholders.date-format` / `time-format` use, over chrono. Letters supported: `y M d E a H k
//! K h m s S z Z u D`; text in single quotes is literal, `''` is a quote. Month and day names are
//! English (`locale: en-US`).

use chrono::{DateTime, Datelike, FixedOffset, Timelike};

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
const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

pub fn format(pattern: &str, t: &DateTime<FixedOffset>) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            if chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        out.push('\'');
                        i += 2;
                        continue;
                    }
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }
        let mut n = 1;
        while chars.get(i + n) == Some(&c) {
            n += 1;
        }
        i += n;
        let num = |v: i64| format!("{v:0n$}");
        match c {
            'y' => {
                let y = t.year() as i64;
                out.push_str(&if n == 2 {
                    format!("{:02}", y.rem_euclid(100))
                } else {
                    num(y)
                });
            }
            'M' => {
                let m = t.month() as usize;
                out.push_str(&match n {
                    1 | 2 => num(m as i64),
                    3 => MONTHS[m - 1][..3].to_owned(),
                    _ => MONTHS[m - 1].to_owned(),
                });
            }
            'd' => out.push_str(&num(t.day() as i64)),
            'D' => out.push_str(&num(t.ordinal() as i64)),
            'E' => {
                let d = DAYS[t.weekday().num_days_from_monday() as usize];
                out.push_str(if n >= 4 { d } else { &d[..3] });
            }
            'u' => out.push_str(&num(t.weekday().number_from_monday() as i64)),
            'a' => out.push_str(if t.hour() < 12 { "AM" } else { "PM" }),
            'H' => out.push_str(&num(t.hour() as i64)),
            'k' => out.push_str(&num(if t.hour() == 0 { 24 } else { t.hour() as i64 })),
            'K' => out.push_str(&num((t.hour() % 12) as i64)),
            'h' => out.push_str(&num(match t.hour() % 12 {
                0 => 12,
                h => h as i64,
            })),
            'm' => out.push_str(&num(t.minute() as i64)),
            's' => out.push_str(&num(t.second() as i64)),
            'S' => {
                let ms = (t.nanosecond() / 1_000_000).min(999) as i64;
                out.push_str(&num(ms));
            }
            'z' | 'Z' | 'X' => {
                let secs = t.offset().local_minus_utc();
                let sign = if secs < 0 { '-' } else { '+' };
                let (h, m) = (secs.abs() / 3600, secs.abs() % 3600 / 60);
                if c == 'z' {
                    out.push_str(&format!("GMT{sign}{h:02}:{m:02}"));
                } else {
                    out.push_str(&format!("{sign}{h:02}{m:02}"));
                }
            }
            other => {
                for _ in 0..n {
                    out.push(other);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn tab_defaults() {
        let t = FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 3, 7, 15, 4, 9)
            .unwrap();
        assert_eq!(format("dd.MM.yyyy", &t), "07.03.2026");
        assert_eq!(format("[HH:mm:ss / h:mm a]", &t), "[15:04:09 / 3:04 PM]");
        assert_eq!(format("EEE, MMM d ''yy 'at' H", &t), "Sat, Mar 7 '26 at 15");
        assert_eq!(format("MM/dd/yy HH:mm:ss", &t), "03/07/26 15:04:09");
    }
}
