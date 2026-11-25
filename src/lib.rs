//! TAB as an arismoko native mod (trait mod on `mod_api`). The logic is the unchanged
//! `horizon-tab-core` crate (`core/`); this file maps its hooks, its host calls and the packets it
//! answers onto arismoko's API. Gaps are marked `TODO(arismoko-api: <name>)`; PORT_NOTES.md lists them.
//!
//! Ported: tab list header and footer (`player_tab_list`), chat lines (`player_tell`), the
//! `/tab` family and the toggle commands (`register_commands` / `run_mod_command`), boss bars
//! (host boss bar calls, one bar per viewer), the tab list name (`player_tab_list_name_format`),
//! the tick (`server_tick`), respawns and the stop.
//!
//! Packets (`player_send_packet`, `team_packet_body`): teams (name tags and sorting, with prefix,
//! suffix, visibility, collision, friendly fire and seeing invisible teammates) are sent to each
//! viewer alone, as are latency and game mode; list names per viewer (`player_set_list_name_for`),
//! the sidebar (`player_set_sidebar`), the list and below-name numbers (`player_set_display_score`).
//! The guard (`watch_outgoing_packets` + `server_packet`) puts back what another sender changes.
//! Permission nodes are declared once (`permission_gather_nodes`) and asked of the game as typed
//! Boolean queries each tick; the groups the game's provider published answer the group lookup.


pub mod horizon_tab;
use crate::horizon_tab::api;
use crate::horizon_tab::engine::PlayerIn;
use crate::horizon_tab::out::{BossOut, Out, Packet};
use crate::horizon_tab::placeholders::Server;
use mod_api::serde_json::Value;
use mod_api::{
    BossBarColor, BossBarOverlay, Command, CommandArgumentValue, CommandSource, Flow, Hooks,
    LoopContext, NodeDefault, PermissionGatherNodesEvent, PermissionLevel, PermissionNode,
    PermissionValueType, PlayerRef, PlayerRespawnedEvent, PlayerTabListNameFormatEvent,
    RegisterCommandsEvent, RunModCommandEvent, ServerAboutToStartEvent, ServerHost,
    ServerStoppingEvent, ServerTickEvent, ServerTickPhase, StatType, Suggestion, TabCompleteEvent,
    Text,
};
use crate::horizon_tab::out::{NumberFmt, ObjectiveMethod, TeamParams};
use mod_api::{
    ClientboundPlayPacket, Identifier, PlayerCommandEvent, ServerPacketEvent, Sidebar, SidebarLine,
    TeamPacketSpec, TeamParameters,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

/// The player cap when the server reports none (no dedicated server).
const DEFAULT_MAX_PLAYERS: i32 = 20;
/// The version the server reports to TAB's placeholders (`%version%`), as the Paper plugin saw it.
const SERVER_VERSION: &str = "26.3";
/// `UPDATE_GAME_MODE` and `UPDATE_LATENCY` of `ClientboundPlayerInfoUpdatePacket`'s action bits.
const INFO_GAME_MODE: u8 = 0x04;
const INFO_LATENCY: u8 = 0x10;

#[derive(Default)]
struct Mod;

fn log(line: &str) {
    eprintln!("[tab] {line}");
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The command line the core parses, rebuilt in the order of the usage that matched. The game gives
/// the literals (in order) and the argument values apart, so the usage says where each value goes:
/// `tab player <name> remove` has the literal `remove` after the name. With no usage matching, the
/// literals come first and the values after them (the order this glue used before).
fn command_line(literals: &[String], values: &[String]) -> String {
    let fixed: Vec<&str> = literals.iter().map(String::as_str).collect();
    for (usage, _) in api::commands() {
        let tokens: Vec<&str> = usage.split_whitespace().collect();
        let words: Vec<&str> = tokens
            .iter()
            .copied()
            .filter(|t| !t.starts_with('<') && !t.starts_with('['))
            .collect();
        let required = tokens.iter().filter(|t| t.starts_with('<')).count();
        let slots = tokens.len() - words.len();
        // The literals must be the usage's words in order; the values fill its slots in order, and
        // only the optional ones (`[...]`, at the end of a usage) may be left out.
        if words != fixed || values.len() < required || values.len() > slots {
            continue;
        }
        let mut next = values.iter();
        let mut parts: Vec<&str> = Vec::new();
        for &token in &tokens {
            if token.starts_with('<') || token.starts_with('[') {
                if let Some(value) = next.next() {
                    parts.push(value.as_str());
                }
            } else {
                parts.push(token);
            }
        }
        return parts.join(" ");
    }
    literals.iter().chain(values.iter()).map(String::as_str).collect::<Vec<&str>>().join(" ")
}

/// A JSON component as the engine reads it (`text_from_json`, as `/tellraw` does). A value that is
/// not one component reads as empty.
fn text_json(host: &dyn ServerHost, value: &Value) -> Text {
    host.text_from_json(&value.to_string())
        .ok()
        .flatten()
        .unwrap_or_else(|| Text::literal(""))
}

/// A JSON chat component as a `Text`: text, colour, decorations and `extra` children. Other
/// component kinds (translate, score, selector, nbt) keep only their text, which is empty here.
/// Used only by the tab-list name hook, which gets no host: `LoopContext` has no text reader.
// TODO(arismoko-api: text_from_component): the tab-list name hook takes only a LoopContext, which
// cannot call `text_from_json`. This covers what TAB writes (plain text, colours, decorations).
// Keybind, translate, score and nbt components lose their content there.
fn text_of(value: &Value) -> Text {
    let mut text = match value {
        Value::String(plain) => Text::literal(plain.as_str()),
        Value::Object(_) => Text::literal(value.get("text").and_then(Value::as_str).unwrap_or("")),
        _ => Text::literal(""),
    };
    if let Some(color) = value.get("color").and_then(Value::as_str) {
        text.style.color = Some(color.to_owned());
    }
    text.style.bold = value.get("bold").and_then(Value::as_bool);
    text.style.italic = value.get("italic").and_then(Value::as_bool);
    text.style.underlined = value.get("underlined").and_then(Value::as_bool);
    text.style.strikethrough = value.get("strikethrough").and_then(Value::as_bool);
    text.style.obfuscated = value.get("obfuscated").and_then(Value::as_bool);
    for child in value
        .get("extra")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        text = text.append(text_of(child));
    }
    text
}

fn ping_of(host: &dyn ServerHost, player: PlayerRef) -> i32 {
    player.latency(host).unwrap_or(0)
}
fn nickname_of(host: &dyn ServerHost, player: PlayerRef) -> Option<String> {
    player.nickname(host).flatten()
}
fn protocol_of(host: &dyn ServerHost, player: PlayerRef) -> i32 {
    player.protocol_version(host).unwrap_or(0)
}
/// `player_hide` hides a player from everyone below a permission level; any level reads as vanished.
fn vanished_of(host: &dyn ServerHost, player: PlayerRef) -> bool {
    player.hidden(host).flatten().is_some()
}

/// The player as the TAB core reads them each tick.
fn player_in(host: &dyn ServerHost, player: PlayerRef) -> PlayerIn {
    let entity = player.entity();
    let [yaw, pitch] = entity.rotation(host).unwrap_or([0.0, 0.0]);
    let health = host
        .living(entity)
        .and_then(|living| living.health(host))
        .unwrap_or(20.0);
    PlayerIn {
        uuid: player.uuid(),
        name: player.name(host).unwrap_or_default(),
        dimension: player
            .dimension(host)
            .map(|d| d.to_string())
            .unwrap_or_default(),
        position: entity.position(host).unwrap_or([0.0; 3]),
        yaw,
        pitch,
        health,
        game_mode: player.game_mode(host).map(|mode| mode.id()).unwrap_or(0),
        op_level: player.permission_level(host).unwrap_or(0),
        ping: ping_of(host, player),
        nickname: nickname_of(host, player),
        vanished: vanished_of(host, player),
        invisible: entity.is_invisible(host).unwrap_or(false),
        deaths: player
            .stat(host, StatType::Custom, "minecraft:deaths")
            .unwrap_or(0),
        entity_id: entity.id() as i32,
        protocol: protocol_of(host, player),
        fake: player.is_fake(host).unwrap_or(false),
    }
}

/// A VarInt as the protocol writes it.
fn write_varint(out: &mut Vec<u8>, mut value: i32) {
    loop {
        if value & !0x7F == 0 {
            out.push(value as u8);
            return;
        }
        out.push((value as u8 & 0x7F) | 0x80);
        value = ((value as u32) >> 7) as i32;
    }
}

/// `ClientboundPlayerInfoUpdatePacket` with one action: each entry is its uuid, then one VarInt
/// (a latency in ms, or a game mode id).
fn player_info_body(action: u8, entries: &[(u128, i32)]) -> Vec<u8> {
    let mut body = vec![action];
    write_varint(&mut body, entries.len() as i32);
    for (uuid, value) in entries {
        body.extend_from_slice(&uuid.to_be_bytes());
        write_varint(&mut body, *value);
    }
    body
}

/// Sends one clientbound play packet to `viewer` alone (`player_send_packet`).
fn send_raw(host: &mut dyn ServerHost, viewer: u128, name: &str, body: Vec<u8>) {
    let Some(player) = host.player(viewer) else { return };
    let Ok(id) = Identifier::parse(name) else { return };
    let _ = host.player_send_packet(player, ClientboundPlayPacket::new(id, body));
}

/// Sends one player-info action to `viewer` alone.
fn send_player_info(host: &mut dyn ServerHost, viewer: u128, action: u8, entries: &[(u128, i32)]) {
    send_raw(host, viewer, "minecraft:player_info_update", player_info_body(action, entries));
}

/// The permission answer for a Boolean node, asked of the game as a typed query. Any refusal
/// (an undeclared node, a player the server does not know) is a denial, never a grant.
fn host_answer(host: &mut dyn ServerHost, uuid: u128, node: &str) -> Option<bool> {
    let Some(player) = host.player(uuid) else {
        return Some(false);
    };
    let node = PermissionNode::new(node, PermissionValueType::Boolean);
    Some(player.permission_bool(host, &node).unwrap_or(false))
}

/// The packets the core answers, sent to the players they name.
fn send(host: &mut dyn ServerHost, packets: Vec<Packet>) {
    let mut changed = HashSet::new();
    for Packet { to, out } in packets {
        match out {
            Out::TabList { header, footer, .. } => {
                let (header, footer) = (text_json(&*host, &header), text_json(&*host, &footer));
                if let Some(player) = host.player(to) {
                    let _ = player.tab_list(host, &header, &footer);
                }
            }
            Out::Chat(line) => {
                let text = text_json(&*host, &line);
                if let Some(player) = host.player(to) {
                    let _ = player.tell(host, text);
                }
            }
            Out::Boss { id, action } => boss(host, to, id, action),
            Out::Latency(entries) => send_player_info(host, to, INFO_LATENCY, &entries),
            Out::GameModes(entries) => send_player_info(host, to, INFO_GAME_MODE, &entries),
            Out::DisplayNames(entries) => display_names(host, to, entries),
            Out::TeamCreate { name, params, players } => {
                let parameters = team_parameters(&*host, &params);
                team_packet(host, to, TeamPacketSpec::Create { name, parameters, players });
            }
            Out::TeamUpdate { name, params } => {
                let parameters = team_parameters(&*host, &params);
                team_packet(host, to, TeamPacketSpec::Update { name, parameters });
            }
            Out::TeamRemove { name } => team_packet(host, to, TeamPacketSpec::Remove { name }),
            out @ (Out::Objective { .. } | Out::DisplaySlot { .. } | Out::Score { .. } | Out::ResetScore { .. }) => {
                let applied = state().views.entry(to).or_default().apply(out);
                if applied.sidebar {
                    changed.insert(to);
                }
                for (slot, owner, score, fancy) in applied.scores {
                    display_score(host, to, slot, &owner, score, fancy.as_ref());
                }
            }
        }
    }
    for viewer in changed {
        sidebar(host, viewer);
    }
}

/// What the TAB core sent so far that the engine keeps elsewhere: the display names set per
/// viewer and each viewer's objectives.
#[derive(Default)]
struct State {
    /// (viewer, player) -> list name JSON as last set (None = the usual name).
    list_names: HashMap<(u128, u128), Option<String>>,
    views: HashMap<u128, View>,
    /// The guard's packet ids are being watched.
    watching: bool,
    /// (viewer, player) list names TAB sent this tick and the guard has not yet seen go out: a
    /// display name packet to a viewer that is a mod's send without one of these is another
    /// sender's (`ServerPacketEvent::from_mod` cannot tell TAB from another mod).
    own_names: HashSet<(u128, u128)>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The tab list slot (`DisplaySlot.LIST`), the sidebar and below the name, as the core numbers them.
const SLOT_LIST: i32 = 0;
const SLOT_SIDEBAR: i32 = 1;
const SLOT_BELOW_NAME: i32 = 2;

/// What one packet changed on a viewer's screen.
#[derive(Default)]
struct Applied {
    /// The sidebar may look different now.
    sidebar: bool,
    /// (slot 0 or 2, score owner, new score or None to take it out of the slot, the fancy value
    /// text the number format shows instead of the number).
    scores: Vec<(u8, String, Option<i32>, Option<Value>)>,
}

/// One viewer's objectives as the core's packets built them. The sidebar goes out whole
/// (`player_set_sidebar`); the list and below-name slots are numbers per player
/// (`player_set_display_score`).
#[derive(Default)]
struct View {
    titles: HashMap<String, Value>,
    /// Objective shown in the slot (sidebar, list, below name).
    sidebar: Option<String>,
    list: Option<String>,
    below: Option<String>,
    /// objective -> owner -> (score, display, number format)
    scores: HashMap<String, BTreeMap<String, (i32, Option<Value>, Option<NumberFmt>)>>,
}

impl View {
    fn numbered_slot(&self, objective: &str) -> Vec<u8> {
        let mut slots = Vec::new();
        if self.list.as_deref() == Some(objective) {
            slots.push(0);
        }
        if self.below.as_deref() == Some(objective) {
            slots.push(2);
        }
        slots
    }

    /// Every score of an objective, as the slot shows it.
    fn push_all(&self, slot: u8, objective: &str, set: bool, applied: &mut Applied) {
        for (owner, (score, _, format)) in self.scores.get(objective).into_iter().flatten() {
            applied.scores.push((slot, owner.clone(), set.then_some(*score), fixed(format.as_ref())));
        }
    }

    /// Applies one packet; says what it changed.
    fn apply(&mut self, out: Out) -> Applied {
        let mut applied = Applied::default();
        match out {
            Out::Objective { name, method, title, .. } => {
                match method {
                    ObjectiveMethod::Remove => {
                        for slot in self.numbered_slot(&name) {
                            self.push_all(slot, &name, false, &mut applied);
                        }
                        if self.list.as_deref() == Some(name.as_str()) {
                            self.list = None;
                        }
                        if self.below.as_deref() == Some(name.as_str()) {
                            self.below = None;
                        }
                        applied.sidebar = self.sidebar.as_deref() == Some(name.as_str());
                        if applied.sidebar {
                            self.sidebar = None;
                        }
                        self.titles.remove(&name);
                        self.scores.remove(&name);
                    }
                    _ => {
                        applied.sidebar = self.sidebar.as_deref() == Some(name.as_str());
                        self.titles.insert(name, title);
                    }
                }
            }
            Out::DisplaySlot { slot, objective } => {
                let new = (!objective.is_empty()).then_some(objective);
                match slot {
                    SLOT_SIDEBAR => {
                        self.sidebar = new;
                        applied.sidebar = true;
                    }
                    SLOT_LIST | SLOT_BELOW_NAME => {
                        let number = slot as u8;
                        let old = if slot == SLOT_LIST { &mut self.list } else { &mut self.below };
                        let old = std::mem::replace(old, new.clone());
                        if old != new {
                            if let Some(old) = old {
                                self.push_all(number, &old, false, &mut applied);
                            }
                            if let Some(new) = new {
                                self.push_all(number, &new, true, &mut applied);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Out::Score { owner, objective, score, display, format } => {
                applied.sidebar = self.sidebar.as_deref() == Some(objective.as_str());
                for slot in self.numbered_slot(&objective) {
                    applied.scores.push((slot, owner.clone(), Some(score), fixed(format.as_ref())));
                }
                self.scores.entry(objective).or_default().insert(owner, (score, display, format));
            }
            Out::ResetScore { owner, objective } => {
                applied.sidebar = self.sidebar.as_deref() == Some(objective.as_str());
                for slot in self.numbered_slot(&objective) {
                    applied.scores.push((slot, owner.clone(), None, None));
                }
                if let Some(s) = self.scores.get_mut(&objective) {
                    s.remove(&owner);
                }
            }
            _ => {}
        }
        applied
    }

    /// The sidebar objective's title and rows (text, right-hand value), as vanilla orders them:
    /// highest score first, equal scores by name, at most 15. The text is the line's display (else
    /// its owner); the right side is the number format's text, nothing for `blank`, else the score.
    fn sidebar_rows(&self) -> Option<(Value, Vec<(Value, Option<Value>)>)> {
        let name = self.sidebar.as_ref()?;
        let title = self.titles.get(name)?.clone();
        let mut rows: Vec<(&String, &(i32, Option<Value>, Option<NumberFmt>))> =
            self.scores.get(name).into_iter().flatten().collect();
        rows.sort_by(|a, b| {
            b.1.0.cmp(&a.1.0).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
        });
        let rows = rows
            .into_iter()
            .take(15)
            .map(|(owner, (score, display, format))| {
                let text = display.clone().unwrap_or_else(|| Value::String(owner.clone()));
                let value = match format {
                    Some(NumberFmt::Blank) => None,
                    Some(NumberFmt::Fixed(v)) => Some(v.clone()),
                    None => Some(Value::String(score.to_string())),
                };
                (text, value)
            })
            .collect();
        Some((title, rows))
    }
}

/// The viewer's sidebar from their objectives (`player_set_sidebar`).
fn sidebar(host: &mut dyn ServerHost, viewer: u128) {
    let Some(player) = host.player(viewer) else { return };
    let rows = state().views.get(&viewer).and_then(View::sidebar_rows);
    let shown = rows.map(|(title, rows)| {
        let lines = rows
            .iter()
            .map(|(text, value)| {
                let value = value.as_ref().map(|v| text_json(&*host, v));
                SidebarLine::new(text_json(&*host, text), value)
            })
            .collect();
        Sidebar::new(text_json(&*host, &title), lines)
    });
    let _ = player.set_sidebar(host, shown);
}

/// The text of a fixed number format (the fancy value), none for the number itself or `blank`.
fn fixed(format: Option<&NumberFmt>) -> Option<Value> {
    match format {
        Some(NumberFmt::Fixed(v)) => Some(v.clone()),
        _ => None,
    }
}

/// A number in the tab list (slot 0) or below a name (slot 2) for `viewer`
/// (`player_set_display_score`); the owner is a player's name. A fancy value ("Ping: 40") is sent
/// over it as the score's fixed number format (`player_set_display_score_text`), when the world
/// shows an objective in that slot; otherwise the number stays.
fn display_score(
    host: &mut dyn ServerHost,
    viewer: u128,
    slot: u8,
    owner: &str,
    score: Option<i32>,
    fancy: Option<&Value>,
) {
    let Some(viewer) = host.player(viewer) else { return };
    let target = host
        .players()
        .server_players()
        .into_iter()
        .find(|p| p.name(&*host).is_some_and(|n| n.eq_ignore_ascii_case(owner)));
    if let Some(target) = target {
        let _ = viewer.set_display_score(host, slot, target, score);
        if let (Some(score), Some(fancy)) = (score, fancy) {
            let name = match slot {
                0 => "list",
                _ => "below_name",
            };
            let text = text_json(&*host, fancy);
            let _ = viewer.set_display_score_text(host, name, owner, score, Some(text));
        }
    }
}

/// TAB's team parameters as the packet's.
fn team_parameters(host: &dyn ServerHost, params: &TeamParams) -> TeamParameters {
    let mut team = TeamParameters::new();
    team.display_name = text_json(host, &params.display_name);
    team.prefix = text_json(host, &params.prefix);
    team.suffix = text_json(host, &params.suffix);
    team.name_tag_visibility = params.visibility;
    team.collision_rule = params.collision;
    team.color = params.color;
    team.options = params.options;
    team
}

/// A team as one viewer's client holds it (`ClientboundSetPlayerTeamPacket`, built by the game
/// and sent to that viewer alone), so prefix, suffix, visibility, collision, friendly fire and
/// seeing invisible teammates differ per viewer as TAB's do.
fn team_packet(host: &mut dyn ServerHost, viewer: u128, spec: TeamPacketSpec) {
    if let Ok(body) = host.team_packet_body(spec) {
        send_raw(host, viewer, "minecraft:set_player_team", body);
    }
}

/// TAB's tab list names, per viewer (`player_set_list_name_for`), and only when one changed.
fn display_names(host: &mut dyn ServerHost, viewer: u128, entries: Vec<(u128, Option<Value>)>) {
    let Some(viewer_ref) = host.player(viewer) else { return };
    for (uuid, name) in entries {
        let key = name.as_ref().map(Value::to_string);
        if state().list_names.get(&(viewer, uuid)) == Some(&key) {
            continue;
        }
        let Some(target) = host.player(uuid) else { continue };
        let text = name.as_ref().map(|json| text_json(&*host, json));
        let _ = viewer_ref.set_list_name_for(host, target, text);
        let mut st = state();
        st.own_names.insert((viewer, uuid));
        st.list_names.insert((viewer, uuid), key);
    }
}

/// Has the engine hold the packets the guard rules on, once and only when TAB would use them.
fn watch_guard(host: &mut dyn ServerHost) {
    if state().watching || !api::guard_needed() {
        return;
    }
    let ids = [crate::horizon_tab::guard::PLAYER_INFO_UPDATE, crate::horizon_tab::guard::SET_PLAYER_TEAM];
    if host.watch_outgoing_packets(&ids, true) {
        state().watching = true;
    }
}

/// The toggle commands that share a vanilla command's name (`/bossbar` by default): the tree
/// refuses them as usages, so `player_command` answers them.
const VANILLA_TOGGLES: [&str; 1] = ["bossbar"];

fn color_of(index: i32) -> Option<BossBarColor> {
    usize::try_from(index)
        .ok()
        .and_then(|i| BossBarColor::ALL.get(i).copied())
}

fn overlay_of(index: i32) -> Option<BossBarOverlay> {
    usize::try_from(index)
        .ok()
        .and_then(|i| BossBarOverlay::ALL.get(i).copied())
}

/// TAB's boss bars. Each bar belongs to one viewer (the core sends per viewer), so it is created
/// under an id naming both, holds that viewer alone and goes when the core removes it. Progress is
/// kept as 0..100 of max 100.
fn boss(host: &mut dyn ServerHost, viewer: u128, id: u128, action: BossOut) {
    let key = format!("tab_{id:032x}_{viewer:032x}");
    match action {
        BossOut::Add {
            title,
            progress,
            color,
            overlay,
        } => {
            let name = text_json(&*host, &title);
            let bar = match host.boss_bar(&key) {
                Some(bar) => Some(bar),
                None => host.create_boss_bar(&key, name.clone()).ok().flatten(),
            };
            let Some(bar) = bar else { return };
            let _ = bar.set_name(host, name);
            let _ = bar.set_max(host, 100);
            let _ = bar.set_value(host, (progress * 100.0).round() as i32);
            if let Some(color) = color_of(color) {
                let _ = bar.set_color(host, color);
            }
            if let Some(overlay) = overlay_of(overlay) {
                let _ = bar.set_overlay(host, overlay);
            }
            if let Some(player) = host.player(viewer) {
                let _ = bar.add_player(host, player);
            }
        }
        BossOut::Remove => {
            if let Some(bar) = host.boss_bar(&key) {
                let _ = bar.remove(host);
            }
        }
        BossOut::Progress(progress) => {
            if let Some(bar) = host.boss_bar(&key) {
                let _ = bar.set_value(host, (progress * 100.0).round() as i32);
            }
        }
        BossOut::Title(title) => {
            let name = text_json(&*host, &title);
            if let Some(bar) = host.boss_bar(&key) {
                let _ = bar.set_name(host, name);
            }
        }
        BossOut::Style { color, overlay } => {
            if let Some(bar) = host.boss_bar(&key) {
                if let Some(color) = color_of(color) {
                    let _ = bar.set_color(host, color);
                }
                if let Some(overlay) = overlay_of(overlay) {
                    let _ = bar.set_overlay(host, overlay);
                }
            }
        }
    }
}

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn permission_gather_nodes(&self, event: &mut PermissionGatherNodesEvent<'_>) {
        // The original plugin.yml declares every `tab.*` node with `default: op`, so each one gets operator
        // level 1 here, `tab.spectatorbypass` included. The `tab.group.<name>` nodes come from the config.
        let mut names = api::command_permission_nodes();
        names.push("tab.staff".to_owned());
        names.push("tab.bypass".to_owned());
        names.push("tab.seevanished".to_owned());
        names.extend(api::group_permission_nodes(Path::new("config")));
        names.extend(api::condition_permission_nodes(Path::new("config")));
        for name in names {
            let node = PermissionNode::new(name, PermissionValueType::Boolean);
            let _ = event.nodes.add(node, NodeDefault::Level(PermissionLevel::from_level(1)));
        }
        let bypass = PermissionNode::new("tab.spectatorbypass", PermissionValueType::Boolean);
        let _ = event.nodes.add(bypass, NodeDefault::Level(PermissionLevel::from_level(1)));
    }

    fn server_about_to_start(&self, _: &mut dyn ServerHost, _: &mut ServerAboutToStartEvent) {
        if api::started() {
            return;
        }
        // The core reads `plugins/TAB/` relative to the server folder, as the Paper plugin did.
        for line in api::start(Path::new("config")) {
            log(&line);
        }
        log("ported: tab list header/footer, one list name per player, boss bars, chat, commands and placeholders.");
        log("Shown: header/footer, per-viewer list names, name tags and sorting (teams), sidebar, list and below-name numbers, ping and game mode, boss bars; the guard watches other senders' team, latency and game mode packets.");
        // The permission groups the game's provider published answer TAB's group lookup.
        crate::horizon_tab::integration::register(Box::new(crate::horizon_tab::integration::HostGroups));
    }

    /// Online names for a `<name:player_name>` slot of `/tab` (the registered usage is a word, which takes
    /// offline names too; `api::player_name_expected` finds the slot).
    fn tab_complete(&self, host: &mut dyn ServerHost, event: &mut TabCompleteEvent<'_>) {
        let line = event.command.trim_start_matches('/');
        let mut words: Vec<&str> = line.split(' ').collect();
        let root = words.remove(0);
        if !root.eq_ignore_ascii_case("tab") || words.is_empty() {
            return;
        }
        // TAB's `/tab` completes nothing for a sender without `tab.tabcomplete` (or `tab.admin`).
        let may_complete = ["tab.admin", "tab.tabcomplete"]
            .iter()
            .any(|node| host_answer(host, event.player, node) == Some(true));
        if !may_complete {
            return;
        }
        let usages: Vec<String> = api::commands().into_iter().map(|(usage, _)| usage).collect();
        if !api::player_name_expected(&usages, root, &words) {
            return;
        }
        let typed = words.last().copied().unwrap_or("").to_lowercase();
        let [start, end] = event.range;
        for player in host.players().server_players() {
            let Some(name) = player.name(&*host) else {
                continue;
            };
            if name.to_lowercase().starts_with(&typed) && !event.suggestions.iter().any(|s| s.text == name) {
                event.suggestions.push(Suggestion::new((start, end), name));
            }
        }
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        for (usage, level) in api::commands() {
            // A word takes any name (an offline one too); `tab_complete` suggests the online names.
            let usage = usage.replace(":player_name>", ":word>");
            event.commands.push(if level == 0 {
                Command::new(&usage)
            } else {
                Command::op(&usage, level)
            });
        }
    }

    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut RunModCommandEvent<'_>,
    ) -> Flow {
        let Some(root) = event.command.literals.first() else {
            return Flow::Continue;
        };
        // `/tab`, the scoreboard and bossbar toggles: the roots the core registered.
        let ours = api::commands()
            .iter()
            .any(|(usage, _)| usage.split_whitespace().next() == Some(root.as_str()));
        if !ours {
            return Flow::Continue;
        }
        // Every argument as the core reads it: a player by name (the core looks names up), numbers and
        // booleans as text. Selectors that are not one player are not used by any usage here.
        let mut values: Vec<String> = Vec::new();
        for argument in &event.command.arguments {
            let text = match &argument.value {
                CommandArgumentValue::String(s) => s.clone(),
                CommandArgumentValue::Integer(i) => i.to_string(),
                CommandArgumentValue::Number(n) => n.to_string(),
                CommandArgumentValue::Boolean(b) => b.to_string(),
                CommandArgumentValue::Player(p) => p.name(&*host).unwrap_or_default(),
                _ => continue,
            };
            values.push(text);
        }
        let line = command_line(&event.command.literals, &values);
        let answer = match &event.source {
            CommandSource::Player { player, permission } => {
                // Ask the game for every command node NOW, so the first command after a join sees the
                // real answer (the per-tick refresh below arrives one tick late and would refuse
                // `tab.admin`/`tab.grouplist` holders; parents: `tab.admin` is checked by the core).
                let now = now_ms();
                for node in api::command_permission_nodes() {
                    let answer = host_answer(host, player.uuid(), &node);
                    api::host_permission_answer(player.uuid(), &node, answer, now);
                }
                api::command(Some((player.uuid(), permission.level())), &line)
            }
            CommandSource::Console { .. } => api::command(None, &line),
            _ => return Flow::Continue,
        };
        if !answer.ran {
            return Flow::Continue;
        }
        for reply in answer.lines {
            let text = text_json(&*host, &reply);
            let _ = host.commands().feedback(text);
        }
        Flow::Cancel
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if !matches!(
            event.phase,
            ServerTickPhase::ServerTickStart {
                world_running: true,
                ..
            }
        ) || !api::started()
        {
            return;
        }
        let (rate, mspt) = host.tick_stats();
        let tps = if mspt > 0.0 {
            (rate as f64).min(1000.0 / mspt)
        } else {
            rate as f64
        };
        let level_name = host
            .world_dir()
            .and_then(|dir| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "world".to_owned());
        let (memory_used_mb, memory_max_mb) = api::process_memory_mb();
        let max_players = host
            .server_max_players()
            .map_or(DEFAULT_MAX_PLAYERS, |n| n as i32);
        let online = host.players().server_players();
        // The permission answers TAB asked for last tick: the game's answer for each node (None when
        // no permissions mod answers, so the operator level decides).
        let now = now_ms();
        for (uuid, node) in api::host_permission_requests(now) {
            let answer = host_answer(host, uuid, &node);
            api::host_permission_answer(uuid, &node, answer, now);
        }
        let present: HashSet<u128> = online.iter().map(|p| p.uuid()).collect();
        crate::horizon_tab::integration::host_permissions_retain(&present);
        crate::horizon_tab::integration::host_groups_set(
            online
                .iter()
                .map(|p| {
                    let groups = host.permission_groups(p.uuid());
                    (p.uuid(), groups.primary, groups.groups)
                })
                .collect(),
        );
        {
            let mut st = state();
            st.list_names.retain(|(viewer, _), _| present.contains(viewer));
            st.views.retain(|viewer, _| present.contains(viewer));
        }
        watch_guard(host);
        let players: Vec<PlayerIn> = online
            .into_iter()
            .map(|player| player_in(&*host, player))
            .collect();
        let server = Server {
            now_ms: now_ms(),
            started_ms: api::started_ms(),
            tps,
            mspt,
            max_players,
            online: 0,
            memory_used_mb,
            memory_max_mb,
            version: SERVER_VERSION.to_owned(),
            level_name,
        };
        let packets = api::tick(now_ms(), server, players);
        state().own_names.clear();
        send(host, packets);
    }

    /// Another sender's packets that undo TAB's: a team move, a ping or a game mode (anti-override).
    fn server_packet(&self, _: &mut dyn ServerHost, event: &mut ServerPacketEvent<'_>) -> Flow {
        // A display name a mod sent that TAB did not: put ours back with the next tick. The game's
        // own packets (`from_mod` false) carry the names of the tab-list name hook already.
        if event.from_mod && event.packet_id == crate::horizon_tab::guard::PLAYER_INFO_UPDATE {
            for uuid in crate::horizon_tab::guard::display_name_uuids(event.body.as_slice()) {
                if !state().own_names.remove(&(event.player, uuid)) {
                    api::display_name_overwritten(uuid);
                }
            }
        }
        match crate::horizon_tab::guard::rule(event.player, event.packet_id, event.body.as_slice()) {
            crate::horizon_tab::guard::Verdict::Keep => Flow::Continue,
            crate::horizon_tab::guard::Verdict::Cancel => Flow::Cancel,
            crate::horizon_tab::guard::Verdict::Replace(body) => {
                *event.body = body;
                Flow::Continue
            }
        }
    }

    /// `/bossbar` (TAB's toggle command by default) is a vanilla command's name, so the command tree
    /// would not take it as a usage: it is answered here, as the `bossbar` usages of `/tab` are.
    fn player_command(&self, host: &mut dyn ServerHost, event: &mut PlayerCommandEvent<'_>) -> Flow {
        let line = event.command.trim_start_matches('/').trim().to_owned();
        let root = line.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        let ours = VANILLA_TOGGLES.contains(&root.as_str())
            && api::commands()
                .iter()
                .any(|(usage, _)| usage.split_whitespace().next() == Some(root.as_str()));
        if !ours {
            return Flow::Continue;
        }
        let Some(player) = host.player(event.player) else { return Flow::Continue };
        let level = player.permission_level(&*host).unwrap_or(0);
        let now = now_ms();
        for node in api::command_permission_nodes() {
            let answer = host_answer(host, event.player, &node);
            api::host_permission_answer(event.player, &node, answer, now);
        }
        let answer = api::command(Some((event.player, level)), &line);
        if !answer.ran {
            return Flow::Continue;
        }
        for reply in answer.lines {
            event.replies.push(text_json(&*host, &reply));
        }
        event.cancelled = true;
        Flow::Cancel
    }

    fn player_respawned(&self, _: &mut dyn ServerHost, event: &mut PlayerRespawnedEvent<'_>) {
        // `event.player` is the UUID (`native_player_respawned` passes `player: u128`).
        api::player_respawned(event.player);
    }

    fn player_tab_list_name_format(
        &self,
        _: &dyn LoopContext,
        event: &mut PlayerTabListNameFormatEvent<'_>,
    ) {
        // TODO(verify): the hook's signature is `player_tab_list_name_format(context, player, name,
        // display_name)` (hooks/player_tab_list_name_format.rs); `event.player` is taken as the UUID,
        // which the engine's native hook is not checked for here.
        if let Some(name) = api::list_name(event.player) {
            *event.display_name = Some(text_of(&name));
        }
    }

    fn server_stopping(&self, host: &mut dyn ServerHost, _: &mut ServerStoppingEvent) {
        let packets = api::stop();
        send(host, packets);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sidebar_rows_sort_by_score_then_name_and_apply_formats() {
        let mut view = View::default();
        view.apply(Out::Objective {
            name: "sb".into(),
            method: ObjectiveMethod::Add,
            title: json!("Title"),
            hearts: false,
            format: None,
        });
        view.apply(Out::DisplaySlot { slot: 1, objective: "sb".into() });
        for (owner, score, display, format) in [
            ("b", 3, None, None),
            ("a", 3, Some(json!("Shown")), Some(NumberFmt::Fixed(json!("x")))),
            ("c", 9, None, Some(NumberFmt::Blank)),
        ] {
            view.apply(Out::Score { owner: owner.into(), objective: "sb".into(), score, display, format });
        }
        let (title, rows) = view.sidebar_rows().unwrap();
        assert_eq!(title, json!("Title"));
        assert_eq!(
            rows,
            vec![
                (json!("c"), None),
                (json!("Shown"), Some(json!("x"))),
                (json!("b"), Some(json!("3"))),
            ]
        );
    }

    #[test]
    fn sidebar_rows_are_none_without_a_sidebar_and_capped_at_15() {
        let mut view = View::default();
        assert!(view.sidebar_rows().is_none());
        view.apply(Out::Objective {
            name: "sb".into(),
            method: ObjectiveMethod::Add,
            title: json!("T"),
            hearts: false,
            format: None,
        });
        view.apply(Out::DisplaySlot { slot: 1, objective: "sb".into() });
        for i in 0..20 {
            view.apply(Out::Score {
                owner: format!("o{i:02}"),
                objective: "sb".into(),
                score: i,
                display: None,
                format: None,
            });
        }
        assert_eq!(view.sidebar_rows().unwrap().1.len(), 15);
    }

    #[test]
    fn list_and_below_name_slots_become_per_player_numbers() {
        let mut view = View::default();
        view.apply(Out::Objective {
            name: "ping".into(),
            method: ObjectiveMethod::Add,
            title: json!("TAB"),
            hearts: false,
            format: None,
        });
        // A score before the slot is shown is only kept.
        let applied = view.apply(Out::Score {
            owner: "a".into(),
            objective: "ping".into(),
            score: 40,
            display: None,
            format: None,
        });
        assert!(applied.scores.is_empty());
        // Showing the objective in the list slot sends what is kept.
        let applied = view.apply(Out::DisplaySlot { slot: 0, objective: "ping".into() });
        assert_eq!(applied.scores, vec![(0, "a".to_owned(), Some(40), None)]);
        // A change and a reset go out for that slot.
        let applied = view.apply(Out::Score {
            owner: "a".into(),
            objective: "ping".into(),
            score: 55,
            display: None,
            format: None,
        });
        assert_eq!(applied.scores, vec![(0, "a".to_owned(), Some(55), None)]);
        // A fancy value rides along as the text of the fixed number format.
        let applied = view.apply(Out::Score {
            owner: "a".into(),
            objective: "ping".into(),
            score: 56,
            display: None,
            format: Some(NumberFmt::Fixed(json!("Ping: 56"))),
        });
        assert_eq!(applied.scores, vec![(0, "a".to_owned(), Some(56), Some(json!("Ping: 56")))]);
        let applied = view.apply(Out::Score {
            owner: "a".into(),
            objective: "ping".into(),
            score: 55,
            display: None,
            format: None,
        });
        assert_eq!(applied.scores, vec![(0, "a".to_owned(), Some(55), None)]);
        let applied = view.apply(Out::ResetScore { owner: "a".into(), objective: "ping".into() });
        assert_eq!(applied.scores, vec![(0, "a".to_owned(), None, None)]);
        // Removing the objective takes its numbers out of the slot.
        view.apply(Out::Score {
            owner: "b".into(),
            objective: "ping".into(),
            score: 7,
            display: None,
            format: None,
        });
        let applied = view.apply(Out::Objective {
            name: "ping".into(),
            method: ObjectiveMethod::Remove,
            title: json!(""),
            hearts: false,
            format: None,
        });
        assert_eq!(applied.scores, vec![(0, "b".to_owned(), None, None)]);
        assert!(view.list.is_none());
    }
}
