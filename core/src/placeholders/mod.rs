//! A small placeholder registry for Horizon server mods, in the spirit of PlaceholderAPI: a mod
//! registers an *expansion* under an identifier, and `%identifier_params%` in any text is replaced
//! by what the expansion answers for a player. Relational placeholders (`%rel_identifier_params%`)
//! get a viewer and a target. `{identifier_params}` is the bracket form.
//!
//! Clean room: PlaceholderAPI is GPL-3.0 and its source was not read. This follows the placeholder
//! syntax its users know (`%player_name%`, `%server_online%`, `%rel_...%`) and nothing else.
//!
//! The registry is process-wide (all native mods are linked into one server binary). Other mods
//! reach it as `horizon_client::placeholders::...` once this bundle is installed:
//!
//! ```ignore
//! use horizon_client::placeholders::{register_fn, Player};
//! register_fn("mymod", |player: Option<&Player>, params: &str| match params {
//!     "coins" => player.map(|p| coins_of(p.uuid).to_string()),
//!     _ => None,
//! });
//! ```
//!
//! The game data a placeholder may need (players, TPS, memory) is a snapshot the TAB mod's tick
//! refreshes ([`update_snapshot`]); `set_placeholders_for(uuid, text)` uses it.

pub mod builtin;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// A player as placeholders see them: a copy taken at the last snapshot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player {
    pub uuid: u128,
    pub name: String,
    /// The shown name (a nickname), when a mod set one.
    pub display_name: Option<String>,
    /// `minecraft:overworld`...
    pub dimension: String,
    /// The world's folder name as Bukkit names it (`world`, `world_nether`, `world_the_end`).
    pub world: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub ping: i32,
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    /// Operator level 0..4.
    pub op_level: u8,
    pub vanished: bool,
    pub entity_id: i32,
    /// Milliseconds since the epoch when they joined this server.
    pub joined_at_ms: i64,
}

impl Player {
    pub fn uuid_string(&self) -> String {
        uuid_string(self.uuid)
    }
}

/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
pub fn uuid_string(u: u128) -> String {
    let h = format!("{u:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// The server as placeholders see it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Server {
    pub now_ms: i64,
    pub started_ms: i64,
    pub tps: f64,
    pub mspt: f64,
    pub max_players: i32,
    pub online: usize,
    pub memory_used_mb: u64,
    pub memory_max_mb: u64,
    pub version: String,
    /// The world folder (`level-name`).
    pub level_name: String,
}

/// What a mod registers.
pub trait Expansion: Send + Sync {
    /// The part before the first `_`, matched without case: `player` in `%player_name%`.
    fn identifier(&self) -> &str;
    fn author(&self) -> &str {
        ""
    }
    fn version(&self) -> &str {
        ""
    }
    /// The value of `%identifier_params%` for `player` (None: no player, such as the console).
    /// None leaves the placeholder as it was written.
    fn request(&self, player: Option<&Player>, params: &str, server: &Server) -> Option<String>;
    /// `%rel_identifier_params%`: the text `viewer` sees for `target`. None by default.
    fn relational(&self, _viewer: &Player, _target: &Player, _params: &str, _server: &Server) -> Option<String> {
        None
    }
}

type Handler = dyn Fn(Option<&Player>, &str) -> Option<String> + Send + Sync;

struct FnExpansion {
    identifier: String,
    handler: Box<Handler>,
}

impl Expansion for FnExpansion {
    fn identifier(&self) -> &str {
        &self.identifier
    }
    fn request(&self, player: Option<&Player>, params: &str, _server: &Server) -> Option<String> {
        (self.handler)(player, params)
    }
}

#[derive(Default)]
struct Registry {
    expansions: HashMap<String, Arc<dyn Expansion>>,
}

fn registry() -> &'static RwLock<Registry> {
    static R: OnceLock<RwLock<Registry>> = OnceLock::new();
    R.get_or_init(|| {
        let mut r = Registry::default();
        for e in builtin::expansions() {
            r.expansions.insert(e.identifier().to_ascii_lowercase(), e);
        }
        RwLock::new(r)
    })
}

#[derive(Default)]
struct Snapshot {
    server: Server,
    players: Vec<Player>,
}

fn snapshot() -> &'static RwLock<Snapshot> {
    static S: OnceLock<RwLock<Snapshot>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(Snapshot::default()))
}

/// Registers an expansion; false (and nothing changes) when its identifier is taken.
pub fn register(expansion: Box<dyn Expansion>) -> bool {
    let id = expansion.identifier().to_ascii_lowercase();
    // The identifier ends at the first `_`; `rel` starts the relational form.
    if id.is_empty() || id == "rel" || id.contains(['%', '_', ' ']) {
        return false;
    }
    let mut r = registry().write().unwrap_or_else(|e| e.into_inner());
    if r.expansions.contains_key(&id) {
        return false;
    }
    r.expansions.insert(id, Arc::from(expansion));
    true
}

/// Registers a closure as the expansion `identifier`.
pub fn register_fn<F>(identifier: &str, handler: F) -> bool
where
    F: Fn(Option<&Player>, &str) -> Option<String> + Send + Sync + 'static,
{
    register(Box::new(FnExpansion {
        identifier: identifier.to_owned(),
        handler: Box::new(handler),
    }))
}

/// Takes an expansion out; whether it was registered.
pub fn unregister(identifier: &str) -> bool {
    let mut r = registry().write().unwrap_or_else(|e| e.into_inner());
    r.expansions.remove(&identifier.to_ascii_lowercase()).is_some()
}

/// The identifiers registered, sorted.
pub fn identifiers() -> Vec<String> {
    let r = registry().read().unwrap_or_else(|e| e.into_inner());
    let mut ids: Vec<String> = r.expansions.keys().cloned().collect();
    ids.sort();
    ids
}

pub fn is_registered(identifier: &str) -> bool {
    let r = registry().read().unwrap_or_else(|e| e.into_inner());
    r.expansions.contains_key(&identifier.to_ascii_lowercase())
}

fn expansion(identifier: &str) -> Option<Arc<dyn Expansion>> {
    let r = registry().read().unwrap_or_else(|e| e.into_inner());
    r.expansions.get(&identifier.to_ascii_lowercase()).cloned()
}

/// Replaces the game snapshot placeholders read (the TAB mod calls this every tick).
pub fn update_snapshot(server: Server, players: Vec<Player>) {
    let mut s = snapshot().write().unwrap_or_else(|e| e.into_inner());
    s.server = server;
    s.players = players;
}

/// The last snapshot's server data.
pub fn server() -> Server {
    snapshot().read().unwrap_or_else(|e| e.into_inner()).server.clone()
}

/// The last snapshot's player with this UUID.
pub fn player(uuid: u128) -> Option<Player> {
    let s = snapshot().read().unwrap_or_else(|e| e.into_inner());
    s.players.iter().find(|p| p.uuid == uuid).cloned()
}

/// The last snapshot's players.
pub fn players() -> Vec<Player> {
    snapshot().read().unwrap_or_else(|e| e.into_inner()).players.clone()
}

/// The value of one placeholder, written without its `%` (`player_name`); None when no expansion
/// answers.
pub fn resolve(player: Option<&Player>, server: &Server, inner: &str) -> Option<String> {
    let (id, params) = match inner.split_once('_') {
        Some((id, params)) => (id, params),
        None => (inner, ""),
    };
    expansion(id)?.request(player, params, server)
}

/// The value of one relational placeholder, written without `%rel_` and `%`.
pub fn resolve_relational(viewer: &Player, target: &Player, server: &Server, inner: &str) -> Option<String> {
    let (id, params) = match inner.split_once('_') {
        Some((id, params)) => (id, params),
        None => (inner, ""),
    };
    expansion(id)?.relational(viewer, target, params, server)
}

/// Splits `text` at `open...close` markers, calling `f` on the inside of each (without markers);
/// an inside `f` declines stays as written.
fn replace_delimited(text: &str, open: char, close: char, mut f: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len_utf8()..];
        match after.find(close) {
            Some(end) => {
                let inner = &after[..end];
                let ok = !inner.is_empty() && !inner.contains(char::is_whitespace) && !inner.contains(open);
                match ok.then(|| f(inner)).flatten() {
                    Some(v) => {
                        out.push_str(&v);
                        rest = &after[end + close.len_utf8()..];
                    }
                    None => {
                        out.push(open);
                        rest = after;
                    }
                }
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Replaces every `%identifier_params%` an expansion answers.
pub fn set_placeholders(player: Option<&Player>, server: &Server, text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    replace_delimited(text, '%', '%', |inner| {
        if inner.starts_with("rel_") {
            return None;
        }
        resolve(player, server, inner)
    })
}

/// The same with `{identifier_params}`.
pub fn set_bracket_placeholders(player: Option<&Player>, server: &Server, text: &str) -> String {
    if !text.contains('{') {
        return text.to_owned();
    }
    replace_delimited(text, '{', '}', |inner| resolve(player, server, inner))
}

/// Replaces every `%rel_identifier_params%` for `viewer` looking at `target`.
pub fn set_relational_placeholders(viewer: &Player, target: &Player, server: &Server, text: &str) -> String {
    if !text.contains("%rel_") {
        return text.to_owned();
    }
    replace_delimited(text, '%', '%', |inner| {
        let inner = inner.strip_prefix("rel_")?;
        resolve_relational(viewer, target, server, inner)
    })
}

/// `set_placeholders` for an online player by UUID, from the last snapshot (no player: as the
/// console).
pub fn set_placeholders_for(uuid: Option<u128>, text: &str) -> String {
    let server = server();
    let p = uuid.and_then(player);
    set_placeholders(p.as_ref(), &server, text)
}

/// Whether `text` holds anything that looks like a placeholder.
pub fn contains_placeholders(text: &str) -> bool {
    let mut found = false;
    replace_delimited(text, '%', '%', |_| {
        found = true;
        None
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steve() -> Player {
        Player {
            uuid: 0x0123456789abcdef0123456789abcdef,
            name: "Steve".into(),
            ping: 42,
            health: 19.5,
            world: "world".into(),
            ..Player::default()
        }
    }

    #[test]
    fn registry_and_parse() {
        assert!(register_fn("testexp", |p, params| match params {
            "hello" => Some(format!("hi {}", p.map(|p| p.name.as_str()).unwrap_or("console"))),
            _ => None,
        }));
        assert!(!register_fn("TESTEXP", |_, _| None), "taken, case-insensitive");
        let s = Server::default();
        let p = steve();
        assert_eq!(
            set_placeholders(Some(&p), &s, "%testexp_hello% / %testexp_nope% / 100%"),
            "hi Steve / %testexp_nope% / 100%"
        );
        assert_eq!(set_placeholders(None, &s, "%testexp_hello%"), "hi console");
        assert_eq!(set_bracket_placeholders(Some(&p), &s, "{testexp_hello}!"), "hi Steve!");
        assert!(unregister("testexp"));
        assert_eq!(set_placeholders(Some(&p), &s, "%testexp_hello%"), "%testexp_hello%");
    }

    #[test]
    fn percent_runs() {
        let s = Server::default();
        let p = steve();
        assert_eq!(set_placeholders(Some(&p), &s, "50% off %player_name%"), "50% off Steve");
        assert_eq!(set_placeholders(Some(&p), &s, "%%player_name%"), "%Steve");
    }

    struct Rel;
    impl Expansion for Rel {
        fn identifier(&self) -> &str {
            "reltest"
        }
        fn request(&self, _: Option<&Player>, _: &str, _: &Server) -> Option<String> {
            None
        }
        fn relational(&self, v: &Player, t: &Player, params: &str, _: &Server) -> Option<String> {
            (params == "same").then(|| (v.uuid == t.uuid).to_string())
        }
    }

    #[test]
    fn relational() {
        assert!(register(Box::new(Rel)));
        let s = Server::default();
        let a = steve();
        let mut b = steve();
        b.uuid = 7;
        assert_eq!(set_relational_placeholders(&a, &b, &s, "%rel_reltest_same%"), "false");
        assert_eq!(set_relational_placeholders(&a, &a, &s, "%rel_reltest_same%"), "true");
    }

    #[test]
    fn uuid_format() {
        assert_eq!(
            uuid_string(0x0123456789abcdef0123456789abcdef),
            "01234567-89ab-cdef-0123-456789abcdef"
        );
    }
}
