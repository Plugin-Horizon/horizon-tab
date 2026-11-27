//! Java `SimpleDateFormat` patterns (`dd.MM.yyyy`, `[HH:mm:ss / h:mm a]`), which TAB's
//! `placeholders.date-format` / `time-format` use, over chrono. Letters supported: `y M d E a H k
//! K h m s S z Z u D`; text in single quotes is literal, `''` is a quote. Month and day names and
//! AM/PM follow `placeholders.locale` (a language tag such as `de-DE`): en, de, fr, es, it, nl, pt,
//! sv, pl, as Java's CLDR data names them; any other tag falls back to `en-US`, as TAB's own
//! fallback for an invalid tag does.

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

/// The names of one language: full months, short months, full days, short days, AM and PM.
struct Names {
    months: [&'static str; 12],
    months_short: [&'static str; 12],
    days: [&'static str; 7],
    days_short: [&'static str; 7],
    am: &'static str,
    pm: &'static str,
}

const EN: Names = Names {
    months: MONTHS,
    months_short: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
    days: DAYS,
    days_short: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    am: "AM",
    pm: "PM",
};
const DE: Names = Names {
    months: ["Januar", "Februar", "M\u{e4}rz", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"],
    months_short: ["Jan.", "Feb.", "M\u{e4}rz", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez."],
    days: ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"],
    days_short: ["Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa.", "So."],
    am: "AM",
    pm: "PM",
};
const FR: Names = Names {
    months: ["janvier", "f\u{e9}vrier", "mars", "avril", "mai", "juin", "juillet", "ao\u{fb}t", "septembre", "octobre", "novembre", "d\u{e9}cembre"],
    months_short: ["janv.", "f\u{e9}vr.", "mars", "avr.", "mai", "juin", "juil.", "ao\u{fb}t", "sept.", "oct.", "nov.", "d\u{e9}c."],
    days: ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"],
    days_short: ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."],
    am: "AM",
    pm: "PM",
};
const ES: Names = Names {
    months: ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"],
    months_short: ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic"],
    days: ["lunes", "martes", "mi\u{e9}rcoles", "jueves", "viernes", "s\u{e1}bado", "domingo"],
    days_short: ["lun", "mar", "mi\u{e9}", "jue", "vie", "s\u{e1}b", "dom"],
    am: "a.\u{a0}m.",
    pm: "p.\u{a0}m.",
};
const IT: Names = Names {
    months: ["gennaio", "febbraio", "marzo", "aprile", "maggio", "giugno", "luglio", "agosto", "settembre", "ottobre", "novembre", "dicembre"],
    months_short: ["gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic"],
    days: ["luned\u{ec}", "marted\u{ec}", "mercoled\u{ec}", "gioved\u{ec}", "venerd\u{ec}", "sabato", "domenica"],
    days_short: ["lun", "mar", "mer", "gio", "ven", "sab", "dom"],
    am: "AM",
    pm: "PM",
};
const NL: Names = Names {
    months: ["januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november", "december"],
    months_short: ["jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec"],
    days: ["maandag", "dinsdag", "woensdag", "donderdag", "vrijdag", "zaterdag", "zondag"],
    days_short: ["ma", "di", "wo", "do", "vr", "za", "zo"],
    am: "a.m.",
    pm: "p.m.",
};
const PT: Names = Names {
    months: ["janeiro", "fevereiro", "mar\u{e7}o", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"],
    months_short: ["jan.", "fev.", "mar.", "abr.", "mai.", "jun.", "jul.", "ago.", "set.", "out.", "nov.", "dez."],
    days: ["segunda-feira", "ter\u{e7}a-feira", "quarta-feira", "quinta-feira", "sexta-feira", "s\u{e1}bado", "domingo"],
    days_short: ["seg.", "ter.", "qua.", "qui.", "sex.", "s\u{e1}b.", "dom."],
    am: "AM",
    pm: "PM",
};
const SV: Names = Names {
    months: ["januari", "februari", "mars", "april", "maj", "juni", "juli", "augusti", "september", "oktober", "november", "december"],
    months_short: ["jan.", "feb.", "mars", "apr.", "maj", "juni", "juli", "aug.", "sep.", "okt.", "nov.", "dec."],
    days: ["m\u{e5}ndag", "tisdag", "onsdag", "torsdag", "fredag", "l\u{f6}rdag", "s\u{f6}ndag"],
    days_short: ["m\u{e5}n", "tis", "ons", "tors", "fre", "l\u{f6}r", "s\u{f6}n"],
    am: "fm",
    pm: "em",
};
const PL: Names = Names {
    months: ["stycze\u{144}", "luty", "marzec", "kwiecie\u{144}", "maj", "czerwiec", "lipiec", "sierpie\u{144}", "wrzesie\u{144}", "pa\u{17a}dziernik", "listopad", "grudzie\u{144}"],
    months_short: ["sty", "lut", "mar", "kwi", "maj", "cze", "lip", "sie", "wrz", "pa\u{17a}", "lis", "gru"],
    days: ["poniedzia\u{142}ek", "wtorek", "\u{15b}roda", "czwartek", "pi\u{105}tek", "sobota", "niedziela"],
    days_short: ["pon.", "wt.", "\u{15b}r.", "czw.", "pt.", "sob.", "niedz."],
    am: "AM",
    pm: "PM",
};

/// The names for a language tag (`de-DE`, `de_DE`, `fr`); None for one that is not in the table.
fn names_for(locale: &str) -> Option<&'static Names> {
    let lang = locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    Some(match lang.as_str() {
        "en" => &EN,
        "de" => &DE,
        "fr" => &FR,
        "es" => &ES,
        "it" => &IT,
        "nl" => &NL,
        "pt" => &PT,
        "sv" => &SV,
        "pl" => &PL,
        _ => return None,
    })
}

/// Is the language tag one this port has names for (`placeholders.locale`)?
pub fn locale_known(locale: &str) -> bool {
    names_for(locale).is_some()
}

pub fn format(pattern: &str, t: &DateTime<FixedOffset>) -> String {
    format_locale(pattern, t, "en-US")
}

pub fn format_locale(pattern: &str, t: &DateTime<FixedOffset>, locale: &str) -> String {
    let names = names_for(locale).unwrap_or(&EN);
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
                    3 => names.months_short[m - 1].to_owned(),
                    _ => names.months[m - 1].to_owned(),
                });
            }
            'd' => out.push_str(&num(t.day() as i64)),
            'D' => out.push_str(&num(t.ordinal() as i64)),
            'E' => {
                let day = t.weekday().num_days_from_monday() as usize;
                out.push_str(if n >= 4 { names.days[day] } else { names.days_short[day] });
            }
            'u' => out.push_str(&num(t.weekday().number_from_monday() as i64)),
            'a' => out.push_str(if t.hour() < 12 { names.am } else { names.pm }),
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

    #[test]
    fn locale_names() {
        let t = FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 3, 7, 15, 4, 9)
            .unwrap();
        assert_eq!(format_locale("EEEE, d. MMMM yyyy", &t, "de-DE"), "Samstag, 7. M\u{e4}rz 2026");
        assert_eq!(format_locale("EEE d MMM", &t, "fr_FR"), "sam. 7 mars");
        assert_eq!(format_locale("EEEE", &t, "es"), "s\u{e1}bado");
        assert_eq!(format_locale("h:mm a", &t, "es-ES"), "3:04 p.\u{a0}m.");
        // An unknown tag shows English, as TAB's fallback does.
        assert_eq!(format_locale("EEEE", &t, "xx-YY"), "Saturday");
        assert!(locale_known("pt-BR") && !locale_known("xx"));
    }
}
