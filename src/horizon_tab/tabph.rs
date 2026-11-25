//! TAB's own placeholders (`%player%`, `%ping%`, `%online%`, `%animation:Name%`,
//! `%condition:name%`...), its output replacements and refresh intervals, with every other
//! `%identifier_params%` handed to the placeholder registry ([`crate::horizon_tab::placeholders`]). Names and
//! default refresh intervals follow TAB's `TabConstants.Placeholder` (Apache-2.0, NOTICE).

use crate::horizon_tab::conditions::Condition;
use crate::horizon_tab::config::Config;
use crate::horizon_tab::integration;
use crate::horizon_tab::placeholders::{self, builtin, Player, Server};
use crate::horizon_tab::text;
use crate::horizon_tab::yaml::Yaml;
use chrono::{FixedOffset, TimeZone};
use std::cell::RefCell;
use std::collections::HashMap;

/// A player with what TAB adds to the registry's view of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabPlayer {
    pub p: Player,
    pub group: String,
    pub bedrock: bool,
    /// Whether the player's team collides (`/tab setcollision`, else `scoreboard-teams.enable-collision`).
    pub collision: bool,
    pub version_id: i32,
    pub version_name: String,
}

pub struct Ctx<'a> {
    pub target: &'a TabPlayer,
    pub viewer: &'a TabPlayer,
    pub all: &'a [TabPlayer],
    pub server: &'a Server,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub interval_ms: i64,
    pub frames: Vec<String>,
}

impl Animation {
    pub fn frame(&self, now_ms: i64) -> &str {
        if self.frames.is_empty() {
            return "";
        }
        let interval = self.interval_ms.max(50);
        let len = self.frames.len() as i64;
        &self.frames[(now_ms.rem_euclid(len * interval) / interval) as usize]
    }
}

pub fn load_animations(y: &Yaml) -> HashMap<String, Animation> {
    y.entries()
        .iter()
        .map(|(name, a)| {
            (
                name.to_ascii_lowercase(),
                Animation {
                    interval_ms: a.get("change-interval").i64_or(1000),
                    frames: a.get("texts").str_list(),
                },
            )
        })
        .collect()
}

/// TAB's built-in refresh intervals (ms; -1 never changes for a player).
fn builtin_interval(token: &str) -> Option<i64> {
    Some(match token {
        "%displayname%" => 500,
        "%group%"
        | "%world%"
        | "%server%"
        | "%player%"
        | "%uuid%"
        | "%player-version%"
        | "%player-version-id%"
        | "%memory-max%"
        | "%memory-max-gb%"
        | "%bedrock%" => -1,
        "%vanished%"
        | "%invisible%"
        | "%collision%"
        | "%deaths%"
        | "%worldonline%"
        | "%serveronline%"
        | "%online%"
        | "%time%"
        | "%tps%"
        | "%luckperms-prefix%"
        | "%luckperms-suffix%"
        | "%luckperms_prefix%"
        | "%luckperms_suffix%"
        | "%luckperms-prefixes%"
        | "%luckperms-suffixes%"
        | "%luckperms-weight%"
        | "%vault-prefix%"
        | "%vault-suffix%" => 1000,
        "%date%" => 60000,
        "%ping%" => 500,
        "%memory-used%" | "%memory-used-gb%" | "%health%" => 200,
        "%staffonline%" | "%nonstaffonline%" => 2000,
        "%gamemode%" | "%mspt%" => 1000,
        t if t.starts_with("%online_") || t.starts_with("%playerlist-group_") => 1000,
        _ => return None,
    })
}

/// Whether a token is a server-wide value (cached once for everyone).
fn server_wide(token: &str) -> bool {
    matches!(
        token,
        "%time%"
            | "%date%"
            | "%online%"
            | "%staffonline%"
            | "%nonstaffonline%"
            | "%memory-used%"
            | "%memory-max%"
            | "%memory-used-gb%"
            | "%memory-max-gb%"
            | "%tps%"
            | "%mspt%"
            | "%serveronline%"
    ) || token.starts_with("%server_")
        || token.starts_with("%online_")
        || token.starts_with("%playerlist-group_")
}

pub struct TabPlaceholders {
    pub date_format: String,
    pub time_format: String,
    pub locale: String,
    pub time_offset_hours: f64,
    pub replacements: Vec<(String, Vec<(String, String)>)>,
    pub default_refresh_ms: i64,
    pub refresh_ms: HashMap<String, i64>,
    pub animations: HashMap<String, Animation>,
    pub conditions: HashMap<String, Condition>,
    pub now_ms: i64,
    cache: RefCell<HashMap<(u128, String), (String, i64)>>,
    /// `/tab cpu`: nanoseconds spent per placeholder while tracking.
    timings: RefCell<HashMap<String, u64>>,
    pub track: std::cell::Cell<bool>,
}

const MAX_DEPTH: usize = 8;

impl TabPlaceholders {
    pub fn new(cfg: &Config, animations: HashMap<String, Animation>) -> TabPlaceholders {
        TabPlaceholders {
            date_format: cfg.date_format.clone(),
            time_format: cfg.time_format.clone(),
            locale: cfg.locale.clone(),
            time_offset_hours: cfg.time_offset_hours,
            replacements: cfg.replacements.clone(),
            default_refresh_ms: cfg.default_refresh_ms,
            refresh_ms: cfg.refresh_ms.clone(),
            animations,
            conditions: cfg
                .conditions
                .iter()
                .map(|c| {
                    (
                        c.name.to_ascii_lowercase(),
                        Condition::new(&c.name, &c.conditions, c.and, &c.yes, &c.no),
                    )
                })
                .collect(),
            now_ms: 0,
            cache: RefCell::new(HashMap::new()),
            timings: RefCell::new(HashMap::new()),
            track: std::cell::Cell::new(false),
        }
    }

    /// The nanoseconds each placeholder took since the last call (`/tab cpu`).
    pub fn take_timings(&self) -> HashMap<String, u64> {
        std::mem::take(&mut *self.timings.borrow_mut())
    }

    /// A placeholder's refresh interval in ms and whether the config sets it.
    pub fn refresh_info(&self, token: &str) -> (i64, bool) {
        (self.interval(token), self.refresh_ms.contains_key(token))
    }

    /// Forgets a player's cached values (when they leave).
    pub fn forget(&self, uuid: u128) {
        self.cache.borrow_mut().retain(|(u, _), _| *u != uuid);
    }

    fn interval(&self, token: &str) -> i64 {
        self.refresh_ms
            .get(token)
            .copied()
            .or_else(|| builtin_interval(token))
            .unwrap_or(self.default_refresh_ms)
    }

    /// Replaces every placeholder in `text`.
    pub fn parse(&self, text: &str, ctx: &Ctx) -> String {
        self.parse_depth(text, ctx, 0)
    }

    fn parse_depth(&self, text: &str, ctx: &Ctx, depth: usize) -> String {
        if depth > MAX_DEPTH || !text.contains('%') {
            return text.to_owned();
        }
        let mut out = String::with_capacity(text.len());
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '%' {
                if let Some(j) = chars[i + 1..].iter().position(|&c| c == '%') {
                    let inner: String = chars[i + 1..i + 1 + j].iter().collect();
                    if inner.is_empty() {
                        // `%%` is a percent sign.
                        out.push('%');
                        i += 2;
                        continue;
                    }
                    if !inner.contains(' ') {
                        let token = format!("%{inner}%");
                        if let Some(v) = self.token(&token, ctx, depth) {
                            out.push_str(&v);
                            i += j + 2;
                            continue;
                        }
                        out.push_str(&token);
                        i += j + 2;
                        continue;
                    }
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    /// One token's value with replacements applied; None when nothing answers it.
    fn token(&self, token: &str, ctx: &Ctx, depth: usize) -> Option<String> {
        let raw = self.raw(token, ctx, depth)?;
        Some(self.replace_output(token, raw, ctx, depth))
    }

    fn replace_output(&self, token: &str, value: String, ctx: &Ctx, depth: usize) -> String {
        let Some((_, rules)) = self.replacements.iter().find(|(k, _)| k == token) else {
            return value;
        };
        self.apply_rules(rules, value, ctx, depth)
    }

    /// `parse`, then the output replacements a proxy sent for the token when this server has none of its own.
    pub fn parse_with_rules(&self, token: &str, ctx: &Ctx, extra: &[(String, Vec<(String, String)>)]) -> String {
        let value = self.parse(token, ctx);
        if self.replacements.iter().any(|(k, _)| k == token) {
            return value;
        }
        match extra.iter().find(|(k, _)| k == token) {
            Some((_, rules)) => self.apply_rules(rules, value, ctx, 0),
            None => value,
        }
    }

    fn apply_rules(&self, rules: &[(String, String)], value: String, ctx: &Ctx, depth: usize) -> String {
        let colored = text::colorize(&value);
        let mut chosen: Option<&String> = rules.iter().find(|(k, _)| text::colorize(k) == colored).map(|(_, v)| v);
        if chosen.is_none() {
            if let Ok(n) = value.trim().replace(',', "").parse::<f64>() {
                chosen = rules
                    .iter()
                    .find(|(k, _)| {
                        k.split_once('-').is_some_and(|(a, b)| {
                            matches!((a.trim().parse::<f64>(), b.trim().parse::<f64>()), (Ok(a), Ok(b)) if a <= n && n <= b)
                        })
                    })
                    .map(|(_, v)| v);
            }
        }
        if chosen.is_none() {
            chosen = rules.iter().find(|(k, _)| k == "else").map(|(_, v)| v);
        }
        match chosen {
            Some(r) => {
                let r = r.replace("%value%", &value);
                self.parse_depth(&r, ctx, depth + 1)
            }
            None => value,
        }
    }

    fn raw(&self, token: &str, ctx: &Ctx, depth: usize) -> Option<String> {
        let inner = &token[1..token.len() - 1];
        // Uncached, time- or other-player-dependent forms first.
        if let Some(name) = inner.strip_prefix("animation:") {
            let a = self.animations.get(&name.to_ascii_lowercase())?;
            return Some(self.parse_depth(a.frame(self.now_ms), ctx, depth + 1));
        }
        if let Some(name) = inner.strip_prefix("condition:") {
            let c = self.conditions.get(&name.to_ascii_lowercase())?;
            let out = if self.is_met(c, ctx, depth) { &c.yes } else { &c.no };
            return Some(self.parse_depth(out, ctx, depth + 1));
        }
        if let Some(name) = inner.strip_prefix("rel_condition:") {
            let c = self.conditions.get(&name.to_ascii_lowercase())?;
            let out = if self.is_met(c, ctx, depth) { &c.yes } else { &c.no };
            return Some(self.parse_depth(out, ctx, depth + 1));
        }
        if let Some(rest) = inner.strip_prefix("viewer:") {
            let vctx = Ctx {
                target: ctx.viewer,
                viewer: ctx.viewer,
                all: ctx.all,
                server: ctx.server,
            };
            return self.raw(&format!("%{rest}%"), &vctx, depth + 1);
        }
        if let Some(rel) = inner.strip_prefix("rel_") {
            return placeholders::resolve_relational(&ctx.viewer.p, &ctx.target.p, ctx.server, rel);
        }
        let interval = self.interval(token);
        let key = (if server_wide(token) { 0 } else { ctx.target.p.uuid }, token.to_owned());
        if let Some((v, at)) = self.cache.borrow().get(&key) {
            if interval < 0 || self.now_ms - at < interval {
                return Some(v.clone());
            }
        }
        let started = self.track.get().then(std::time::Instant::now);
        let v = self.compute(token, inner, ctx);
        if let Some(at) = started {
            *self.timings.borrow_mut().entry(token.to_owned()).or_insert(0) += at.elapsed().as_nanos() as u64;
        }
        let v = v?;
        self.cache.borrow_mut().insert(key, (v.clone(), self.now_ms));
        Some(v)
    }

    fn now_local(&self) -> chrono::DateTime<FixedOffset> {
        let offset = chrono::Local::now().offset().local_minus_utc() + (self.time_offset_hours * 3600.0) as i32;
        let tz = FixedOffset::east_opt(offset).unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero"));
        tz.timestamp_millis_opt(self.now_ms)
            .single()
            .unwrap_or_else(|| tz.timestamp_millis_opt(0).single().expect("epoch"))
    }

    fn compute(&self, token: &str, inner: &str, ctx: &Ctx) -> Option<String> {
        let t = ctx.target;
        let p = &t.p;
        let visible = || ctx.all.iter().filter(|o| !o.p.vanished);
        let staff = |o: &TabPlayer| integration::permission(o.p.uuid, "tab.staff", o.p.op_level);
        Some(match token {
            "%player%" => p.name.clone(),
            "%uuid%" => p.uuid_string(),
            "%world%" => p.world.clone(),
            "%server%" => ctx.server.level_name.clone(),
            "%displayname%" => p.display_name.clone().unwrap_or_else(|| p.name.clone()),
            "%group%" => t.group.clone(),
            "%ping%" => p.ping.to_string(),
            "%vanished%" => p.vanished.to_string(),
            "%invisible%" => p.invisible.to_string(),
            "%deaths%" => p.deaths.to_string(),
            "%collision%" => t.collision.to_string(),
            "%gamemode%" => p.game_mode.to_string(),
            "%health%" => ((p.health as f64).ceil() as i64).to_string(),
            "%bedrock%" => t.bedrock.to_string(),
            "%player-version%" => t.version_name.clone(),
            "%player-version-id%" => t.version_id.to_string(),
            "%online%" => (visible().count() + ctx.server.proxy_players.iter().filter(|r| !r.vanished).count()).to_string(),
            "%serveronline%" => (visible().count()
                + ctx.server.proxy_players.iter().filter(|r| !r.vanished && r.server.eq_ignore_ascii_case(&ctx.server.level_name)).count())
            .to_string(),
            "%worldonline%" => visible().filter(|o| o.p.world == p.world).count().to_string(),
            "%staffonline%" => (visible().filter(|o| staff(*o)).count()
                + ctx.server.proxy_players.iter().filter(|r| !r.vanished && r.staff).count())
            .to_string(),
            "%nonstaffonline%" => visible().filter(|o| !staff(*o)).count().to_string(),
            "%time%" => crate::horizon_tab::datefmt::format_locale(&self.time_format, &self.now_local(), &self.locale),
            "%date%" => crate::horizon_tab::datefmt::format_locale(&self.date_format, &self.now_local(), &self.locale),
            "%memory-used%" => ctx.server.memory_used_mb.to_string(),
            "%memory-max%" => ctx.server.memory_max_mb.to_string(),
            "%memory-used-gb%" => builtin::two_decimals(ctx.server.memory_used_mb as f64 / 1024.0),
            "%memory-max-gb%" => builtin::two_decimals(ctx.server.memory_max_mb as f64 / 1024.0),
            "%tps%" => builtin::two_decimals(ctx.server.tps.min(20.0)),
            "%mspt%" => builtin::two_decimals(ctx.server.mspt),
            "%luckperms-prefix%" | "%luckperms_prefix%" | "%vault-prefix%" => integration::first(|i| i.prefix(p.uuid)).unwrap_or_default(),
            "%luckperms-suffix%" | "%luckperms_suffix%" | "%vault-suffix%" => integration::first(|i| i.suffix(p.uuid)).unwrap_or_default(),
            "%luckperms-prefixes%" => integration::first(|i| i.prefixes(p.uuid)).unwrap_or_default(),
            "%luckperms-suffixes%" => integration::first(|i| i.suffixes(p.uuid)).unwrap_or_default(),
            "%luckperms-weight%" => integration::first(|i| i.weight(p.uuid))
                .map(|w| w.to_string())
                .unwrap_or_default(),
            _ => {
                if let Some(server) = inner.strip_prefix("online_") {
                    // This server's own name counts the players here; the others come from the
                    // players other TAB instances announced (`proxy-support`).
                    let here = if server.eq_ignore_ascii_case(&ctx.server.level_name) { visible().count() } else { 0 };
                    let remote = ctx
                        .server
                        .proxy_players
                        .iter()
                        .filter(|r| !r.vanished && r.server.eq_ignore_ascii_case(server))
                        .count();
                    return Some((here + remote).to_string());
                }
                if let Some(group) = inner.strip_prefix("playerlist-group_") {
                    // `GlobalPlayerList`'s group counters: only with the feature on.
                    let g = ctx.server.global.as_ref()?;
                    let local = visible().map(|_| (ctx.server.level_name.clone(), false));
                    let remote = ctx.server.proxy_players.iter().map(|r| (r.server.clone(), r.vanished));
                    return Some(g.group_count(group, local.chain(remote)).to_string());
                }
                return placeholders::resolve(Some(p), ctx.server, inner);
            }
        })
    }

    /// Evaluates a condition for the context's target (and viewer for relational content).
    pub fn is_met(&self, c: &Condition, ctx: &Ctx, depth: usize) -> bool {
        let mut side = |s: &str| text::colorize(&self.parse_depth(s, ctx, depth + 1));
        let target = ctx.target;
        let mut perm = |node: &str| integration::permission(target.p.uuid, node, target.p.op_level);
        c.is_met(&mut side, &mut perm)
    }

    /// A short-form condition (display/disable conditions), compiled once per text.
    pub fn short_met(&self, text: &str, ctx: &Ctx) -> bool {
        thread_local! {
            static COMPILED: RefCell<HashMap<String, Condition>> = RefCell::new(HashMap::new());
        }
        let c = COMPILED.with(|m| {
            m.borrow_mut()
                .entry(text.to_owned())
                .or_insert_with(|| Condition::short(text))
                .clone()
        });
        self.is_met(&c, ctx, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(name: &str, uuid: u128) -> TabPlayer {
        TabPlayer {
            p: Player {
                uuid,
                name: name.into(),
                world: "world".into(),
                ping: 33,
                health: 17.2,
                ..Player::default()
            },
            group: "default".into(),
            version_id: 775,
            ..TabPlayer::default()
        }
    }

    fn ph() -> TabPlaceholders {
        let cfg = Config::from_yaml(&crate::horizon_tab::yaml::parse(include_str!("../../data/tab/testdata/config.yml")).unwrap());
        let anim = load_animations(&crate::horizon_tab::yaml::parse(include_str!("../../data/tab/testdata/animations.yml")).unwrap());
        TabPlaceholders::new(&cfg, anim)
    }

    #[test]
    fn builtins_and_animation() {
        let mut ph = ph();
        let all = vec![player("Steve", 1), player("Alex", 2)];
        let server = Server {
            level_name: "world".into(),
            ..Server::default()
        };
        let ctx = Ctx {
            target: &all[0],
            viewer: &all[0],
            all: &all,
            server: &server,
        };
        assert_eq!(
            ph.parse("%player% %ping% %online% %worldonline% %health%", &ctx),
            "Steve 33 2 2 18"
        );
        ph.now_ms = 0;
        assert_eq!(ph.parse("%animation:barcolors%", &ctx), "RED");
        ph.now_ms = 1000;
        assert_eq!(ph.parse("%animation:barcolors%", &ctx), "BLUE");
        // Unknown stays; %% is %.
        assert_eq!(ph.parse("%nope_x% 100%%", &ctx), "%nope_x% 100%");
        assert_eq!(ph.parse("%player_name%", &ctx), "Steve");
    }

    #[test]
    fn condition_and_replacement() {
        let ph = ph();
        let all = vec![player("Steve", 1)];
        let server = Server::default();
        let ctx = Ctx {
            target: &all[0],
            viewer: &all[0],
            all: &all,
            server: &server,
        };
        // %essentials_nickname% is unknown here, so the condition is false.
        assert_eq!(ph.parse("%condition:nick%", &ctx), "~%essentials_nickname%");
        assert!(ph.short_met("%world%=world", &ctx));
        assert!(!ph.short_met("%world%=disabledworld", &ctx));
        assert!(ph.short_met("%player-version-id%>=765;%bedrock%=false", &ctx));
    }

    #[test]
    fn deaths_invisible_collision_placeholders() {
        let ph = ph();
        let mut t = player("Steve", 1);
        t.p.deaths = 7;
        t.p.invisible = true;
        t.collision = true;
        let all = vec![t.clone()];
        let server = Server::default();
        let ctx = Ctx { target: &all[0], viewer: &all[0], all: &all, server: &server };
        assert_eq!(ph.parse("%deaths% %invisible% %collision%", &ctx), "7 true true");
    }

    #[test]
    fn cache_respects_interval() {
        let mut ph = ph();
        let mut all = vec![player("Steve", 1)];
        let server = Server::default();
        {
            let ctx = Ctx {
                target: &all[0],
                viewer: &all[0],
                all: &all,
                server: &server,
            };
            assert_eq!(ph.parse("%ping%", &ctx), "33");
        }
        all[0].p.ping = 99;
        ph.now_ms = 100;
        {
            let ctx = Ctx {
                target: &all[0],
                viewer: &all[0],
                all: &all,
                server: &server,
            };
            assert_eq!(ph.parse("%ping%", &ctx), "33", "within 500 ms");
        }
        ph.now_ms = 600;
        let ctx = Ctx {
            target: &all[0],
            viewer: &all[0],
            all: &all,
            server: &server,
        };
        assert_eq!(ph.parse("%ping%", &ctx), "99");
    }

    #[test]
    fn output_replacement_rules() {
        let mut cfg = Config::default();
        cfg.replacements = vec![(
            "%ping%".into(),
            vec![("0-50".into(), "&agood".into()), ("else".into(), "&c%value%".into())],
        )];
        let ph = TabPlaceholders::new(&cfg, HashMap::new());
        let mut all = vec![player("Steve", 1)];
        let server = Server::default();
        {
            let ctx = Ctx {
                target: &all[0],
                viewer: &all[0],
                all: &all,
                server: &server,
            };
            assert_eq!(ph.parse("%ping%", &ctx), "&agood");
        }
        ph.forget(1);
        all[0].p.ping = 120;
        let ctx = Ctx {
            target: &all[0],
            viewer: &all[0],
            all: &all,
            server: &server,
        };
        assert_eq!(ph.parse("%ping%", &ctx), "&c120");
    }
}
