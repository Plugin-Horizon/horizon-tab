//! Turning TAB's text formats into chat components (vanilla JSON): legacy codes (`&a`, `§l`),
//! the RGB forms TAB accepts (`&#RRGGBB`, `{#RRGGBB}`, `#<RRGGBB>`, `&x&R&R&G&G&B&B`), its gradients
//! (`<#RRGGBB>text</#RRGGBB>`, `{#RRGGBB>}text{#RRGGBB<}`, `<$#RRGGBB>text<$#RRGGBB>`,
//! `<gradient:#RRGGBB>text</gradient:#RRGGBB>`) and, when `components.minimessage-support` is on,
//! the common MiniMessage tags (colours, decorations, `<reset>`, `<gradient:...>`).
//!
//! The format list and the order (gradients first, then the RGB forms) follow TAB's `RGBUtils`
//! (Apache-2.0, see NOTICE); the code is new.

use serde_json::{json, Map, Value};

/// The 16 legacy colours: code, name, RGB.
pub const LEGACY: [(char, &str, u32); 16] = [
    ('0', "black", 0x000000),
    ('1', "dark_blue", 0x0000AA),
    ('2', "dark_green", 0x00AA00),
    ('3', "dark_aqua", 0x00AAAA),
    ('4', "dark_red", 0xAA0000),
    ('5', "dark_purple", 0xAA00AA),
    ('6', "gold", 0xFFAA00),
    ('7', "gray", 0xAAAAAA),
    ('8', "dark_gray", 0x555555),
    ('9', "blue", 0x5555FF),
    ('a', "green", 0x55FF55),
    ('b', "aqua", 0x55FFFF),
    ('c', "red", 0xFF5555),
    ('d', "light_purple", 0xFF55FF),
    ('e', "yellow", 0xFFFF55),
    ('f', "white", 0xFFFFFF),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// Index into [`LEGACY`] (also the team colour id, 0 black .. 15 white).
    Legacy(u8),
    Rgb(u32),
}

impl Color {
    pub fn rgb(self) -> u32 {
        match self {
            Color::Legacy(i) => LEGACY[i as usize].2,
            Color::Rgb(c) => c,
        }
    }
    /// The nearest legacy colour (for a team's colour, which only knows 16).
    pub fn nearest_legacy(self) -> u8 {
        match self {
            Color::Legacy(i) => i,
            Color::Rgb(c) => {
                let (r, g, b) = ((c >> 16) as i32 & 255, (c >> 8) as i32 & 255, c as i32 & 255);
                let mut best = (i32::MAX, 15u8);
                for (i, (_, _, l)) in LEGACY.iter().enumerate() {
                    let (lr, lg, lb) = ((*l >> 16) as i32 & 255, (*l >> 8) as i32 & 255, *l as i32 & 255);
                    let d = (r - lr).pow(2) + (g - lg).pow(2) + (b - lb).pow(2);
                    if d < best.0 {
                        best = (d, i as u8);
                    }
                }
                best.1
            }
        }
    }
    fn json(self) -> Value {
        match self {
            Color::Legacy(i) => Value::from(LEGACY[i as usize].1),
            Color::Rgb(c) => Value::from(format!("#{c:06X}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub color: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underlined: bool,
    pub strikethrough: bool,
    pub obfuscated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

fn legacy_index(c: char) -> Option<u8> {
    c.to_digit(16).map(|d| d as u8)
}

/// TAB's `EnumChatFormat.color`: `&` before a format character becomes `§`.
pub fn colorize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if let Some(&n) = chars.peek() {
                if "0123456789AaBbCcDdEeFfKkLlMmNnOoRrXx#".contains(n) {
                    out.push('§');
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

fn parse_hex(s: &str) -> Option<u32> {
    if s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        u32::from_str_radix(s, 16).ok()
    } else {
        None
    }
}

/// Interpolates `colors` over `n` characters.
fn gradient_colors(colors: &[u32], n: usize) -> Vec<u32> {
    if n == 0 {
        return Vec::new();
    }
    if colors.len() == 1 || n == 1 {
        return vec![colors[0]; n];
    }
    let segments = colors.len() - 1;
    (0..n)
        .map(|i| {
            let t = i as f64 / (n - 1) as f64 * segments as f64;
            let seg = (t.floor() as usize).min(segments - 1);
            let local = t - seg as f64;
            let (a, b) = (colors[seg], colors[seg + 1]);
            let mix = |shift: u32| {
                let (x, y) = (((a >> shift) & 255) as f64, ((b >> shift) & 255) as f64);
                ((x + (y - x) * local).round() as u32) & 255
            };
            (mix(16) << 16) | (mix(8) << 8) | mix(0)
        })
        .collect()
}

/// A gradient's message with a `§#RRGGBB` before every visible character; the decorations
/// (`§l`...) inside it stay on after each colour, as a colour code would reset them.
fn apply_gradient(message: &str, colors: &[u32]) -> String {
    let mut visible = 0;
    {
        let mut chars = message.chars();
        while let Some(c) = chars.next() {
            if c == '§' {
                chars.next();
            } else {
                visible += 1;
            }
        }
    }
    let palette = gradient_colors(colors, visible);
    let mut out = String::new();
    let mut decorations = String::new();
    let mut i = 0;
    let mut chars = message.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            if let Some(code) = chars.next() {
                let lc = code.to_ascii_lowercase();
                if ('k'..='o').contains(&lc) {
                    decorations.push('§');
                    decorations.push(lc);
                } else if lc == 'r' {
                    decorations.clear();
                }
            }
            continue;
        }
        out.push_str(&format!("§#{:06X}", palette[i]));
        out.push_str(&decorations);
        out.push(c);
        i += 1;
    }
    out
}

/// Replaces every `open...close` gradient form in `text`.
fn replace_gradients(text: &str, minimessage: bool) -> String {
    let mut s = text.to_owned();
    // <#RRGGBB>text</#RRGGBB>, {#RRGGBB>}text{#RRGGBB<}, <$#RRGGBB>text<$#RRGGBB>,
    // <gradient:#RRGGBB>text</gradient:#RRGGBB>
    let forms: [(&str, &str, &str, &str); 4] = [
        ("{#", ">}", "{#", "<}"),
        ("<#", ">", "</#", ">"),
        ("<$#", ">", "<$#", ">"),
        ("<gradient:#", ">", "</gradient:#", ">"),
    ];
    s = expand_cmi_shortcut(&s);
    for (o1, o2, c1, c2) in forms {
        // A moving search start: an opener that is not a gradient (an ordinary `{#RRGGBB}`) is
        // skipped and the scan goes on after it, as TAB's regex matching does.
        let mut from = 0;
        let mut guard = 0;
        while guard < 256 {
            guard += 1;
            let Some(rel) = s.get(from..).and_then(|t| t.find(o1)) else { break };
            let start = from + rel;
            from = start + o1.len();
            let Some(a) = s.get(start + o1.len()..start + o1.len() + 6).and_then(parse_hex) else {
                continue;
            };
            let open_end = start + o1.len() + 6;
            if s.get(open_end..open_end + o2.len()) != Some(o2) {
                continue;
            }
            let body_start = open_end + o2.len();
            let Some(rel_close) = s[body_start..].find(c1) else {
                continue;
            };
            let close = body_start + rel_close;
            let Some(b) = s.get(close + c1.len()..close + c1.len() + 6).and_then(parse_hex) else {
                continue;
            };
            let close_end = close + c1.len() + 6;
            if s.get(close_end..close_end + c2.len()) != Some(c2) {
                continue;
            }
            let body = &s[body_start..close];
            if o1 == "<#" {
                if let Some(inner) = body.rfind("<#") {
                    // A nested opener: the gradient starts at the inner one.
                    if body.get(inner + 2..inner + 8).and_then(parse_hex).is_some() {
                        let fake_start = body_start + inner;
                        let replaced = apply_gradient(
                            &s[fake_start + 9..close],
                            &[parse_hex(&s[fake_start + 2..fake_start + 8]).unwrap_or(a), b],
                        );
                        s.replace_range(fake_start..close_end + c2.len(), &replaced);
                        from = fake_start + replaced.len();
                        continue;
                    }
                }
            }
            let replaced = apply_gradient(body, &[a, b]);
            s.replace_range(start..close_end + c2.len(), &replaced);
            from = start + replaced.len();
        }
    }
    if minimessage {
        s = replace_mm_gradients(&s);
    }
    s
}

/// CMI's `{#RRGGBB<>}`: the end of one gradient and the start of the next, written once;
/// expanded to `{#RRGGBB<}{#RRGGBB>}` before matching (TAB's `CMIGradient`).
fn expand_cmi_shortcut(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("{#") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        match (tail.get(2..8).and_then(parse_hex), tail.get(8..11)) {
            (Some(_), Some("<>}")) => {
                let hex = &tail[2..8];
                out.push_str(&format!("{{#{hex}<}}{{#{hex}>}}"));
                rest = &tail[11..];
            }
            _ => {
                out.push_str("{#");
                rest = &tail[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// MiniMessage `<gradient:#a:#b[:#c...]>text</gradient>` (named colours allowed too).
fn replace_mm_gradients(text: &str) -> String {
    let mut s = text.to_owned();
    for _ in 0..32 {
        let Some(start) = s.find("<gradient:") else { break };
        let Some(open_rel) = s[start..].find('>') else { break };
        let open_end = start + open_rel;
        let args = &s[start + "<gradient:".len()..open_end];
        let colors: Vec<u32> = args.split(':').filter_map(named_or_hex).collect();
        if colors.is_empty() {
            break;
        }
        let body_start = open_end + 1;
        let (body_end, close_end) = match s[body_start..].find("</gradient>") {
            Some(r) => (body_start + r, body_start + r + "</gradient>".len()),
            None => (s.len(), s.len()),
        };
        let replaced = apply_gradient(&s[body_start..body_end], &colors);
        s.replace_range(start..close_end, &replaced);
    }
    s
}

fn named_or_hex(name: &str) -> Option<u32> {
    let name = name.trim();
    if let Some(h) = name.strip_prefix('#') {
        return parse_hex(h);
    }
    let lower = name.to_ascii_lowercase();
    let lower = if lower == "grey" {
        "gray".to_owned()
    } else {
        lower.replace("dark_grey", "dark_gray")
    };
    LEGACY.iter().find(|(_, n, _)| *n == lower).map(|(_, _, c)| *c)
}

fn named_legacy(name: &str) -> Option<u8> {
    let lower = name.trim().to_ascii_lowercase();
    let lower = match lower.as_str() {
        "grey" => "gray".to_owned(),
        "dark_grey" => "dark_gray".to_owned(),
        _ => lower,
    };
    LEGACY.iter().position(|(_, n, _)| *n == lower).map(|i| i as u8)
}

/// The other RGB forms as `§#RRGGBB`: `{#RRGGBB}`, `#<RRGGBB>`.
fn replace_rgb_forms(text: &str) -> String {
    let mut s = text.to_owned();
    for (open, close) in [("{#", "}"), ("#<", ">")] {
        let mut from = 0;
        while let Some(rel) = s[from..].find(open) {
            let at = from + rel;
            let hex = s.get(at + open.len()..at + open.len() + 6);
            let tail = s.get(at + open.len() + 6..at + open.len() + 6 + close.len());
            match (hex.and_then(parse_hex), tail) {
                (Some(c), Some(t)) if t == close => {
                    let code = format!("§#{c:06X}");
                    s.replace_range(at..at + open.len() + 6 + close.len(), &code);
                    from = at + code.len();
                }
                _ => from = at + open.len(),
            }
        }
    }
    s
}

/// Parses TAB-formatted text into styled spans.
pub fn spans(text: &str, minimessage: bool) -> Vec<Span> {
    let text = colorize(text);
    let text = replace_gradients(&text, minimessage);
    let text = replace_rgb_forms(&text);
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Span> = Vec::new();
    let mut style = Style::default();
    let mut stack: Vec<(String, Style)> = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, style: Style, out: &mut Vec<Span>| {
        if current.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if last.style == style => last.text.push_str(current),
            _ => out.push(Span {
                text: std::mem::take(current),
                style,
            }),
        }
        current.clear();
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '§' && i + 1 < chars.len() {
            let code = chars[i + 1].to_ascii_lowercase();
            // §#RRGGBB
            if code == '#' && i + 8 <= chars.len() {
                let hex: String = chars[i + 2..i + 8].iter().collect();
                if let Some(rgb) = parse_hex(&hex) {
                    flush(&mut current, style, &mut out);
                    style = Style {
                        color: Some(Color::Rgb(rgb)),
                        ..Style::default()
                    };
                    i += 8;
                    continue;
                }
            }
            // §x§R§R§G§G§B§B
            if code == 'x' && i + 14 <= chars.len() {
                let mut hex = String::new();
                let mut ok = true;
                for k in 0..6 {
                    let (s, h) = (chars[i + 2 + k * 2], chars[i + 3 + k * 2]);
                    if s != '§' || !h.is_ascii_hexdigit() {
                        ok = false;
                        break;
                    }
                    hex.push(h);
                }
                if ok {
                    flush(&mut current, style, &mut out);
                    style = Style {
                        color: Some(Color::Rgb(parse_hex(&hex).unwrap_or(0xFFFFFF))),
                        ..Style::default()
                    };
                    i += 14;
                    continue;
                }
            }
            if let Some(idx) = legacy_index(code) {
                flush(&mut current, style, &mut out);
                style = Style {
                    color: Some(Color::Legacy(idx)),
                    ..Style::default()
                };
                i += 2;
                continue;
            }
            if ('k'..='o').contains(&code) {
                flush(&mut current, style, &mut out);
                match code {
                    'k' => style.obfuscated = true,
                    'l' => style.bold = true,
                    'm' => style.strikethrough = true,
                    'n' => style.underlined = true,
                    _ => style.italic = true,
                }
                i += 2;
                continue;
            }
            if code == 'r' {
                flush(&mut current, style, &mut out);
                style = Style::default();
                i += 2;
                continue;
            }
        }
        if minimessage && c == '<' {
            if let Some(end) = chars[i + 1..].iter().position(|&ch| ch == '>' || ch == '<') {
                if chars[i + 1 + end] == '>' {
                    let tag: String = chars[i + 1..i + 1 + end].iter().collect();
                    if let Some(applied) = mm_tag(&tag, style, &mut stack) {
                        flush(&mut current, style, &mut out);
                        style = applied;
                        i += end + 2;
                        continue;
                    }
                }
            }
        }
        current.push(c);
        i += 1;
    }
    flush(&mut current, style, &mut out);
    out
}

/// Applies one MiniMessage tag; None for a tag it does not know (left as text).
fn mm_tag(tag: &str, style: Style, stack: &mut Vec<(String, Style)>) -> Option<Style> {
    let tag = tag.trim();
    if let Some(name) = tag.strip_prefix('/') {
        let name = name.split(':').next().unwrap_or("").to_ascii_lowercase();
        let canonical = canonical_tag(&name)?;
        let pos = stack.iter().rposition(|(n, _)| *n == canonical)?;
        let before = stack[pos].1;
        stack.truncate(pos);
        return Some(before);
    }
    let lower = tag.to_ascii_lowercase();
    if lower == "reset" {
        stack.clear();
        return Some(Style::default());
    }
    let (negate, name) = match lower.strip_prefix('!') {
        Some(n) => (true, n.to_owned()),
        None => (false, lower.clone()),
    };
    let mut next = style;
    let canonical;
    if let Some(color) = name
        .strip_prefix("color:")
        .or_else(|| name.strip_prefix("colour:"))
        .or_else(|| name.strip_prefix("c:"))
    {
        next.color = Some(mm_color(color)?);
        canonical = "color".to_owned();
    } else if let Some(c) = mm_color_opt(&name) {
        next.color = Some(c);
        canonical = "color".to_owned();
    } else {
        canonical = canonical_tag(&name)?;
        let on = !negate;
        match canonical.as_str() {
            "bold" => next.bold = on,
            "italic" => next.italic = on,
            "underlined" => next.underlined = on,
            "strikethrough" => next.strikethrough = on,
            "obfuscated" => next.obfuscated = on,
            _ => return None,
        }
    }
    stack.push((canonical, style));
    Some(next)
}

fn canonical_tag(name: &str) -> Option<String> {
    Some(
        match name {
            "bold" | "b" => "bold",
            "italic" | "i" | "em" => "italic",
            "underlined" | "u" => "underlined",
            "strikethrough" | "st" => "strikethrough",
            "obfuscated" | "obf" => "obfuscated",
            "color" | "colour" | "c" => "color",
            n if mm_color_opt(n).is_some() => "color",
            _ => return None,
        }
        .to_owned(),
    )
}

fn mm_color_opt(name: &str) -> Option<Color> {
    if let Some(h) = name.strip_prefix('#') {
        return parse_hex(h).map(Color::Rgb);
    }
    named_legacy(name).map(Color::Legacy)
}

fn mm_color(name: &str) -> Option<Color> {
    mm_color_opt(name)
}

fn span_json(span: &Span) -> Value {
    let mut o = Map::new();
    o.insert("text".into(), Value::from(span.text.clone()));
    let s = &span.style;
    if let Some(c) = s.color {
        o.insert("color".into(), c.json());
    }
    for (key, on) in [
        ("bold", s.bold),
        ("italic", s.italic),
        ("underlined", s.underlined),
        ("strikethrough", s.strikethrough),
        ("obfuscated", s.obfuscated),
    ] {
        if on {
            o.insert(key.into(), Value::Bool(true));
        }
    }
    Value::Object(o)
}

/// TAB-formatted text as a vanilla chat component (JSON).
pub fn component(text: &str, minimessage: bool) -> Value {
    let spans = spans(text, minimessage);
    match spans.as_slice() {
        [] => json!({ "text": "" }),
        [one] => span_json(one),
        many => json!({ "text": "", "extra": many.iter().map(span_json).collect::<Vec<_>>() }),
    }
}

/// The text without any formatting.
pub fn plain(text: &str, minimessage: bool) -> String {
    spans(text, minimessage).into_iter().map(|s| s.text).collect()
}

/// The colour the text ends in (for a team's colour: the name in the name tag takes it).
pub fn last_color(text: &str, minimessage: bool) -> Option<Color> {
    let spans = spans(text, minimessage);
    // A trailing colour code with no text after it counts too: parse with a marker.
    let marked = spans_with_marker(text, minimessage);
    marked.or_else(|| spans.iter().rev().find_map(|s| s.style.color))
}

fn spans_with_marker(text: &str, minimessage: bool) -> Option<Color> {
    let probe = format!("{text}\u{1}");
    spans(&probe, minimessage)
        .last()
        .filter(|s| s.text.ends_with('\u{1}'))
        .and_then(|s| s.style.color)
}

/// Strips formatting from the component JSON (the plain string the client would show).
pub fn component_plain(v: &Value) -> String {
    let mut out = String::new();
    fn walk(v: &Value, out: &mut String) {
        match v {
            Value::String(s) => out.push_str(s),
            Value::Object(o) => {
                if let Some(Value::String(t)) = o.get("text") {
                    out.push_str(t);
                }
                if let Some(Value::Array(extra)) = o.get("extra") {
                    for e in extra {
                        walk(e, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|e| walk(e, out)),
            _ => {}
        }
    }
    walk(v, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_codes() {
        let s = spans("&4&l[Core] &f&lName", false);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].text, "[Core] ");
        assert_eq!(s[0].style.color, Some(Color::Legacy(4)));
        assert!(s[0].style.bold);
        assert_eq!(s[1].style.color, Some(Color::Legacy(15)));
        assert!(s[1].style.bold);
    }

    #[test]
    fn color_resets_decorations() {
        let s = spans("&lA&cB", false);
        assert!(s[0].style.bold);
        assert!(!s[1].style.bold);
    }

    #[test]
    fn rgb_forms() {
        for t in ["&#FF0000x", "{#FF0000}x", "#<FF0000>x", "&x&f&f&0&0&0&0x"] {
            let s = spans(t, false);
            assert_eq!(
                s,
                vec![Span {
                    text: "x".into(),
                    style: Style {
                        color: Some(Color::Rgb(0xFF0000)),
                        ..Style::default()
                    }
                }],
                "{t}"
            );
        }
    }

    #[test]
    fn cmi_gradient_after_a_plain_hex_colour() {
        let t = "{#FF0000}x {#00FF00>}ab{#0000FF<}";
        assert_eq!(plain(t, false), "x ab");
        let s = spans(t, false);
        assert!(s.iter().any(|sp| sp.style.color == Some(Color::Rgb(0x00FF00))));
    }

    #[test]
    fn cmi_shortcut_joins_two_gradients() {
        assert_eq!(expand_cmi_shortcut("{#FF0000>}ab{#00FF00<>}cd{#0000FF<}"), "{#FF0000>}ab{#00FF00<}{#00FF00>}cd{#0000FF<}");
        assert_eq!(plain("{#FF0000>}ab{#00FF00<>}cd{#0000FF<}", false), "abcd");
    }

    #[test]
    fn tab_gradient_keeps_decorations() {
        let s = spans("<#FFFFFF>&mab</#000000>", false);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].style.color, Some(Color::Rgb(0xFFFFFF)));
        assert_eq!(s[1].style.color, Some(Color::Rgb(0x000000)));
        assert!(s[0].style.strikethrough && s[1].style.strikethrough);
    }

    #[test]
    fn minimessage_subset() {
        let s = spans("<red>a<bold>b</bold>c</red>d", true);
        assert_eq!(
            s[0],
            Span {
                text: "a".into(),
                style: Style {
                    color: Some(Color::Legacy(12)),
                    ..Style::default()
                }
            }
        );
        assert!(s[1].style.bold);
        assert!(!s[2].style.bold && s[2].style.color == Some(Color::Legacy(12)));
        assert_eq!(s[3].style, Style::default());
        // Unknown tags stay text; off when minimessage is off.
        assert_eq!(plain("<foo>x", true), "<foo>x");
        assert_eq!(plain("<red>x", false), "<red>x");
    }

    #[test]
    fn last_color_for_team() {
        assert_eq!(last_color("&4&l[Core] &f&l", false), Some(Color::Legacy(15)));
        assert_eq!(last_color("&0&l[&7&lPlayer&0&l] &3", false), Some(Color::Legacy(3)));
        assert_eq!(last_color("plain", false), None);
        assert_eq!(Color::Rgb(0xFE5050).nearest_legacy(), 12);
    }

    #[test]
    fn component_json() {
        assert_eq!(component("", false), json!({"text": ""}));
        assert_eq!(component("&aHi", false), json!({"text": "Hi", "color": "green"}));
        let c = component("&aA&#123456B", false);
        assert_eq!(c["extra"][1]["color"], "#123456");
        assert_eq!(component_plain(&c), "AB");
    }
}
