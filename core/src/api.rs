//! The mod's entry points for the game's helpers (`helpers/*.rs`), and its small API for other
//! mods. Everything runs on the server thread; the state sits behind one mutex.
//!
//! For other mods (as `horizon_client::tab::...`): [`register_integration`] (permissions, groups,
//! prefixes, vanish, Bedrock), [`parse`] (TAB placeholders for an online player) and the TAB
//! expansion's values (`%tab_tabprefix%`... when `placeholders.register-tab-expansion` is on).

use crate::commands::{self, Output, Seg, Sender};
use crate::engine::{Engine, PlayerIn};
use crate::out::Packet;
use crate::placeholders::{self, Player, Server};
use crate::properties::VALID_PROPERTIES;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};

pub use crate::integration::{
    host_permission_answer, host_permission_requests, register as register_integration,
    unregister as unregister_integration, Integration,
};

/// The permission nodes a `/tab` command can ask about, so the glue can have the game's answers in
/// before the command runs (a command cannot wait for the next tick).
pub fn command_permission_nodes() -> Vec<String> {
    let mut nodes: Vec<String> = [
        "tab.admin",
        "tab.tabcomplete",
        "tab.announce.bar",
        "tab.announce.scoreboard",
        "tab.bossbar.toggle",
        "tab.bossbar.toggle.other",
        "tab.cpu",
        "tab.debug",
        "tab.dump",
        "tab.groupinfo",
        "tab.grouplist",
        "tab.mysql.download",
        "tab.mysql.upload",
        "tab.nametag.view",
        "tab.nametag.view.other",
        "tab.nametag.visibility",
        "tab.nametag.visibility.other",
        "tab.parse",
        "tab.reload",
        "tab.remove",
        "tab.scoreboard.show",
        "tab.scoreboard.show.other",
        "tab.scoreboard.toggle",
        "tab.scoreboard.toggle.other",
        "tab.setcollision",
    ]
    .iter()
    .map(|n| (*n).to_owned())
    .collect();
    nodes.extend(VALID_PROPERTIES.iter().map(|p| format!("tab.change.{p}")));
    nodes
}

/// A packet the game is about to send `viewer` was seen on its way out (the glue hands the parts
/// of a clientbound packet it understands here). Each function answers what TAB wants instead, or
/// `None`/`false` to let the packet pass.
pub fn guard_team(viewer: u128, team: &str, players: &[String]) -> bool {
    let guard = state().lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().is_some_and(|s| s.engine.guard_team(viewer, team, players))
}

/// The latency TAB's ping spoof shows, if it is on.
pub fn guard_latency() -> Option<i32> {
    let guard = state().lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().and_then(|s| s.engine.guard_latency())
}

/// The game mode to show `viewer` for `uuid` instead of `mode` (spectators as survival).
pub fn guard_game_mode(viewer: u128, uuid: u128, mode: i32) -> Option<i32> {
    let guard = state().lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().and_then(|s| s.engine.guard_game_mode(viewer, uuid, mode))
}

/// The display name `viewer` was last sent for `uuid`, when another sender's packet carries a
/// different one.
pub fn guard_display_name(viewer: u128, uuid: u128) -> Option<Value> {
    let guard = state().lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().and_then(|s| s.engine.guard_display_name(viewer, uuid))
}

/// `viewer` respawned: forget what their client holds from us, so the next tick sends it again.
pub fn player_respawned(viewer: u128) {
    if let Some(s) = state().lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        s.engine.forget_viewer(viewer);
    }
}

struct State {
    engine: Engine,
    started_ms: i64,
    /// Packets a command produced, sent with the next tick (a command runs inside a player's hook,
    /// where the other players may be out of reach).
    pending: Vec<Packet>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Whether [`start`] ran.
pub fn started() -> bool {
    state().lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// When [`start`] ran (ms since the epoch), for `%server_uptime%`.
pub fn started_ms() -> i64 {
    state()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.started_ms)
        .unwrap_or_else(now_ms)
}

/// This process's resident memory and the machine's total memory, in MB (Linux `/proc`; zeros
/// elsewhere). A JVM's `-Xmx` has no equivalent here, so `%memory-max%` is the machine's memory
/// unless `HORIZON_TAB_MEMORY_MAX_MB` is set.
pub fn process_memory_mb() -> (u64, u64) {
    let field = |text: &str, key: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|kb| kb.parse::<u64>().ok())
            .map(|kb| kb / 1024)
    };
    let used = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|t| field(&t, "VmRSS:"))
        .unwrap_or(0);
    let max = std::env::var("HORIZON_TAB_MEMORY_MAX_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .or_else(|| {
            std::fs::read_to_string("/proc/meminfo")
                .ok()
                .and_then(|t| field(&t, "MemTotal:"))
        })
        .unwrap_or(0);
    (used, max)
}

fn state() -> &'static Mutex<Option<State>> {
    static S: OnceLock<Mutex<Option<State>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

/// The display names the game's own player list packets should carry (read by the hub).
fn list_names() -> &'static RwLock<HashMap<u128, Value>> {
    static N: OnceLock<RwLock<HashMap<u128, Value>>> = OnceLock::new();
    N.get_or_init(|| RwLock::new(HashMap::new()))
}

fn tab_values() -> &'static RwLock<HashMap<u128, HashMap<String, String>>> {
    static V: OnceLock<RwLock<HashMap<u128, HashMap<String, String>>>> = OnceLock::new();
    V.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The folder TAB's files live in: `<config>/tab/` (made, and filled with defaults, when missing).
pub fn files_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("tab")
}

const DEFAULT_CONFIG: &str = include_str!("../defaults/config.yml");
const DEFAULT_GROUPS: &str = include_str!("../defaults/groups.yml");
const DEFAULT_ANIMATIONS: &str = include_str!("../defaults/animations.yml");
const DEFAULT_MESSAGES: &str = include_str!("../defaults/messages.yml");
const DEFAULT_USERS: &str = include_str!("../defaults/users.yml");

/// Starts the mod: writes default files where missing, loads them. Log lines to print.
pub fn start(config_dir: &Path) -> Vec<String> {
    let dir = files_dir(config_dir);
    let mut log = Vec::new();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log.push(format!("cannot make {}: {e}", dir.display()));
    }
    for (name, text) in [
        ("config.yml", DEFAULT_CONFIG),
        ("groups.yml", DEFAULT_GROUPS),
        ("animations.yml", DEFAULT_ANIMATIONS),
        ("messages.yml", DEFAULT_MESSAGES),
        ("users.yml", DEFAULT_USERS),
        ("playerdata.yml", ""),
    ] {
        let path = dir.join(name);
        if !path.exists() {
            if let Err(e) = std::fs::write(&path, text) {
                log.push(format!("cannot write {}: {e}", path.display()));
            }
        }
    }
    let engine = Engine::load(&dir);
    log.extend(engine.warnings.iter().cloned());
    if engine.cfg.register_tab_expansion {
        register_tab_expansion();
    }
    log.push(format!("loaded from {}", dir.display()));
    *state().lock().unwrap_or_else(|e| e.into_inner()) = Some(State {
        engine,
        started_ms: now_ms(),
        pending: Vec::new(),
    });
    log
}

/// One server tick: the packets to send.
pub fn tick(now_ms: i64, server: Server, players: Vec<PlayerIn>) -> Vec<Packet> {
    let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = guard.as_mut() else { return Vec::new() };
    let mut packets = std::mem::take(&mut s.pending);
    packets.extend(s.engine.tick(now_ms, server, players));
    *list_names().write().unwrap_or_else(|e| e.into_inner()) = s.engine.list_names();
    crate::integration::host_permissions_retain(&s.engine.all_players().iter().map(|p| p.p.uuid).collect());
    if s.engine.cfg.register_tab_expansion {
        let mut values = HashMap::new();
        for p in s.engine.all_players() {
            let Some(i) = s.engine.index_of(p.p.uuid) else { continue };
            let mut m = HashMap::new();
            for prop in VALID_PROPERTIES {
                let (raw, _) = s.engine.property_of(i, prop);
                m.insert(format!("{prop}_raw"), raw.clone());
                m.insert(prop.to_owned(), s.engine.parse_for(i, &raw));
            }
            m.insert(
                "scoreboard_visible".into(),
                if s.engine.scoreboard_shown(i) {
                    "Enabled"
                } else {
                    "Disabled"
                }
                .into(),
            );
            m.insert(
                "bossbar_visible".into(),
                if s.engine.bossbar_shown(i) {
                    "Enabled"
                } else {
                    "Disabled"
                }
                .into(),
            );
            m.insert("group".into(), p.group.clone());
            values.insert(p.p.uuid, m);
        }
        *tab_values().write().unwrap_or_else(|e| e.into_inner()) = values;
    }
    packets
}

/// The display name for the game's own player info entry of `uuid`, when TAB formats it.
pub fn list_name(uuid: u128) -> Option<Value> {
    list_names()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&uuid)
        .cloned()
}

/// Another sender (a nickname change) replaced `uuid`'s display name on every client.
pub fn display_name_overwritten(uuid: u128) {
    if let Some(s) = state().lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        s.engine.display_name_overwritten(uuid);
    }
}

/// A literal word the client can complete: lowercase letters, digits, `_` and `-` (the usage parser's rule).
fn literal_word(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || "_-".contains(ch))
}

/// The commands to register: (usage, operator level). Permission nodes are checked when run.
/// One usage per sub-command path with typed arguments, so the client completes the words and the
/// fixed choices itself. The `text` catch-all comes last: free text and unknown words still reach
/// `/tab`'s own help and messages. Every usage dispatches to the same `commands::tab` code.
pub fn commands() -> Vec<(String, u8)> {
    let guard = state().lock().unwrap_or_else(|e| e.into_inner());
    let mut u: Vec<String> = [
        "tab reload",
        "tab groups",
        "tab debug",
        "tab cpu",
        "tab dump",
        "tab dump <player:player>",
        "tab parse me <placeholder:text>",
        "tab parse <player:player> <placeholder:text>",
        "tab mysql",
        "tab mysql upload",
        "tab mysql download",
        "tab setcollision <player:player> true",
        "tab setcollision <player:player> false",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    // `player`/`playeruuid` take an offline name too (`player_name`); `group` takes a group name.
    for (root, kind) in [("player", "player_name"), ("playeruuid", "player_name"), ("group", "word")] {
        u.push(format!("tab {root} <name:{kind}>"));
        u.push(format!("tab {root} <name:{kind}> remove"));
        for p in crate::commands::COMMAND_PROPERTIES {
            u.push(format!("tab {root} <name:{kind}> {p} [value:text]"));
        }
    }
    let mut boards: Vec<String> = Vec::new();
    let mut bars: Vec<String> = Vec::new();
    let mut announce_bars: Vec<String> = Vec::new();
    if let Some(s) = guard.as_ref() {
        let c = &s.engine.cfg;
        boards = c.scoreboard.boards.iter().map(|b| b.name.clone()).filter(|n| literal_word(n)).collect();
        bars = c.bossbar.bars.iter().map(|b| b.name.clone()).filter(|n| literal_word(n)).collect();
        announce_bars = c
            .bossbar
            .bars
            .iter()
            .filter(|b| b.announcement)
            .map(|b| b.name.clone())
            .filter(|n| literal_word(n))
            .collect();
        // The toggle commands (`/sb`, `/bossbar` by default) forward to `/tab <feature> toggle`.
        for (enabled, cmd) in [
            (c.scoreboard.enabled, &c.scoreboard.toggle_command),
            (c.bossbar.enabled, &c.bossbar.toggle_command),
        ] {
            let word = cmd.trim().trim_start_matches('/').to_ascii_lowercase();
            if enabled && word != "tab" && literal_word(&word) {
                u.push(word.clone());
                u.push(format!("{word} -s"));
                u.push(format!("{word} <player:player>"));
                u.push(format!("{word} <player:player> -s"));
            }
        }
    }
    // `[on|off|toggle] [player] [-s]`, the same shape for both features.
    for root in ["scoreboard", "bossbar"] {
        for action in ["on", "off", "toggle"] {
            u.push(format!("tab {root} {action}"));
            u.push(format!("tab {root} {action} -s"));
            u.push(format!("tab {root} {action} <player:player>"));
            u.push(format!("tab {root} {action} <player:player> -s"));
        }
    }
    // `scoreboard show <name> [player]`, `scoreboard announce <name> <length>`.
    for name in &boards {
        u.push(format!("tab scoreboard show {name}"));
        u.push(format!("tab scoreboard show {name} <player:player>"));
        u.push(format!("tab scoreboard announce {name} <length:integer>"));
    }
    u.push("tab scoreboard show <name:word>".to_owned());
    u.push("tab scoreboard show <name:word> <player:player>".to_owned());
    u.push("tab scoreboard announce <name:word> <length:integer>".to_owned());
    // `bossbar announce <bar> <length>`, `bossbar send <player> <bar> <length>`.
    for name in &announce_bars {
        u.push(format!("tab bossbar announce {name} <length:integer>"));
    }
    u.push("tab bossbar announce <name:word> <length:integer>".to_owned());
    for name in &bars {
        u.push(format!("tab bossbar send <player:player> {name} <length:integer>"));
    }
    u.push("tab bossbar send <player:player> <name:word> <length:integer>".to_owned());
    // Nametags: `<show|opaque|hide|toggle> [player] [viewer] [-s]` and the `view` variants.
    for action in ["show", "opaque", "hide", "toggle"] {
        u.push(format!("tab nametag {action}"));
        u.push(format!("tab nametag {action} -s"));
        u.push(format!("tab nametag {action} <player:player>"));
        u.push(format!("tab nametag {action} <player:player> -s"));
        u.push(format!("tab nametag {action} <player:player> <viewer:player>"));
        u.push(format!("tab nametag {action} <player:player> <viewer:player> -s"));
    }
    for action in ["showview", "opaqueview", "hideview", "toggleview"] {
        u.push(format!("tab nametag {action}"));
        u.push(format!("tab nametag {action} -s"));
        u.push(format!("tab nametag {action} <viewer:player>"));
        u.push(format!("tab nametag {action} <viewer:player> -s"));
    }
    let mut out: Vec<(String, u8)> = u.into_iter().map(|usage| (usage, 0)).collect();
    // Anything else (`/tab`, `/tab help`, unknown words) still reaches `/tab`'s own answer.
    out.push(("tab [arguments:text]".to_owned(), 0));
    out
}

/// What a command answers: lines for the sender (components) and packets.
pub struct Answer {
    pub ran: bool,
    pub lines: Vec<Value>,
    pub plain: Vec<String>,
    /// Always empty: a command's packets go out with the next tick.
    pub packets: Vec<Packet>,
}

fn answer(o: Output, minimessage: bool, pending: &mut Vec<Packet>) -> Answer {
    pending.extend(o.packets);
    let mut lines = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    for (i, line) in o.replies.iter().enumerate() {
        match o.rich.iter().find(|(at, _)| *at == i) {
            Some((_, segs)) => {
                let parts: Vec<Value> = segs
                    .iter()
                    .map(|seg| match seg {
                        Seg::Fmt(t) => crate::text::component(t, minimessage),
                        Seg::Lit(t, color) => serde_json::json!({ "text": t, "color": color }),
                    })
                    .collect();
                lines.push(serde_json::json!({ "text": "", "extra": parts }));
                plain.push(
                    segs.iter()
                        .map(|seg| match seg {
                            Seg::Fmt(t) => crate::text::plain(t, minimessage),
                            Seg::Lit(t, _) => t.clone(),
                        })
                        .collect(),
                );
            }
            None => {
                lines.push(crate::text::component(line, minimessage));
                plain.push(crate::text::plain(line, minimessage));
            }
        }
    }
    Answer {
        ran: true,
        lines,
        plain,
        packets: Vec::new(),
    }
}

/// A command line (no slash) from a player (`Some((uuid, op level))`) or the console (None).
pub fn command(sender: Option<(u128, u8)>, line: &str) -> Answer {
    let none = Answer {
        ran: false,
        lines: Vec::new(),
        plain: Vec::new(),
        packets: Vec::new(),
    };
    let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = guard.as_mut() else { return none };
    let sender = match sender {
        Some((uuid, op_level)) => Sender::Player { uuid, op_level },
        None => Sender::Console,
    };
    let (word, rest) = match line.trim().split_once(' ') {
        Some((w, r)) => (w.to_ascii_lowercase(), r),
        None => (line.trim().to_ascii_lowercase(), ""),
    };
    let mm = s.engine.cfg.minimessage;
    let cfg = &s.engine.cfg;
    let sb = cfg
        .scoreboard
        .toggle_command
        .trim()
        .trim_start_matches('/')
        .to_ascii_lowercase();
    let bb = cfg
        .bossbar
        .toggle_command
        .trim()
        .trim_start_matches('/')
        .to_ascii_lowercase();
    let (sb_on, bb_on) = (cfg.scoreboard.enabled, cfg.bossbar.enabled);
    if word == "tab" {
        let o = commands::tab(&mut s.engine, sender, rest);
        // A reload may have switched the expansion on.
        if s.engine.cfg.register_tab_expansion {
            register_tab_expansion();
        }
        return answer(o, mm, &mut s.pending);
    }
    if sb_on && word == sb {
        let o = commands::scoreboard_toggle(&mut s.engine, sender, rest);
        return answer(o, mm, &mut s.pending);
    }
    if bb_on && word == bb {
        let o = commands::bossbar_toggle(&mut s.engine, sender, rest);
        return answer(o, mm, &mut s.pending);
    }
    none
}

/// TAB placeholders for an online player (`%player%`, `%animation:x%`, registry ones...).
pub fn parse(uuid: u128, text: &str) -> Option<String> {
    let guard = state().lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.as_ref()?;
    let i = s.engine.index_of(uuid)?;
    Some(s.engine.parse_for(i, text))
}

/// At stop: what was shown goes away (players are still connected).
pub fn stop() -> Vec<Packet> {
    let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = guard.as_mut() else { return Vec::new() };
    s.engine.unload()
}

struct TabExpansion;

impl placeholders::Expansion for TabExpansion {
    fn identifier(&self) -> &str {
        "tab"
    }
    fn author(&self) -> &str {
        "horizon-tab"
    }
    fn request(&self, player: Option<&Player>, params: &str, _server: &Server) -> Option<String> {
        let p = player?;
        if let Some(inner) = params.strip_prefix("replace_") {
            // %tab_replace_<placeholder>%: TAB's value of %<placeholder>% for them.
            return parse_unlocked(p.uuid, &format!("%{inner}%"));
        }
        if let Some(inner) = params.strip_prefix("placeholder_") {
            return parse_unlocked(p.uuid, &format!("%{inner}%"));
        }
        tab_values()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&p.uuid)?
            .get(params)
            .cloned()
    }
}

/// `parse`, but safe to call while the state is locked (an expansion runs inside a tick): then
/// it answers None.
fn parse_unlocked(uuid: u128, text: &str) -> Option<String> {
    let guard = state().try_lock().ok()?;
    let s = guard.as_ref()?;
    let i = s.engine.index_of(uuid)?;
    Some(s.engine.parse_for(i, text))
}

fn register_tab_expansion() {
    if !placeholders::is_registered("tab") {
        placeholders::register(Box::new(TabExpansion));
    }
}
