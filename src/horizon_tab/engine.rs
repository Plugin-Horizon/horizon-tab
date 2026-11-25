//! The TAB features, as a state machine fed once a tick with the players and the server, which
//! answers with the packets that bring every player's client to what it should show. It keeps,
//! per viewer, what their client was sent, and sends only changes.
//!
//! Features: header/footer designs, Tab list names (`tabprefix` + `customtabname` + `tabsuffix`),
//! name tags and sorting through teams (`tagprefix`/`tagsuffix`, team colour, collision, nametag
//! visibility), the player list objective (yellow number / fancy value), the below-name
//! objective, the sidebar scoreboard, boss bars, spectator fix and ping spoof. How each one
//! behaves follows TAB's feature classes (Apache-2.0, see NOTICE).

use crate::horizon_tab::bridge::{Bridge, BridgePlayer, FromProxy, PlaceholderValue, ToProxy};
use crate::horizon_tab::config::{Board, Config};
use crate::horizon_tab::proxy::ProxyState;
use crate::horizon_tab::integration;
use crate::horizon_tab::mysql;
use crate::horizon_tab::out::{ListEntry, BossOut, NumberFmt, ObjectiveMethod, Out, Packet, TeamParams};
use crate::horizon_tab::placeholders::{self, uuid_string, Player, Server};
use crate::horizon_tab::properties::{Properties, PropertyFile};
use crate::horizon_tab::sorting::{self, SortingType};
use crate::horizon_tab::tabph::{self, Ctx, TabPlaceholders, TabPlayer};
use crate::horizon_tab::text;
use crate::horizon_tab::layout::{self, Content};
use crate::horizon_tab::yaml::{self, Yaml};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const LIST_OBJECTIVE: &str = "TAB-PlayerList";
pub const BELOW_OBJECTIVE: &str = "TAB-BelowName";
pub const SIDEBAR_OBJECTIVE: &str = "TAB-Scoreboard";
const SPECTATOR_RESEND_MS: i64 = 2000;
const PING_SPOOF_RESEND_MS: i64 = 15000;

/// A player as the game reports them each tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerIn {
    pub uuid: u128,
    pub name: String,
    pub dimension: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub game_mode: i32,
    pub op_level: u8,
    pub ping: i32,
    pub nickname: Option<String>,
    pub vanished: bool,
    pub invisible: bool,
    pub deaths: i32,
    pub entity_id: i32,
    pub protocol: i32,
    /// A bot with no client (a fake player): it is shown to the others but nothing is built or
    /// sent for it as a viewer. local perf patch for fake players, candidate upstream PR.
    pub fake: bool,
    /// The `textures` property of the profile (value, signature), for layout slots.
    pub skin: Option<(String, String)>,
}

/// Bukkit's world folder name for a dimension (`world`, `world_nether`, `world_the_end`).
pub fn world_name(level: &str, dimension: &str) -> String {
    let level = if level.is_empty() { "world" } else { level };
    match dimension {
        "minecraft:overworld" | "" => level.to_owned(),
        "minecraft:the_nether" => format!("{level}_nether"),
        "minecraft:the_end" => format!("{level}_the_end"),
        other => other.rsplit(':').next().unwrap_or(other).to_owned(),
    }
}

/// The client's version name for a protocol number (what `%player-version%` shows).
fn version_name(protocol: i32) -> String {
    match protocol {
        773 => "1.21.10".into(),
        774 => "1.21.11".into(),
        775 => "26.1.2".into(),
        p if p > 775 => "26.3".into(),
        p => format!("protocol {p}"),
    }
}

#[derive(Clone, Debug, PartialEq)]
struct SidebarSent {
    title: Value,
    lines: Vec<(Value, Option<Value>)>,
}

#[derive(Clone, Debug, PartialEq)]
struct BarSent {
    id: u128,
    title: Value,
    progress: f32,
    color: i32,
    overlay: i32,
}

/// What one viewer's client holds from us.
#[derive(Clone, Debug, Default)]
struct Sent {
    header: Option<(Value, Value)>,
    names: HashMap<u128, Option<Value>>,
    teams: HashMap<u128, (String, TeamParams)>,
    list_objective: Option<Value>,
    list_scores: HashMap<u128, (String, i32, Option<Value>)>,
    below_objective: Option<(Value, Option<Value>)>,
    below_scores: HashMap<u128, (String, i32, Option<Value>)>,
    sidebar: Option<SidebarSent>,
    bars: HashMap<String, BarSent>,
    last_spectator_ms: i64,
    last_ping_spoof_ms: i64,
    /// The team member entry sent for each target.
    team_members: HashMap<u128, String>,
    /// Players hidden from this viewer by the per-world playerlist.
    pw_hidden: HashSet<u128>,
    layout: Option<LayoutSent>,
    /// Remote players the global playerlist added (tab list id -> what was sent).
    global: HashMap<u128, String>,
}

/// What a viewer's client holds of the layout.
#[derive(Clone, Debug, PartialEq)]
struct LayoutSent {
    name: String,
    slots: std::collections::BTreeMap<u32, SlotSig>,
    /// Real players taken out of the list.
    hidden: HashSet<u128>,
}

#[derive(Clone, Debug, PartialEq)]
struct SlotSig {
    skin: Option<(String, String)>,
    latency: i32,
    display: Value,
    who: Option<u128>,
}

#[derive(Clone, Debug)]
struct PState {
    tp: TabPlayer,
    /// No client to build anything for (`PlayerIn::fake`).
    fake: bool,
    joined_ms: i64,
    team: Option<String>,
    sort_chars: String,
    /// Toggles, as flips from the configured default: shown = (toggled == hidden-by-default).
    scoreboard_toggled: bool,
    bossbar_toggled: bool,
    nametag_hidden: bool,
    nametag_view_hidden: bool,
    /// Viewers this player's name tag is hidden for (`/tab nametag hide <player> <viewer>`).
    nametag_hidden_for: HashSet<u128>,
    /// `/tab nametag opaque`: this player's name tag shows only with a line of sight, for everyone
    /// or for the viewers in `opaque_for`.
    opaque: bool,
    opaque_for: HashSet<u128>,
    /// `/tab nametag opaqueview`: this player sees only the name tags they have a line of sight to.
    opaque_view: bool,
    collision: Option<bool>,
    forced_board: Option<String>,
    /// Bars sent to this player alone (`/tab bossbar send`), until a time.
    sent_bars: Vec<(String, i64)>,
    sent: Sent,
}

/// Per-target texts computed once a tick (viewer = target).
#[derive(Clone, Debug, Default)]
struct TargetTexts {
    names_disabled: bool,
    teams_disabled: bool,
    tab_raw: String,
    tab_rel: bool,
    tab: Value,
    tag_prefix_raw: String,
    tag_suffix_raw: String,
    tag_rel: bool,
    tag_prefix: Value,
    tag_suffix: Value,
    tag_color: Option<i32>,
    list_value: i32,
    below_value: i32,
}

pub struct Engine {
    pub dir: PathBuf,
    pub cfg: Config,
    pub props: Properties,
    pub ph: TabPlaceholders,
    pub messages: Yaml,
    pub sorting: Vec<SortingType>,
    pub warnings: Vec<String>,
    playerdata: Yaml,
    players: Vec<PState>,
    announced_bars: Vec<(String, i64)>,
    announced_board: Option<(String, i64)>,
    now_ms: i64,
    server: Server,
    /// Names whose display name another sender overwrote on every client (a nickname change).
    overwritten: HashSet<u128>,
    /// `playerdata.yml` exists but could not be read: toggles are then kept in memory only.
    playerdata_broken: bool,
    /// `/tab cpu`.
    pub cpu: crate::horizon_tab::cpu::Cpu,
    /// The textures property of each online player (layout slots show it).
    skins: HashMap<u128, (String, String)>,
    /// The name a player's own tab-list entry carries when another plugin renamed it
    /// (`NickCompatibility`); teams and scores name this entry.
    entry_names: HashMap<u128, String>,
    /// (viewer, target) pairs whose name tag the opaque mode hides: no line of sight.
    occluded: HashSet<(u128, u128)>,
    /// `proxy-support`: the players of other TAB instances and the messages for them.
    pub proxy: ProxyState,
    /// The backend half of TAB's proxy bridge.
    pub bridge: Bridge,
}

fn read_yaml(path: &Path, warnings: &mut Vec<String>) -> Yaml {
    read_yaml_checked(path, warnings).0
}

/// Like `read_yaml`, and true when the file exists but could not be parsed.
fn read_yaml_checked(path: &Path, warnings: &mut Vec<String>) -> (Yaml, bool) {
    match std::fs::read_to_string(path) {
        Ok(text) => match yaml::parse(&text) {
            Ok(y) => (y, false),
            Err(e) => {
                warnings.push(format!("{}: {e}", path.display()));
                (Yaml::empty_map(), true)
            }
        },
        Err(_) => (Yaml::empty_map(), false),
    }
}

fn bar_color(name: &str) -> i32 {
    match name.trim().to_ascii_uppercase().as_str() {
        "PINK" => 0,
        "BLUE" => 1,
        "RED" => 2,
        "GREEN" => 3,
        "YELLOW" => 4,
        "PURPLE" => 5,
        _ => 6,
    }
}

fn bar_overlay(name: &str) -> i32 {
    match name.trim().to_ascii_uppercase().as_str() {
        "NOTCHED_6" => 1,
        "NOTCHED_10" => 2,
        "NOTCHED_12" => 3,
        "NOTCHED_20" => 4,
        _ => 0,
    }
}

/// A stable UUID for a boss bar name (FNV-1a over the name, version 4 bits set).
fn bar_id(name: &str) -> u128 {
    let mut h: u128 = 0x6c62272e07bb014262b821756295c58d;
    for b in format!("tab-bossbar:{name}").bytes() {
        h ^= b as u128;
        h = h.wrapping_mul(0x0000000001000000000000000000013B);
    }
    (h & !(0xF000u128 << 64)) | (0x4000u128 << 64)
}

/// The score a value text gives: an integer, a rounded decimal, else 0 (TAB's `getValue`).
fn score_value(text: &str) -> i32 {
    let t = text::plain(text, false);
    let t = t.trim();
    t.parse::<i32>()
        .ok()
        .or_else(|| t.parse::<f64>().ok().map(|f| f.round() as i32))
        .unwrap_or(0)
}

impl Engine {
    /// Loads `config.yml`, `groups.yml`, `users.yml`, `animations.yml`, `messages.yml` and
    /// `playerdata.yml` from `dir` (missing files: defaults).
    pub fn load(dir: &Path) -> Engine {
        let mut warnings = Vec::new();
        let config = read_yaml(&dir.join("config.yml"), &mut warnings);
        let cfg = Config::from_yaml(&config);
        let animations = tabph::load_animations(&read_yaml(&dir.join("animations.yml"), &mut warnings));
        let messages = read_yaml(&dir.join("messages.yml"), &mut warnings);
        let (playerdata, playerdata_broken) = read_yaml_checked(&dir.join("playerdata.yml"), &mut warnings);
        let groups = PropertyFile::load(&dir.join("groups.yml"), "group").unwrap_or_else(|e| {
            warnings.push(e.clone());
            PropertyFile {
                path: Some(dir.join("groups.yml")),
                category: "group",
                broken: Some(e.clone()),
                ..PropertyFile::default()
            }
        });
        let users = PropertyFile::load(&dir.join("users.yml"), "user").unwrap_or_else(|e| {
            warnings.push(e.clone());
            PropertyFile {
                path: Some(dir.join("users.yml")),
                category: "user",
                broken: Some(e.clone()),
                ..PropertyFile::default()
            }
        });
        let mut props = Properties { groups, users };
        mysql::attach(&cfg.mysql, &mut props, &mut warnings);
        let mut engine = Engine::new(
            dir.to_owned(),
            cfg,
            props,
            animations,
            messages,
            playerdata,
            warnings,
        );
        engine.playerdata_broken = playerdata_broken;
        engine
    }

    pub fn new(
        dir: PathBuf,
        cfg: Config,
        props: Properties,
        animations: HashMap<String, tabph::Animation>,
        messages: Yaml,
        playerdata: Yaml,
        mut warnings: Vec<String>,
    ) -> Engine {
        let (sorting, w) = sorting::compile(&cfg.teams.sorting_types);
        warnings.extend(w);
        warnings.extend(cfg.config_warnings.iter().cloned());
        for feature in &cfg.unsupported {
            warnings.push(format!(
                "{feature} is enabled in config.yml but not supported by this port; ignored"
            ));
        }
        let ph = TabPlaceholders::new(&cfg, animations);
        Engine {
            dir,
            cfg,
            props,
            ph,
            messages,
            sorting,
            warnings,
            playerdata,
            players: Vec::new(),
            announced_bars: Vec::new(),
            announced_board: None,
            now_ms: 0,
            server: Server::default(),
            playerdata_broken: false,
            overwritten: HashSet::new(),
            cpu: Default::default(),
            skins: HashMap::new(),
            entry_names: HashMap::new(),
            occluded: HashSet::new(),
            proxy: ProxyState::new(new_instance_id()),
            bridge: Bridge::default(),
        }
        .with_proxy_flag()
    }

    fn with_proxy_flag(mut self) -> Engine {
        self.proxy.enabled = self.cfg.proxy_support.is_some();
        self
    }

    pub fn now_ms(&self) -> i64 {
        self.now_ms
    }

    /// The message `key` of `messages.yml` (a dotted path), or `default`.
    pub fn message(&self, key: &str, default: &str) -> String {
        let path: Vec<&str> = key.split('.').collect();
        let node = self.messages.at(&path);
        match node {
            Yaml::List(_) => node.str_list().join("\n"),
            other => other.as_str().unwrap_or_else(|| default.to_owned()),
        }
    }

    fn group_of(&self, uuid: u128, op_level: u8) -> String {
        if self.cfg.assign_groups_by_permissions {
            for g in &self.cfg.primary_group_finding_list {
                if integration::permission(uuid, &format!("tab.group.{g}"), 0) {
                    return g.clone();
                }
            }
            return "NONE".into();
        }
        integration::first(|i| i.primary_group(uuid)).unwrap_or_else(|| {
            let _ = op_level;
            "default".into()
        })
    }

    fn toggle_list(&self, section: &str) -> Vec<String> {
        self.playerdata.get(section).str_list()
    }

    fn save_toggle(&mut self, section: &str, uuid: u128, on: bool) {
        let id = uuid_string(uuid);
        let mut list = self.toggle_list(section);
        list.retain(|x| x != &id);
        if on {
            list.push(id);
        }
        self.playerdata
            .set(section, Yaml::List(list.into_iter().map(Yaml::Str).collect()));
        if !self.playerdata_broken {
            let _ = std::fs::write(self.dir.join("playerdata.yml"), yaml::write(&self.playerdata));
        }
    }

    /// Someone else sent this player's display name to every client (a nickname change): send
    /// ours again.
    pub fn display_name_overwritten(&mut self, uuid: u128) {
        self.overwritten.insert(uuid);
    }

    /// Display names as the game's own Tab list packets should carry them (viewer-independent),
    /// for the hub's `player_info`.
    pub fn list_names(&self) -> HashMap<u128, Value> {
        self.players
            .iter()
            .filter_map(|p| {
                p.sent
                    .names
                    .get(&p.tp.p.uuid)
                    .cloned()
                    .flatten()
                    .map(|v| (p.tp.p.uuid, v))
            })
            .collect()
    }

    /// One tick: players as they are now; the packets to send.
    pub fn tick(&mut self, now_ms: i64, mut server: Server, inputs: Vec<PlayerIn>) -> Vec<Packet> {
        self.now_ms = now_ms;
        self.ph.now_ms = now_ms;
        server.now_ms = now_ms;
        server.online = inputs.iter().filter(|p| !p.vanished).count();
        let mut out = Vec::new();
        self.ph.track.set(self.cpu.tracking());
        self.cpu.start_lap();
        self.sync_players(&server, inputs, &mut out);
        self.cpu.lap("Player tracking", "Joining, leaving and updating players");
        server.proxy_players = self.proxy.brief();
        server.global = self.cfg.global.clone();
        let snapshot: Vec<Player> = self.players.iter().map(|p| p.tp.p.clone()).collect();
        placeholders::update_snapshot(server.clone(), snapshot);
        self.server = server;
        if self.proxy.enabled {
            let local = self.proxy_local();
            self.proxy.sync_local(&local);
        }
        self.announced_bars.retain(|(_, until)| *until > now_ms);
        if self.announced_board.as_ref().is_some_and(|(_, until)| *until <= now_ms) {
            self.announced_board = None;
        }
        self.assign_team_names();
        self.cpu.lap("Sorting", "Choosing team names");
        let all: Vec<TabPlayer> = self.players.iter().map(|p| p.tp.clone()).collect();
        let targets: Vec<TargetTexts> = (0..all.len()).map(|i| self.target_texts(i, &all)).collect();
        self.cpu.lap("Placeholders", "Parsing player texts");
        self.bridge_tick(&all);
        self.cpu.lap("Bridge", "Answering the proxy");
        for vi in 0..self.players.len() {
            // A fake player has no client: its view of the list would be built for no one.
            if self.players[vi].fake {
                continue;
            }
            self.update_viewer(vi, &all, &targets, &mut out);
        }
        self.overwritten.clear();
        self.cpu.roll(now_ms, self.ph.take_timings());
        out
    }

    fn sync_players(&mut self, server: &Server, inputs: Vec<PlayerIn>, out: &mut Vec<Packet>) {
        let present: HashSet<u128> = inputs.iter().map(|p| p.uuid).collect();
        // Leavers: their team and scores go from everyone else.
        let gone: Vec<PState> = self
            .players
            .iter()
            .filter(|p| !present.contains(&p.tp.p.uuid))
            .cloned()
            .collect();
        self.players.retain(|p| present.contains(&p.tp.p.uuid));
        for g in &gone {
            let uuid = g.tp.p.uuid;
            self.ph.forget(uuid);
            self.skins.remove(&uuid);
            self.entry_names.remove(&uuid);
            self.occluded.retain(|(v, t)| *v != uuid && *t != uuid);
            for v in &mut self.players {
                if v.fake {
                    continue;
                }
                let to = v.tp.p.uuid;
                if let Some((team, _)) = v.sent.teams.remove(&uuid) {
                    out.push(Packet {
                        to,
                        out: Out::TeamRemove { name: team },
                    });
                }
                if let Some((owner, _, _)) = v.sent.list_scores.remove(&uuid) {
                    out.push(Packet {
                        to,
                        out: Out::ResetScore {
                            owner,
                            objective: LIST_OBJECTIVE.into(),
                        },
                    });
                }
                if let Some((owner, _, _)) = v.sent.below_scores.remove(&uuid) {
                    out.push(Packet {
                        to,
                        out: Out::ResetScore {
                            owner,
                            objective: BELOW_OBJECTIVE.into(),
                        },
                    });
                }
                v.sent.names.remove(&uuid);
            }
        }
        let scoreboard_list = self.toggle_list("scoreboard-off");
        let bossbar_list = self.toggle_list("bossbar-off");
        for input in inputs {
            let group = self.group_of(input.uuid, input.op_level);
            let bedrock = integration::first(|i| i.is_bedrock(input.uuid)).unwrap_or(false);
            let vanished = input.vanished || integration::first(|i| i.is_vanished(input.uuid)).unwrap_or(false);
            let existing = self.players.iter().position(|p| p.tp.p.uuid == input.uuid);
            match &input.skin {
                Some(s) => {
                    self.skins.insert(input.uuid, s.clone());
                }
                None => {
                    self.skins.remove(&input.uuid);
                }
            }
            if existing.is_none() {
                self.load_mysql_user(&input.name, input.uuid);
            }
            let joined_ms = existing.map(|i| self.players[i].joined_ms).unwrap_or(self.now_ms);
            let tp = TabPlayer {
                p: Player {
                    uuid: input.uuid,
                    name: input.name.clone(),
                    display_name: input.nickname.clone(),
                    dimension: input.dimension.clone(),
                    world: world_name(&server.level_name, &input.dimension),
                    position: input.position,
                    yaw: input.yaw,
                    pitch: input.pitch,
                    health: input.health,
                    ping: input.ping,
                    game_mode: input.game_mode,
                    op_level: input.op_level,
                    vanished,
                    invisible: input.invisible,
                    deaths: input.deaths,
                    entity_id: input.entity_id,
                    joined_at_ms: joined_ms,
                },
                group,
                bedrock,
                collision: existing
                    .and_then(|i| self.players[i].collision)
                    .unwrap_or(self.cfg.teams.enable_collision),
                version_id: input.protocol,
                version_name: version_name(input.protocol),
            };
            match existing {
                Some(i) => {
                    self.players[i].tp = tp;
                    self.players[i].fake = input.fake;
                }
                None => {
                    let id = uuid_string(input.uuid);
                    self.players.push(PState {
                        tp,
                        fake: input.fake,
                        joined_ms,
                        team: None,
                        sort_chars: String::new(),
                        scoreboard_toggled: self.cfg.scoreboard.remember_toggle_choice && scoreboard_list.contains(&id),
                        bossbar_toggled: self.cfg.bossbar.remember_toggle_choice && bossbar_list.contains(&id),
                        nametag_hidden: false,
                        nametag_view_hidden: false,
                        nametag_hidden_for: HashSet::new(),
                        opaque: false,
                        opaque_for: HashSet::new(),
                        opaque_view: false,
                        collision: None,
                        forced_board: None,
                        sent_bars: Vec::new(),
                        sent: Sent::default(),
                    });
                }
            }
        }
    }

    fn ctx<'a>(&'a self, all: &'a [TabPlayer], target: usize, viewer: usize) -> Ctx<'a> {
        Ctx {
            target: &all[target],
            viewer: &all[viewer],
            all,
            server: &self.server,
        }
    }

    fn property(&self, tp: &TabPlayer, property: &str, if_not_set: &str) -> String {
        let world = Some(tp.p.world.as_str());
        let server = Some(self.server.level_name.as_str());
        self.props
            .lookup(&tp.p.name, &tp.p.uuid_string(), &tp.group, property, world, server)
            .map(|f| f.value)
            .unwrap_or_else(|| if_not_set.to_owned())
    }

    fn component(&self, s: &str) -> Value {
        text::component(s, self.cfg.minimessage)
    }

    fn assign_team_names(&mut self) {
        if !self.cfg.teams.enabled {
            return;
        }
        let all: Vec<TabPlayer> = self.players.iter().map(|p| p.tp.clone()).collect();
        let case = self.cfg.teams.case_sensitive_sorting;
        let chars: Vec<String> = (0..all.len())
            .map(|i| {
                let ctx = self.ctx(&all, i, i);
                let mut parse = |s: &str| self.ph.parse(s, &ctx);
                let uuid = all[i].p.uuid;
                let op = all[i].p.op_level;
                let mut perm = |n: &str| integration::permission(uuid, n, op);
                self.sorting
                    .iter()
                    .map(|t| t.chars(&all[i].group, case, &mut parse, &mut perm))
                    .collect()
            })
            .collect();
        for i in 0..self.players.len() {
            let base = &chars[i];
            let keep = self.players[i].sort_chars == *base && self.players[i].team.is_some();
            if keep {
                continue;
            }
            let others: Vec<String> = self
                .players
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .filter_map(|(_, p)| p.team.clone())
                .collect();
            let name = sorting::team_name(base, &|n| others.iter().any(|o| o == n));
            self.players[i].team = Some(name);
            self.players[i].sort_chars = base.clone();
        }
    }

    fn target_texts(&self, i: usize, all: &[TabPlayer]) -> TargetTexts {
        let tp = &all[i];
        let ctx = self.ctx(all, i, i);
        let mut t = TargetTexts::default();
        if self.cfg.tablist_names.enabled {
            t.names_disabled = self
                .cfg
                .tablist_names
                .disable_condition
                .as_deref()
                .is_some_and(|c| self.ph.short_met(c, &ctx));
            t.tab_raw = format!(
                "{}{}{}",
                self.property(tp, "tabprefix", ""),
                self.property(tp, "customtabname", "%player%"),
                self.property(tp, "tabsuffix", "")
            );
            t.tab_rel = t.tab_raw.contains("%rel_") || t.tab_raw.contains("%viewer:");
            t.tab = self.component(&self.ph.parse(&t.tab_raw, &ctx));
        }
        if self.cfg.teams.enabled {
            t.teams_disabled = self
                .cfg
                .teams
                .disable_condition
                .as_deref()
                .is_some_and(|c| self.ph.short_met(c, &ctx));
            t.tag_prefix_raw = self.property(tp, "tagprefix", "");
            t.tag_suffix_raw = self.property(tp, "tagsuffix", "");
            t.tag_rel = [&t.tag_prefix_raw, &t.tag_suffix_raw]
                .iter()
                .any(|s| s.contains("%rel_") || s.contains("%viewer:"));
            let prefix = self.ph.parse(&t.tag_prefix_raw, &ctx);
            t.tag_color = text::last_color(&prefix, self.cfg.minimessage).map(|c| c.nearest_legacy() as i32);
            t.tag_prefix = self.component(&prefix);
            t.tag_suffix = self.component(&self.ph.parse(&t.tag_suffix_raw, &ctx));
        }
        if self.cfg.list_objective.enabled {
            t.list_value = score_value(&self.ph.parse(&self.cfg.list_objective.value, &ctx));
        }
        if self.cfg.belowname.enabled {
            t.below_value = score_value(&self.ph.parse(&self.cfg.belowname.value, &ctx));
        }
        t
    }

    fn condition(&self, cond: &Option<String>, ctx: &Ctx) -> bool {
        cond.as_deref().is_none_or(|c| self.ph.short_met(c, ctx))
    }

    fn update_viewer(&mut self, vi: usize, all: &[TabPlayer], targets: &[TargetTexts], out: &mut Vec<Packet>) {
        let to = all[vi].p.uuid;
        let mut sent = std::mem::take(&mut self.players[vi].sent);
        let push = |out: &mut Vec<Packet>, o: Out| out.push(Packet { to, out: o });
        let vctx = self.ctx(all, vi, vi);
        self.cpu.start_lap();

        // Header and footer.
        if self.cfg.header_footer.enabled {
            let disabled = self
                .cfg
                .header_footer
                .disable_condition
                .as_deref()
                .is_some_and(|c| self.ph.short_met(c, &vctx));
            let design = (!disabled)
                .then(|| {
                    self.cfg
                        .header_footer
                        .designs
                        .iter()
                        .find(|d| self.condition(&d.display_condition, &vctx))
                })
                .flatten();
            let wanted = design.map(|d| {
                let h = self.ph.parse(&d.header.join("\n"), &vctx);
                let f = self.ph.parse(&d.footer.join("\n"), &vctx);
                (h, f)
            });
            match wanted {
                Some((h, f)) => {
                    let pair = (self.component(&h), self.component(&f));
                    if sent.header.as_ref() != Some(&pair) {
                        push(
                            out,
                            Out::TabList {
                                header: pair.0.clone(),
                                footer: pair.1.clone(),
                                header_plain: text::plain(&h, self.cfg.minimessage),
                                footer_plain: text::plain(&f, self.cfg.minimessage),
                            },
                        );
                        sent.header = Some(pair);
                    }
                }
                None => {
                    if sent.header.take().is_some() {
                        push(
                            out,
                            Out::TabList {
                                header: json!({"text": ""}),
                                footer: json!({"text": ""}),
                                header_plain: String::new(),
                                footer_plain: String::new(),
                            },
                        );
                    }
                }
            }
        }

        self.cpu.lap("Header/Footer", "Updating");

        // Tab list names.
        if self.cfg.tablist_names.enabled {
            let mut changed = Vec::new();
            for (ti, t) in targets.iter().enumerate() {
                let uuid = all[ti].p.uuid;
                let wanted = if t.names_disabled {
                    // What the game itself would show: the nickname, if a mod set one.
                    all[ti].p.display_name.as_ref().map(|n| json!({ "text": n }))
                } else if t.tab_rel {
                    let ctx = self.ctx(all, ti, vi);
                    Some(self.component(&self.ph.parse(&t.tab_raw, &ctx)))
                } else {
                    Some(t.tab.clone())
                };
                let previous = sent.names.get(&uuid);
                let stale = self.overwritten.contains(&uuid);
                if previous != Some(&wanted) || stale {
                    // None of ours to take back: leave the game's own name (or nickname) alone.
                    if wanted.is_none() && previous.is_none_or(|p| p.is_none()) {
                        continue;
                    }
                    changed.push((uuid, wanted.clone()));
                    sent.names.insert(uuid, wanted);
                }
            }
            if !changed.is_empty() {
                push(out, Out::DisplayNames(changed));
            }
        }

        self.cpu.lap("Tablist name formatting", "Updating");

        // Teams: name tags and sorting.
        if self.cfg.teams.enabled {
            let viewer_hides = self.players[vi].nametag_view_hidden;
            for (ti, t) in targets.iter().enumerate() {
                let uuid = all[ti].p.uuid;
                let target = &self.players[ti];
                // The entry name a nick plugin gave the client; a changed one moves the member.
                let member = self.entry_name(&all[ti]);
                if sent.team_members.get(&uuid).is_some_and(|m| *m != member) {
                    if let Some((old, _)) = sent.teams.remove(&uuid) {
                        push(out, Out::TeamRemove { name: old });
                    }
                }
                let wanted = match (&target.team, t.teams_disabled) {
                    (Some(team), false) => {
                        let (prefix, suffix, color) = if t.tag_rel {
                            let ctx = self.ctx(all, ti, vi);
                            let p = self.ph.parse(&t.tag_prefix_raw, &ctx);
                            let color = text::last_color(&p, self.cfg.minimessage).map(|c| c.nearest_legacy() as i32);
                            (
                                self.component(&p),
                                self.component(&self.ph.parse(&t.tag_suffix_raw, &ctx)),
                                color,
                            )
                        } else {
                            (t.tag_prefix.clone(), t.tag_suffix.clone(), t.tag_color)
                        };
                        let hidden =
                            self.cfg.teams.invisible_nametags
                                || target.nametag_hidden
                                || target.nametag_hidden_for.contains(&self.players[vi].tp.p.uuid)
                                || self.occluded.contains(&(self.players[vi].tp.p.uuid, uuid))
                                || (viewer_hides && ti != vi);
                        let collision = target.collision.unwrap_or(self.cfg.teams.enable_collision);
                        Some((
                            team.clone(),
                            TeamParams {
                                display_name: json!({ "text": team }),
                                prefix,
                                suffix,
                                visibility: if hidden { 1 } else { 0 },
                                collision: if collision { 0 } else { 1 },
                                color,
                                options: if self.cfg.teams.can_see_friendly_invisibles {
                                    2
                                } else {
                                    0
                                },
                            },
                        ))
                    }
                    _ => None,
                };
                match (sent.teams.get(&uuid).cloned(), wanted) {
                    (None, Some((name, params))) => {
                        push(
                            out,
                            Out::TeamCreate {
                                name: name.clone(),
                                params: params.clone(),
                                players: vec![member.clone()],
                            },
                        );
                        sent.teams.insert(uuid, (name, params));
                        sent.team_members.insert(uuid, member);
                    }
                    (Some((old, _)), None) => {
                        push(out, Out::TeamRemove { name: old });
                        sent.teams.remove(&uuid);
                        sent.team_members.remove(&uuid);
                    }
                    (Some((old, old_params)), Some((name, params))) => {
                        if old != name {
                            push(out, Out::TeamRemove { name: old });
                            push(
                                out,
                                Out::TeamCreate {
                                    name: name.clone(),
                                    params: params.clone(),
                                    players: vec![member.clone()],
                                },
                            );
                            sent.teams.insert(uuid, (name, params));
                            sent.team_members.insert(uuid, member);
                        } else if old_params != params {
                            push(
                                out,
                                Out::TeamUpdate {
                                    name: name.clone(),
                                    params: params.clone(),
                                },
                            );
                            sent.teams.insert(uuid, (name, params));
                        }
                    }
                    (None, None) => {}
                }
            }
        }

        self.cpu.lap("Scoreboard teams", "Updating");

        // Player list objective.
        if self.cfg.list_objective.enabled {
            let c = &self.cfg.list_objective;
            let disabled = c
                .disable_condition
                .as_deref()
                .is_some_and(|d| self.ph.short_met(d, &vctx));
            if disabled {
                if sent.list_objective.take().is_some() {
                    push(
                        out,
                        Out::Objective {
                            name: LIST_OBJECTIVE.into(),
                            method: ObjectiveMethod::Remove,
                            title: json!({"text": ""}),
                            hearts: false,
                            format: None,
                        },
                    );
                    sent.list_scores.clear();
                }
            } else {
                let title = self.component(&self.ph.parse(&c.title, &vctx));
                match &sent.list_objective {
                    None => {
                        push(
                            out,
                            Out::Objective {
                                name: LIST_OBJECTIVE.into(),
                                method: ObjectiveMethod::Add,
                                title: title.clone(),
                                hearts: c.hearts,
                                format: None,
                            },
                        );
                        push(
                            out,
                            Out::DisplaySlot {
                                slot: 0,
                                objective: LIST_OBJECTIVE.into(),
                            },
                        );
                        sent.list_objective = Some(title);
                    }
                    Some(old) if *old != title => {
                        push(
                            out,
                            Out::Objective {
                                name: LIST_OBJECTIVE.into(),
                                method: ObjectiveMethod::Change,
                                title: title.clone(),
                                hearts: c.hearts,
                                format: None,
                            },
                        );
                        sent.list_objective = Some(title);
                    }
                    _ => {}
                }
                for (ti, t) in targets.iter().enumerate() {
                    let fancy = (!c.fancy_value.is_empty()).then(|| {
                        let ctx = self.ctx(all, ti, vi);
                        self.component(&self.ph.parse(&c.fancy_value, &ctx))
                    });
                    let entry = (self.entry_name(&all[ti]), t.list_value, fancy.clone());
                    if sent.list_scores.get(&all[ti].p.uuid) != Some(&entry) {
                        if let Some((old, _, _)) = sent.list_scores.get(&all[ti].p.uuid) {
                            if *old != entry.0 {
                                push(out, Out::ResetScore { owner: old.clone(), objective: LIST_OBJECTIVE.into() });
                            }
                        }
                        push(
                            out,
                            Out::Score {
                                owner: entry.0.clone(),
                                objective: LIST_OBJECTIVE.into(),
                                score: entry.1,
                                display: None,
                                format: fancy.map(NumberFmt::Fixed),
                            },
                        );
                        sent.list_scores.insert(all[ti].p.uuid, entry);
                    }
                }
            }
        }

        self.cpu.lap("Playerlist objective", "Updating");

        // Below name objective.
        if self.cfg.belowname.enabled {
            let c = &self.cfg.belowname;
            let disabled = c
                .disable_condition
                .as_deref()
                .is_some_and(|d| self.ph.short_met(d, &vctx));
            if disabled {
                if sent.below_objective.take().is_some() {
                    push(
                        out,
                        Out::Objective {
                            name: BELOW_OBJECTIVE.into(),
                            method: ObjectiveMethod::Remove,
                            title: json!({"text": ""}),
                            hearts: false,
                            format: None,
                        },
                    );
                    sent.below_scores.clear();
                }
            } else {
                let title = self.component(&self.ph.parse(&c.title, &vctx));
                let default = (!c.fancy_value_default.is_empty())
                    .then(|| self.component(&self.ph.parse(&c.fancy_value_default, &vctx)));
                let wanted = (title.clone(), default.clone());
                let method = match &sent.below_objective {
                    None => Some(ObjectiveMethod::Add),
                    Some(old) if *old != wanted => Some(ObjectiveMethod::Change),
                    _ => None,
                };
                if let Some(method) = method {
                    let add = method == ObjectiveMethod::Add;
                    push(
                        out,
                        Out::Objective {
                            name: BELOW_OBJECTIVE.into(),
                            method,
                            title,
                            hearts: false,
                            format: default.map(NumberFmt::Fixed),
                        },
                    );
                    if add {
                        push(
                            out,
                            Out::DisplaySlot {
                                slot: 2,
                                objective: BELOW_OBJECTIVE.into(),
                            },
                        );
                    }
                    sent.below_objective = Some(wanted);
                }
                for (ti, t) in targets.iter().enumerate() {
                    let fancy = (!c.fancy_value.is_empty()).then(|| {
                        let ctx = self.ctx(all, ti, vi);
                        self.component(&self.ph.parse(&c.fancy_value, &ctx))
                    });
                    let entry = (self.entry_name(&all[ti]), t.below_value, fancy.clone());
                    if sent.below_scores.get(&all[ti].p.uuid) != Some(&entry) {
                        if let Some((old, _, _)) = sent.below_scores.get(&all[ti].p.uuid) {
                            if *old != entry.0 {
                                push(out, Out::ResetScore { owner: old.clone(), objective: BELOW_OBJECTIVE.into() });
                            }
                        }
                        push(
                            out,
                            Out::Score {
                                owner: entry.0.clone(),
                                objective: BELOW_OBJECTIVE.into(),
                                score: entry.1,
                                display: None,
                                format: fancy.map(NumberFmt::Fixed),
                            },
                        );
                        sent.below_scores.insert(all[ti].p.uuid, entry);
                    }
                }
            }
        }

        self.cpu.lap("Belowname", "Updating");

        // Sidebar scoreboard.
        if self.cfg.scoreboard.enabled {
            let ps = &self.players[vi];
            let sc = &self.cfg.scoreboard;
            let visible =
                ps.scoreboard_toggled == sc.hidden_by_default && self.now_ms - ps.joined_ms >= sc.delay_on_join_ms;
            let board: Option<&Board> = if !visible {
                None
            } else if let Some(name) = ps
                .forced_board
                .as_ref()
                .or(self.announced_board.as_ref().map(|(n, _)| n))
            {
                sc.boards.iter().find(|b| b.name.eq_ignore_ascii_case(name))
            } else {
                sc.boards.iter().find(|b| self.condition(&b.display_condition, &vctx))
            };
            match board {
                Some(b) => {
                    let title = self.component(&self.ph.parse(&b.title, &vctx));
                    let lines: Vec<(Value, Option<Value>)> = b
                        .lines
                        .iter()
                        .map(|l| {
                            let parsed = self.ph.parse(l, &vctx);
                            match parsed.split_once("||") {
                                Some((left, right)) => (self.component(left), Some(self.component(right))),
                                None => (self.component(&parsed), None),
                            }
                        })
                        .collect();
                    self.sidebar_diff(&mut sent, title, lines, to, out);
                }
                None => {
                    if sent.sidebar.take().is_some() {
                        push(
                            out,
                            Out::Objective {
                                name: SIDEBAR_OBJECTIVE.into(),
                                method: ObjectiveMethod::Remove,
                                title: json!({"text": ""}),
                                hearts: false,
                                format: None,
                            },
                        );
                    }
                }
            }
        }

        self.cpu.lap("Scoreboard", "Updating");

        // Boss bars.
        if self.cfg.bossbar.enabled {
            let ps = &self.players[vi];
            let bb = &self.cfg.bossbar;
            let shown = ps.bossbar_toggled == bb.hidden_by_default;
            let mut wanted: HashMap<String, BarSent> = HashMap::new();
            for bar in &bb.bars {
                let announced = self
                    .announced_bars
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case(&bar.name));
                let sent_alone = shown
                    && ps
                        .sent_bars
                        .iter()
                        .any(|(n, until)| n.eq_ignore_ascii_case(&bar.name) && *until > self.now_ms);
                let on = if bar.announcement {
                    announced || sent_alone
                } else {
                    (shown && self.condition(&bar.display_condition, &vctx)) || sent_alone
                };
                if !on {
                    continue;
                }
                let progress = self
                    .ph
                    .parse(&bar.progress, &vctx)
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(100.0);
                wanted.insert(
                    bar.name.clone(),
                    BarSent {
                        id: bar_id(&bar.name),
                        title: self.component(&self.ph.parse(&bar.text, &vctx)),
                        progress: (progress / 100.0).clamp(0.0, 1.0),
                        color: bar_color(&self.ph.parse(&bar.color, &vctx)),
                        overlay: bar_overlay(&self.ph.parse(&bar.style, &vctx)),
                    },
                );
            }
            let old: Vec<String> = sent.bars.keys().cloned().collect();
            for name in old {
                if !wanted.contains_key(&name) {
                    if let Some(b) = sent.bars.remove(&name) {
                        push(
                            out,
                            Out::Boss {
                                id: b.id,
                                action: BossOut::Remove,
                            },
                        );
                    }
                }
            }
            // In config order, so they stack as listed.
            for bar in &bb.bars {
                let Some(w) = wanted.remove(&bar.name) else { continue };
                match sent.bars.get(&bar.name) {
                    None => push(
                        out,
                        Out::Boss {
                            id: w.id,
                            action: BossOut::Add {
                                title: w.title.clone(),
                                progress: w.progress,
                                color: w.color,
                                overlay: w.overlay,
                            },
                        },
                    ),
                    Some(s) => {
                        if s.title != w.title {
                            push(
                                out,
                                Out::Boss {
                                    id: w.id,
                                    action: BossOut::Title(w.title.clone()),
                                },
                            );
                        }
                        if s.progress != w.progress {
                            push(
                                out,
                                Out::Boss {
                                    id: w.id,
                                    action: BossOut::Progress(w.progress),
                                },
                            );
                        }
                        if s.color != w.color || s.overlay != w.overlay {
                            push(
                                out,
                                Out::Boss {
                                    id: w.id,
                                    action: BossOut::Style {
                                        color: w.color,
                                        overlay: w.overlay,
                                    },
                                },
                            );
                        }
                    }
                }
                sent.bars.insert(bar.name.clone(), w);
            }
        }

        self.cpu.lap("BossBar", "Updating");

        // Spectators shown as survival to the others.
        if self.cfg.prevent_spectator_effect && self.now_ms - sent.last_spectator_ms >= SPECTATOR_RESEND_MS {
            let bypass = integration::permission(to, "tab.spectatorbypass", 0);
            if !bypass {
                let list: Vec<(u128, i32)> = all
                    .iter()
                    .enumerate()
                    .filter(|(ti, p)| *ti != vi && p.p.game_mode == 3)
                    .map(|(_, p)| (p.p.uuid, 0))
                    .collect();
                if !list.is_empty() {
                    push(out, Out::GameModes(list));
                }
            }
            sent.last_spectator_ms = self.now_ms;
        }

        self.cpu.lap("Spectator fix", "Updating");

        // Ping spoof.
        if let Some(value) = self.cfg.ping_spoof {
            if self.now_ms - sent.last_ping_spoof_ms >= PING_SPOOF_RESEND_MS || sent.last_ping_spoof_ms == 0 {
                push(out, Out::Latency(all.iter().map(|p| (p.p.uuid, value)).collect()));
                sent.last_ping_spoof_ms = self.now_ms.max(1);
            }
        }

        self.cpu.lap("Ping spoof", "Updating");

        self.layout_for(vi, all, targets, &mut sent, out);
        self.cpu.lap("Layout", "Updating");
        self.per_world_for(vi, all, &mut sent, out);
        self.cpu.lap("Per world playerlist", "Updating");
        self.global_for(vi, all, &mut sent, out);
        self.cpu.lap("Global playerlist", "Updating");
        self.players[vi].sent = sent;
    }

    fn sidebar_diff(
        &self,
        sent: &mut Sent,
        title: Value,
        lines: Vec<(Value, Option<Value>)>,
        to: u128,
        out: &mut Vec<Packet>,
    ) {
        let owner = |i: usize| format!("TAB-Line-{i:02}");
        let score = |len: usize, i: usize| (len - i) as i32;
        let line_out = |i: usize, len: usize, l: &(Value, Option<Value>)| Out::Score {
            owner: owner(i),
            objective: SIDEBAR_OBJECTIVE.into(),
            score: score(len, i),
            display: Some(l.0.clone()),
            format: Some(match &l.1 {
                Some(r) => NumberFmt::Fixed(r.clone()),
                None => NumberFmt::Blank,
            }),
        };
        let lines: Vec<_> = lines.into_iter().take(15).collect();
        let len = lines.len();
        match sent.sidebar.take() {
            None => {
                out.push(Packet {
                    to,
                    out: Out::Objective {
                        name: SIDEBAR_OBJECTIVE.into(),
                        method: ObjectiveMethod::Add,
                        title: title.clone(),
                        hearts: false,
                        format: Some(NumberFmt::Blank),
                    },
                });
                out.push(Packet {
                    to,
                    out: Out::DisplaySlot {
                        slot: 1,
                        objective: SIDEBAR_OBJECTIVE.into(),
                    },
                });
                for (i, l) in lines.iter().enumerate() {
                    out.push(Packet {
                        to,
                        out: line_out(i, len, l),
                    });
                }
            }
            Some(old) => {
                if old.title != title {
                    out.push(Packet {
                        to,
                        out: Out::Objective {
                            name: SIDEBAR_OBJECTIVE.into(),
                            method: ObjectiveMethod::Change,
                            title: title.clone(),
                            hearts: false,
                            format: Some(NumberFmt::Blank),
                        },
                    });
                }
                let resend_all = old.lines.len() != len;
                for (i, l) in lines.iter().enumerate() {
                    if resend_all || old.lines.get(i) != Some(l) {
                        out.push(Packet {
                            to,
                            out: line_out(i, len, l),
                        });
                    }
                }
                for i in len..old.lines.len() {
                    out.push(Packet {
                        to,
                        out: Out::ResetScore {
                            owner: owner(i),
                            objective: SIDEBAR_OBJECTIVE.into(),
                        },
                    });
                }
            }
        }
        sent.sidebar = Some(SidebarSent { title, lines });
    }

    /// A line for the console when `debug` is on in config.yml.
    pub fn debug_note(&self, line: &str) {
        if self.cfg.debug {
            eprintln!("[tab] [debug] {line}");
        }
    }

    // ---- nick compatibility ---------------------------------------------------------------------

    /// The name of a player's own tab-list entry as the client holds it: the profile name, or the
    /// one a nick plugin put in its place (`NickCompatibility`).
    pub(crate) fn entry_name(&self, tp: &TabPlayer) -> String {
        self.entry_names.get(&tp.p.uuid).cloned().unwrap_or_else(|| tp.p.name.clone())
    }

    /// An entry-add packet another sender sent `viewer` carries `name` for `uuid`. TAB reads it only
    /// when the player is its own receiver (`packetPlayer == packetReceiver`); a changed name makes
    /// the teams and the list and below-name scores name the new entry. True when it changed.
    pub fn set_entry_name(&mut self, viewer: u128, uuid: u128, name: &str) -> bool {
        if viewer != uuid || name.is_empty() {
            return false;
        }
        let Some(i) = self.index_of(uuid) else { return false };
        let current = self.entry_name(&self.players[i].tp);
        if current == name {
            return false;
        }
        if name == self.players[i].tp.p.name {
            self.entry_names.remove(&uuid);
        } else {
            self.entry_names.insert(uuid, name.to_owned());
        }
        true
    }

    // ---- opaque name tags -----------------------------------------------------------------------

    /// `/tab nametag opaque <player> [viewer]`: the name tag shows only with a line of sight. Shows
    /// the tag first, as TAB does (the hide command's reason is lifted).
    pub(crate) fn set_opaque(&mut self, p: usize, viewer: Option<usize>) {
        match viewer {
            Some(v) => {
                let u = self.players[v].tp.p.uuid;
                self.players[p].nametag_hidden_for.remove(&u);
                self.players[p].opaque_for.insert(u);
            }
            None => {
                self.players[p].nametag_hidden = false;
                self.players[p].opaque = true;
            }
        }
    }

    /// Show, hide and toggle end the opaque mode they are used on.
    pub(crate) fn clear_opaque(&mut self, p: usize, viewer: Option<usize>) {
        match viewer {
            Some(v) => {
                let u = self.players[v].tp.p.uuid;
                self.players[p].opaque_for.remove(&u);
                self.occluded.remove(&(u, self.players[p].tp.p.uuid));
            }
            None => {
                self.players[p].opaque = false;
                self.players[p].opaque_for.clear();
                let t = self.players[p].tp.p.uuid;
                self.occluded.retain(|(_, target)| *target != t);
            }
        }
    }

    pub(crate) fn set_opaque_view(&mut self, i: usize, on: bool) {
        self.players[i].opaque_view = on;
        if on {
            self.players[i].nametag_view_hidden = false;
        } else {
            let v = self.players[i].tp.p.uuid;
            self.occluded.retain(|(viewer, _)| *viewer != v);
        }
    }

    /// `VisibilityManager.isOpaqueNameTagMode`: does `viewer` see `target`'s name tag only with a
    /// line of sight?
    fn opaque_applies(&self, viewer: usize, target: usize) -> bool {
        let (v, t) = (&self.players[viewer], &self.players[target]);
        viewer != target && (t.opaque || t.opaque_for.contains(&v.tp.p.uuid) || v.opaque_view)
    }

    /// The pairs (viewer, target) whose line of sight the game has to answer: opaque mode applies,
    /// both are in the same world and the viewer can see the target at all. Pairs that do not apply
    /// any more lose their verdict.
    pub fn opaque_pairs(&mut self) -> Vec<(u128, u128)> {
        let mut pairs = Vec::new();
        if !self.cfg.teams.enabled {
            return pairs;
        }
        for v in 0..self.players.len() {
            if self.players[v].fake {
                continue;
            }
            for t in 0..self.players.len() {
                if !self.opaque_applies(v, t) {
                    continue;
                }
                let (vp, tp) = (&self.players[v].tp.p, &self.players[t].tp.p);
                if vp.world == tp.world && !tp.vanished {
                    pairs.push((vp.uuid, tp.uuid));
                }
            }
        }
        self.occluded.retain(|pair| pairs.contains(pair));
        pairs
    }

    /// The game's answers: (viewer, target, has line of sight). A target the viewer cannot see
    /// (another world) is hidden whatever the game says, as `canSeeForOpaque` has it.
    pub fn set_occlusion(&mut self, results: &[(u128, u128, bool)]) {
        for (viewer, target, sees) in results {
            if *sees {
                self.occluded.remove(&(*viewer, *target));
            } else {
                self.occluded.insert((*viewer, *target));
            }
        }
    }

    // ---- per-world playerlist and layout --------------------------------------------------------

    fn per_world_for(&self, vi: usize, all: &[TabPlayer], sent: &mut Sent, out: &mut Vec<Packet>) {
        let Some(pw) = &self.cfg.per_world else { return };
        let me = &all[vi];
        let to = me.p.uuid;
        let bypass = integration::permission(to, "tab.bypass", me.p.op_level);
        let sees_vanished = integration::permission(to, "tab.seevanished", me.p.op_level);
        let wanted: HashSet<u128> = all
            .iter()
            .enumerate()
            .filter(|(ti, t)| {
                *ti != vi && (!pw.sees(&me.p.world, &t.p.world, bypass) || (t.p.vanished && !sees_vanished))
            })
            .map(|(_, t)| t.p.uuid)
            .collect();
        let list = sent.layout.is_none();
        let mut hide: Vec<u128> = wanted.difference(&sent.pw_hidden).copied().collect();
        let mut show: Vec<u128> = sent
            .pw_hidden
            .difference(&wanted)
            .copied()
            .filter(|u| all.iter().any(|t| t.p.uuid == *u))
            .collect();
        hide.sort_unstable();
        show.sort_unstable();
        if !hide.is_empty() {
            out.push(Packet { to, out: Out::EntityVisible { ids: hide, visible: false, list } });
        }
        if !show.is_empty() {
            out.push(Packet { to, out: Out::EntityVisible { ids: show, visible: true, list } });
        }
        sent.pw_hidden = wanted;
    }

    /// The skin a layout skin text names, when this port can resolve it.
    fn skin_of(&self, spec: &str) -> Option<(String, String)> {
        if let Some(rest) = spec.strip_prefix("signed_texture:") {
            let mut it = rest.splitn(2, ';');
            let value = it.next().filter(|v| !v.is_empty())?;
            return Some((value.to_owned(), it.next().unwrap_or("").to_owned()));
        }
        if let Some(name) = spec.strip_prefix("player:") {
            let p = self.players.iter().find(|p| p.tp.p.name.eq_ignore_ascii_case(name.trim()))?;
            return self.skins.get(&p.tp.p.uuid).cloned();
        }
        None
    }

    /// The tab-list text of `ti` for `vi`: their TAB format, or their name.
    fn tab_display(&self, vi: usize, ti: usize, all: &[TabPlayer], targets: &[TargetTexts]) -> Value {
        let t = &targets[ti];
        if self.cfg.tablist_names.enabled && !t.names_disabled {
            if t.tab_rel {
                self.component(&self.ph.parse(&t.tab_raw, &self.ctx(all, ti, vi)))
            } else {
                t.tab.clone()
            }
        } else {
            json!({ "text": all[ti].p.name })
        }
    }

    fn layout_for(&self, vi: usize, all: &[TabPlayer], targets: &[TargetTexts], sent: &mut Sent, out: &mut Vec<Packet>) {
        let Some(lc) = &self.cfg.layout else { return };
        let me = &all[vi];
        let to = me.p.uuid;
        let vctx = self.ctx(all, vi, vi);
        let def = if me.bedrock { None } else { lc.layouts.iter().find(|l| self.condition(&l.condition, &vctx)) };
        let Some(def) = def else {
            if let Some(old) = sent.layout.take() {
                self.layout_remove(to, lc.direction, &old, all, out);
            }
            return;
        };
        if sent.layout.as_ref().is_some_and(|l| l.name != def.name) {
            if let Some(old) = sent.layout.take() {
                self.layout_remove(to, lc.direction, &old, all, out);
            }
        }
        let mut state = sent.layout.take().unwrap_or(LayoutSent {
            name: def.name.clone(),
            slots: Default::default(),
            hidden: HashSet::new(),
        });

        let mut wanted: std::collections::BTreeMap<u32, SlotSig> = Default::default();
        // Fixed slots.
        for f in &def.fixed {
            let text = self.ph.parse(&f.text, &vctx);
            let latency = match f.ping.as_deref().map(|p| self.ph.parse(p, &vctx)) {
                Some(p) => p.trim().parse::<f64>().map(|v| v.round() as i32).unwrap_or(lc.empty_ping),
                None => lc.empty_ping,
            };
            let skin = self.skin_of(&self.ph.parse(&def.fixed_skin(lc, f), &vctx));
            wanted.insert(f.slot, SlotSig { skin, latency, display: self.component(&text), who: None });
        }
        // Players, in the order the sorting gives, as the viewer may see them.
        let sees_vanished = integration::permission(to, "tab.seevanished", me.p.op_level);
        let mut order: Vec<usize> = (0..all.len())
            .filter(|&ti| ti == vi || !all[ti].p.vanished || sees_vanished)
            .collect();
        order.sort_by(|&a, &b| {
            let (ta, tb) = (self.players[a].team.as_deref().unwrap_or(""), self.players[b].team.as_deref().unwrap_or(""));
            ta.cmp(tb).then_with(|| all[a].p.name.to_lowercase().cmp(&all[b].p.name.to_lowercase()))
        });
        let placed = layout::assign(lc, def, &order, &mut |g, p| {
            g.condition.as_deref().is_none_or(|c| self.ph.short_met(c, &self.ctx(all, p, vi)))
        });
        for (_, g) in &def.groups {
            for slot in &g.slots {
                let sig = match placed.get(slot) {
                    Some(Content::Player(ti)) => SlotSig {
                        skin: self.skins.get(&all[*ti].p.uuid).cloned(),
                        latency: self.cfg.ping_spoof.unwrap_or(all[*ti].p.ping),
                        display: self.tab_display(vi, *ti, all, targets),
                        who: Some(all[*ti].p.uuid),
                    },
                    Some(Content::Text(t)) => SlotSig {
                        skin: self.skin_of(def.empty_skin(lc, *slot).as_str()),
                        latency: lc.empty_ping,
                        display: self.component(t),
                        who: None,
                    },
                    None => continue,
                };
                wanted.insert(*slot, sig);
            }
        }
        // Everything else is an empty entry.
        for slot in def.empty_slots() {
            wanted.insert(
                slot,
                SlotSig {
                    skin: self.skin_of(def.empty_skin(lc, slot).as_str()),
                    latency: lc.empty_ping,
                    display: json!({ "text": "" }),
                    who: None,
                },
            );
        }

        let entry = |slot: u32, sig: &SlotSig| ListEntry {
            uuid: layout::slot_uuid(lc.direction, slot),
            skin: sig.skin.clone(),
            latency: sig.latency,
            display: sig.display.clone(),
            order: layout::list_order(lc.direction, slot),
        };
        let (mut removes, mut adds, mut names, mut latencies) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (slot, sig) in &wanted {
            let id = layout::slot_uuid(lc.direction, *slot);
            match state.slots.get(slot) {
                None => adds.push(entry(*slot, sig)),
                Some(old) if old.skin != sig.skin || old.who != sig.who => {
                    removes.push(id);
                    adds.push(entry(*slot, sig));
                }
                Some(old) => {
                    if old.display != sig.display {
                        names.push((id, sig.display.clone()));
                    }
                    if old.latency != sig.latency {
                        latencies.push((id, sig.latency));
                    }
                }
            }
        }
        for slot in state.slots.keys() {
            if !wanted.contains_key(slot) {
                removes.push(layout::slot_uuid(lc.direction, *slot));
            }
        }
        let mut push = |o: Out| out.push(Packet { to, out: o });
        if !removes.is_empty() {
            push(Out::ListRemove(removes));
        }
        if !adds.is_empty() {
            push(Out::ListAdd(adds));
        }
        if !names.is_empty() {
            push(Out::ListNames(names));
        }
        if !latencies.is_empty() {
            push(Out::Latency(latencies));
        }
        // The real entries leave the list while a layout shows (`hideAllPlayers`).
        state.hidden.retain(|u| all.iter().any(|t| t.p.uuid == *u));
        let fresh: Vec<u128> = all.iter().map(|t| t.p.uuid).filter(|u| !state.hidden.contains(u)).collect();
        if !fresh.is_empty() {
            state.hidden.extend(fresh.iter().copied());
            push(Out::Listed { ids: fresh, listed: false });
        }
        state.slots = wanted;
        sent.layout = Some(state);
    }

    /// A layout taken away: its entries removed and the real ones listed again (`showAllPlayers`).
    fn layout_remove(&self, to: u128, direction: layout::Direction, old: &LayoutSent, all: &[TabPlayer], out: &mut Vec<Packet>) {
        let ids: Vec<u128> = old.slots.keys().map(|s| layout::slot_uuid(direction, *s)).collect();
        if !ids.is_empty() {
            out.push(Packet { to, out: Out::ListRemove(ids) });
        }
        let real: Vec<u128> = all.iter().map(|t| t.p.uuid).filter(|u| old.hidden.contains(u)).collect();
        if !real.is_empty() {
            out.push(Packet { to, out: Out::Listed { ids: real, listed: true } });
        }
    }

    /// Everything this mod showed, taken away (before a reload or at stop), and the sent state
    /// forgotten so the next tick sends it all again.
    pub fn unload(&mut self) -> Vec<Packet> {
        let mut out = Vec::new();
        let online: Vec<u128> = self.players.iter().map(|p| p.tp.p.uuid).collect();
        let direction = self.cfg.layout.as_ref().map(|l| l.direction);
        for v in &mut self.players {
            let to = v.tp.p.uuid;
            let s = std::mem::take(&mut v.sent);
            if let (Some(l), Some(direction)) = (&s.layout, direction) {
                let ids: Vec<u128> = l.slots.keys().map(|slot| layout::slot_uuid(direction, *slot)).collect();
                if !ids.is_empty() {
                    out.push(Packet { to, out: Out::ListRemove(ids) });
                }
                let real: Vec<u128> = online.iter().copied().filter(|u| l.hidden.contains(u)).collect();
                if !real.is_empty() {
                    out.push(Packet { to, out: Out::Listed { ids: real, listed: true } });
                }
            }
            if !s.global.is_empty() {
                let mut ids: Vec<u128> = s.global.keys().copied().collect();
                ids.sort_unstable();
                out.push(Packet { to, out: Out::ListRemove(ids) });
            }
            let back: Vec<u128> = s.pw_hidden.iter().copied().filter(|u| online.contains(u)).collect();
            if !back.is_empty() {
                out.push(Packet {
                    to,
                    out: Out::EntityVisible { ids: back, visible: true, list: s.layout.is_none() },
                });
            }
            if s.header.is_some() {
                out.push(Packet {
                    to,
                    out: Out::TabList {
                        header: json!({"text": ""}),
                        footer: json!({"text": ""}),
                        header_plain: String::new(),
                        footer_plain: String::new(),
                    },
                });
            }
            let names: Vec<(u128, Option<Value>)> = s
                .names
                .iter()
                .filter(|(_, v)| v.is_some())
                .map(|(u, _)| (*u, None))
                .collect();
            if !names.is_empty() {
                out.push(Packet {
                    to,
                    out: Out::DisplayNames(names),
                });
            }
            for (_, (team, _)) in s.teams {
                out.push(Packet {
                    to,
                    out: Out::TeamRemove { name: team },
                });
            }
            for (present, name) in [
                (s.list_objective.is_some(), LIST_OBJECTIVE),
                (s.below_objective.is_some(), BELOW_OBJECTIVE),
                (s.sidebar.is_some(), SIDEBAR_OBJECTIVE),
            ] {
                if present {
                    out.push(Packet {
                        to,
                        out: Out::Objective {
                            name: name.into(),
                            method: ObjectiveMethod::Remove,
                            title: json!({"text": ""}),
                            hearts: false,
                            format: None,
                        },
                    });
                }
            }
            for (_, b) in s.bars {
                out.push(Packet {
                    to,
                    out: Out::Boss {
                        id: b.id,
                        action: BossOut::Remove,
                    },
                });
            }
            v.team = None;
            v.sort_chars.clear();
        }
        out
    }

    /// Reloads the files (or the database), keeping the online players and their toggles.
    pub fn reload(&mut self) -> Vec<Packet> {
        let out = self.unload();
        let mut fresh = Engine::load(&self.dir);
        fresh.players = std::mem::take(&mut self.players);
        fresh.now_ms = self.now_ms;
        fresh.server = self.server.clone();
        fresh.bridge = std::mem::take(&mut self.bridge);
        let mut proxy = std::mem::take(&mut self.proxy);
        proxy.enabled = fresh.cfg.proxy_support.is_some();
        if !proxy.enabled {
            proxy.clear();
        }
        fresh.proxy = proxy;
        let online: Vec<(String, u128)> = fresh
            .players
            .iter()
            .map(|p| (p.tp.p.name.clone(), p.tp.p.uuid))
            .collect();
        for (name, uuid) in &online {
            fresh.load_mysql_user(name, *uuid);
        }
        *self = fresh;
        out
    }

    /// `MySQLUserConfiguration.load`: a player's rows, read on join (and after a reload for the
    /// players online). Nothing happens while the users are in a file.
    fn load_mysql_user(&mut self, name: &str, uuid: u128) {
        if let Err(e) = self.props.users.load_user_rows(name, &uuid_string(uuid)) {
            eprintln!("[tab] Failed to execute MySQL query due to error: {e}");
        }
    }

    // ---- what commands change ------------------------------------------------------------

    // ---- the game's packets on their way out (anti-override) ----------------------------------

    /// Whether a team packet to `viewer` would move a player TAB put in a team of its own into
    /// another one (a datapack's or `/team`'s `SetPlayerTeam`).
    pub fn guard_team(&self, viewer: u128, team: &str, players: &[String]) -> bool {
        if !self.cfg.teams.enabled || players.is_empty() {
            return false;
        }
        let Some(v) = self.index_of(viewer) else { return false };
        let sent = &self.players[v].sent;
        if sent.teams.values().any(|(name, _)| name == team) {
            return false;
        }
        self.players.iter().any(|p| {
            sent.teams.contains_key(&p.tp.p.uuid) && players.iter().any(|n| n == &p.tp.p.name)
        })
    }

    pub fn guard_latency(&self) -> Option<i32> {
        self.cfg.ping_spoof
    }

    pub fn guard_game_mode(&self, viewer: u128, uuid: u128, mode: i32) -> Option<i32> {
        if !self.cfg.prevent_spectator_effect || mode != 3 || uuid == viewer {
            return None;
        }
        (!integration::permission(viewer, "tab.spectatorbypass", 0)).then_some(0)
    }

    pub fn guard_display_name(&self, viewer: u128, uuid: u128) -> Option<Value> {
        let v = self.index_of(viewer)?;
        self.players[v].sent.names.get(&uuid).cloned().flatten()
    }

    /// The client of `viewer` may have lost what we sent it: send everything again.
    pub fn forget_viewer(&mut self, viewer: u128) {
        if let Some(i) = self.index_of(viewer) {
            self.players[i].sent = Sent::default();
        }
    }

    pub(crate) fn find(&self, name: &str) -> Option<usize> {
        self.players.iter().position(|p| p.tp.p.name.eq_ignore_ascii_case(name))
    }

    pub(crate) fn index_of(&self, uuid: u128) -> Option<usize> {
        self.players.iter().position(|p| p.tp.p.uuid == uuid)
    }

    pub(crate) fn tab_player(&self, i: usize) -> &TabPlayer {
        &self.players[i].tp
    }

    pub(crate) fn all_players(&self) -> Vec<TabPlayer> {
        self.players.iter().map(|p| p.tp.clone()).collect()
    }

    pub(crate) fn parse_for(&self, i: usize, text: &str) -> String {
        let all = self.all_players();
        let ctx = self.ctx(&all, i, i);
        self.ph.parse(text, &ctx)
    }

    pub(crate) fn property_of(&self, i: usize, property: &str) -> (String, String) {
        let tp = &self.players[i].tp;
        let world = Some(tp.p.world.as_str());
        let server = Some(self.server.level_name.as_str());
        match self
            .props
            .lookup(&tp.p.name, &tp.p.uuid_string(), &tp.group, property, world, server)
        {
            Some(f) => (f.value, f.source),
            None => (String::new(), "None".into()),
        }
    }

    pub(crate) fn team_of(&self, i: usize) -> Option<String> {
        self.players[i].team.clone()
    }

    pub(crate) fn sort_values(&self, i: usize) -> Vec<(String, String)> {
        let all = self.all_players();
        let ctx = self.ctx(&all, i, i);
        self.cfg
            .teams
            .sorting_types
            .iter()
            .zip(self.sorting.iter())
            .map(|(line, t)| {
                let v = match t {
                    SortingType::Groups(_) => all[i].group.clone(),
                    SortingType::Permissions(_) => "(permissions)".into(),
                    SortingType::Placeholder { placeholder, .. } => self.ph.parse(placeholder, &ctx),
                    SortingType::AToZ(p)
                    | SortingType::ZToA(p)
                    | SortingType::LowToHigh(p)
                    | SortingType::HighToLow(p) => self.ph.parse(p, &ctx),
                };
                (line.clone(), v)
            })
            .collect()
    }

    /// Scoreboard visibility for a player; whether it is shown after the change.
    pub(crate) fn set_scoreboard(&mut self, i: usize, on: Option<bool>, silent_save: bool) -> bool {
        let hidden_default = self.cfg.scoreboard.hidden_by_default;
        let shown_now = self.players[i].scoreboard_toggled == hidden_default;
        let shown = on.unwrap_or(!shown_now);
        self.players[i].scoreboard_toggled = shown == hidden_default;
        if self.cfg.scoreboard.remember_toggle_choice && !silent_save {
            let uuid = self.players[i].tp.p.uuid;
            let flipped = self.players[i].scoreboard_toggled;
            self.save_toggle("scoreboard-off", uuid, flipped);
        }
        shown
    }

    pub(crate) fn set_bossbar(&mut self, i: usize, on: Option<bool>) -> bool {
        let hidden_default = self.cfg.bossbar.hidden_by_default;
        let shown_now = self.players[i].bossbar_toggled == hidden_default;
        let shown = on.unwrap_or(!shown_now);
        self.players[i].bossbar_toggled = shown == hidden_default;
        if self.cfg.bossbar.remember_toggle_choice {
            let uuid = self.players[i].tp.p.uuid;
            let flipped = self.players[i].bossbar_toggled;
            self.save_toggle("bossbar-off", uuid, flipped);
        }
        shown
    }

    pub(crate) fn show_board(&mut self, i: usize, board: Option<String>) {
        self.players[i].forced_board = board;
    }

    pub(crate) fn announce_board(&mut self, name: String, secs: i64) {
        self.announced_board = Some((name, self.now_ms + secs * 1000));
    }

    pub(crate) fn announce_bar(&mut self, name: String, secs: i64) {
        self.announced_bars.push((name, self.now_ms + secs * 1000));
    }

    pub(crate) fn send_bar(&mut self, i: usize, name: String, secs: i64) {
        let until = self.now_ms + secs * 1000;
        self.players[i].sent_bars.push((name, until));
    }

    pub(crate) fn set_nametag_view_hidden(&mut self, i: usize, hidden: Option<bool>) -> bool {
        self.set_opaque_view(i, false);
        let h = hidden.unwrap_or(!self.players[i].nametag_view_hidden);
        self.players[i].nametag_view_hidden = h;
        h
    }

    /// `/tab nametag show|hide|toggle <player> [viewer]`: whether the name tag is hidden afterwards.
    pub(crate) fn nametag_target(&mut self, p: usize, viewer: Option<usize>, action: &str) -> bool {
        self.clear_opaque(p, viewer);
        let viewer_uuid = viewer.map(|v| self.players[v].tp.p.uuid);
        let now = match viewer_uuid {
            Some(u) => self.players[p].nametag_hidden_for.contains(&u),
            None => self.players[p].nametag_hidden,
        };
        let hide = match action {
            "show" => false,
            "hide" => true,
            _ => !now,
        };
        match viewer_uuid {
            Some(u) if hide => {
                self.players[p].nametag_hidden_for.insert(u);
            }
            Some(u) => {
                self.players[p].nametag_hidden_for.remove(&u);
            }
            None => self.players[p].nametag_hidden = hide,
        }
        hide
    }

    pub(crate) fn nametag_view_is_hidden(&self, i: usize) -> bool {
        self.players[i].nametag_view_hidden
    }

    pub(crate) fn bar_announced(&self, name: &str) -> bool {
        self.announced_bars.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    pub(crate) fn set_collision(&mut self, i: usize, collision: bool) {
        self.players[i].collision = Some(collision);
    }

    pub(crate) fn scoreboard_shown(&self, i: usize) -> bool {
        self.players[i].scoreboard_toggled == self.cfg.scoreboard.hidden_by_default
    }

    pub(crate) fn bossbar_shown(&self, i: usize) -> bool {
        self.players[i].bossbar_toggled == self.cfg.bossbar.hidden_by_default
    }
}

// ---- proxy support, global playerlist and the proxy bridge -----------------------------------------

/// A fresh instance id for `ProxySupport` (TAB uses a random UUID per start).
fn new_instance_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);
    uuid_string(nanos ^ ((std::process::id() as u128) << 96) ^ 0x5441_4250_524f_5859)
}

fn visibility_code(s: &str) -> i32 {
    match s {
        "never" => 1,
        "hideForOtherTeams" => 2,
        "hideForOwnTeam" => 3,
        _ => 0,
    }
}

fn collision_code(s: &str) -> i32 {
    match s {
        "never" => 1,
        "pushOtherTeams" => 2,
        "pushOwnTeam" => 3,
        _ => 0,
    }
}

impl Engine {
    /// The local players as the other instances are told about them.
    pub fn proxy_local(&self) -> Vec<crate::horizon_tab::proxy::PlayerData> {
        self.players
            .iter()
            .filter(|p| !p.fake)
            .map(|p| crate::horizon_tab::proxy::PlayerData {
                unique_id: p.tp.p.uuid,
                tablist_id: p.tp.p.uuid,
                name: p.tp.p.name.clone(),
                server: self.server.level_name.clone(),
                vanished: p.tp.p.vanished,
                staff: integration::permission(p.tp.p.uuid, "tab.staff", p.tp.p.op_level),
                skin: self
                    .skins
                    .get(&p.tp.p.uuid)
                    .map(|(v, s)| (v.clone(), if s.is_empty() { None } else { Some(s.clone()) })),
            })
            .collect()
    }

    /// One `proxy-support` message from the transport (a Base64 string).
    pub fn proxy_receive(&mut self, raw: &str) {
        let local = self.proxy_local();
        self.proxy.receive(raw, &local);
    }

    /// The `proxy-support` messages to send, with the channel they go to (None: feature off).
    pub fn proxy_outbox(&mut self) -> Option<(String, Vec<String>)> {
        let channel = self.cfg.proxy_support.as_ref()?.channel.clone();
        Some((channel, self.proxy.take_outbox()))
    }

    /// The remote players the viewer's list shows (`GlobalPlayerList.shouldSee`).
    fn global_wanted(&self, vi: usize, all: &[TabPlayer]) -> std::collections::BTreeMap<u128, (String, ListEntry)> {
        use crate::horizon_tab::proxy::remote_visible;
        let mut wanted = std::collections::BTreeMap::new();
        let Some(g) = &self.cfg.global else { return wanted };
        let me = &all[vi];
        let sees_vanished = integration::permission(me.p.uuid, "tab.seevanished", me.p.op_level);
        for rp in self.proxy.players.values() {
            let connected = all.iter().any(|t| t.p.uuid == rp.unique_id || t.p.uuid == rp.tablist_id);
            // Players of this very server are listed by the game itself.
            if rp.server == self.server.level_name
                || !remote_visible(g, &self.server.level_name, sees_vanished, rp, connected)
            {
                continue;
            }
            let skin = rp.skin.as_ref().map(|(v, s)| (v.clone(), s.clone().unwrap_or_default()));
            let sig = format!("{}|{:?}", rp.nickname, skin);
            wanted.insert(
                rp.tablist_id,
                (
                    sig,
                    ListEntry {
                        uuid: rp.tablist_id,
                        skin,
                        latency: 0,
                        display: self.component(&rp.nickname),
                        order: 0,
                    },
                ),
            );
        }
        wanted
    }

    /// `global-playerlist`: the players of other servers in this viewer's list, kept in step.
    fn global_for(&self, vi: usize, all: &[TabPlayer], sent: &mut Sent, out: &mut Vec<Packet>) {
        let to = all[vi].p.uuid;
        // A layout owns the list; the remote players wait for it to go.
        let wanted = if sent.layout.is_some() { Default::default() } else { self.global_wanted(vi, all) };
        let mut removes: Vec<u128> = sent
            .global
            .iter()
            .filter(|(id, sig)| wanted.get(*id).is_none_or(|(w, _)| w != *sig))
            .map(|(id, _)| *id)
            .collect();
        removes.sort_unstable();
        let adds: Vec<ListEntry> = wanted
            .iter()
            .filter(|(id, (sig, _))| sent.global.get(*id) != Some(sig))
            .map(|(_, (_, e))| e.clone())
            .collect();
        for id in &removes {
            sent.global.remove(id);
        }
        for (id, (sig, _)) in &wanted {
            sent.global.insert(*id, sig.clone());
        }
        if !removes.is_empty() {
            out.push(Packet { to, out: Out::ListRemove(removes) });
        }
        if !adds.is_empty() {
            out.push(Packet { to, out: Out::ListAdd(adds) });
        }
    }

    // ---- the bridge ---------------------------------------------------------------------------------

    /// A component from a string the proxy sent: JSON as it is, anything else as TAB-formatted text.
    fn bridge_component(&self, s: &str) -> Value {
        let t = s.trim();
        if t.starts_with(['{', '[', '"']) {
            if let Ok(v) = serde_json::from_str::<Value>(t) {
                return v;
            }
        }
        self.component(s)
    }

    fn bridge_number_format(&self, f: &Option<String>) -> Option<NumberFmt> {
        match f.as_deref() {
            None | Some("") => None,
            Some("blank") => Some(NumberFmt::Blank),
            Some(other) => Some(NumberFmt::Fixed(self.bridge_component(other))),
        }
    }

    /// One placeholder's value for a player, with the proxy's output replacements.
    fn bridge_value(&self, ident: &str, all: &[TabPlayer], target: usize, viewer: usize, bp: &BridgePlayer) -> Option<String> {
        let ctx = self.ctx(all, target, viewer);
        let v = self.ph.parse_with_rules(ident, &ctx, &bp.replacements);
        // An identifier nothing here answers stays as written: the proxy keeps its own value.
        (v != ident).then_some(v)
    }

    /// The answers a join gives: world, group, every placeholder, game mode.
    fn bridge_join_response(&self, all: &[TabPlayer], vi: usize, bp: &mut BridgePlayer) -> ToProxy {
        let me = &all[vi];
        let mut values = Vec::new();
        for (ident, _) in bp.placeholders.clone() {
            if crate::horizon_tab::bridge::is_relational(&ident) {
                let mut rows = Vec::new();
                for (ti, other) in all.iter().enumerate() {
                    if ti == vi {
                        continue;
                    }
                    if let Some(v) = self.bridge_value(&ident, all, ti, vi, bp) {
                        bp.sent.insert(format!("{ident}\u{0}{}", other.p.name), v.clone());
                        rows.push((other.p.name.clone(), v));
                    }
                }
                values.push((ident, PlaceholderValue::Relational(rows)));
            } else if let Some(v) = self.bridge_value(&ident, all, vi, vi, bp) {
                bp.sent.insert(ident.clone(), v.clone());
                values.push((ident, PlaceholderValue::Plain(v)));
            }
        }
        bp.world = Some(me.p.world.clone());
        bp.game_mode = Some(me.p.game_mode);
        bp.group = Some(me.group.clone());
        ToProxy::PlayerJoinResponse {
            world: me.p.world.clone(),
            group: bp.forward_group.then(|| me.group.clone()),
            values,
            game_mode: me.p.game_mode,
        }
    }

    /// One message from TAB on the proxy, for a player's connection. Scoreboard messages come back as
    /// packets for that player; answers wait in `bridge_outbox`.
    pub fn bridge_receive(&mut self, player: u128, bytes: &[u8]) -> Vec<Packet> {
        let msg = match crate::horizon_tab::bridge::decode_from_proxy(bytes) {
            Ok(m) => m,
            Err(e) => {
                self.bridge.problems.push(e);
                return Vec::new();
            }
        };
        let Some(vi) = self.index_of(player) else { return Vec::new() };
        let all = self.all_players();
        let me = all[vi].clone();
        let mut out = Vec::new();
        let mut push = |o: Out| out.push(Packet { to: player, out: o });
        match msg {
            FromProxy::PlayerJoin { forward_group, placeholders, replacements } => {
                let mut bp = BridgePlayer { forward_group, placeholders, replacements, ..BridgePlayer::default() };
                let response = self.bridge_join_response(&all, vi, &mut bp);
                // The new server starts from "not vanished, visible": say so only when it is not.
                bp.vanished = Some(me.p.vanished);
                bp.invisible = Some(me.p.invisible);
                self.bridge.players.insert(player, bp);
                self.bridge.send(player, &response);
                if me.p.vanished {
                    self.bridge.send(player, &ToProxy::Vanished(true));
                }
                if me.p.invisible {
                    self.bridge.send(player, &ToProxy::Invisible(true));
                }
            }
            FromProxy::Permission(node) => {
                let value = integration::permission(player, &node, me.p.op_level);
                if let Some(bp) = self.bridge.players.get_mut(&player) {
                    bp.permissions.insert(node.clone(), value);
                }
                self.bridge.send(player, &ToProxy::HasPermission { permission: node, value });
            }
            FromProxy::Expansion { placeholder, value } => {
                if let Some(bp) = self.bridge.players.get_mut(&player) {
                    bp.expansion.insert(placeholder, value);
                }
            }
            FromProxy::Placeholder { identifier, refresh } => {
                if let Some(bp) = self.bridge.players.get_mut(&player) {
                    bp.placeholders.retain(|(i, _)| *i != identifier);
                    bp.placeholders.push((identifier, refresh));
                }
            }
            FromProxy::Unload => {
                if let Some(bp) = self.bridge.players.remove(&player) {
                    for t in bp.teams {
                        push(Out::TeamRemove { name: t });
                    }
                    for o in bp.objectives {
                        push(Out::Objective {
                            name: o,
                            method: ObjectiveMethod::Remove,
                            title: json!({"text": ""}),
                            hearts: false,
                            format: None,
                        });
                    }
                }
            }
            FromProxy::DisplayObjective { slot, objective } => push(Out::DisplaySlot { slot, objective }),
            FromProxy::Objective { name, action, title, display, number_format } => {
                let bp = self.bridge.players.entry(player).or_default();
                let method = match action {
                    0 => {
                        if !bp.objectives.contains(&name) {
                            bp.objectives.push(name.clone());
                        }
                        ObjectiveMethod::Add
                    }
                    1 => {
                        bp.objectives.retain(|o| *o != name);
                        ObjectiveMethod::Remove
                    }
                    _ => ObjectiveMethod::Change,
                };
                let (title, format) = if action == 1 {
                    (json!({"text": ""}), None)
                } else {
                    (self.bridge_component(&title), self.bridge_number_format(&number_format))
                };
                push(Out::Objective { name, method, title, hearts: display == 1, format });
            }
            FromProxy::Score { objective, action, holder, score, display_name, number_format } => {
                if action == 0 {
                    push(Out::Score {
                        owner: holder,
                        objective,
                        score,
                        display: display_name.as_deref().map(|d| self.bridge_component(d)),
                        format: self.bridge_number_format(&number_format),
                    });
                } else {
                    push(Out::ResetScore { owner: holder, objective });
                }
            }
            FromProxy::Team { name, action, prefix, suffix, options, visibility, collision, color, players } => {
                if action == 1 {
                    if let Some(bp) = self.bridge.players.get_mut(&player) {
                        bp.teams.retain(|t| *t != name);
                    }
                    push(Out::TeamRemove { name });
                } else {
                    let params = TeamParams {
                        display_name: json!({"text": name}),
                        prefix: self.bridge_component(&prefix),
                        suffix: self.bridge_component(&suffix),
                        visibility: visibility_code(&visibility),
                        collision: collision_code(&collision),
                        color: (0..=15).contains(&color).then_some(color),
                        options: options as i8,
                    };
                    if action == 0 {
                        let bp = self.bridge.players.entry(player).or_default();
                        if !bp.teams.contains(&name) {
                            bp.teams.push(name.clone());
                        }
                        push(Out::TeamCreate { name, params, players });
                    } else {
                        push(Out::TeamUpdate { name, params });
                    }
                }
            }
        }
        out
    }

    /// The bridge answers waiting for the transport: `(player, bytes)` for channel `tab:bridge-6`.
    pub fn bridge_outbox(&mut self) -> Vec<(u128, Vec<u8>)> {
        self.bridge.take_outbox()
    }

    /// What changed for bridged players since the last tick: world, game mode, group, vanish,
    /// invisibility and the proxy's placeholders at their refresh intervals.
    fn bridge_tick(&mut self, all: &[TabPlayer]) {
        if self.bridge.players.is_empty() {
            return;
        }
        self.bridge.players.retain(|u, _| all.iter().any(|t| t.p.uuid == *u));
        let now = self.now_ms;
        let ids: Vec<u128> = self.bridge.players.keys().copied().collect();
        for uuid in ids {
            let Some(vi) = all.iter().position(|t| t.p.uuid == uuid) else { continue };
            let me = &all[vi];
            let mut bp = self.bridge.players[&uuid].clone();
            let mut msgs: Vec<ToProxy> = Vec::new();
            if bp.world.as_deref() != Some(me.p.world.as_str()) {
                msgs.push(ToProxy::SetWorld(me.p.world.clone()));
                bp.world = Some(me.p.world.clone());
            }
            if bp.game_mode != Some(me.p.game_mode) {
                msgs.push(ToProxy::UpdateGameMode(me.p.game_mode));
                bp.game_mode = Some(me.p.game_mode);
            }
            if bp.forward_group && bp.group.as_deref() != Some(me.group.as_str()) {
                msgs.push(ToProxy::SetGroup(me.group.clone()));
                bp.group = Some(me.group.clone());
            }
            if bp.vanished != Some(me.p.vanished) {
                msgs.push(ToProxy::Vanished(me.p.vanished));
                bp.vanished = Some(me.p.vanished);
            }
            if bp.invisible != Some(me.p.invisible) {
                msgs.push(ToProxy::Invisible(me.p.invisible));
                bp.invisible = Some(me.p.invisible);
            }
            for (ident, refresh) in bp.placeholders.clone() {
                let due = bp.last_eval.get(&ident).is_none_or(|t| now - *t >= i64::from(refresh).max(50));
                if !due {
                    continue;
                }
                bp.last_eval.insert(ident.clone(), now);
                if crate::horizon_tab::bridge::is_relational(&ident) {
                    for (ti, other) in all.iter().enumerate() {
                        if ti == vi {
                            continue;
                        }
                        if let Some(v) = self.bridge_value(&ident, all, ti, vi, &bp) {
                            let key = format!("{ident}\u{0}{}", other.p.name);
                            if bp.sent.get(&key) != Some(&v) {
                                bp.sent.insert(key, v.clone());
                                msgs.push(ToProxy::UpdatePlaceholder {
                                    identifier: ident.clone(),
                                    target: Some(other.p.name.clone()),
                                    value: v,
                                });
                            }
                        }
                    }
                } else if let Some(v) = self.bridge_value(&ident, all, vi, vi, &bp) {
                    if bp.sent.get(&ident) != Some(&v) {
                        bp.sent.insert(ident.clone(), v.clone());
                        msgs.push(ToProxy::UpdatePlaceholder { identifier: ident, target: None, value: v });
                    }
                }
            }
            self.bridge.players.insert(uuid, bp);
            for m in msgs {
                self.bridge.send(uuid, &m);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizon_tab::properties::PropertyFile;

    fn engine(config: &str, users: &str) -> Engine {
        let cfg = Config::from_yaml(&yaml::parse(config).unwrap());
        let props = Properties {
            groups: PropertyFile::from_str(include_str!("../../data/tab/testdata/groups.yml"), "group").unwrap(),
            users: PropertyFile::from_str(users, "user").unwrap(),
        };
        let anim = tabph::load_animations(&yaml::parse(include_str!("../../data/tab/testdata/animations.yml")).unwrap());
        Engine::new(
            PathBuf::from("/nonexistent"),
            cfg,
            props,
            anim,
            Yaml::empty_map(),
            Yaml::empty_map(),
            Vec::new(),
        )
    }

    fn player(name: &str, uuid: u128) -> PlayerIn {
        PlayerIn {
            uuid,
            name: name.into(),
            dimension: "minecraft:overworld".into(),
            health: 20.0,
            ping: 25,
            protocol: 775,
            ..PlayerIn::default()
        }
    }

    fn server() -> Server {
        Server {
            level_name: "world".into(),
            ..Server::default()
        }
    }

    fn for_whom(packets: &[Packet], to: u128) -> Vec<&Out> {
        packets.iter().filter(|p| p.to == to).map(|p| &p.out).collect()
    }

    #[test]
    fn our_config_two_players() {
        let mut e = engine(
            include_str!("../../data/tab/testdata/config.yml"),
            "Steve:\n  tabprefix: '&c&l[OP] &f'\n  tagprefix: '&4&l[Core] &f&l'\n",
        );
        let packets = e.tick(1000, server(), vec![player("Steve", 1), player("Alex", 2)]);
        let steve = for_whom(&packets, 1);
        // Display names: Steve's own prefix; Alex gets _DEFAULT_ (%luckperms-prefix% is empty
        // without a permissions mod, customtabname %displayname% = name).
        let names = steve
            .iter()
            .find_map(|o| match o {
                Out::DisplayNames(n) => Some(n.clone()),
                _ => None,
            })
            .unwrap();
        let steve_name = names.iter().find(|(u, _)| *u == 1).unwrap().1.clone().unwrap();
        assert_eq!(text::component_plain(&steve_name), "[OP] Steve");
        let alex_name = names.iter().find(|(u, _)| *u == 2).unwrap().1.clone().unwrap();
        assert_eq!(text::component_plain(&alex_name), "Alex");
        // Teams for both, with Steve's tag prefix and its colour (white, the last one).
        let teams: Vec<_> = steve
            .iter()
            .filter_map(|o| match o {
                Out::TeamCreate { name, params, players } => Some((name.clone(), params.clone(), players.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(teams.len(), 2);
        let st = teams.iter().find(|t| t.2 == vec!["Steve".to_owned()]).unwrap();
        assert_eq!(text::component_plain(&st.1.prefix), "[Core] ");
        assert_eq!(st.1.color, Some(15));
        assert_eq!(st.1.collision, 0);
        // Both in group default: sorted by name, Alex first.
        let at = teams.iter().find(|t| t.2 == vec!["Alex".to_owned()]).unwrap();
        assert!(at.0 < st.0, "{} < {}", at.0, st.0);
        // Player list objective: created, shown in the list slot, a score with the fancy value.
        assert!(steve
            .iter()
            .any(|o| matches!(o, Out::Objective { name, method: ObjectiveMethod::Add, .. } if name == LIST_OBJECTIVE)));
        assert!(steve.iter().any(|o| matches!(o, Out::DisplaySlot { slot: 0, .. })));
        let score = steve
            .iter()
            .find_map(|o| match o {
                Out::Score {
                    owner, score, format, ..
                } if owner == "Alex" => Some((*score, format.clone())),
                _ => None,
            })
            .unwrap();
        assert_eq!(score.0, 25);
        match score.1 {
            Some(NumberFmt::Fixed(v)) => assert_eq!(text::component_plain(&v), "Ping: 25"),
            other => panic!("{other:?}"),
        }
        // Disabled features send nothing.
        assert!(!steve
            .iter()
            .any(|o| matches!(o, Out::TabList { .. } | Out::Boss { .. })));

        // Nothing changed: nothing sent.
        let again = e.tick(1050, server(), vec![player("Steve", 1), player("Alex", 2)]);
        assert!(again.is_empty(), "{again:?}");

        // Alex's ping changes (after its 500 ms refresh): one score per viewer.
        let mut alex = player("Alex", 2);
        alex.ping = 80;
        let changed = e.tick(1700, server(), vec![player("Steve", 1), alex.clone()]);
        assert_eq!(changed.len(), 2, "{changed:?}");
        assert!(changed
            .iter()
            .all(|p| matches!(&p.out, Out::Score { owner, score: 80, .. } if owner == "Alex")));

        // Alex leaves: Steve loses the team and the score.
        let left = e.tick(1800, server(), vec![player("Steve", 1)]);
        assert!(left.iter().any(|p| matches!(&p.out, Out::TeamRemove { .. })));
        assert!(left
            .iter()
            .any(|p| matches!(&p.out, Out::ResetScore { owner, .. } if owner == "Alex")));
    }

    #[test]
    fn disable_condition_by_world() {
        let mut e = engine(include_str!("../../data/tab/testdata/config.yml"), "");
        let mut p = player("Steve", 1);
        p.dimension = "minecraft:the_nether".into();
        let packets = e.tick(
            0,
            Server {
                level_name: "disabledworld_x".into(),
                ..Server::default()
            },
            vec![p.clone()],
        );
        assert!(packets.iter().any(|p| matches!(&p.out, Out::TeamCreate { .. })));
        // A world named like the disable condition: no formatting.
        let mut e = engine(include_str!("../../data/tab/testdata/config.yml"), "");
        let packets = e.tick(
            0,
            Server {
                level_name: "disabledworld".into(),
                ..Server::default()
            },
            vec![player("Steve", 1)],
        );
        assert!(!packets
            .iter()
            .any(|p| matches!(&p.out, Out::TeamCreate { .. } | Out::DisplayNames(_))));
        assert!(!packets.iter().any(|p| matches!(&p.out, Out::Objective { .. })));
    }

    const SB: &str = "scoreboard-teams:\n  enabled: false\ntablist-name-formatting:\n  enabled: false\nscoreboard:\n  enabled: true\n  scoreboards:\n    main:\n      title: '&eTitle'\n      lines:\n        - 'Online:||%online%'\n        - '&7%player%'\nheader-footer:\n  enabled: true\n  designs:\n    default:\n      header:\n        - '&aHello %player%'\n      footer:\n        - 'Ping %ping%'\nbossbar:\n  enabled: true\n  bars:\n    Info:\n      style: NOTCHED_10\n      color: '%animation:barcolors%'\n      progress: '50'\n      text: '&fBar'\n";

    #[test]
    fn scoreboard_header_bossbar() {
        let mut e = engine(SB, "");
        let packets = e.tick(0, server(), vec![player("Steve", 1)]);
        let outs = for_whom(&packets, 1);
        let header = outs
            .iter()
            .find_map(|o| match o {
                Out::TabList {
                    header_plain,
                    footer_plain,
                    ..
                } => Some((header_plain.clone(), footer_plain.clone())),
                _ => None,
            })
            .unwrap();
        assert_eq!(header, ("Hello Steve".to_owned(), "Ping 25".to_owned()));
        assert!(outs
            .iter()
            .any(|o| matches!(o, Out::DisplaySlot { slot: 1, objective } if objective == SIDEBAR_OBJECTIVE)));
        let lines: Vec<_> = outs
            .iter()
            .filter_map(|o| match o {
                Out::Score {
                    objective,
                    score,
                    display,
                    format,
                    ..
                } if objective == SIDEBAR_OBJECTIVE => Some((*score, display.clone(), format.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, 2);
        assert_eq!(text::component_plain(lines[0].1.as_ref().unwrap()), "Online:");
        assert!(matches!(&lines[0].2, Some(NumberFmt::Fixed(v)) if text::component_plain(v) == "1"));
        assert!(matches!(&lines[1].2, Some(NumberFmt::Blank)));
        let bar = outs
            .iter()
            .find_map(|o| match o {
                Out::Boss {
                    action:
                        BossOut::Add {
                            progress,
                            color,
                            overlay,
                            ..
                        },
                    ..
                } => Some((*progress, *color, *overlay)),
                _ => None,
            })
            .unwrap();
        assert_eq!(bar, (0.5, 2, 2)); // RED at t=0, NOTCHED_10
                                      // One second later the colour animation moves on: only a style update.
        let packets = e.tick(1000, server(), vec![player("Steve", 1)]);
        assert_eq!(packets.len(), 1, "{packets:?}");
        assert!(matches!(
            &packets[0].out,
            Out::Boss {
                action: BossOut::Style { color: 1, overlay: 2 },
                ..
            }
        ));
        // Toggle the scoreboard off: it is removed.
        let i = e.index_of(1).unwrap();
        assert!(!e.set_scoreboard(i, None, true));
        let packets = e.tick(1100, server(), vec![player("Steve", 1)]);
        assert!(packets.iter().any(|p| matches!(&p.out, Out::Objective { method: ObjectiveMethod::Remove, name, .. } if name == SIDEBAR_OBJECTIVE)));
    }

    #[test]
    fn nametag_view_hidden() {
        let mut e = engine(include_str!("../../data/tab/testdata/config.yml"), "");
        e.tick(0, server(), vec![player("Steve", 1), player("Alex", 2)]);
        let i = e.index_of(1).unwrap();
        assert!(e.set_nametag_view_hidden(i, None));
        let packets = e.tick(100, server(), vec![player("Steve", 1), player("Alex", 2)]);
        // Steve's client: Alex's team now hides its name tag; Steve's own stays.
        assert_eq!(packets.len(), 1, "{packets:?}");
        assert!(matches!(&packets[0], Packet { to: 1, out: Out::TeamUpdate { params, .. } } if params.visibility == 1));
    }

    #[test]
    fn unload_takes_everything_away() {
        let mut e = engine(include_str!("../../data/tab/testdata/config.yml"), "");
        e.tick(0, server(), vec![player("Steve", 1)]);
        let out = e.unload();
        assert!(out.iter().any(|p| matches!(&p.out, Out::TeamRemove { .. })));
        assert!(out
            .iter()
            .any(|p| matches!(&p.out, Out::DisplayNames(n) if n[0].1.is_none())));
        assert!(out.iter().any(|p| matches!(
            &p.out,
            Out::Objective {
                method: ObjectiveMethod::Remove,
                ..
            }
        )));
        // And the next tick sends it all again.
        let again = e.tick(100, server(), vec![player("Steve", 1)]);
        assert!(again.iter().any(|p| matches!(&p.out, Out::TeamCreate { .. })));
    }

    #[test]
    fn world_names() {
        assert_eq!(world_name("world", "minecraft:the_nether"), "world_nether");
        assert_eq!(world_name("lobby", "minecraft:overworld"), "lobby");
        assert_eq!(world_name("w", "mymod:moon"), "moon");
    }

    #[test]
    fn guards_keep_our_values_against_other_senders() {
        let mut e = engine(
            include_str!("../../data/tab/testdata/config.yml"),
            "Steve:\n  tabprefix: '&c&l[OP] &f'\n  tagprefix: '&4&l[Core] &f&l'\n",
        );
        let packets = e.tick(1000, server(), vec![player("Steve", 1), player("Alex", 2)]);
        let own = for_whom(&packets, 2)
            .iter()
            .find_map(|o| match o {
                Out::TeamCreate { name, players, .. } if players == &["Steve".to_owned()] => Some(name.clone()),
                _ => None,
            })
            .unwrap();
        // Our own team packet passes; a foreign team taking Steve is stopped; one taking nobody
        // of ours, or Steve to the team we already made, is not.
        assert!(!e.guard_team(2, &own, &["Steve".to_owned()]));
        assert!(e.guard_team(2, "datapack_team", &["Steve".to_owned()]));
        assert!(!e.guard_team(2, "datapack_team", &["Nobody".to_owned()]));
        assert!(!e.guard_team(2, "datapack_team", &[]));
        // The display name Alex's client holds for Steve is the one a foreign packet is put back to.
        let name = e.guard_display_name(2, 1).unwrap();
        assert_eq!(text::component_plain(&name), "[OP] Steve");
        // Nothing configured here: no ping spoof, no spectator fix.
        assert_eq!(e.guard_latency(), None);
        assert_eq!(e.guard_game_mode(2, 1, 3), None);
        // A respawn forgets what the client holds; the next tick sends it all again.
        e.forget_viewer(2);
        assert_eq!(e.guard_display_name(2, 1), None);
        let again = e.tick(1500, server(), vec![player("Steve", 1), player("Alex", 2)]);
        assert!(for_whom(&again, 2).iter().any(|o| matches!(o, Out::TeamCreate { .. })));
    }

    #[test]
    fn winter_teams_disabled_end_to_end() {
        // The winter lobby's config differs from the other lobby's only here: scoreboard-teams.enabled false.
        let config = include_str!("../../data/tab/testdata/config.yml").replacen(
            "scoreboard-teams:\n  enabled: true",
            "scoreboard-teams:\n  enabled: false",
            1,
        );
        assert!(config.contains("scoreboard-teams:\n  enabled: false"));
        let mut e = engine(&config, "Steve:\n  tabprefix: '&c[OP] '\n  tagprefix: '&4[Core] '\n");
        let packets = e.tick(1000, server(), vec![player("Steve", 1), player("Alex", 2)]);
        // No team packet of any kind reaches any viewer.
        for p in &packets {
            assert!(
                !matches!(p.out, Out::TeamCreate { .. } | Out::TeamUpdate { .. } | Out::TeamRemove { .. }),
                "{p:?}"
            );
        }
        // The packet guard does not act on teams either: a foreign team is let through.
        assert!(!e.guard_team(2, "datapack_team", &["Steve".to_owned()]));
        // A later tick (a third player joins) still sends no team.
        let later = e.tick(2000, server(), vec![player("Steve", 1), player("Alex", 2), player("Sam", 3)]);
        assert!(!later.iter().any(|p| matches!(p.out, Out::TeamCreate { .. } | Out::TeamUpdate { .. })));
    }

    #[test]
    fn spectator_fix_and_ping_spoof_guards() {
        let mut e = engine(
            "prevent-spectator-effect:\n  enabled: true\nping-spoof:\n  enabled: true\n  value: 0\n",
            "",
        );
        e.tick(1000, server(), vec![player("Steve", 1), player("Alex", 2)]);
        assert_eq!(e.guard_game_mode(2, 1, 3), Some(0));
        assert_eq!(e.guard_game_mode(2, 2, 3), None, "the spectator's own client keeps the mode");
        assert_eq!(e.guard_game_mode(2, 1, 1), None);
        assert_eq!(e.guard_latency(), Some(0));
    }


    // ---- layout, per-world playerlist, opaque name tags, nick compatibility -------------------

    const LAYOUT: &str = "layout:\n  enabled: true\n  empty-slot-ping-value: 500\n  layouts:\n    default:\n      slot-count: 6\n      fixed-slots:\n        - '1|&3Hello|signed_texture:VAL;SIG|42'\n      groups:\n        all:\n          slots:\n            - 2-4\n";

    #[test]
    fn layout_fills_slots_and_hides_the_real_entries() {
        let mut e = engine(LAYOUT, "");
        let packets = e.tick(1000, server(), vec![player("Steve", 1), player("Alex", 2)]);
        let mine = for_whom(&packets, 1);
        let adds: Vec<&Vec<crate::horizon_tab::out::ListEntry>> =
            mine.iter().filter_map(|o| if let Out::ListAdd(a) = o { Some(a) } else { None }).collect();
        assert_eq!(adds.len(), 1);
        let a = adds[0];
        assert_eq!(a.iter().map(|x| x.uuid).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(a[0].skin, Some(("VAL".to_owned(), "SIG".to_owned())));
        assert_eq!(a[0].latency, 42);
        assert_eq!(a[0].order, i32::MAX - 1);
        assert_eq!(a[0].display["text"], "Hello");
        // Both have the same sorting characters, so TAB's unique team names (`checkTeamName`: A, B...)
        // go by who was processed first: Steve's team sorts before Alex's, and the layout follows it.
        assert_eq!(a[1].latency, 25);
        assert_eq!(a[1].display["text"], "Steve");
        assert_eq!(a[2].display["text"], "Alex");
        assert_eq!(a[3].display["text"], "");
        assert_eq!(a[3].latency, 500);
        assert!(mine.iter().any(|o| matches!(o, Out::Listed { ids, listed: false } if ids == &vec![1, 2])));
        // Nothing changes: nothing is sent.
        let again = e.tick(1500, server(), vec![player("Steve", 1), player("Alex", 2)]);
        assert!(for_whom(&again, 1).iter().all(|o| !matches!(
            o,
            Out::ListAdd(_) | Out::ListRemove(_) | Out::ListNames(_) | Out::Listed { .. }
        )));
        // A third player takes slot 4 (the entry is replaced) and leaves the list.
        let later = e.tick(2000, server(), vec![player("Steve", 1), player("Alex", 2), player("Sam", 3)]);
        let mine = for_whom(&later, 1);
        assert!(mine.iter().any(|o| matches!(o, Out::ListRemove(ids) if ids.contains(&3) || ids.contains(&4))));
        assert!(mine.iter().any(|o| matches!(o, Out::Listed { ids, listed: false } if ids == &vec![3])));
    }

    #[test]
    fn the_layout_goes_away_with_unload() {
        let mut e = engine(LAYOUT, "");
        e.tick(1000, server(), vec![player("Steve", 1)]);
        let gone = e.unload();
        let mine = for_whom(&gone, 1);
        assert!(mine.iter().any(|o| matches!(o, Out::ListRemove(ids) if ids.len() == 6)));
        assert!(mine.iter().any(|o| matches!(o, Out::Listed { ids, listed: true } if ids == &vec![1])));
    }

    #[test]
    fn a_layout_waits_for_its_condition() {
        let config = LAYOUT.replace("    default:\n", "    default:\n      display-condition: '%world%=elsewhere'\n");
        let mut e = engine(&config, "");
        let packets = e.tick(1000, server(), vec![player("Steve", 1)]);
        assert!(for_whom(&packets, 1).iter().all(|o| !matches!(o, Out::ListAdd(_) | Out::Listed { .. })));
    }

    fn nether(name: &str, uuid: u128) -> PlayerIn {
        PlayerIn { dimension: "minecraft:the_nether".into(), ..player(name, uuid) }
    }

    #[test]
    fn per_world_playerlist_hides_other_worlds_and_shows_them_again() {
        let mut e = engine("per-world-playerlist:\n  enabled: true\n", "");
        let packets = e.tick(1000, server(), vec![player("Steve", 1), nether("Alex", 2)]);
        assert!(for_whom(&packets, 1)
            .iter()
            .any(|o| matches!(o, Out::EntityVisible { ids, visible: false, list: true } if ids == &vec![2])));
        assert!(for_whom(&packets, 2)
            .iter()
            .any(|o| matches!(o, Out::EntityVisible { ids, visible: false, list: true } if ids == &vec![1])));
        // Nothing new while they stay apart.
        let same = e.tick(1500, server(), vec![player("Steve", 1), nether("Alex", 2)]);
        assert!(same.iter().all(|p| !matches!(p.out, Out::EntityVisible { .. })));
        // Alex comes to the overworld: both see each other again.
        let back = e.tick(2000, server(), vec![player("Steve", 1), player("Alex", 2)]);
        assert!(for_whom(&back, 1)
            .iter()
            .any(|o| matches!(o, Out::EntityVisible { ids, visible: true, .. } if ids == &vec![2])));
    }

    #[test]
    fn opaque_name_tags_follow_the_line_of_sight() {
        let mut e = engine(include_str!("../../data/tab/testdata/config.yml"), "");
        let ps = || vec![player("Steve", 1), player("Alex", 2)];
        let first = e.tick(1000, server(), ps());
        assert!(for_whom(&first, 2).iter().any(|o| matches!(o, Out::TeamCreate { params, .. } if params.visibility == 0)));
        let steve = e.find("Steve").unwrap();
        e.set_opaque(steve, None);
        let mut pairs = e.opaque_pairs();
        pairs.sort();
        assert_eq!(pairs, vec![(2, 1)]);
        e.set_occlusion(&[(2, 1, false)]);
        let hidden = e.tick(2000, server(), ps());
        assert!(for_whom(&hidden, 2).iter().any(|o| matches!(o, Out::TeamUpdate { params, .. } if params.visibility == 1)));
        // In sight again: the tag returns.
        e.set_occlusion(&[(2, 1, true)]);
        let shown = e.tick(3000, server(), ps());
        assert!(for_whom(&shown, 2).iter().any(|o| matches!(o, Out::TeamUpdate { params, .. } if params.visibility == 0)));
        // `show` ends the mode.
        e.clear_opaque(steve, None);
        assert!(e.opaque_pairs().is_empty());
    }

    #[test]
    fn a_nick_plugin_rename_moves_the_team_member() {
        let mut e = engine(include_str!("../../data/tab/testdata/config.yml"), "");
        let ps = || vec![player("Steve", 1), player("Alex", 2)];
        e.tick(1000, server(), ps());
        assert!(!e.set_entry_name(2, 1, "Nick"), "only the receiver's own entry counts");
        assert!(e.set_entry_name(1, 1, "Nick"));
        assert!(!e.set_entry_name(1, 1, "Nick"));
        let packets = e.tick(2000, server(), ps());
        let created: Vec<&Out> = for_whom(&packets, 2)
            .into_iter()
            .filter(|o| matches!(o, Out::TeamCreate { players, .. } if players == &vec!["Nick".to_owned()]))
            .collect();
        assert_eq!(created.len(), 1, "{packets:?}");
        // The old name comes back when the plugin restores it.
        assert!(e.set_entry_name(1, 1, "Steve"));
    }

    // ---- proxy support, global playerlist, the bridge -----------------------------------------------

    const GLOBAL: &str = "global-playerlist:\n  enabled: true\n  server-groups:\n    g:\n      - world\n      - lobby\nproxy-support:\n  enabled: true\n  channel-name-suffix: net\n";

    fn remote(id: u128, name: &str, server: &str, vanished: bool) -> crate::horizon_tab::proxy::PlayerData {
        crate::horizon_tab::proxy::PlayerData {
            unique_id: id,
            tablist_id: id,
            name: name.into(),
            server: server.into(),
            vanished,
            staff: true,
            skin: Some(("V".into(), Some("S".into()))),
        }
    }

    fn tell(e: &mut Engine, m: crate::horizon_tab::proxy::ProxyMsg) {
        e.proxy_receive(&crate::horizon_tab::proxy::encode("other", &m));
    }

    #[test]
    fn remote_players_join_the_global_list_and_the_counts() {
        use crate::horizon_tab::proxy::ProxyMsg;
        let mut e = engine(GLOBAL, "");
        let first = e.tick(1000, server(), vec![player("Steve", 1)]);
        assert!(for_whom(&first, 1).iter().all(|o| !matches!(o, Out::ListAdd(_))));
        assert_eq!(e.proxy_outbox().map(|(c, m)| (c, m.len())), Some(("TAB_4_net".to_owned(), 2)), "a load request and our join");
        tell(&mut e, ProxyMsg::PlayerJoin(remote(50, "Far", "lobby", false)));
        tell(&mut e, ProxyMsg::PlayerJoin(remote(51, "Other", "elsewhere", false)));
        let packets = e.tick(3000, server(), vec![player("Steve", 1)]);
        let adds: Vec<_> = for_whom(&packets, 1)
            .into_iter()
            .filter_map(|o| if let Out::ListAdd(a) = o { Some(a.clone()) } else { None })
            .collect();
        // "elsewhere" is not in the viewer's group, "lobby" is.
        assert_eq!(adds.len(), 1, "{packets:?}");
        assert_eq!(adds[0].iter().map(|x| x.uuid).collect::<Vec<_>>(), vec![50]);
        assert_eq!(adds[0][0].skin, Some(("V".to_owned(), "S".to_owned())));
        assert_eq!(adds[0][0].display["text"], "Far");
        // Counts: two remote players plus Steve; one on "lobby".
        assert_eq!(e.parse_for(0, "%online%"), "3");
        assert_eq!(e.parse_for(0, "%online_lobby%"), "1");
        assert_eq!(e.parse_for(0, "%online_world%"), "1");
        assert_eq!(e.parse_for(0, "%staffonline%"), "2", "Steve is not staff, both remote players are");
        assert_eq!(e.parse_for(0, "%playerlist-group_g%"), "2", "Steve and Far are in the group");
        // Nothing is sent again while nothing changes.
        let again = e.tick(5000, server(), vec![player("Steve", 1)]);
        assert!(for_whom(&again, 1).iter().all(|o| !matches!(o, Out::ListAdd(_) | Out::ListRemove(_))));
        // A vanished player leaves the list for a viewer without tab.seevanished.
        tell(&mut e, ProxyMsg::UpdateVanishStatus(50, true));
        let packets = e.tick(7000, server(), vec![player("Steve", 1)]);
        assert!(for_whom(&packets, 1).iter().any(|o| matches!(o, Out::ListRemove(ids) if ids == &vec![50])));
        assert_eq!(e.parse_for(0, "%online_lobby%"), "0");
        assert_eq!(e.parse_for(0, "%online%"), "2", "Steve and the player on the other server");
        // That player quits.
        tell(&mut e, ProxyMsg::PlayerQuit(51));
        let _ = e.tick(9000, server(), vec![player("Steve", 1)]);
        assert_eq!(e.parse_for(0, "%online%"), "1");
    }

    #[test]
    fn unloading_takes_the_remote_entries_away() {
        use crate::horizon_tab::proxy::ProxyMsg;
        let mut e = engine(GLOBAL, "");
        tell(&mut e, ProxyMsg::PlayerJoin(remote(50, "Far", "lobby", false)));
        let _ = e.tick(1000, server(), vec![player("Steve", 1)]);
        let out = e.unload();
        assert!(for_whom(&out, 1).iter().any(|o| matches!(o, Out::ListRemove(ids) if ids == &vec![50])));
    }

    fn bridge_answers(e: &mut Engine) -> Vec<crate::horizon_tab::bridge::ToProxy> {
        e.bridge_outbox()
            .into_iter()
            .map(|(_, b)| crate::horizon_tab::bridge::decode_to_proxy(&b, false).unwrap_or_else(|_| crate::horizon_tab::bridge::decode_to_proxy(&b, true).unwrap()))
            .collect()
    }

    #[test]
    fn the_bridge_answers_a_join_and_follows_the_player() {
        use crate::horizon_tab::bridge::{encode_from_proxy, FromProxy, PlaceholderValue, ToProxy};
        let mut e = engine("", "");
        let _ = e.tick(1000, server(), vec![player("Steve", 1)]);
        let join = FromProxy::PlayerJoin {
            forward_group: false,
            placeholders: vec![("%player%".into(), 1000), ("%ping%".into(), 100), ("%nothing_known%".into(), 100)],
            replacements: vec![("%player%".into(), vec![("Steve".into(), "\u{a7}bSteve!".into())])],
        };
        assert!(e.bridge_receive(1, &encode_from_proxy(&join)).is_empty());
        let answers = bridge_answers(&mut e);
        let ToProxy::PlayerJoinResponse { world, group, values, game_mode } = &answers[0] else { panic!("{answers:?}") };
        assert_eq!((world.as_str(), group.as_ref(), *game_mode), ("world", None, 0));
        // The proxy's replacement applies, the ping is this server's, the unknown one is left to the proxy.
        assert_eq!(values.len(), 2, "{values:?}");
        assert_eq!(values[0], ("%player%".to_owned(), PlaceholderValue::Plain("\u{a7}bSteve!".to_owned())));
        assert_eq!(values[1], ("%ping%".to_owned(), PlaceholderValue::Plain("25".to_owned())));
        // Nothing changed: nothing is sent. A new ping and a world change are.
        let _ = e.tick(1100, server(), vec![player("Steve", 1)]);
        assert!(bridge_answers(&mut e).is_empty(), "nothing changed");
        let mut moved = player("Steve", 1);
        moved.dimension = "minecraft:the_nether".into();
        moved.ping = 80;
        moved.game_mode = 1;
        let _ = e.tick(1700, server(), vec![moved]);
        let answers = bridge_answers(&mut e);
        assert!(answers.contains(&ToProxy::SetWorld("world_nether".into())), "{answers:?}");
        assert!(answers.contains(&ToProxy::UpdateGameMode(1)), "{answers:?}");
        assert!(answers.contains(&ToProxy::UpdatePlaceholder { identifier: "%ping%".into(), target: None, value: "80".into() }), "{answers:?}");
    }

    #[test]
    fn the_bridge_answers_permission_questions_and_builds_scoreboards() {
        use crate::horizon_tab::bridge::{encode_from_proxy, FromProxy, ToProxy};
        let mut e = engine("", "");
        let _ = e.tick(1000, server(), vec![player("Steve", 1)]);
        let send = |e: &mut Engine, m: FromProxy| e.bridge_receive(1, &encode_from_proxy(&m));
        send(&mut e, FromProxy::PlayerJoin { forward_group: true, placeholders: vec![], replacements: vec![] });
        let answers = bridge_answers(&mut e);
        assert!(matches!(&answers[0], ToProxy::PlayerJoinResponse { group: Some(_), .. }), "{answers:?}");
        send(&mut e, FromProxy::Permission("tab.staff".into()));
        let answers = bridge_answers(&mut e);
        assert_eq!(answers, vec![ToProxy::HasPermission { permission: "tab.staff".into(), value: false }]);
        // Scoreboard messages become packets for that player.
        let team = send(
            &mut e,
            FromProxy::Team {
                name: "01Steve".into(),
                action: 0,
                prefix: "&cP".into(),
                suffix: String::new(),
                options: 3,
                visibility: "never".into(),
                collision: "pushOwnTeam".into(),
                color: 12,
                players: vec!["Steve".into()],
            },
        );
        let Out::TeamCreate { name, params, players } = &team[0].out else { panic!("{team:?}") };
        assert_eq!((name.as_str(), params.visibility, params.collision, params.color, params.options), ("01Steve", 1, 3, Some(12), 3));
        assert_eq!(players, &vec!["Steve".to_owned()]);
        assert_eq!(team[0].to, 1);
        let obj = send(
            &mut e,
            FromProxy::Objective { name: "o".into(), action: 0, title: "{\"text\":\"T\"}".into(), display: 1, number_format: Some("blank".into()) },
        );
        assert!(matches!(&obj[0].out, Out::Objective { method: ObjectiveMethod::Add, hearts: true, format: Some(NumberFmt::Blank), .. }));
        let score = send(&mut e, FromProxy::Score { objective: "o".into(), action: 0, holder: "Steve".into(), score: 7, display_name: None, number_format: None });
        assert!(matches!(&score[0].out, Out::Score { score: 7, .. }));
        // Unload takes the team and the objective away again.
        let gone = send(&mut e, FromProxy::Unload);
        assert!(gone.iter().any(|p| matches!(&p.out, Out::TeamRemove { name } if name == "01Steve")));
        assert!(gone.iter().any(|p| matches!(&p.out, Out::Objective { method: ObjectiveMethod::Remove, .. })));
        assert!(!e.bridge.active(1));
    }

    #[test]
    fn a_broken_bridge_message_is_noted_not_fatal() {
        let mut e = engine("", "");
        let _ = e.tick(1000, server(), vec![player("Steve", 1)]);
        assert!(e.bridge_receive(1, &[0, 3, b'x', b'y', b'z']).is_empty());
        assert_eq!(e.bridge.problems.len(), 1);
    }
}
