//! TAB's conditions: `%a%=%b%`, `!=`, `>=`, `<=`, `>`, `<`, `<-` contains, `|-` starts with,
//! `-|` ends with (and their `!` forms), `permission:node`, `!permission:node`; a condition's lines
//! joined with AND or OR; the short form of display conditions (`a;b` is AND, `a|b` is OR).
//! Operator precedence and splitting follow TAB's `ConditionalExpression` (Apache-2.0, NOTICE).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Ge,
    Ne,
    Le,
    Eq,
    NotContains,
    NotStartsWith,
    NotEndsWith,
    EndsWith,
    StartsWith,
    Contains,
    Lt,
    Gt,
}

/// In TAB's detection order: the first symbol found (outside placeholders) decides.
const OPS: [(&str, Op); 12] = [
    (">=", Op::Ge),
    ("!=", Op::Ne),
    ("<=", Op::Le),
    ("=", Op::Eq),
    ("!<-", Op::NotContains),
    ("!|-", Op::NotStartsWith),
    ("!-|", Op::NotEndsWith),
    ("-|", Op::EndsWith),
    ("|-", Op::StartsWith),
    ("<-", Op::Contains),
    ("<", Op::Lt),
    (">", Op::Gt),
];

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Compare { left: String, op: Op, right: String },
    Permission { node: String, negate: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub name: String,
    pub exprs: Vec<Expr>,
    pub and: bool,
    pub yes: String,
    pub no: String,
}

/// Placeholders (`%...%`) in `text`, as TAB's `detectPlaceholders` finds them.
pub fn detect_placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '%' {
            if let Some(j) = bytes[i + 1..].iter().position(|&c| c == '%') {
                let inner: String = bytes[i + 1..i + 1 + j].iter().collect();
                if !inner.is_empty() && !inner.contains(' ') {
                    out.push(format!("%{inner}%"));
                    i += j + 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Splits at `delimiter` outside `%...%` blocks, trimming the parts.
fn split_and_trim(s: &str, delimiter: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut inside = false;
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest.chars().next().unwrap_or(' ');
        if c == '%' {
            inside = !inside;
            current.push('%');
            i += 1;
            continue;
        }
        if !inside && rest.starts_with(delimiter) {
            out.push(current.trim().to_owned());
            current.clear();
            i += delimiter.len();
        } else {
            current.push(c);
            i += c.len_utf8();
        }
    }
    out.push(current.trim().to_owned());
    out
}

pub fn compile(line: &str) -> Option<Expr> {
    let line = line.trim();
    let mut stripped = line.to_owned();
    for p in detect_placeholders(line) {
        stripped = stripped.replace(&p, "");
    }
    if stripped.contains("!permission:") {
        let node = split_and_trim(line, ":").get(1)?.clone();
        return Some(Expr::Permission { node, negate: true });
    }
    for (sym, op) in OPS {
        if stripped.contains(sym) {
            let parts = split_and_trim(line, sym);
            let left = parts.first()?.clone();
            let right = parts.get(1).cloned().unwrap_or_default();
            return Some(Expr::Compare { left, op, right });
        }
    }
    if stripped.contains("permission:") {
        let node = split_and_trim(line, ":").get(1)?.clone();
        return Some(Expr::Permission { node, negate: false });
    }
    None
}

impl Condition {
    pub fn new(name: &str, lines: &[String], and: bool, yes: &str, no: &str) -> Condition {
        Condition {
            name: name.to_owned(),
            exprs: lines.iter().filter_map(|l| compile(l)).collect(),
            and,
            yes: yes.to_owned(),
            no: no.to_owned(),
        }
    }

    /// The short form of a display or disable condition.
    pub fn short(text: &str) -> Condition {
        let (and, parts): (bool, Vec<String>) = if text.contains(';') {
            (true, text.split(';').map(str::to_owned).collect())
        } else {
            (false, split_or(text))
        };
        Condition::new(&format!("AnonymousCondition[{text}]"), &parts, and, "true", "false")
    }

    /// Whether the condition holds; `side` parses a side's text (placeholders resolved, colours
    /// translated) and `perm` checks a permission.
    pub fn is_met(&self, side: &mut dyn FnMut(&str) -> String, perm: &mut dyn FnMut(&str) -> bool) -> bool {
        let mut eval = |e: &Expr| match e {
            Expr::Permission { node, negate } => perm(node) != *negate,
            Expr::Compare { left, op, right } => compare(&side(left), *op, &side(right)),
        };
        if self.and {
            self.exprs.iter().all(&mut eval)
        } else {
            self.exprs.iter().any(&mut eval)
        }
    }
}

/// `|` splits, except in `|-` and `-|`.
fn split_or(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let prev_dash = i > 0 && chars[i - 1] == '-';
        let next_dash = chars.get(i + 1) == Some(&'-');
        if c == '|' && !prev_dash && !next_dash {
            out.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    out.push(current);
    out
}

fn number(s: &str) -> f64 {
    let t = s.trim().replace(',', "");
    // A coloured number (`&a20`): TAB parses after colouring, so codes make it invalid (0).
    t.parse().unwrap_or(0.0)
}

pub fn compare(left: &str, op: Op, right: &str) -> bool {
    match op {
        Op::Ge => number(left) >= number(right),
        Op::Gt => number(left) > number(right),
        Op::Le => number(left) <= number(right),
        Op::Lt => number(left) < number(right),
        Op::NotContains => !left.contains(right),
        Op::NotStartsWith => !left.starts_with(right),
        Op::NotEndsWith => !left.ends_with(right),
        Op::Contains => left.contains(right),
        Op::StartsWith => left.starts_with(right),
        Op::EndsWith => left.ends_with(right),
        Op::Ne => left != right,
        Op::Eq => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: &Condition, vars: &[(&str, &str)], perms: &[&str]) -> bool {
        let mut side = |s: &str| {
            let mut out = s.to_owned();
            for (k, v) in vars {
                out = out.replace(k, v);
            }
            out
        };
        let mut perm = |n: &str| perms.contains(&n);
        c.is_met(&mut side, &mut perm)
    }

    #[test]
    fn short_forms() {
        let c = Condition::short("%player-version-id%>=765;%bedrock%=false");
        assert_eq!(c.exprs.len(), 2);
        assert!(run(&c, &[("%player-version-id%", "775"), ("%bedrock%", "false")], &[]));
        assert!(!run(&c, &[("%player-version-id%", "775"), ("%bedrock%", "true")], &[]));
        let c = Condition::short("%world%=disabledworld");
        assert!(run(&c, &[("%world%", "disabledworld")], &[]));
        assert!(!run(&c, &[("%world%", "world")], &[]));
        let c = Condition::short("permission:tab.staff|%world%=a");
        assert!(run(&c, &[("%world%", "b")], &["tab.staff"]));
        assert!(!run(&c, &[("%world%", "b")], &[]));
        let c = Condition::short("%player%|-Ste");
        assert_eq!(
            c.exprs,
            vec![Expr::Compare {
                left: "%player%".into(),
                op: Op::StartsWith,
                right: "Ste".into()
            }]
        );
        assert!(run(&c, &[("%player%", "Steve")], &[]));
    }

    #[test]
    fn symbols_inside_placeholders_do_not_count() {
        assert_eq!(
            compile("%a=b%!=x"),
            Some(Expr::Compare {
                left: "%a=b%".into(),
                op: Op::Ne,
                right: "x".into()
            })
        );
        assert_eq!(
            compile("!permission:x.y"),
            Some(Expr::Permission {
                node: "x.y".into(),
                negate: true
            })
        );
        assert_eq!(compile("nothing"), None);
    }

    #[test]
    fn detect() {
        assert_eq!(detect_placeholders("a %b% c %d_e% 50 % x"), vec!["%b%", "%d_e%"]);
    }
}
