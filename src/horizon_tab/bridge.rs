//! The backend half of TAB's proxy bridge (channel `tab:bridge-6`): the messages TAB on a proxy
//! sends to a backend (`proxy/message/outgoing/*`) and the answers a backend gives
//! (`proxy/message/incoming/*`, chosen by their first byte, `PluginMessageDecodeTask`).
//!
//! With this half running, a TAB on Velocity gets this server's placeholder values, permission
//! answers, world, game mode, vanish and invisibility state, and draws its scoreboard objects,
//! teams and scores for the player through the backend. The transport (a plugin message to the
//! player's connection) is the host's: see ENGINE_NEEDS `plugin_message_channel`.

use crate::horizon_tab::proxy::{write_utf, Reader};
use std::collections::HashMap;

pub const CHANNEL: &str = "tab:bridge-6";

/// What TAB on the proxy sends (read by the backend).
#[derive(Clone, Debug, PartialEq)]
pub enum FromProxy {
    /// `PlayerJoin`: the proxy's placeholders (with refresh intervals) and output replacements.
    PlayerJoin {
        forward_group: bool,
        placeholders: Vec<(String, i32)>,
        replacements: Vec<(String, Vec<(String, String)>)>,
    },
    Permission(String),
    /// `Expansion`: the value of a placeholder the proxy computes (`%placeholder%` -> value).
    Expansion { placeholder: String, value: String },
    Placeholder { identifier: String, refresh: i32 },
    Unload,
    /// `PacketPlayOutScoreboardDisplayObjective`: `DisplaySlot` ordinal 0 list, 1 sidebar, 2 below name.
    DisplayObjective { slot: i32, objective: String },
    /// `PacketPlayOutScoreboardObjective`: action 0 register, 1 unregister, 2 update.
    Objective { name: String, action: i32, title: String, display: i32, number_format: Option<String> },
    /// `PacketPlayOutScoreboardScore`: action 0 change, 1 remove.
    Score { objective: String, action: i32, holder: String, score: i32, display_name: Option<String>, number_format: Option<String> },
    /// `PacketPlayOutScoreboardTeam`: action 0 create, 1 remove, 2 update.
    Team {
        name: String,
        action: i32,
        prefix: String,
        suffix: String,
        options: i32,
        visibility: String,
        collision: String,
        color: i32,
        players: Vec<String>,
    },
}

pub fn decode_from_proxy(bytes: &[u8]) -> Result<FromProxy, String> {
    let mut r = Reader::new(bytes);
    let kind = r.utf()?;
    Ok(match kind.as_str() {
        "PlayerJoin" => {
            let _protocol = r.int()?;
            let forward_group = r.bool()?;
            let mut placeholders = Vec::new();
            for _ in 0..r.int()?.max(0) {
                let name = r.utf()?;
                placeholders.push((name, r.int()?));
            }
            let mut replacements = Vec::new();
            for _ in 0..r.int()?.max(0) {
                let token = r.utf()?;
                let mut rules = Vec::new();
                for _ in 0..r.int()?.max(0) {
                    let from = r.utf()?;
                    rules.push((from, r.utf()?));
                }
                replacements.push((token, rules));
            }
            // The last boolean is the deleted unlimited name tags flag; a short message omits it.
            if r.rest() > 0 {
                let _ = r.bool()?;
            }
            FromProxy::PlayerJoin { forward_group, placeholders, replacements }
        }
        "Permission" => FromProxy::Permission(r.utf()?),
        "Expansion" => {
            let placeholder = r.utf()?;
            FromProxy::Expansion { placeholder, value: r.utf()? }
        }
        "Placeholder" => {
            let identifier = r.utf()?;
            FromProxy::Placeholder { identifier, refresh: r.int()? }
        }
        "Unload" => FromProxy::Unload,
        "PacketPlayOutScoreboardDisplayObjective" => {
            let slot = r.int()?;
            FromProxy::DisplayObjective { slot, objective: r.utf()? }
        }
        "PacketPlayOutScoreboardObjective" => {
            let name = r.utf()?;
            let action = r.int()?;
            let (mut title, mut display, mut number_format) = (String::new(), 0, None);
            if action == 0 || action == 2 {
                title = r.utf()?;
                display = r.int()?;
                if r.bool()? {
                    number_format = Some(r.utf()?);
                }
            }
            FromProxy::Objective { name, action, title, display, number_format }
        }
        "PacketPlayOutScoreboardScore" => {
            let objective = r.utf()?;
            let action = r.int()?;
            let holder = r.utf()?;
            let (mut score, mut display_name, mut number_format) = (0, None, None);
            if action == 0 {
                score = r.int()?;
                if r.bool()? {
                    display_name = Some(r.utf()?);
                }
                if r.bool()? {
                    number_format = Some(r.utf()?);
                }
            }
            FromProxy::Score { objective, action, holder, score, display_name, number_format }
        }
        "PacketPlayOutScoreboardTeam" => {
            let name = r.utf()?;
            let action = r.int()?;
            let (mut prefix, mut suffix, mut options, mut visibility, mut collision, mut color) =
                (String::new(), String::new(), 0, String::new(), String::new(), 0);
            let mut players = Vec::new();
            if action == 0 || action == 2 {
                prefix = r.utf()?;
                suffix = r.utf()?;
                options = r.int()?;
                visibility = r.utf()?;
                collision = r.utf()?;
                color = r.int()?;
            }
            if action == 0 {
                for _ in 0..r.int()?.max(0) {
                    players.push(r.utf()?);
                }
            }
            FromProxy::Team { name, action, prefix, suffix, options, visibility, collision, color, players }
        }
        other => return Err(format!("unknown bridge message \"{other}\"")),
    })
}

/// What the backend tells the proxy (the first byte picks the message).
#[derive(Clone, Debug, PartialEq)]
pub enum ToProxy {
    PlaceholderError { message: String, stack: Vec<String> },
    UpdateGameMode(i32),
    HasPermission { permission: String, value: bool },
    Invisible(bool),
    Disguised(bool),
    SetWorld(String),
    SetGroup(String),
    Vanished(bool),
    /// `target` only for `%rel_...%` placeholders.
    UpdatePlaceholder { identifier: String, target: Option<String>, value: String },
    PlayerJoinResponse {
        world: String,
        /// Present when the proxy asked for the group (`forward_group`).
        group: Option<String>,
        values: Vec<(String, PlaceholderValue)>,
        game_mode: i32,
    },
    RegisterPlaceholder(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlaceholderValue {
    Plain(String),
    /// `%rel_...%`: the value for each other player (by name).
    Relational(Vec<(String, String)>),
}

pub fn encode_to_proxy(m: &ToProxy) -> Vec<u8> {
    let mut o = Vec::new();
    match m {
        ToProxy::PlaceholderError { message, stack } => {
            o.push(0);
            write_utf(&mut o, message);
            o.extend_from_slice(&(stack.len() as i32).to_be_bytes());
            for line in stack {
                write_utf(&mut o, line);
            }
        }
        ToProxy::UpdateGameMode(g) => {
            o.push(1);
            o.extend_from_slice(&g.to_be_bytes());
        }
        ToProxy::HasPermission { permission, value } => {
            o.push(2);
            write_utf(&mut o, permission);
            o.push(*value as u8);
        }
        ToProxy::Invisible(b) => o.extend_from_slice(&[3, *b as u8]),
        ToProxy::Disguised(b) => o.extend_from_slice(&[4, *b as u8]),
        ToProxy::SetWorld(w) => {
            o.push(5);
            write_utf(&mut o, w);
        }
        ToProxy::SetGroup(g) => {
            o.push(6);
            write_utf(&mut o, g);
        }
        ToProxy::Vanished(b) => o.extend_from_slice(&[7, *b as u8]),
        ToProxy::UpdatePlaceholder { identifier, target, value } => {
            o.push(8);
            write_utf(&mut o, identifier);
            if identifier.starts_with("%rel_") {
                write_utf(&mut o, target.as_deref().unwrap_or(""));
            }
            write_utf(&mut o, value);
        }
        ToProxy::PlayerJoinResponse { world, group, values, game_mode } => {
            o.push(9);
            write_utf(&mut o, world);
            if let Some(g) = group {
                write_utf(&mut o, g);
            }
            o.extend_from_slice(&(values.len() as i32).to_be_bytes());
            for (identifier, value) in values {
                write_utf(&mut o, identifier);
                match value {
                    PlaceholderValue::Plain(v) => write_utf(&mut o, v),
                    PlaceholderValue::Relational(rows) => {
                        o.extend_from_slice(&(rows.len() as i32).to_be_bytes());
                        for (name, v) in rows {
                            write_utf(&mut o, name);
                            write_utf(&mut o, v);
                        }
                    }
                }
            }
            o.extend_from_slice(&game_mode.to_be_bytes());
        }
        ToProxy::RegisterPlaceholder(id) => {
            o.push(10);
            write_utf(&mut o, id);
        }
    }
    o
}

/// The proxy's side of the same bytes (the decoder of `PluginMessageDecodeTask`), for tests and for
/// a mod that wants to read what this server sends. `group_present` is the proxy's `forward_group`.
pub fn decode_to_proxy(bytes: &[u8], group_present: bool) -> Result<ToProxy, String> {
    let mut r = Reader::new(bytes);
    Ok(match r.byte()? {
        0 => {
            let message = r.utf()?;
            let mut stack = Vec::new();
            for _ in 0..r.int()?.max(0) {
                stack.push(r.utf()?);
            }
            ToProxy::PlaceholderError { message, stack }
        }
        1 => ToProxy::UpdateGameMode(r.int()?),
        2 => {
            let permission = r.utf()?;
            ToProxy::HasPermission { permission, value: r.bool()? }
        }
        3 => ToProxy::Invisible(r.bool()?),
        4 => ToProxy::Disguised(r.bool()?),
        5 => ToProxy::SetWorld(r.utf()?),
        6 => ToProxy::SetGroup(r.utf()?),
        7 => ToProxy::Vanished(r.bool()?),
        8 => {
            let identifier = r.utf()?;
            let target = if identifier.starts_with("%rel_") { Some(r.utf()?) } else { None };
            ToProxy::UpdatePlaceholder { identifier, target, value: r.utf()? }
        }
        9 => {
            let world = r.utf()?;
            let group = if group_present { Some(r.utf()?) } else { None };
            let mut values = Vec::new();
            for _ in 0..r.int()?.max(0) {
                let identifier = r.utf()?;
                if identifier.starts_with("%rel_") {
                    let mut rows = Vec::new();
                    for _ in 0..r.int()?.max(0) {
                        let name = r.utf()?;
                        rows.push((name, r.utf()?));
                    }
                    values.push((identifier, PlaceholderValue::Relational(rows)));
                } else {
                    values.push((identifier, PlaceholderValue::Plain(r.utf()?)));
                }
            }
            ToProxy::PlayerJoinResponse { world, group, values, game_mode: r.int()? }
        }
        10 => ToProxy::RegisterPlaceholder(r.utf()?),
        other => return Err(format!("unknown bridge answer {other}")),
    })
}

/// Encodes a proxy message the way TAB writes it (for tests of the decoder).
pub fn encode_from_proxy(m: &FromProxy) -> Vec<u8> {
    let mut o = Vec::new();
    let int = |o: &mut Vec<u8>, v: i32| o.extend_from_slice(&v.to_be_bytes());
    match m {
        FromProxy::PlayerJoin { forward_group, placeholders, replacements } => {
            write_utf(&mut o, "PlayerJoin");
            int(&mut o, 0);
            o.push(*forward_group as u8);
            int(&mut o, placeholders.len() as i32);
            for (n, r) in placeholders {
                write_utf(&mut o, n);
                int(&mut o, *r);
            }
            int(&mut o, replacements.len() as i32);
            for (t, rules) in replacements {
                write_utf(&mut o, t);
                int(&mut o, rules.len() as i32);
                for (a, b) in rules {
                    write_utf(&mut o, a);
                    write_utf(&mut o, b);
                }
            }
            o.push(0);
        }
        FromProxy::Permission(p) => {
            write_utf(&mut o, "Permission");
            write_utf(&mut o, p);
        }
        FromProxy::Expansion { placeholder, value } => {
            write_utf(&mut o, "Expansion");
            write_utf(&mut o, placeholder);
            write_utf(&mut o, value);
        }
        FromProxy::Placeholder { identifier, refresh } => {
            write_utf(&mut o, "Placeholder");
            write_utf(&mut o, identifier);
            int(&mut o, *refresh);
        }
        FromProxy::Unload => write_utf(&mut o, "Unload"),
        FromProxy::DisplayObjective { slot, objective } => {
            write_utf(&mut o, "PacketPlayOutScoreboardDisplayObjective");
            int(&mut o, *slot);
            write_utf(&mut o, objective);
        }
        FromProxy::Objective { name, action, title, display, number_format } => {
            write_utf(&mut o, "PacketPlayOutScoreboardObjective");
            write_utf(&mut o, name);
            int(&mut o, *action);
            if *action == 0 || *action == 2 {
                write_utf(&mut o, title);
                int(&mut o, *display);
                o.push(number_format.is_some() as u8);
                if let Some(f) = number_format {
                    write_utf(&mut o, f);
                }
            }
        }
        FromProxy::Score { objective, action, holder, score, display_name, number_format } => {
            write_utf(&mut o, "PacketPlayOutScoreboardScore");
            write_utf(&mut o, objective);
            int(&mut o, *action);
            write_utf(&mut o, holder);
            if *action == 0 {
                int(&mut o, *score);
                o.push(display_name.is_some() as u8);
                if let Some(d) = display_name {
                    write_utf(&mut o, d);
                }
                o.push(number_format.is_some() as u8);
                if let Some(f) = number_format {
                    write_utf(&mut o, f);
                }
            }
        }
        FromProxy::Team { name, action, prefix, suffix, options, visibility, collision, color, players } => {
            write_utf(&mut o, "PacketPlayOutScoreboardTeam");
            write_utf(&mut o, name);
            int(&mut o, *action);
            if *action == 0 || *action == 2 {
                write_utf(&mut o, prefix);
                write_utf(&mut o, suffix);
                int(&mut o, *options);
                write_utf(&mut o, visibility);
                write_utf(&mut o, collision);
                int(&mut o, *color);
            }
            if *action == 0 {
                int(&mut o, players.len() as i32);
                for p in players {
                    write_utf(&mut o, p);
                }
            }
        }
    }
    o
}

/// What the backend remembers of one player's bridge link.
#[derive(Clone, Debug, Default)]
pub struct BridgePlayer {
    pub forward_group: bool,
    /// The proxy's placeholders and their refresh interval in ms.
    pub placeholders: Vec<(String, i32)>,
    pub replacements: Vec<(String, Vec<(String, String)>)>,
    /// Last values sent: `identifier` or `identifier\u{0}target` -> value.
    pub sent: HashMap<String, String>,
    pub last_eval: HashMap<String, i64>,
    pub world: Option<String>,
    pub game_mode: Option<i32>,
    pub vanished: Option<bool>,
    pub invisible: Option<bool>,
    pub group: Option<String>,
    /// Values of the proxy's own placeholders (`Expansion`).
    pub expansion: HashMap<String, String>,
    /// Permission nodes the proxy asked about and the last answer.
    pub permissions: HashMap<String, bool>,
    /// Objectives, teams and scores the proxy created, so `Unload` can take them away.
    pub objectives: Vec<String>,
    pub teams: Vec<String>,
}

/// The links of all bridged players and the bytes waiting for the transport.
#[derive(Clone, Debug, Default)]
pub struct Bridge {
    pub players: HashMap<u128, BridgePlayer>,
    outbox: Vec<(u128, Vec<u8>)>,
    pub problems: Vec<String>,
}

const OUTBOX_MAX: usize = 4000;

impl Bridge {
    pub fn send(&mut self, player: u128, m: &ToProxy) {
        if self.outbox.len() >= OUTBOX_MAX {
            self.outbox.remove(0);
        }
        self.outbox.push((player, encode_to_proxy(m)));
    }

    /// The `(player, bytes)` waiting for the transport.
    pub fn take_outbox(&mut self) -> Vec<(u128, Vec<u8>)> {
        std::mem::take(&mut self.outbox)
    }

    pub fn forget(&mut self, player: u128) {
        self.players.remove(&player);
    }

    pub fn active(&self, player: u128) -> bool {
        self.players.contains_key(&player)
    }
}

/// `%rel_<name>%`-style identifiers carry a viewer and a target.
pub fn is_relational(identifier: &str) -> bool {
    identifier.starts_with("%rel_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_messages_read_back() {
        let msgs = [
            FromProxy::PlayerJoin {
                forward_group: true,
                placeholders: vec![("%ping%".into(), 500), ("%rel_x%".into(), 1000)],
                replacements: vec![("%vanished%".into(), vec![("true".into(), "&7V".into()), ("else".into(), "".into())])],
            },
            FromProxy::Permission("tab.staff".into()),
            FromProxy::Expansion { placeholder: "%online_lobby%".into(), value: "12".into() },
            FromProxy::Placeholder { identifier: "%tps%".into(), refresh: 1000 },
            FromProxy::Unload,
            FromProxy::DisplayObjective { slot: 1, objective: "TAB-Sidebar".into() },
            FromProxy::Objective { name: "o".into(), action: 0, title: "{\"text\":\"T\"}".into(), display: 1, number_format: Some("blank".into()) },
            FromProxy::Objective { name: "o".into(), action: 1, title: String::new(), display: 0, number_format: None },
            FromProxy::Score { objective: "o".into(), action: 0, holder: "Steve".into(), score: 4, display_name: Some("d".into()), number_format: None },
            FromProxy::Score { objective: "o".into(), action: 1, holder: "Steve".into(), score: 0, display_name: None, number_format: None },
            FromProxy::Team {
                name: "01Steve".into(),
                action: 0,
                prefix: "p".into(),
                suffix: "s".into(),
                options: 3,
                visibility: "always".into(),
                collision: "never".into(),
                color: 12,
                players: vec!["Steve".into(), "Alex".into()],
            },
            FromProxy::Team {
                name: "01Steve".into(),
                action: 1,
                prefix: String::new(),
                suffix: String::new(),
                options: 0,
                visibility: String::new(),
                collision: String::new(),
                color: 0,
                players: vec![],
            },
        ];
        for m in msgs {
            assert_eq!(decode_from_proxy(&encode_from_proxy(&m)).unwrap(), m);
        }
        assert!(decode_from_proxy(&{
            let mut b = Vec::new();
            write_utf(&mut b, "Mystery");
            b
        })
        .is_err());
    }

    #[test]
    fn a_player_join_has_the_originals_bytes() {
        let m = FromProxy::PlayerJoin { forward_group: true, placeholders: vec![("%a%".into(), 7)], replacements: vec![] };
        let mut want = vec![0, 10];
        want.extend_from_slice(b"PlayerJoin");
        want.extend_from_slice(&0i32.to_be_bytes()); // protocol version, no longer used
        want.push(1);
        want.extend_from_slice(&1i32.to_be_bytes());
        want.extend_from_slice(&[0, 3, b'%', b'a', b'%']);
        want.extend_from_slice(&7i32.to_be_bytes());
        want.extend_from_slice(&0i32.to_be_bytes());
        want.push(0); // deleted unlimited name tags
        assert_eq!(encode_from_proxy(&m), want);
    }

    #[test]
    fn answers_start_with_their_message_number() {
        let cases: Vec<(ToProxy, u8)> = vec![
            (ToProxy::PlaceholderError { message: "m".into(), stack: vec!["a".into()] }, 0),
            (ToProxy::UpdateGameMode(1), 1),
            (ToProxy::HasPermission { permission: "x".into(), value: true }, 2),
            (ToProxy::Invisible(true), 3),
            (ToProxy::Disguised(false), 4),
            (ToProxy::SetWorld("w".into()), 5),
            (ToProxy::SetGroup("g".into()), 6),
            (ToProxy::Vanished(true), 7),
            (ToProxy::UpdatePlaceholder { identifier: "%ping%".into(), target: None, value: "5".into() }, 8),
            (ToProxy::RegisterPlaceholder("%x%".into()), 10),
        ];
        for (m, n) in cases {
            let b = encode_to_proxy(&m);
            assert_eq!(b[0], n);
            assert_eq!(decode_to_proxy(&b, false).unwrap(), m);
        }
    }

    #[test]
    fn the_join_response_carries_plain_and_relational_values() {
        let m = ToProxy::PlayerJoinResponse {
            world: "world".into(),
            group: Some("admin".into()),
            values: vec![
                ("%ping%".into(), PlaceholderValue::Plain("42".into())),
                ("%rel_color%".into(), PlaceholderValue::Relational(vec![("Alex".into(), "&c".into())])),
            ],
            game_mode: 1,
        };
        let b = encode_to_proxy(&m);
        assert_eq!(decode_to_proxy(&b, true).unwrap(), m);
        // Without forward_group the group is not on the wire: the proxy reads the world and then the count.
        let no_group = ToProxy::PlayerJoinResponse { group: None, ..m.clone() };
        assert_eq!(decode_to_proxy(&encode_to_proxy(&no_group), false).unwrap(), no_group);
        // The relational update names its target.
        let u = ToProxy::UpdatePlaceholder { identifier: "%rel_color%".into(), target: Some("Alex".into()), value: "&a".into() };
        assert_eq!(decode_to_proxy(&encode_to_proxy(&u), false).unwrap(), u);
    }

    #[test]
    fn the_outbox_is_bounded_and_drained() {
        let mut b = Bridge::default();
        for _ in 0..OUTBOX_MAX + 10 {
            b.send(1, &ToProxy::Vanished(true));
        }
        assert_eq!(b.take_outbox().len(), OUTBOX_MAX);
        assert!(b.take_outbox().is_empty());
    }
}
