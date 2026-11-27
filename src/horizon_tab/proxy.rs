//! `proxy-support` and `global-playerlist`: TAB's message protocol between several TAB instances
//! (`features/proxy/ProxySupport` and its six messages) and the global playerlist rules
//! (`GlobalPlayerListConfiguration`, `Server.canSee`, `DataManager.computeServerGroup`).
//!
//! The codec is Java's `DataOutput`/`DataInput` (modified UTF-8, big-endian numbers) inside Base64,
//! as `ProxySupport.sendMessage(ProxyMessage)` writes it. The transport (a plugin channel, Redis or
//! RabbitMQ) is not part of the logic: `ProxyState::take_outbox` hands out the encoded strings and
//! `ProxyState::receive` takes one in. See ENGINE_NEEDS `plugin_message_channel`.

use crate::horizon_tab::perworld::matches_pattern;
use crate::horizon_tab::yaml::Yaml;
use std::collections::{BTreeMap, HashMap};

/// The channel name `ProxySupportConfiguration` builds: `TAB_4_<channel-name-suffix>`.
pub fn channel_name(suffix: &str) -> String {
    format!("TAB_4_{suffix}")
}

// ---- Java DataOutput / DataInput -------------------------------------------------------------

/// `DataOutput.writeUTF`: modified UTF-8 with a two-byte length.
pub fn write_utf(out: &mut Vec<u8>, s: &str) {
    let mut body: Vec<u8> = Vec::with_capacity(s.len());
    for unit in s.encode_utf16() {
        match unit {
            0x0001..=0x007F => body.push(unit as u8),
            0x0000 | 0x0080..=0x07FF => {
                body.push(0xC0 | (unit >> 6) as u8);
                body.push(0x80 | (unit & 0x3F) as u8);
            }
            _ => {
                body.push(0xE0 | (unit >> 12) as u8);
                body.push(0x80 | ((unit >> 6) & 0x3F) as u8);
                body.push(0x80 | (unit & 0x3F) as u8);
            }
        }
    }
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(&body);
}

pub fn write_uuid(out: &mut Vec<u8>, id: u128) {
    out.extend_from_slice(&id.to_be_bytes());
}

/// A cursor over received bytes (`ByteArrayDataInput`).
pub struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.b.len()).ok_or("message ends too early")?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    pub fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool, String> {
        Ok(self.byte()? != 0)
    }
    pub fn int(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().map_err(|_| "int")?))
    }
    pub fn uuid(&mut self) -> Result<u128, String> {
        Ok(u128::from_be_bytes(self.take(16)?.try_into().map_err(|_| "uuid")?))
    }
    /// `DataInput.readUTF`.
    pub fn utf(&mut self) -> Result<String, String> {
        let len = u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| "len")?) as usize;
        let raw = self.take(len)?;
        let mut units: Vec<u16> = Vec::with_capacity(len);
        let mut i = 0;
        while i < raw.len() {
            let c = raw[i];
            match c >> 4 {
                0..=7 => {
                    units.push(c as u16);
                    i += 1;
                }
                12 | 13 => {
                    let d = *raw.get(i + 1).ok_or("bad UTF")?;
                    units.push((((c & 0x1F) as u16) << 6) | (d & 0x3F) as u16);
                    i += 2;
                }
                14 => {
                    let (d, e) = (*raw.get(i + 1).ok_or("bad UTF")?, *raw.get(i + 2).ok_or("bad UTF")?);
                    units.push((((c & 0x0F) as u16) << 12) | (((d & 0x3F) as u16) << 6) | (e & 0x3F) as u16);
                    i += 3;
                }
                _ => return Err("bad UTF".into()),
            }
        }
        Ok(String::from_utf16_lossy(&units))
    }
    pub fn rest(&self) -> usize {
        self.b.len() - self.at
    }
}

// ---- Base64 (RFC 4648, with padding; `java.util.Base64`) -----------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        s.push(B64[(n >> 18) as usize & 63] as char);
        s.push(B64[(n >> 12) as usize & 63] as char);
        s.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    s
}

pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in text.trim_end_matches('=').bytes() {
        let v = B64.iter().position(|b| *b == c).ok_or("not Base64")? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

// ---- messages ------------------------------------------------------------------------------------

/// A skin as TAB passes it: the textures value and an optional signature.
pub type Skin = (String, Option<String>);

/// What `PlayerJoin` carries about one player.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerData {
    pub unique_id: u128,
    pub tablist_id: u128,
    pub name: String,
    pub server: String,
    pub vanished: bool,
    pub staff: bool,
    pub skin: Option<Skin>,
}

fn write_player(out: &mut Vec<u8>, p: &PlayerData) {
    write_uuid(out, p.unique_id);
    write_uuid(out, p.tablist_id);
    write_utf(out, &p.name);
    write_utf(out, &p.server);
    out.push(p.vanished as u8);
    out.push(p.staff as u8);
    match &p.skin {
        None => out.push(0),
        Some((value, signature)) => {
            out.push(1);
            write_utf(out, value);
            match signature {
                None => out.push(0),
                Some(s) => {
                    out.push(1);
                    write_utf(out, s);
                }
            }
        }
    }
}

fn read_player(r: &mut Reader) -> Result<PlayerData, String> {
    let unique_id = r.uuid()?;
    let tablist_id = r.uuid()?;
    let name = r.utf()?;
    let server = r.utf()?;
    let vanished = r.bool()?;
    let staff = r.bool()?;
    let skin = if r.bool()? {
        let value = r.utf()?;
        let signature = if r.bool()? { Some(r.utf()?) } else { None };
        Some((value, signature))
    } else {
        None
    };
    Ok(PlayerData { unique_id, tablist_id, name, server, vanished, staff, skin })
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProxyMsg {
    Load(Vec<PlayerData>),
    LoadRequest,
    PlayerJoin(PlayerData),
    PlayerQuit(u128),
    ServerSwitch(u128, String),
    UpdateVanishStatus(u128, bool),
}

impl ProxyMsg {
    fn action(&self) -> &'static str {
        match self {
            ProxyMsg::Load(_) => "Load",
            ProxyMsg::LoadRequest => "LoadRequest",
            ProxyMsg::PlayerJoin(_) => "PlayerJoin",
            ProxyMsg::PlayerQuit(_) => "PlayerQuit",
            ProxyMsg::ServerSwitch(..) => "ServerSwitch",
            ProxyMsg::UpdateVanishStatus(..) => "UpdateVanishStatus",
        }
    }
}

/// `ProxySupport.sendMessage(ProxyMessage)`: the sender's id, the action name, the body, as Base64.
pub fn encode(sender: &str, msg: &ProxyMsg) -> String {
    let mut out = Vec::new();
    write_utf(&mut out, sender);
    write_utf(&mut out, msg.action());
    match msg {
        ProxyMsg::Load(players) => {
            out.extend_from_slice(&(players.len() as i32).to_be_bytes());
            for p in players {
                write_player(&mut out, p);
            }
        }
        ProxyMsg::LoadRequest => {}
        ProxyMsg::PlayerJoin(p) => write_player(&mut out, p),
        ProxyMsg::PlayerQuit(id) => write_uuid(&mut out, *id),
        ProxyMsg::ServerSwitch(id, server) => {
            write_uuid(&mut out, *id);
            write_utf(&mut out, server);
        }
        ProxyMsg::UpdateVanishStatus(id, vanished) => {
            write_uuid(&mut out, *id);
            out.push(*vanished as u8);
        }
    }
    base64_encode(&out)
}

/// `ProxySupport.processMessage`'s decoding: the sender's id and the message; an unknown action is an error.
pub fn decode(text: &str) -> Result<(String, ProxyMsg), String> {
    let bytes = base64_decode(text.trim())?;
    let mut r = Reader::new(&bytes);
    let sender = r.utf()?;
    let action = r.utf()?;
    let msg = match action.as_str() {
        "Load" => {
            let n = r.int()?;
            let mut players = Vec::new();
            for _ in 0..n.max(0) {
                players.push(read_player(&mut r)?);
            }
            ProxyMsg::Load(players)
        }
        "LoadRequest" => ProxyMsg::LoadRequest,
        "PlayerJoin" => ProxyMsg::PlayerJoin(read_player(&mut r)?),
        "PlayerQuit" => ProxyMsg::PlayerQuit(r.uuid()?),
        "ServerSwitch" => {
            let id = r.uuid()?;
            ProxyMsg::ServerSwitch(id, r.utf()?)
        }
        "UpdateVanishStatus" => {
            let id = r.uuid()?;
            ProxyMsg::UpdateVanishStatus(id, r.bool()?)
        }
        other => return Err(format!("unknown proxy message \"{other}\"")),
    };
    Ok((sender, msg))
}

// ---- state -------------------------------------------------------------------------------------

/// A player of another TAB instance (`ProxyPlayer`).
#[derive(Clone, Debug, PartialEq)]
pub struct ProxyPlayer {
    pub unique_id: u128,
    pub tablist_id: u128,
    pub name: String,
    pub nickname: String,
    pub server: String,
    pub vanished: bool,
    pub staff: bool,
    pub skin: Option<Skin>,
}

impl From<&PlayerData> for ProxyPlayer {
    fn from(d: &PlayerData) -> Self {
        ProxyPlayer {
            unique_id: d.unique_id,
            tablist_id: d.tablist_id,
            name: d.name.clone(),
            nickname: d.name.clone(),
            server: d.server.clone(),
            vanished: d.vanished,
            staff: d.staff,
            skin: d.skin.clone(),
        }
    }
}

/// Data that arrived before the player's join (`QueuedData`).
#[derive(Clone, Debug, Default, PartialEq)]
struct Queued {
    vanished: Option<bool>,
}

const OUTBOX_MAX: usize = 2000;

#[derive(Clone, Debug, Default)]
pub struct ProxyState {
    /// This instance's id: messages with the same id are ours and are ignored.
    pub id: String,
    pub enabled: bool,
    pub players: BTreeMap<u128, ProxyPlayer>,
    queued: HashMap<u128, Queued>,
    /// What we told the others about our own players.
    announced: HashMap<u128, PlayerData>,
    outbox: Vec<String>,
    started: bool,
    /// Messages that could not be decoded or used, for `/tab debug` and the log.
    pub problems: Vec<String>,
}

impl ProxyState {
    pub fn new(id: String) -> ProxyState {
        ProxyState { id, ..ProxyState::default() }
    }

    fn send(&mut self, msg: ProxyMsg) {
        if !self.enabled {
            return;
        }
        if self.outbox.len() >= OUTBOX_MAX {
            self.outbox.remove(0);
        }
        let line = encode(&self.id, &msg);
        self.outbox.push(line);
    }

    /// The encoded messages waiting for the transport.
    pub fn take_outbox(&mut self) -> Vec<String> {
        std::mem::take(&mut self.outbox)
    }

    /// `ProxySupport.load`: ask the others for their players.
    pub fn start(&mut self) {
        if !self.started {
            self.started = true;
            self.send(ProxyMsg::LoadRequest);
        }
    }

    /// Compares the local players with what the others were told and sends the difference
    /// (`onJoin`, `onQuit`, `onServerChange`, `onVanishStatusChange` of `ProxySupport`).
    pub fn sync_local(&mut self, local: &[PlayerData]) {
        if !self.enabled {
            return;
        }
        self.start();
        let gone: Vec<u128> = self.announced.keys().filter(|id| !local.iter().any(|p| p.unique_id == **id)).copied().collect();
        for id in gone {
            self.announced.remove(&id);
            self.send(ProxyMsg::PlayerQuit(id));
        }
        for p in local {
            match self.announced.get(&p.unique_id).cloned() {
                None => {
                    self.send(ProxyMsg::PlayerJoin(p.clone()));
                }
                Some(old) => {
                    if old.server != p.server {
                        self.send(ProxyMsg::ServerSwitch(p.unique_id, p.server.clone()));
                    }
                    if old.vanished != p.vanished {
                        self.send(ProxyMsg::UpdateVanishStatus(p.unique_id, p.vanished));
                    }
                }
            }
            self.announced.insert(p.unique_id, p.clone());
        }
    }

    /// `ProxySupport.processMessage` for one received string. `local` answers a `LoadRequest`.
    pub fn receive(&mut self, raw: &str, local: &[PlayerData]) {
        if !self.enabled {
            return;
        }
        match decode(raw) {
            Err(e) => self.problems.push(e),
            Ok((sender, _)) if sender == self.id => {}
            Ok((_, msg)) => self.apply(msg, local),
        }
    }

    fn apply(&mut self, msg: ProxyMsg, local: &[PlayerData]) {
        match msg {
            ProxyMsg::Load(list) => {
                for p in list {
                    self.join(p);
                }
            }
            ProxyMsg::LoadRequest => {
                self.send(ProxyMsg::Load(local.to_vec()));
            }
            ProxyMsg::PlayerJoin(p) => self.join(p),
            ProxyMsg::PlayerQuit(id) => {
                if self.players.remove(&id).is_none() {
                    self.queued.remove(&id);
                }
            }
            ProxyMsg::ServerSwitch(id, server) => {
                if let Some(p) = self.players.get_mut(&id) {
                    p.server = server;
                }
            }
            ProxyMsg::UpdateVanishStatus(id, vanished) => match self.players.get_mut(&id) {
                Some(p) => p.vanished = vanished,
                None => self.queued.entry(id).or_default().vanished = Some(vanished),
            },
        }
    }

    /// `PlayerJoin.process`: a player already known is not added twice; queued data is applied.
    fn join(&mut self, data: PlayerData) {
        if self.players.contains_key(&data.unique_id) {
            return;
        }
        let mut p = ProxyPlayer::from(&data);
        if let Some(q) = self.queued.remove(&data.unique_id) {
            if let Some(v) = q.vanished {
                p.vanished = v;
            }
        }
        self.players.insert(data.unique_id, p);
    }

    /// What the placeholders count: `(server, vanished, staff)` of every remote player.
    pub fn brief(&self) -> Vec<ProxyBrief> {
        self.players
            .values()
            .map(|p| ProxyBrief { server: p.server.clone(), vanished: p.vanished, staff: p.staff })
            .collect()
    }

    pub fn clear(&mut self) {
        self.players.clear();
        self.queued.clear();
        self.announced.clear();
        self.started = false;
    }
}

/// One remote player as `%online%` and its relatives count it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProxyBrief {
    pub server: String,
    pub vanished: bool,
    pub staff: bool,
}

// ---- global playerlist ----------------------------------------------------------------------------

/// `GlobalPlayerListConfiguration` and the server groups it defines.
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalCfg {
    pub others_as_spectators: bool,
    pub vanished_as_spectators: bool,
    pub isolate_unlisted: bool,
    pub update_latency: bool,
    pub spy_servers: Vec<String>,
    /// Group name -> server patterns, in file order.
    pub groups: Vec<(String, Vec<String>)>,
}

impl GlobalCfg {
    pub fn from_yaml(y: &Yaml, warnings: &mut Vec<String>) -> GlobalCfg {
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        let mut taken: Vec<(String, String)> = Vec::new();
        for (name, v) in y.get("server-groups").entries() {
            let servers = v.str_list();
            for s in &servers {
                if let Some((_, first)) = taken.iter().find(|(server, _)| server == s) {
                    warnings.push(format!(
                        "Server \"{s}\" is defined in global playerlist groups \"{first}\" and \"{name}\", but it can only be a part of one group."
                    ));
                } else {
                    taken.push((s.clone(), name.clone()));
                }
            }
            groups.push((name.clone(), servers));
        }
        GlobalCfg {
            others_as_spectators: y.get("display-others-as-spectators").bool_or(false),
            vanished_as_spectators: y.get("display-vanished-players-as-spectators").bool_or(true),
            isolate_unlisted: y.get("isolate-unlisted-servers").bool_or(false),
            update_latency: y.get("update-latency").bool_or(false),
            spy_servers: if y.get("spy-servers").is_null() {
                vec!["spyserver1".to_owned()]
            } else {
                y.get("spy-servers").str_list()
            },
            groups,
        }
    }

    /// `computeServerGroup`: the listed group, else the shared default, else the server alone.
    pub fn group_of(&self, server: &str) -> String {
        for (name, patterns) in &self.groups {
            if patterns.iter().any(|p| matches_pattern(server, p)) {
                return name.clone();
            }
        }
        if self.isolate_unlisted {
            format!("<isolated: {server}>")
        } else {
            "<DEFAULT>".to_owned()
        }
    }

    pub fn is_spy(&self, server: &str) -> bool {
        self.spy_servers.iter().any(|p| matches_pattern(server, p))
    }

    /// `Server.canSee`.
    pub fn can_see(&self, viewer_server: &str, other_server: &str) -> bool {
        viewer_server == other_server || self.is_spy(viewer_server) || self.group_of(viewer_server) == self.group_of(other_server)
    }

    /// The count `%global_playerlist_<group>%` gives: players on the group's servers, not vanished.
    pub fn group_count(&self, group: &str, servers: impl Iterator<Item = (String, bool)>) -> usize {
        let Some((_, patterns)) = self.groups.iter().find(|(n, _)| n == group) else { return 0 };
        servers.filter(|(s, vanished)| !vanished && patterns.iter().any(|p| matches_pattern(s, p))).count()
    }
}

/// `GlobalPlayerList.shouldSee(TabPlayer, ProxyPlayer)`: the remote player shows in the viewer's list.
pub fn remote_visible(
    cfg: &GlobalCfg,
    viewer_server: &str,
    viewer_sees_vanished: bool,
    target: &ProxyPlayer,
    connected_here: bool,
) -> bool {
    if connected_here {
        return false;
    }
    cfg.can_see(viewer_server, &target.server) && (!target.vanished || viewer_sees_vanished)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizon_tab::yaml;

    fn player(id: u128, name: &str, server: &str) -> PlayerData {
        PlayerData {
            unique_id: id,
            tablist_id: id,
            name: name.into(),
            server: server.into(),
            vanished: false,
            staff: false,
            skin: Some(("VAL".into(), Some("SIG".into()))),
        }
    }

    #[test]
    fn java_utf_and_base64_match_the_known_bytes() {
        let mut b = Vec::new();
        write_utf(&mut b, "A\u{e9}\u{0}\u{20ac}");
        // length, 'A', C3 A9, C0 80 (modified NUL), E2 82 AC.
        assert_eq!(b, [0, 8, 0x41, 0xC3, 0xA9, 0xC0, 0x80, 0xE2, 0x82, 0xAC]);
        assert_eq!(Reader::new(&b).utf().unwrap(), "A\u{e9}\u{0}\u{20ac}");
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_decode("TWE=").unwrap(), b"Ma");
        assert!(base64_decode("T$E=").is_err());
    }

    #[test]
    fn every_message_reads_back() {
        let msgs = [
            ProxyMsg::LoadRequest,
            ProxyMsg::PlayerJoin(player(7, "Steve", "lobby")),
            ProxyMsg::PlayerQuit(7),
            ProxyMsg::ServerSwitch(7, "survival".into()),
            ProxyMsg::UpdateVanishStatus(7, true),
            ProxyMsg::Load(vec![player(1, "A", "x"), PlayerData { skin: None, ..player(2, "B", "y") }]),
        ];
        for m in msgs {
            let text = encode("proxy-1", &m);
            assert_eq!(decode(&text).unwrap(), ("proxy-1".to_owned(), m));
        }
        assert!(decode(&base64_encode(&{
            let mut b = Vec::new();
            write_utf(&mut b, "p");
            write_utf(&mut b, "Nope");
            b
        }))
        .is_err());
    }

    #[test]
    fn a_player_join_has_the_originals_byte_layout() {
        let mut want = Vec::new();
        want.extend_from_slice(&[0, 1, b'p', 0, 10]);
        want.extend_from_slice(b"PlayerJoin");
        want.extend_from_slice(&1u128.to_be_bytes());
        want.extend_from_slice(&2u128.to_be_bytes());
        want.extend_from_slice(&[0, 1, b'N', 0, 1, b'S', 1, 0, 0]);
        let p = PlayerData { unique_id: 1, tablist_id: 2, name: "N".into(), server: "S".into(), vanished: true, staff: false, skin: None };
        assert_eq!(encode("p", &ProxyMsg::PlayerJoin(p)), base64_encode(&want));
    }

    #[test]
    fn players_of_other_instances_are_tracked() {
        let mut me = ProxyState::new("me".into());
        me.enabled = true;
        let mut them = ProxyState::new("them".into());
        them.enabled = true;
        them.sync_local(&[player(5, "Alex", "survival")]);
        for line in them.take_outbox() {
            me.receive(&line, &[]);
        }
        assert_eq!(me.players.len(), 1, "the join arrived");
        // Our own messages are ignored.
        let own = encode("me", &ProxyMsg::PlayerJoin(player(9, "Me", "x")));
        me.receive(&own, &[]);
        assert_eq!(me.players.len(), 1);
        // A switch, a vanish and a quit.
        them.sync_local(&[PlayerData { server: "lobby".into(), vanished: true, ..player(5, "Alex", "x") }]);
        for line in them.take_outbox() {
            me.receive(&line, &[]);
        }
        let p = &me.players[&5];
        assert_eq!((p.server.as_str(), p.vanished), ("lobby", true));
        assert_eq!(me.brief(), vec![ProxyBrief { server: "lobby".into(), vanished: true, staff: false }]);
        them.sync_local(&[]);
        for line in them.take_outbox() {
            me.receive(&line, &[]);
        }
        assert!(me.players.is_empty());
    }

    #[test]
    fn a_load_request_is_answered_with_a_load() {
        let mut me = ProxyState::new("me".into());
        me.enabled = true;
        let ask = encode("other", &ProxyMsg::LoadRequest);
        me.receive(&ask, &[player(1, "Steve", "here")]);
        let sent = me.take_outbox();
        assert_eq!(sent.len(), 1);
        assert_eq!(decode(&sent[0]).unwrap(), ("me".to_owned(), ProxyMsg::Load(vec![player(1, "Steve", "here")])));
    }

    #[test]
    fn vanish_before_join_is_queued() {
        let mut me = ProxyState::new("me".into());
        me.enabled = true;
        me.receive(&encode("o", &ProxyMsg::UpdateVanishStatus(3, true)), &[]);
        assert!(me.players.is_empty());
        me.receive(&encode("o", &ProxyMsg::PlayerJoin(player(3, "V", "s"))), &[]);
        assert!(me.players[&3].vanished);
        // A second join of the same player is dropped.
        me.receive(&encode("o", &ProxyMsg::PlayerJoin(PlayerData { name: "Other".into(), ..player(3, "V", "s") })), &[]);
        assert_eq!(me.players[&3].name, "V");
    }

    #[test]
    fn a_disabled_state_stays_silent() {
        let mut s = ProxyState::new("me".into());
        s.sync_local(&[player(1, "A", "x")]);
        assert!(s.take_outbox().is_empty());
        s.receive(&encode("o", &ProxyMsg::PlayerJoin(player(3, "V", "s"))), &[]);
        assert!(s.players.is_empty());
    }

    fn cfg(text: &str) -> GlobalCfg {
        GlobalCfg::from_yaml(&yaml::parse(text).unwrap(), &mut Vec::new())
    }

    #[test]
    fn server_groups_follow_the_originals_rules() {
        let c = cfg("server-groups:\n  lobbies:\n    - lobby*\n  g2:\n    - survival\nspy-servers:\n  - spy\n");
        assert!(c.can_see("lobby1", "lobby2"));
        assert!(!c.can_see("lobby1", "survival"));
        assert!(c.can_see("spy", "survival"), "a spy server sees every group");
        assert!(!c.can_see("survival", "spy"));
        // Two unlisted servers share the default group until they are isolated.
        assert!(c.can_see("a", "b"));
        let iso = cfg("isolate-unlisted-servers: true\n");
        assert!(!iso.can_see("a", "b"));
        assert!(iso.can_see("a", "a"));
        let defaults = cfg("x: 1\n");
        assert!(defaults.vanished_as_spectators && !defaults.others_as_spectators && !defaults.update_latency);
        assert_eq!(defaults.spy_servers, vec!["spyserver1"]);
    }

    #[test]
    fn a_server_in_two_groups_is_warned_about() {
        let mut w = Vec::new();
        GlobalCfg::from_yaml(&yaml::parse("server-groups:\n  a:\n    - s1\n  b:\n    - s1\n").unwrap(), &mut w);
        assert_eq!(w, vec!["Server \"s1\" is defined in global playerlist groups \"a\" and \"b\", but it can only be a part of one group."]);
    }

    #[test]
    fn the_group_count_skips_vanished_players() {
        let c = cfg("server-groups:\n  lobbies:\n    - lobby*\n");
        let n = c.group_count("lobbies", vec![("lobby1".to_owned(), false), ("lobby2".to_owned(), true), ("x".to_owned(), false)].into_iter());
        assert_eq!(n, 1);
    }

    #[test]
    fn remote_visibility() {
        let c = cfg("server-groups:\n  l:\n    - lobby\n");
        let mut t = ProxyPlayer::from(&player(1, "T", "lobby"));
        assert!(remote_visible(&c, "lobby", false, &t, false));
        assert!(!remote_visible(&c, "lobby", false, &t, true), "already listed by the server itself");
        assert!(!remote_visible(&c, "other", false, &t, false), "another group");
        t.vanished = true;
        assert!(!remote_visible(&c, "lobby", false, &t, false));
        assert!(remote_visible(&c, "lobby", true, &t, false), "tab.seevanished");
    }
}
