//! Adventure MiniMessage as TAB's `components.minimessage-support` enables it, written out as a
//! vanilla chat component (JSON). `text.rs` keeps the common colour and decoration tags; this
//! module takes over when a text uses one of the tags below, so the usual texts stay unchanged.
//!
//! Tags: colours (`<red>`, `<#rrggbb>`, `<color:..>`), decorations and their `!`/`:false` forms,
//! `<reset>`, `<newline>`/`<br>`, `<gradient>` (with phase), `<rainbow>` (phase, `!` reversed),
//! `<transition>`, `<pride>`, `<shadow>`, `<font>`, `<insertion>`, `<click>`, `<hover>`
//! (`show_text`, `show_item`, `show_entity`), `<key>`, `<lang>`/`<lang_or>`, `<selector>`,
//! `<score>`, `<nbt>`, and `\<` to escape. TAB's own `&` codes and RGB forms work in the same text,
//! as TAB runs them before MiniMessage. An unknown tag stays as text, as MiniMessage does.
//! The gradient, rainbow and transition formulas follow MiniMessage's documentation; Adventure's
//! code is not in this repository, so the odd corners (negative phase) are the documented ones.

use crate::horizon_tab::text::{self, Color};
use serde_json::{json, Map, Value};

/// Tag names (lower case) that this module handles and `text.rs` does not.
const ADVANCED: &[&str] = &[
    "click", "hover", "insertion", "font", "key", "keybind", "lang", "tr", "translate", "lang_or", "tr_or",
    "translate_or", "rainbow", "transition", "newline", "br", "selector", "sel", "score", "nbt", "data",
    "pride", "shadow",
];

/// Does the text use a tag only this module knows?
pub fn needs_full(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '<' {
            let name: String = chars[i + 1..]
                .iter()
                .take_while(|c| !matches!(c, ':' | '>' | ' ' | '<'))
                .collect();
            let name = name.trim_start_matches(['!', '/']).to_ascii_lowercase();
            if ADVANCED.contains(&name.as_str()) {
                return true;
            }
        }
        i += 1;
    }
    false
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Fmt {
    color: Option<Color>,
    bold: Option<bool>,
    italic: Option<bool>,
    underlined: Option<bool>,
    strikethrough: Option<bool>,
    obfuscated: Option<bool>,
    font: Option<String>,
    insertion: Option<String>,
    click: Option<Value>,
    hover: Option<Value>,
    shadow: Option<i64>,
}

impl Fmt {
    /// `patch`'s set fields replace this one's.
    fn overlay(&mut self, patch: &Fmt) {
        macro_rules! take {
            ($($f:ident),*) => { $( if patch.$f.is_some() { self.$f = patch.$f.clone(); } )* };
        }
        take!(color, bold, italic, underlined, strikethrough, obfuscated, font, insertion, click, hover, shadow);
    }
}

#[derive(Clone, Debug)]
enum Content {
    Text(String),
    Raw(Map<String, Value>),
}

#[derive(Clone, Debug)]
struct Seg {
    content: Content,
    fmt: Fmt,
    effect: Option<usize>,
}

#[derive(Clone, Debug)]
enum EffectKind {
    Gradient { colors: Vec<u32>, phase: f64 },
    Rainbow { phase: i64, reverse: bool },
    Transition { colors: Vec<u32>, phase: f64 },
    Bands(Vec<u32>),
}

#[derive(Clone, Debug)]
struct Frame {
    name: String,
    patch: Fmt,
    reset: bool,
    effect: Option<(usize, EffectKind)>,
}

struct Parser {
    segs: Vec<Seg>,
    stack: Vec<Frame>,
    legacy: Fmt,
    next_effect: usize,
    depth: usize,
}

/// The canonical name a (closing) tag closes.
fn canon(name: &str) -> String {
    match name {
        "bold" | "b" => "bold",
        "italic" | "i" | "em" => "italic",
        "underlined" | "u" => "underlined",
        "strikethrough" | "st" => "strikethrough",
        "obfuscated" | "obf" => "obfuscated",
        "color" | "colour" | "c" => "color",
        "keybind" => "key",
        "tr" | "translate" => "lang",
        "tr_or" | "translate_or" | "lang_or" => "lang",
        "sel" => "selector",
        "data" => "nbt",
        n if text::mm_color_opt(n).is_some() => "color",
        n => n,
    }
    .to_owned()
}

/// Splits tag arguments on `:`, keeping quoted parts (`'..'`, `".."`) whole.
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == '\\' && chars.peek() == Some(&q) {
                    cur.push(q);
                    chars.next();
                } else if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            None => {
                if (c == '\'' || c == '"') && cur.is_empty() {
                    quote = Some(c);
                } else if c == ':' {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.push(c);
                }
            }
        }
    }
    out.push(cur);
    out
}

fn hsv(h: f64) -> u32 {
    let h = (h.rem_euclid(1.0)) * 6.0;
    let x = 1.0 - ((h % 2.0) - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    let c = |v: f64| ((v * 255.0).round() as u32) & 255;
    (c(r) << 16) | (c(g) << 8) | c(b)
}

fn mix(colors: &[u32], t: f64) -> u32 {
    match colors.len() {
        0 => 0xFFFFFF,
        1 => colors[0],
        n => {
            let t = t.clamp(0.0, 1.0) * (n - 1) as f64;
            let seg = (t.floor() as usize).min(n - 2);
            let local = t - seg as f64;
            let (a, b) = (colors[seg], colors[seg + 1]);
            let ch = |shift: u32| {
                let (x, y) = (((a >> shift) & 255) as f64, ((b >> shift) & 255) as f64);
                ((x + (y - x) * local).round() as u32) & 255
            };
            (ch(16) << 16) | (ch(8) << 8) | ch(0)
        }
    }
}

/// The colour of character `i` of `n` for an effect.
fn effect_colors(kind: &EffectKind, n: usize) -> Vec<u32> {
    match kind {
        EffectKind::Gradient { colors, phase } => {
            if *phase == 0.0 {
                return text::gradient_colors(colors, n);
            }
            let mut palette = colors.clone();
            let mut phase = *phase;
            if phase < 0.0 {
                palette.reverse();
                phase = 1.0 + phase;
            }
            (0..n)
                .map(|i| {
                    let base = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 };
                    let t = (base + phase).rem_euclid(2.0);
                    // Past the end the palette runs back, so a shifted gradient has no jump.
                    mix(&palette, if t > 1.0 { 2.0 - t } else { t })
                })
                .collect()
        }
        EffectKind::Rainbow { phase, reverse } => (0..n)
            .map(|i| {
                let i = if *reverse { n - 1 - i } else { i };
                let step = (i as i64 + phase).rem_euclid(n.max(1) as i64);
                hsv(step as f64 / n.max(1) as f64)
            })
            .collect(),
        EffectKind::Transition { colors, phase } => vec![mix(colors, phase.clamp(0.0, 1.0)); n],
        EffectKind::Bands(colors) => (0..n)
            .map(|i| colors[(i * colors.len() / n.max(1)).min(colors.len() - 1)])
            .collect(),
    }
}

impl Parser {
    fn new(depth: usize) -> Self {
        Parser { segs: Vec::new(), stack: Vec::new(), legacy: Fmt::default(), next_effect: 0, depth }
    }

    /// The format and the effect now in force.
    fn current(&self) -> (Fmt, Option<usize>) {
        let mut fmt = Fmt::default();
        let mut effect = None;
        for f in &self.stack {
            if f.reset {
                fmt = Fmt::default();
                effect = None;
            }
            fmt.overlay(&f.patch);
            if let Some((id, _)) = &f.effect {
                effect = Some(*id);
            }
        }
        fmt.overlay(&self.legacy);
        if self.legacy.color.is_some() {
            effect = None;
        }
        (fmt, effect)
    }

    fn push_content(&mut self, content: Content) {
        let (fmt, effect) = self.current();
        let effect = if fmt.color.is_none() { effect } else { None };
        self.segs.push(Seg { content, fmt, effect });
    }

    fn push_text(&mut self, t: &str) {
        if !t.is_empty() {
            self.push_content(Content::Text(t.to_owned()));
        }
    }

    /// Plain text with `§` codes (TAB's formats after `prepare`): the codes change the legacy format.
    fn text_chunk(&mut self, chunk: &str) {
        let chars: Vec<char> = chunk.chars().collect();
        let mut cur = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '§' && i + 1 < chars.len() {
                let code = chars[i + 1].to_ascii_lowercase();
                let mut consumed = 0;
                let mut new_color: Option<Color> = None;
                if code == '#' && i + 8 <= chars.len() {
                    let hex: String = chars[i + 2..i + 8].iter().collect();
                    if let Some(rgb) = text::parse_hex(&hex) {
                        new_color = Some(Color::Rgb(rgb));
                        consumed = 8;
                    }
                } else if code == 'x' && i + 14 <= chars.len() {
                    let mut hex = String::new();
                    for k in 0..6 {
                        if chars[i + 2 + k * 2] == '§' {
                            hex.push(chars[i + 3 + k * 2]);
                        }
                    }
                    if let Some(rgb) = text::parse_hex(&hex) {
                        new_color = Some(Color::Rgb(rgb));
                        consumed = 14;
                    }
                } else if let Some(d) = code.to_digit(16) {
                    new_color = Some(Color::Legacy(d as u8));
                    consumed = 2;
                }
                if consumed > 0 {
                    self.push_text(&std::mem::take(&mut cur));
                    self.legacy = Fmt { color: new_color, ..Fmt::default() };
                    i += consumed;
                    continue;
                }
                if ('k'..='o').contains(&code) {
                    self.push_text(&std::mem::take(&mut cur));
                    match code {
                        'k' => self.legacy.obfuscated = Some(true),
                        'l' => self.legacy.bold = Some(true),
                        'm' => self.legacy.strikethrough = Some(true),
                        'n' => self.legacy.underlined = Some(true),
                        _ => self.legacy.italic = Some(true),
                    }
                    i += 2;
                    continue;
                }
                if code == 'r' {
                    self.push_text(&std::mem::take(&mut cur));
                    self.legacy = Fmt::default();
                    i += 2;
                    continue;
                }
            }
            cur.push(c);
            i += 1;
        }
        self.push_text(&cur);
    }

    fn open(&mut self, name: &str, patch: Fmt, effect: Option<EffectKind>) {
        let effect = effect.map(|k| {
            self.next_effect += 1;
            (self.next_effect, k)
        });
        self.stack.push(Frame { name: name.to_owned(), patch, reset: false, effect });
    }

    /// Closes the frames from `pos` up, giving the colour effects their colours.
    fn close_from(&mut self, pos: usize) {
        let closed: Vec<Frame> = self.stack.drain(pos..).collect();
        for f in closed.into_iter().rev() {
            if let Some((id, kind)) = f.effect {
                self.resolve(id, &kind);
            }
        }
    }

    fn resolve(&mut self, id: usize, kind: &EffectKind) {
        let total: usize = self
            .segs
            .iter()
            .filter(|s| s.effect == Some(id))
            .map(|s| match &s.content {
                Content::Text(t) => t.chars().count(),
                Content::Raw(_) => 0,
            })
            .sum();
        if total == 0 {
            return;
        }
        let palette = effect_colors(kind, total);
        let mut at = 0;
        let old = std::mem::take(&mut self.segs);
        for seg in old {
            if seg.effect != Some(id) {
                self.segs.push(seg);
                continue;
            }
            let Content::Text(t) = &seg.content else {
                self.segs.push(seg);
                continue;
            };
            for ch in t.chars() {
                let mut fmt = seg.fmt.clone();
                fmt.color = Some(Color::Rgb(palette[at.min(palette.len() - 1)]));
                at += 1;
                self.segs.push(Seg { content: Content::Text(ch.to_string()), fmt, effect: None });
            }
        }
    }

    /// Handles one tag; false when it is not one (it stays as text).
    fn tag(&mut self, inner: &str) -> bool {
        let inner = inner.trim();
        if let Some(close) = inner.strip_prefix('/') {
            let name = canon(&split_args(close)[0].to_ascii_lowercase());
            return match self.stack.iter().rposition(|f| f.name == name) {
                Some(pos) => {
                    self.close_from(pos);
                    true
                }
                None => false,
            };
        }
        let args = split_args(inner);
        let raw_name = args[0].to_ascii_lowercase();
        let (negate, name) = match raw_name.strip_prefix('!') {
            Some(n) => (true, n.to_owned()),
            None => (false, raw_name),
        };
        let rest = &args[1..];
        let canonical = canon(&name);
        let mut patch = Fmt::default();
        match name.as_str() {
            "reset" => {
                self.close_from(0);
                self.legacy = Fmt::default();
                self.stack.push(Frame { name: "reset".into(), patch: Fmt::default(), reset: true, effect: None });
                return true;
            }
            "newline" | "br" => {
                self.push_text("\n");
                return true;
            }
            "bold" | "b" | "italic" | "i" | "em" | "underlined" | "u" | "strikethrough" | "st" | "obfuscated" | "obf" => {
                let on = !negate && rest.first().map(|a| a != "false").unwrap_or(true);
                let slot = Some(on);
                match canonical.as_str() {
                    "bold" => patch.bold = slot,
                    "italic" => patch.italic = slot,
                    "underlined" => patch.underlined = slot,
                    "strikethrough" => patch.strikethrough = slot,
                    _ => patch.obfuscated = slot,
                }
            }
            "color" | "colour" | "c" => {
                let Some(c) = rest.first().and_then(|v| text::mm_color_opt(&v.to_ascii_lowercase())) else {
                    return false;
                };
                patch.color = Some(c);
            }
            "shadow" => {
                if negate {
                    patch.shadow = Some(0);
                } else {
                    let Some(rgb) = rest.first().and_then(|v| text::mm_color_opt(&v.to_ascii_lowercase())) else {
                        return false;
                    };
                    let alpha = rest.get(1).and_then(|a| a.parse::<f64>().ok()).unwrap_or(0.25).clamp(0.0, 1.0);
                    patch.shadow = Some((((alpha * 255.0).round() as i64) << 24) | rgb.rgb() as i64);
                }
            }
            "font" => {
                if rest.is_empty() {
                    return false;
                }
                patch.font = Some(rest.join(":"));
            }
            "insertion" => {
                if rest.is_empty() {
                    return false;
                }
                patch.insertion = Some(rest.join(":"));
            }
            "click" => {
                let Some(action) = rest.first().map(|a| a.to_ascii_lowercase()) else { return false };
                let value = rest[1..].join(":");
                patch.click = Some(match action.as_str() {
                    "open_url" => json!({ "action": "open_url", "url": value }),
                    "run_command" => json!({ "action": "run_command", "command": value }),
                    "suggest_command" => json!({ "action": "suggest_command", "command": value }),
                    "copy_to_clipboard" => json!({ "action": "copy_to_clipboard", "value": value }),
                    "change_page" => match value.parse::<i64>() {
                        Ok(p) => json!({ "action": "change_page", "page": p }),
                        Err(_) => return false,
                    },
                    _ => return false,
                });
            }
            "hover" => {
                let Some(action) = rest.first().map(|a| a.to_ascii_lowercase()) else { return false };
                patch.hover = Some(match action.as_str() {
                    "show_text" => {
                        if self.depth >= 8 {
                            return false;
                        }
                        let body = rest[1..].join(":");
                        json!({ "action": "show_text", "value": component_at(&body, self.depth + 1) })
                    }
                    "show_item" => {
                        // show_item:<id>[:count]  (an id has its own `:`, so a trailing number is the count)
                        let mut parts: Vec<&str> = rest[1..].iter().map(String::as_str).collect();
                        let mut count = 1;
                        if parts.len() > 1 {
                            if let Ok(c) = parts[parts.len() - 1].parse::<i64>() {
                                count = c;
                                parts.pop();
                            }
                        }
                        if parts.is_empty() {
                            return false;
                        }
                        json!({ "action": "show_item", "id": parts.join(":"), "count": count })
                    }
                    "show_entity" => {
                        // show_entity:<type>:<uuid>[:name]; a namespaced type has its own `:`.
                        if rest.len() < 3 {
                            return false;
                        }
                        let (ty, uuid, name) = if rest[2].contains('-') {
                            (rest[1].clone(), rest[2].clone(), rest.get(3))
                        } else if rest.len() >= 4 {
                            (format!("{}:{}", rest[1], rest[2]), rest[3].clone(), rest.get(4))
                        } else {
                            return false;
                        };
                        let mut v = json!({ "action": "show_entity", "id": ty, "uuid": uuid });
                        if let Some(n) = name {
                            v["name"] = component_at(n, self.depth + 1);
                        }
                        v
                    }
                    _ => return false,
                });
            }
            "rainbow" => {
                let mut reverse = false;
                let mut phase = 0;
                if let Some(a) = rest.first() {
                    let a = if let Some(r) = a.strip_prefix('!') {
                        reverse = true;
                        r
                    } else {
                        a.as_str()
                    };
                    if !a.is_empty() {
                        match a.parse::<i64>() {
                            Ok(p) => phase = p,
                            Err(_) => return false,
                        }
                    }
                }
                self.open("rainbow", patch, Some(EffectKind::Rainbow { phase, reverse }));
                return true;
            }
            "gradient" => {
                let mut colors = Vec::new();
                let mut phase = 0.0;
                for (k, a) in rest.iter().enumerate() {
                    match text::mm_color_opt(&a.to_ascii_lowercase()) {
                        Some(c) => colors.push(c.rgb()),
                        None if k == rest.len() - 1 => match a.parse::<f64>() {
                            Ok(p) => phase = p,
                            Err(_) => return false,
                        },
                        None => return false,
                    }
                }
                if colors.is_empty() {
                    colors = vec![0xFFFFFF, 0x000000];
                }
                self.open("gradient", patch, Some(EffectKind::Gradient { colors, phase }));
                return true;
            }
            "transition" => {
                let mut colors = Vec::new();
                let mut phase = 0.0;
                for (k, a) in rest.iter().enumerate() {
                    match text::mm_color_opt(&a.to_ascii_lowercase()) {
                        Some(c) => colors.push(c.rgb()),
                        None if k == rest.len() - 1 => match a.parse::<f64>() {
                            Ok(p) => phase = p,
                            Err(_) => return false,
                        },
                        None => return false,
                    }
                }
                if colors.is_empty() {
                    return false;
                }
                self.open("transition", patch, Some(EffectKind::Transition { colors, phase }));
                return true;
            }
            "pride" => {
                let flag = rest.first().map(|f| f.to_ascii_lowercase()).unwrap_or_else(|| "pride".into());
                let bands: Vec<u32> = match flag.as_str() {
                    "pride" => vec![0xE40303, 0xFF8C00, 0xFFED00, 0x008026, 0x24408E, 0x732982],
                    "trans" => vec![0x5BCEFA, 0xF5A9B8, 0xFFFFFF, 0xF5A9B8, 0x5BCEFA],
                    "bi" => vec![0xD60270, 0xD60270, 0x9B4F96, 0x0038A8, 0x0038A8],
                    "pan" => vec![0xFF218C, 0xFFD800, 0x21B1FF],
                    "nb" => vec![0xFCF434, 0xFFFFFF, 0x9C59D1, 0x2C2C2C],
                    "ace" => vec![0x000000, 0xA3A3A3, 0xFFFFFF, 0x800080],
                    _ => return false,
                };
                self.open("pride", patch, Some(EffectKind::Bands(bands)));
                return true;
            }
            "key" | "keybind" => {
                let Some(k) = rest.first() else { return false };
                self.raw(json!({ "keybind": k }));
                return true;
            }
            "lang" | "tr" | "translate" | "lang_or" | "tr_or" | "translate_or" => {
                let or = name.ends_with("_or");
                let Some(key) = rest.first() else { return false };
                let mut o = json!({ "translate": key });
                let mut first_arg = 1;
                if or {
                    let Some(fallback) = rest.get(1) else { return false };
                    o["fallback"] = Value::from(fallback.as_str());
                    first_arg = 2;
                }
                if rest.len() > first_arg && self.depth < 8 {
                    let with: Vec<Value> = rest[first_arg..].iter().map(|a| component_at(a, self.depth + 1)).collect();
                    o["with"] = Value::Array(with);
                }
                self.raw(o);
                return true;
            }
            "selector" | "sel" => {
                let Some(s) = rest.first() else { return false };
                self.raw(json!({ "selector": s }));
                return true;
            }
            "score" => {
                if rest.len() < 2 {
                    return false;
                }
                self.raw(json!({ "score": { "name": rest[0], "objective": rest[1] } }));
                return true;
            }
            "nbt" | "data" => {
                if rest.len() < 3 {
                    return false;
                }
                let key = match rest[0].to_ascii_lowercase().as_str() {
                    "block" => "block",
                    "entity" => "entity",
                    "storage" => "storage",
                    _ => return false,
                };
                self.raw(json!({ "nbt": rest[2], key: rest[1] }));
                return true;
            }
            _ => {
                let Some(c) = text::mm_color_opt(&name) else { return false };
                patch.color = Some(c);
            }
        }
        self.open(&canonical, patch, None);
        true
    }

    fn raw(&mut self, o: Value) {
        if let Value::Object(m) = o {
            self.push_content(Content::Raw(m));
        }
    }

    fn run(&mut self, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        let mut cur = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '<' | '\\') {
                cur.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '<' {
                if let Some(end) = tag_end(&chars, i) {
                    let inner: String = chars[i + 1..end].iter().collect();
                    self.text_chunk(&std::mem::take(&mut cur));
                    if self.tag(&inner) {
                        i = end + 1;
                        continue;
                    }
                    // Not a tag: its characters go on as text.
                }
            }
            cur.push(c);
            i += 1;
        }
        self.text_chunk(&cur);
        self.close_from(0);
    }

    fn finish(self) -> Vec<Seg> {
        self.segs
    }
}

/// The index of the `>` that ends the tag starting at `start`, if it is one.
fn tag_end(chars: &[char], start: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut prev = '<';
    for (k, &c) in chars.iter().enumerate().skip(start + 1) {
        match quote {
            Some(q) => {
                if c == q && prev != '\\' {
                    quote = None;
                }
            }
            None => {
                if c == '>' {
                    return (k > start + 1).then_some(k);
                }
                if c == '<' {
                    return None;
                }
                if (c == '\'' || c == '"') && prev == ':' {
                    quote = Some(c);
                }
            }
        }
        prev = c;
    }
    None
}

fn seg_json(s: &Seg) -> Value {
    let mut o = Map::new();
    match &s.content {
        Content::Text(t) => {
            o.insert("text".into(), Value::from(t.clone()));
        }
        Content::Raw(m) => {
            for (k, v) in m {
                o.insert(k.clone(), v.clone());
            }
        }
    }
    let f = &s.fmt;
    if let Some(c) = f.color {
        o.insert("color".into(), c.json());
    }
    for (key, v) in [
        ("bold", f.bold),
        ("italic", f.italic),
        ("underlined", f.underlined),
        ("strikethrough", f.strikethrough),
        ("obfuscated", f.obfuscated),
    ] {
        if let Some(b) = v {
            o.insert(key.into(), Value::Bool(b));
        }
    }
    if let Some(font) = &f.font {
        o.insert("font".into(), Value::from(font.clone()));
    }
    if let Some(i) = &f.insertion {
        o.insert("insertion".into(), Value::from(i.clone()));
    }
    if let Some(c) = &f.click {
        o.insert("click_event".into(), c.clone());
    }
    if let Some(h) = &f.hover {
        o.insert("hover_event".into(), h.clone());
    }
    if let Some(s) = f.shadow {
        o.insert("shadow_color".into(), Value::from(s));
    }
    Value::Object(o)
}

fn component_at(text: &str, depth: usize) -> Value {
    let mut p = Parser::new(depth);
    p.run(&text::prepare(text));
    let mut merged: Vec<Seg> = Vec::new();
    for seg in p.finish() {
        let mut done = false;
        if let (Some(last), Content::Text(add)) = (merged.last_mut(), &seg.content) {
            if last.fmt == seg.fmt {
                if let Content::Text(t) = &mut last.content {
                    t.push_str(add);
                    done = true;
                }
            }
        }
        if !done {
            merged.push(seg);
        }
    }
    match merged.as_slice() {
        [] => json!({ "text": "" }),
        [one] => seg_json(one),
        many => json!({ "text": "", "extra": many.iter().map(seg_json).collect::<Vec<_>>() }),
    }
}

/// MiniMessage text (with TAB's own formats) as a chat component.
pub fn component(text: &str) -> Value {
    component_at(text, 0)
}

/// The text without any tags.
pub fn plain(text: &str) -> String {
    text::component_plain(&component(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_needs_nothing_special() {
        assert!(!needs_full("<red>a</red> &bb"));
        assert!(needs_full("<click:run_command:/x>a"));
        assert!(needs_full("a<br>b"));
        assert!(!needs_full("\\<click:x>"));
    }

    #[test]
    fn the_text_module_hands_full_minimessage_over() {
        let on = text::component("<click:run_command:/tab>&3Help", true);
        assert_eq!(on["click_event"]["command"], "/tab");
        assert_eq!(on["color"], "dark_aqua");
        assert_eq!(text::plain("<click:run_command:/tab>Help", true), "Help");
        // Off, the text stays as it is.
        assert_eq!(text::plain("<click:run_command:/tab>Help", false), "<click:run_command:/tab>Help");
    }

    #[test]
    fn click_and_hover() {
        let c = component("<click:run_command:/tab reload><hover:show_text:'<red>Reload'>Go</hover></click>");
        assert_eq!(
            c,
            json!({
                "text": "Go",
                "click_event": { "action": "run_command", "command": "/tab reload" },
                "hover_event": { "action": "show_text", "value": { "text": "Reload", "color": "red" } }
            })
        );
    }

    #[test]
    fn open_url_keeps_its_colons() {
        let c = component("<click:open_url:https://example.org/a:b>x");
        assert_eq!(c["click_event"], json!({ "action": "open_url", "url": "https://example.org/a:b" }));
    }

    #[test]
    fn close_tag_ends_the_format() {
        let c = component("<insertion:hi>a</insertion>b");
        assert_eq!(c, json!({ "text": "", "extra": [ { "text": "a", "insertion": "hi" }, { "text": "b" } ] }));
    }

    #[test]
    fn rainbow_of_six_steps_the_hue() {
        let c = component("<rainbow>abcdef");
        let colors: Vec<&str> = c["extra"].as_array().unwrap().iter().map(|e| e["color"].as_str().unwrap()).collect();
        assert_eq!(colors, vec!["#FF0000", "#FFFF00", "#00FF00", "#00FFFF", "#0000FF", "#FF00FF"]);
        let rev = component("<rainbow:!>abcdef");
        assert_eq!(rev["extra"][0]["color"], "#FF00FF");
        assert_eq!(rev["extra"][5]["color"], "#FF0000");
    }

    #[test]
    fn gradient_with_three_characters() {
        let c = component("<gradient:#000000:#ffffff>abc");
        let colors: Vec<&str> = c["extra"].as_array().unwrap().iter().map(|e| e["color"].as_str().unwrap()).collect();
        assert_eq!(colors, vec!["#000000", "#808080", "#FFFFFF"]);
    }

    #[test]
    fn transition_is_one_colour() {
        let c = component("<transition:#000000:#ffffff:0.5>ab");
        assert_eq!(c["color"], "#808080");
        assert_eq!(c["text"], "ab");
    }

    #[test]
    fn newline_font_key_and_lang() {
        assert_eq!(component("a<br>b"), json!({ "text": "a\nb" }));
        assert_eq!(component("<font:uniform>x"), json!({ "text": "x", "font": "uniform" }));
        assert_eq!(component("<key:key.jump>"), json!({ "keybind": "key.jump" }));
        assert_eq!(
            component("<lang:chat.type.text:'<red>Steve':hi>"),
            json!({ "translate": "chat.type.text", "with": [ { "text": "Steve", "color": "red" }, { "text": "hi" } ] })
        );
        assert_eq!(
            component("<lang_or:a.b:Fallback>"),
            json!({ "translate": "a.b", "fallback": "Fallback" })
        );
    }

    #[test]
    fn decoration_negation_and_reset() {
        let c = component("<bold>a<!bold>b<reset>c<br>");
        assert_eq!(
            c,
            json!({ "text": "", "extra": [
                { "text": "a", "bold": true },
                { "text": "b", "bold": false },
                { "text": "c\n" }
            ] })
        );
    }

    #[test]
    fn tab_codes_work_inside_tags() {
        let c = component("&c<insertion:i>Hello</insertion> &aworld");
        assert_eq!(
            c,
            json!({ "text": "", "extra": [
                { "text": "Hello", "color": "red", "insertion": "i" },
                { "text": " ", "color": "red" },
                { "text": "world", "color": "green" }
            ] })
        );
    }

    #[test]
    fn unknown_tag_and_escape_stay_text() {
        assert_eq!(plain("<click:bogus:x>a"), "<click:bogus:x>a");
        assert_eq!(plain("\\<br>x<br>"), "<br>x\n");
    }

    #[test]
    fn shadow_and_score_selector_nbt() {
        assert_eq!(component("<shadow:#ff0000>x")["shadow_color"], json!(0x40FF0000i64));
        assert_eq!(component("<score:Steve:kills>"), json!({ "score": { "name": "Steve", "objective": "kills" } }));
        assert_eq!(component("<selector:@a>"), json!({ "selector": "@a" }));
        assert_eq!(component("<nbt:block:1 2 3:Items>"), json!({ "nbt": "Items", "block": "1 2 3" }));
    }

    #[test]
    fn show_item_and_entity() {
        let c = component("<hover:show_item:minecraft:diamond:3>x");
        assert_eq!(c["hover_event"], json!({ "action": "show_item", "id": "minecraft:diamond", "count": 3 }));
        let c = component("<hover:show_entity:minecraft:pig:00000000-0000-0000-0000-000000000001>x");
        assert_eq!(
            c["hover_event"],
            json!({ "action": "show_entity", "id": "minecraft:pig", "uuid": "00000000-0000-0000-0000-000000000001" })
        );
    }
}
