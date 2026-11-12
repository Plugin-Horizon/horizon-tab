//! A small YAML reader and writer for the files TAB keeps (`config.yml`, `groups.yml`, `users.yml`,
//! `animations.yml`, `messages.yml`, `playerdata.yml`), so the same files work with both the Java
//! plugin and this mod. Horizon's `Cargo.lock` has no YAML crate.
//!
//! Supported: block mappings and sequences (also a sequence at its key's indent), plain, single- and
//! double-quoted scalars, `#` comments, empty `[]` / `{}` and simple flow sequences, `|` / `>` block
//! scalars, `- key: value` mapping items. Not supported: anchors, tags, multi-document streams,
//! nested flow collections. Mapping keys are looked up ignoring case, as TAB does.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Yaml {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Yaml>),
    /// Insertion order is kept, as SnakeYAML's `LinkedHashMap`.
    Map(Vec<(String, Yaml)>),
}

impl Default for Yaml {
    fn default() -> Self {
        Yaml::Null
    }
}

static NULL: Yaml = Yaml::Null;

impl Yaml {
    pub fn empty_map() -> Yaml {
        Yaml::Map(Vec::new())
    }

    /// The value under `key`, ignoring case; `Null` when absent or when this is no mapping.
    pub fn get(&self, key: &str) -> &Yaml {
        match self {
            Yaml::Map(entries) => entries
                .iter()
                .find(|(k, _)| k == key)
                .or_else(|| entries.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)))
                .map(|(_, v)| v)
                .unwrap_or(&NULL),
            _ => &NULL,
        }
    }

    /// A path of keys.
    pub fn at(&self, path: &[&str]) -> &Yaml {
        path.iter().fold(self, |node, key| node.get(key))
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Yaml> {
        match self {
            Yaml::Map(entries) => {
                let pos = entries
                    .iter()
                    .position(|(k, _)| k == key)
                    .or_else(|| entries.iter().position(|(k, _)| k.eq_ignore_ascii_case(key)))?;
                Some(&mut entries[pos].1)
            }
            _ => None,
        }
    }

    /// Sets `key` (keeping its position and spelling when it exists, ignoring case), turning a
    /// non-mapping into an empty mapping first.
    pub fn set(&mut self, key: &str, value: Yaml) {
        if !matches!(self, Yaml::Map(_)) {
            *self = Yaml::empty_map();
        }
        if let Some(slot) = self.get_mut(key) {
            *slot = value;
            return;
        }
        if let Yaml::Map(entries) = self {
            entries.push((key.to_owned(), value));
        }
    }

    /// Removes `key` (ignoring case); whether it was there.
    pub fn remove(&mut self, key: &str) -> bool {
        if let Yaml::Map(entries) = self {
            let before = entries.len();
            entries.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
            return entries.len() != before;
        }
        false
    }

    /// The child mapping under `key`, made when missing.
    pub fn section_mut(&mut self, key: &str) -> &mut Yaml {
        if !matches!(self.get(key), Yaml::Map(_)) {
            self.set(key, Yaml::empty_map());
        }
        self.get_mut(key).expect("set above")
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Yaml::Null)
    }

    /// A scalar as text (numbers and booleans as Java's `toString` writes them); None for null,
    /// lists and mappings.
    pub fn as_str(&self) -> Option<String> {
        match self {
            Yaml::Str(s) => Some(s.clone()),
            Yaml::Int(i) => Some(i.to_string()),
            Yaml::Float(f) => Some(java_double(*f)),
            Yaml::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }

    pub fn str_or(&self, default: &str) -> String {
        self.as_str().unwrap_or_else(|| default.to_owned())
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Yaml::Bool(b) => Some(*b),
            Yaml::Str(s) if s.eq_ignore_ascii_case("true") => Some(true),
            Yaml::Str(s) if s.eq_ignore_ascii_case("false") => Some(false),
            _ => None,
        }
    }

    pub fn bool_or(&self, default: bool) -> bool {
        self.as_bool().unwrap_or(default)
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Yaml::Int(i) => Some(*i),
            Yaml::Float(f) => Some(*f as i64),
            Yaml::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Yaml::Int(i) => Some(*i as f64),
            Yaml::Float(f) => Some(*f),
            Yaml::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn i64_or(&self, default: i64) -> i64 {
        self.as_i64().unwrap_or(default)
    }

    /// A list of scalars as text; a single scalar counts as a one-element list (TAB accepts both
    /// for header/footer lines).
    pub fn str_list(&self) -> Vec<String> {
        match self {
            Yaml::List(items) => items.iter().filter_map(Yaml::as_str).collect(),
            Yaml::Null | Yaml::Map(_) => Vec::new(),
            scalar => scalar.as_str().into_iter().collect(),
        }
    }

    /// Keys of a mapping, in order.
    pub fn keys(&self) -> Vec<String> {
        match self {
            Yaml::Map(entries) => entries.iter().map(|(k, _)| k.clone()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn entries(&self) -> &[(String, Yaml)] {
        match self {
            Yaml::Map(entries) => entries,
            _ => &[],
        }
    }
}

/// Java's `Double.toString` for the values a config holds (`100.0`, `0.5`).
fn java_double(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e7 {
        format!("{f:.1}")
    } else {
        f.to_string()
    }
}

#[derive(Clone, Debug)]
struct Line {
    indent: usize,
    text: String,
    number: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct YamlError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for YamlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for YamlError {}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, YamlError> {
    Err(YamlError {
        line,
        message: message.into(),
    })
}

/// Parses one YAML document. An empty file is an empty mapping (SnakeYAML gives null, which TAB
/// treats as empty).
pub fn parse(source: &str) -> Result<Yaml, YamlError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let raw: Vec<&str> = source.lines().collect();
    let mut lines = Vec::new();
    for (n, l) in raw.iter().enumerate() {
        let l = l.trim_end_matches('\r');
        let trimmed = l.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" || trimmed == "..." {
            continue;
        }
        if l.starts_with('\t') {
            return err(n + 1, "tabs are not allowed for indentation");
        }
        lines.push(Line {
            indent: l.len() - trimmed.len(),
            text: trimmed.to_owned(),
            number: n + 1,
        });
    }
    if lines.is_empty() {
        return Ok(Yaml::empty_map());
    }
    let mut p = Parser {
        lines,
        raw: raw.iter().map(|s| s.trim_end_matches('\r').to_owned()).collect(),
        i: 0,
    };
    let indent = p.lines[0].indent;
    let node = p.node(indent)?;
    if p.i < p.lines.len() {
        let l = &p.lines[p.i];
        return err(l.number, format!("unexpected content `{}`", l.text));
    }
    Ok(node)
}

struct Parser {
    lines: Vec<Line>,
    raw: Vec<String>,
    i: usize,
}

fn is_seq_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

impl Parser {
    fn node(&mut self, indent: usize) -> Result<Yaml, YamlError> {
        let line = self.lines[self.i].clone();
        if is_seq_item(&line.text) {
            self.seq(line.indent)
        } else if split_key(&line.text).is_some() {
            self.map(line.indent)
        } else {
            self.i += 1;
            let _ = indent;
            scalar(&line.text, line.number)
        }
    }

    fn map(&mut self, indent: usize) -> Result<Yaml, YamlError> {
        let mut entries: Vec<(String, Yaml)> = Vec::new();
        while self.i < self.lines.len() {
            let line = self.lines[self.i].clone();
            if line.indent < indent {
                break;
            }
            if line.indent > indent {
                return err(line.number, "bad indentation");
            }
            if is_seq_item(&line.text) {
                break;
            }
            let Some((key, rest)) = split_key(&line.text) else {
                return err(line.number, format!("expected `key: value`, got `{}`", line.text));
            };
            let key = key_text(&key, line.number)?;
            self.i += 1;
            let rest = rest.trim();
            let value = if rest.is_empty() || rest.starts_with('#') {
                let next = self.lines.get(self.i).map(|n| (n.indent, is_seq_item(&n.text)));
                match next {
                    Some((ind, _)) if ind > indent => self.node(ind)?,
                    Some((ind, true)) if ind == indent => self.seq(indent)?,
                    _ => Yaml::Null,
                }
            } else if rest.starts_with('|') || rest.starts_with('>') {
                self.block_scalar(indent, rest.starts_with('>'), rest.contains('-'), line.number)
            } else {
                scalar(rest, line.number)?
            };
            if let Some(slot) = entries.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = value;
            } else {
                entries.push((key, value));
            }
        }
        Ok(Yaml::Map(entries))
    }

    fn seq(&mut self, indent: usize) -> Result<Yaml, YamlError> {
        let mut items = Vec::new();
        while self.i < self.lines.len() {
            let line = self.lines[self.i].clone();
            if line.indent != indent || !is_seq_item(&line.text) {
                if line.indent > indent {
                    return err(line.number, "bad indentation in a list");
                }
                break;
            }
            let rest = line.text[1..].trim_start();
            let offset = line.text.len() - rest.len();
            if rest.is_empty() || rest.starts_with('#') {
                self.i += 1;
                match self.lines.get(self.i).map(|n| n.indent) {
                    Some(ind) if ind > indent => items.push(self.node(ind)?),
                    _ => items.push(Yaml::Null),
                }
            } else if is_seq_item(rest) || (!starts_quoted(rest) && split_key(rest).is_some()) {
                // `- key: value` or `- - x`: the item continues as a node at the column of `rest`.
                self.lines[self.i] = Line {
                    indent: indent + offset,
                    text: rest.to_owned(),
                    number: line.number,
                };
                items.push(self.node(indent + offset)?);
            } else {
                self.i += 1;
                items.push(scalar(rest, line.number)?);
            }
        }
        Ok(Yaml::List(items))
    }

    /// `|` / `>` scalars: the raw lines indented deeper than the key.
    fn block_scalar(&mut self, indent: usize, folded: bool, strip: bool, start: usize) -> Yaml {
        let mut body: Vec<String> = Vec::new();
        let mut n = start; // 1-based number of the key line; raw[n] is the next line
        let mut block_indent = None;
        while n < self.raw.len() {
            let l = &self.raw[n];
            let trimmed = l.trim_start();
            let ind = l.len() - trimmed.len();
            if !trimmed.is_empty() && ind <= indent {
                break;
            }
            if trimmed.is_empty() {
                body.push(String::new());
            } else {
                let bi = *block_indent.get_or_insert(ind);
                body.push(l.get(bi.min(ind)..).unwrap_or("").to_owned());
            }
            n += 1;
        }
        while self.i < self.lines.len() && self.lines[self.i].number <= n {
            self.i += 1;
        }
        while body.last().is_some_and(String::is_empty) {
            body.pop();
        }
        let mut text = if folded { body.join(" ") } else { body.join("\n") };
        if !strip {
            text.push('\n');
        }
        Yaml::Str(text)
    }
}

fn starts_quoted(s: &str) -> bool {
    s.starts_with('"') || s.starts_with('\'')
}

/// Splits `key: rest` (or `key:` at the end). Quoted keys may hold `: `.
fn split_key(text: &str) -> Option<(String, String)> {
    if text.starts_with('[') || text.starts_with('{') {
        return None;
    }
    if starts_quoted(text) {
        let q = text.chars().next()?;
        let end = closing_quote(text, q)?;
        let after = &text[end + 1..];
        let after_trim = after.trim_start();
        let rest = after_trim.strip_prefix(':')?;
        if !rest.is_empty() && !rest.starts_with(' ') {
            return None;
        }
        return Some((text[..=end].to_owned(), rest.to_owned()));
    }
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && i > 0 && bytes[i - 1] == b' ' {
            return None;
        }
        if b == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' ') {
            return Some((text[..i].trim_end().to_owned(), text[i + 1..].to_owned()));
        }
    }
    None
}

fn key_text(key: &str, line: usize) -> Result<String, YamlError> {
    if starts_quoted(key) {
        match scalar(key, line)? {
            Yaml::Str(s) => Ok(s),
            other => Ok(other.as_str().unwrap_or_default()),
        }
    } else {
        Ok(key.to_owned())
    }
}

/// Byte index of the quote closing the quoted scalar starting at 0.
fn closing_quote(text: &str, q: char) -> Option<usize> {
    let mut chars = text.char_indices().skip(1).peekable();
    while let Some((i, c)) = chars.next() {
        if q == '"' && c == '\\' {
            chars.next();
            continue;
        }
        if c == q {
            if q == '\'' && chars.peek().map(|(_, c)| *c) == Some('\'') {
                chars.next();
                continue;
            }
            return Some(i);
        }
    }
    None
}

fn strip_comment(rest: &str) -> &str {
    let t = rest.trim_start();
    if t.is_empty() || t.starts_with('#') {
        ""
    } else {
        t
    }
}

fn scalar(text: &str, line: usize) -> Result<Yaml, YamlError> {
    let text = text.trim();
    if let Some(q) = text.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let Some(end) = closing_quote(text, q) else {
            return err(line, "unterminated quoted string");
        };
        if !strip_comment(&text[end + 1..]).is_empty() {
            return err(line, format!("unexpected text after a quoted string: `{text}`"));
        }
        let inner = &text[1..end];
        return Ok(Yaml::Str(if q == '\'' {
            inner.replace("''", "'")
        } else {
            unescape_double(inner, line)?
        }));
    }
    // Plain: a comment starts at ` #`.
    let plain = match text.find(" #") {
        Some(i) => text[..i].trim_end(),
        None => text,
    };
    if plain.starts_with('[') {
        return flow_list(plain, line);
    }
    if plain == "{}" {
        return Ok(Yaml::empty_map());
    }
    Ok(typed_plain(plain))
}

fn typed_plain(plain: &str) -> Yaml {
    match plain {
        "" | "~" | "null" | "Null" | "NULL" => return Yaml::Null,
        "true" | "True" | "TRUE" => return Yaml::Bool(true),
        "false" | "False" | "FALSE" => return Yaml::Bool(false),
        _ => {}
    }
    let digits = plain.strip_prefix(['-', '+']).unwrap_or(plain);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(i) = plain.parse::<i64>() {
            return Yaml::Int(i);
        }
    }
    if digits.contains('.')
        && digits
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b'e' || b == b'E' || b == b'-' || b == b'+')
        && digits.bytes().next().is_some_and(|b| b.is_ascii_digit() || b == b'.')
    {
        if let Ok(f) = plain.parse::<f64>() {
            return Yaml::Float(f);
        }
    }
    Yaml::Str(plain.to_owned())
}

fn flow_list(text: &str, line: usize) -> Result<Yaml, YamlError> {
    let Some(inner) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) else {
        return err(line, "unterminated flow list");
    };
    let mut items = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in inner.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                current.push(c);
            }
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                current.push(c);
            }
            None if c == ',' => {
                items.push(scalar(&current, line)?);
                current.clear();
            }
            None => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        items.push(scalar(&current, line)?);
    }
    Ok(Yaml::List(items))
}

fn unescape_double(s: &str, line: usize) -> Result<String, YamlError> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some(' ') => out.push(' '),
            Some(k @ ('x' | 'u' | 'U')) => {
                let n = match k {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let hex: String = chars.by_ref().take(n).collect();
                let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) else {
                    return err(line, format!("bad escape \\{k}{hex}"));
                };
                out.push(ch);
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Ok(out)
}

/// Writes `value` as block YAML with two-space indentation. Comments of the file read are not
/// kept (the Java plugin's writer does not keep them either).
pub fn write(value: &Yaml) -> String {
    let mut out = String::new();
    match value {
        Yaml::Map(entries) if entries.is_empty() => out.push_str("{}\n"),
        Yaml::Map(entries) => write_map(&mut out, entries, 0),
        Yaml::List(items) => write_list(&mut out, items, 0),
        scalar => {
            out.push_str(&scalar_text(scalar));
            out.push('\n');
        }
    }
    out
}

fn write_map(out: &mut String, entries: &[(String, Yaml)], indent: usize) {
    for (k, v) in entries {
        let pad = " ".repeat(indent);
        let key = key_out(k);
        match v {
            Yaml::Map(m) if !m.is_empty() => {
                let _ = writeln!(out, "{pad}{key}:");
                write_map(out, m, indent + 2);
            }
            Yaml::List(l) if !l.is_empty() => {
                let _ = writeln!(out, "{pad}{key}:");
                write_list(out, l, indent + 2);
            }
            other => {
                let _ = writeln!(out, "{pad}{key}: {}", scalar_text(other));
            }
        }
    }
}

fn write_list(out: &mut String, items: &[Yaml], indent: usize) {
    let pad = " ".repeat(indent);
    for item in items {
        match item {
            Yaml::Map(m) if !m.is_empty() => {
                let mut inner = String::new();
                write_map(&mut inner, m, indent + 2);
                // The first entry goes on the dash's line.
                let inner = inner.trim_start_matches(' ');
                let _ = write!(out, "{pad}- {inner}");
            }
            Yaml::List(l) if !l.is_empty() => {
                let _ = writeln!(out, "{pad}-");
                write_list(out, l, indent + 2);
            }
            other => {
                let _ = writeln!(out, "{pad}- {}", scalar_text(other));
            }
        }
    }
}

fn key_out(k: &str) -> String {
    let plain_ok = !k.is_empty()
        && k.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.+".contains(&b))
        && !matches!(typed_plain(k), Yaml::Bool(_) | Yaml::Null)
        && !snake_non_string(k);
    if plain_ok {
        k.to_owned()
    } else {
        quote_single(k)
    }
}

/// True when SnakeYAML's YAML 1.1 resolver (as TAB loads files) would not read the plain
/// text `k` as a string: booleans and nulls in its three casings, and anything that starts
/// like a number (`0123` octal, `1_000`, `0x1f`, `1:30`, `.5`, `-1`). Such keys must be quoted,
/// else TAB looks the user/group up under the number's text and never finds it.
fn snake_non_string(k: &str) -> bool {
    // Quoting a word in any casing is harmless; SnakeYAML only resolves three of them.
    const WORDS: [&str; 8] = ["yes", "no", "true", "false", "on", "off", "null", "~"];
    let lower = k.to_ascii_lowercase();
    if WORDS.contains(&lower.as_str()) {
        return true;
    }
    let body = k.strip_prefix(['+', '-']).unwrap_or(k);
    let digits = |t: &str, ok: fn(char) -> bool| !t.is_empty() && t.chars().all(|c| ok(c) || c == '_');
    let dec = |c: char| c.is_ascii_digit();
    if let Some(hex) = body.strip_prefix("0x") {
        return digits(hex, |c| c.is_ascii_hexdigit());
    }
    if let Some(bin) = body.strip_prefix("0b") {
        return digits(bin, |c| c == '0' || c == '1');
    }
    if digits(body, dec) {
        return true; // decimal, octal (leading 0) or with `_`
    }
    // Sexagesimal `1:30`, `190:20:30`.
    let mut parts = body.split(':');
    if let (Some(head), true) = (parts.next(), body.contains(':')) {
        if digits(head, dec) && parts.all(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(dec)) {
            return true;
        }
    }
    // Floats: `1.5`, `.5`, `1.`, `1.0e+5`, `.inf`, `.nan`.
    let (mantissa, exp) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let exp_ok = exp.is_none_or(|e| {
        e.strip_prefix(['+', '-']).is_some_and(|d| !d.is_empty() && d.chars().all(dec))
    });
    let float = mantissa.contains('.')
        && mantissa.chars().all(|c| dec(c) || c == '_' || c == '.')
        && mantissa.matches('.').count() == 1
        && mantissa.chars().any(dec)
        && exp_ok;
    float || matches!(lower.as_str(), ".inf" | "-.inf" | "+.inf" | ".nan")
}

fn quote_single(s: &str) -> String {
    if s.contains('\n') || s.chars().any(|c| c.is_control()) {
        return serde_json::to_string(s).unwrap_or_else(|_| format!("'{}'", s.replace('\'', "''")));
    }
    format!("'{}'", s.replace('\'', "''"))
}

fn scalar_text(v: &Yaml) -> String {
    match v {
        Yaml::Null => "null".into(),
        Yaml::Bool(b) => b.to_string(),
        Yaml::Int(i) => i.to_string(),
        Yaml::Float(f) => java_double(*f),
        Yaml::Str(s) => quote_single(s),
        Yaml::List(_) => "[]".into(),
        Yaml::Map(_) => "{}".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_style_file() {
        let src = r#"
# comment
header-footer:
  enabled: false
  designs:
    default:
      display-condition: "%world%=test" # inline comment
      header:
        - "<#FFFFFF>&m   </#FFFF00>"
        - '&3&lServer name'
        - ""
scoreboard-teams:
  sorting-types:
    - "GROUPS:core,admin"
  case-sensitive-sorting: true
placeholder-refresh-intervals:
  default-refresh-interval: 500
  "%server_uptime%": 1000
ServerName:
  change-interval: 300
  texts:
  - '&c&lServerName'
  - '&b&lServerName'
redis:
rabbitmq:
  url: 'amqp://guest:guest@localhost:5672/%2F'
"#;
        let y = parse(src).unwrap();
        assert_eq!(y.at(&["header-footer", "enabled"]), &Yaml::Bool(false));
        assert_eq!(
            y.at(&["header-footer", "designs", "default", "display-condition"])
                .as_str()
                .unwrap(),
            "%world%=test"
        );
        assert_eq!(
            y.at(&["header-footer", "designs", "default", "header"]).str_list(),
            vec!["<#FFFFFF>&m   </#FFFF00>", "&3&lServer name", ""]
        );
        assert_eq!(
            y.at(&["placeholder-refresh-intervals", "%server_uptime%"]).as_i64(),
            Some(1000)
        );
        assert_eq!(y.at(&["ServerName", "texts"]).str_list().len(), 2);
        assert!(y.get("redis").is_null());
        assert_eq!(y.get("HEADER-FOOTER").get("Enabled"), &Yaml::Bool(false));
    }

    #[test]
    fn groups_file_with_odd_comment_indent() {
        let src = "#Owner:\n#  tabprefix: \"x\"\nPlayer:\n  tabprefix: \"&0&l[&7&lPlayer&0&l] &3\"\n\n \
# default settings\n_DEFAULT_:\n  tabprefix: \"%luckperms-prefix%\"\nper-world:\n  world1:\n    Owner:\n      tagprefix: \"&2&lOwner &a\"\n";
        let y = parse(src).unwrap();
        assert_eq!(
            y.at(&["player", "tabprefix"]).as_str().unwrap(),
            "&0&l[&7&lPlayer&0&l] &3"
        );
        assert_eq!(
            y.at(&["per-world", "world1", "Owner", "tagprefix"]).as_str().unwrap(),
            "&2&lOwner &a"
        );
    }

    #[test]
    fn round_trip() {
        let mut y = Yaml::empty_map();
        y.section_mut("Steve").set("tabprefix", Yaml::Str("&c&l[OP] &f".into()));
        y.section_mut("Steve").set("tabsuffix", Yaml::Str(" it's".into()));
        y.set("scoreboard-off", Yaml::List(vec![Yaml::Str("0000-1".into())]));
        y.set("empty", Yaml::List(vec![]));
        let text = write(&y);
        assert_eq!(parse(&text).unwrap(), y);
    }

    #[test]
    fn keys_snakeyaml_would_not_read_as_strings_are_quoted() {
        for k in ["0123", "1_000", "12345", "0x1f", "0b101", "1:30", "1.5", ".5", "1.0e+5", "Yes", "Off", "NULL", "~", "-1"] {
            assert!(key_out(k).starts_with('\''), "{k} must be quoted, got {}", key_out(k));
        }
        for k in ["Steve", "Owner", "_DEFAULT_", "01234567-89ab-cdef-0123-456789abcdef", "1abc", "a1", "yesman"] {
            assert_eq!(key_out(k), k, "{k} stays plain");
        }
        let y = Yaml::Map(vec![("0123".into(), Yaml::Map(vec![("tabprefix".into(), Yaml::Str("x".into()))]))]);
        assert!(write(&y).starts_with("'0123':"));
        assert_eq!(parse(&write(&y)).unwrap(), y);
    }

    #[test]
    fn list_of_maps_and_flow() {
        let y = parse("a:\n  - name: x\n    v: 1\n  - name: y\nb: [1, 'two', three]\nc: []\n").unwrap();
        assert_eq!(
            y.get("a"),
            &Yaml::List(vec![
                Yaml::Map(vec![("name".into(), Yaml::Str("x".into())), ("v".into(), Yaml::Int(1))]),
                Yaml::Map(vec![("name".into(), Yaml::Str("y".into()))]),
            ])
        );
        assert_eq!(y.get("b").str_list(), vec!["1", "two", "three"]);
        assert_eq!(y.get("c"), &Yaml::List(vec![]));
    }

    #[test]
    fn escapes_and_numbers() {
        let y = parse("a: \"x\\ny \\\"q\\\" \\u00e4\"\nb: 'it''s'\nc: 1.5\nd: -3\ne: 1.20.3\n").unwrap();
        assert_eq!(y.get("a").as_str().unwrap(), "x\ny \"q\" \u{e4}");
        assert_eq!(y.get("b").as_str().unwrap(), "it's");
        assert_eq!(y.get("c"), &Yaml::Float(1.5));
        assert_eq!(y.get("d"), &Yaml::Int(-3));
        assert_eq!(y.get("e").as_str().unwrap(), "1.20.3");
    }

    #[test]
    fn block_scalar() {
        let y = parse("a: |\n  one\n  two\nb: 1\n").unwrap();
        assert_eq!(y.get("a").as_str().unwrap(), "one\ntwo\n");
        assert_eq!(y.get("b").as_i64(), Some(1));
    }
}
