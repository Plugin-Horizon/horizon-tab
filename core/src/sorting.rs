//! Sorting players in the Tab list through their team names, as TAB does (`Sorting`,
//! `SortingType` and its types, Apache-2.0, see NOTICE): every sorting type adds characters, the
//! client orders teams by name. Types: `GROUPS`, `PERMISSIONS`, `PLACEHOLDER`,
//! `PLACEHOLDER_A_TO_Z`, `PLACEHOLDER_Z_TO_A`, `PLACEHOLDER_LOW_TO_HIGH`,
//! `PLACEHOLDER_HIGH_TO_LOW`.
//!
//! One difference: TAB packs numbers into UTF-16 code units up to U+FFFE, which may be lone
//! surrogates; Rust strings cannot hold those, so numbers use base U+D7FF (all below the
//! surrogates, so the client's UTF-16 order is the code point order) with the same skip of `\`.

use crate::text;

pub const TEAM_NAME_LENGTH: usize = 16;
const DEFAULT_NUMBER: f64 = (i32::MAX / 2) as f64;

#[derive(Clone, Debug, PartialEq)]
pub enum SortingType {
    Groups(Vec<Vec<String>>),
    Permissions(Vec<Vec<String>>),
    Placeholder {
        placeholder: String,
        values: Vec<Vec<String>>,
    },
    AToZ(String),
    ZToA(String),
    LowToHigh(String),
    HighToLow(String),
}

/// `a|b,c` -> positions: [["a","b"],["c"]] (lower case, colours translated), as
/// `convertSortingElements`.
fn elements(list: &str) -> Vec<Vec<String>> {
    list.split(',')
        .map(|e| e.split('|').map(|x| text::colorize(&x.trim().to_lowercase())).collect())
        .collect()
}

fn valid_placeholder(p: &str) -> bool {
    p.len() >= 2 && p.starts_with('%') && p.ends_with('%')
}

pub fn compile(lines: &[String]) -> (Vec<SortingType>, Vec<String>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    for line in lines {
        let (kind, rest) = match line.split_once(':') {
            Some((k, r)) => (k.to_ascii_uppercase(), r.to_owned()),
            None => (line.to_ascii_uppercase(), String::new()),
        };
        let t = match kind.as_str() {
            "GROUPS" => SortingType::Groups(elements(&rest)),
            "PERMISSIONS" => SortingType::Permissions(elements(&rest)),
            "PLACEHOLDER" => {
                // %placeholder%:value1,value2
                let Some(end) = rest[1.min(rest.len())..].find('%').map(|i| i + 2) else {
                    warnings.push(format!("Invalid sorting line \"{line}\""));
                    continue;
                };
                let placeholder = rest[..end].to_owned();
                let values = rest.get(end + 1..).unwrap_or("");
                if !valid_placeholder(&placeholder) || !rest[end..].starts_with(':') {
                    warnings.push(format!(
                        "Invalid sorting line \"{line}\": expected \"%placeholder%:value1,value2\""
                    ));
                    continue;
                }
                SortingType::Placeholder {
                    placeholder,
                    values: elements(values),
                }
            }
            "PLACEHOLDER_A_TO_Z" | "PLACEHOLDER_Z_TO_A" | "PLACEHOLDER_LOW_TO_HIGH" | "PLACEHOLDER_HIGH_TO_LOW" => {
                if !valid_placeholder(&rest) {
                    warnings.push(format!("Invalid sorting placeholder \"{rest}\" in {kind}"));
                    continue;
                }
                match kind.as_str() {
                    "PLACEHOLDER_A_TO_Z" => SortingType::AToZ(rest),
                    "PLACEHOLDER_Z_TO_A" => SortingType::ZToA(rest),
                    "PLACEHOLDER_LOW_TO_HIGH" => SortingType::LowToHigh(rest),
                    _ => SortingType::HighToLow(rest),
                }
            }
            other => {
                warnings.push(format!("Unknown sorting type \"{other}\""));
                continue;
            }
        };
        out.push(t);
    }
    (out, warnings)
}

fn position_char(pos: usize) -> char {
    char::from_u32(pos as u32 + 47).unwrap_or('~')
}

fn position_in(list: &[Vec<String>], value: &str) -> usize {
    list.iter()
        .position(|alts| alts.iter().any(|a| a == value))
        .map(|i| i + 1)
        .unwrap_or(list.len() + 1)
}

/// Base for packed numbers: below the surrogates.
const BASE: u32 = 0xD7FF;

fn digit(v: u32) -> char {
    let v = if v >= '\\' as u32 { v + 1 } else { v };
    char::from_u32(v).unwrap_or('\u{D7FF}')
}

/// TAB's `compressNumber`, with a base that stays below the surrogates.
pub fn compress_number(number: f64) -> String {
    let number = number.max(0.0);
    let mut whole = number as u64;
    let decimal = ((number - whole as f64) * (BASE - 1) as f64) as u32;
    let mut digits = Vec::new();
    while whole > 0 {
        digits.push(digit((whole % BASE as u64) as u32));
        whole /= BASE as u64;
    }
    while digits.len() < 2 {
        digits.push('\0');
    }
    digits.reverse();
    let mut s: String = digits.into_iter().collect();
    s.push(digit(decimal));
    s
}

fn parse_number(output: &str) -> f64 {
    output.trim().replace(',', ".").parse().unwrap_or(0.0)
}

impl SortingType {
    /// The characters this type adds for a player. `parse` resolves a placeholder for them;
    /// `perm` checks a permission; `group` is their group.
    pub fn chars(
        &self,
        group: &str,
        case_sensitive: bool,
        parse: &mut dyn FnMut(&str) -> String,
        perm: &mut dyn FnMut(&str) -> bool,
    ) -> String {
        match self {
            SortingType::Groups(list) => position_char(position_in(list, &group.to_lowercase())).to_string(),
            SortingType::Permissions(list) => {
                let pos = list
                    .iter()
                    .position(|alts| alts.iter().any(|n| perm(n)))
                    .map(|i| i + 1)
                    .unwrap_or(list.len() + 1);
                position_char(pos).to_string()
            }
            SortingType::Placeholder { placeholder, values } => {
                let out = text::colorize(&parse(placeholder)).trim().to_lowercase();
                position_char(position_in(values, &out)).to_string()
            }
            SortingType::AToZ(p) => {
                let out = parse(p);
                if case_sensitive {
                    out
                } else {
                    out.to_lowercase()
                }
            }
            SortingType::ZToA(p) => {
                let out: String = parse(p)
                    .chars()
                    .map(|c| match c {
                        'A'..='Z' => char::from_u32(155 - c as u32).unwrap_or(c),
                        'a'..='z' => char::from_u32(219 - c as u32).unwrap_or(c),
                        other => other,
                    })
                    .collect();
                if case_sensitive {
                    out
                } else {
                    out.to_lowercase()
                }
            }
            SortingType::LowToHigh(p) => compress_number(DEFAULT_NUMBER + parse_number(&parse(p))),
            SortingType::HighToLow(p) => compress_number(DEFAULT_NUMBER - parse_number(&parse(p))),
        }
    }
}

/// The short team name from the sorting characters, made unique among `taken` with `A`, `B`...
/// as TAB's `checkTeamName`.
pub fn team_name(sort_chars: &str, taken: &dyn Fn(&str) -> bool) -> String {
    // Room for the unique suffix (TAB: setLength(15) when >= 16).
    let base: String = sort_chars.chars().take(TEAM_NAME_LENGTH - 1).collect();
    let mut id = 'A';
    loop {
        let candidate = format!("{base}{id}");
        if !taken(&candidate) {
            return candidate;
        }
        id = char::from_u32(id as u32 + 1).unwrap_or('Z');
        if id as u32 > 0xD7FF {
            return candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_sorting_lines() {
        let (types, warnings) = compile(&[
            "GROUPS:owner,admin,op,vip,guest,default".to_owned(),
            "PLACEHOLDER_A_TO_Z:%player%".to_owned(),
        ]);
        assert!(warnings.is_empty());
        let name = |n: &'static str| move |_: &str| n.to_owned();
        let mut no = |_: &str| false;
        let a = types[0].chars("Admin", true, &mut name("x"), &mut no);
        assert_eq!(a, "1"); // position 2 -> char 49
        let unknown = types[0].chars("builder", true, &mut name("x"), &mut no);
        assert_eq!(unknown, "6"); // 6 entries + 1 = 7 -> char 54
        let steve = types[1].chars("default", true, &mut name("Steve"), &mut no);
        assert_eq!(steve, "Steve");
        // Owner sorts above admin, both above unknown groups.
        assert!(types[0].chars("owner", true, &mut name(""), &mut no) < a && a < unknown);
    }

    #[test]
    fn numbers_keep_order() {
        let n = |x: f64| compress_number(DEFAULT_NUMBER + x);
        assert!(n(1.0) < n(2.0));
        assert!(n(-5.0) < n(0.0));
        assert!(n(0.25) < n(0.5));
        assert!(n(100000.0) < n(1e9));
        for c in n(123456.789).chars() {
            assert!((c as u32) < 0xD800 && c != '\\');
        }
        let h = |x: f64| compress_number(DEFAULT_NUMBER - x);
        assert!(h(10.0) < h(1.0));
    }

    #[test]
    fn unique_names() {
        let taken = |n: &str| n == "1SteveA";
        assert_eq!(team_name("1Steve", &taken), "1SteveB");
        assert_eq!(team_name("1234567890ABCDEFGH", &|_| false), "1234567890ABCDEA");
    }

    #[test]
    fn z_to_a_and_placeholder_values() {
        let (types, w) = compile(&[
            "PLACEHOLDER_Z_TO_A:%player%".into(),
            "PLACEHOLDER:%world%:world,world_nether".into(),
        ]);
        assert!(w.is_empty(), "{w:?}");
        let mut no = |_: &str| false;
        let a = types[0].chars("", true, &mut |_| "a".into(), &mut no);
        let b = types[0].chars("", true, &mut |_| "b".into(), &mut no);
        assert!(b < a);
        let nether = types[1].chars("", true, &mut |_| "world_nether".into(), &mut no);
        assert_eq!(nether, "1");
    }
}
