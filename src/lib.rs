//! TAB as an arismoko native mod (trait mod on `mod_api`). The logic is the unchanged
//! `horizon-tab-core` crate (`core/`); this file maps its hooks, its host calls and the packets it
//! answers onto arismoko's API. Gaps are marked `TODO(arismoko-api: <name>)`; PORT_NOTES.md lists them.
//!
//! Ported: tab list header and footer (`player_tab_list`), chat lines (`player_tell`), the
//! `/tab` family and the toggle commands (`register_commands` / `run_mod_command`), boss bars
//! (host boss bar calls, one bar per viewer), the tab list name (`player_tab_list_name_format`),
//! the tick (`server_tick`), respawns and the stop.
//!
//! Packets: latency and game mode per viewer were to go out through `player_send_packet`, which
//! Horizon main lacks (HMOD-GAP). Permission nodes are declared once (`permission_gather_nodes`)
//! and asked of the game as typed Boolean queries each tick. Not ported: teams (name tags),
//! objectives (tab list ping and health, below name, sidebar), scores, per-viewer display names,
//! the packet guard, and the LuckPerms bridge (its placeholders and prefixes).


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
    ServerStoppingEvent, ServerTickEvent, ServerTickPhase, StatType, Text,
};
use crate::horizon_tab::out::{NumberFmt, ObjectiveMethod};
// Calls that exist on Horizon main only (feature `horizon_main_api`, off by default; see HMOD_PORT.md).
#[cfg(feature = "horizon_main_api")]
use mod_api::{
    ClientboundPlayPacket, Identifier, ScoreHolder, Sidebar, SidebarLine, TeamColor,
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

// `player_latency`, `player_nickname` and `player_hidden` (#17878) exist on Horizon main only. The
// pinned SDK has no read, so without the feature `%ping%` is 0, no nickname is read and nobody is vanished.
#[cfg(feature = "horizon_main_api")]
fn ping_of(host: &dyn ServerHost, player: PlayerRef) -> i32 {
    player.latency(host).unwrap_or(0)
}
#[cfg(not(feature = "horizon_main_api"))]
fn ping_of(_: &dyn ServerHost, _: PlayerRef) -> i32 {
    0
}
#[cfg(feature = "horizon_main_api")]
fn nickname_of(host: &dyn ServerHost, player: PlayerRef) -> Option<String> {
    player.nickname(host).flatten()
}
#[cfg(not(feature = "horizon_main_api"))]
fn nickname_of(_: &dyn ServerHost, _: PlayerRef) -> Option<String> {
    None
}
/// `player_hide` hides a player from everyone below a permission level; any level reads as vanished.
#[cfg(feature = "horizon_main_api")]
fn vanished_of(host: &dyn ServerHost, player: PlayerRef) -> bool {
    player.hidden(host).flatten().is_some()
}
#[cfg(not(feature = "horizon_main_api"))]
fn vanished_of(_: &dyn ServerHost, _: PlayerRef) -> bool {
    false
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
        // TODO(arismoko-api: player_client_protocol): no protocol read; `%player-version%` is wrong.
        protocol: 0,
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

/// Sends one player-info action to `viewer` alone.
// HMOD-GAP(player_send_packet): Horizon main has no server-side call that sends a clientbound play
// packet to one player (only the client's `send_packet`). Latency and game mode per viewer are not
// sent; the bodies are still built by `player_info_body`.
#[cfg(not(feature = "horizon_main_api"))]
fn send_player_info(host: &mut dyn ServerHost, viewer: u128, action: u8, entries: &[(u128, i32)]) {
    let _ = (host, viewer, player_info_body(action, entries));
}

/// `player_send_packet` of Horizon main sends the packet to that player as it is.
#[cfg(feature = "horizon_main_api")]
fn send_player_info(host: &mut dyn ServerHost, viewer: u128, action: u8, entries: &[(u128, i32)]) {
    let Some(player) = host.player(viewer) else { return };
    let Ok(id) = Identifier::parse("minecraft:player_info_update") else { return };
    let packet = ClientboundPlayPacket::new(id, player_info_body(action, entries));
    let _ = host.player_send_packet(player, packet);
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
            // TODO(arismoko-api: watch_server_packets): the anti-override guard (`guard_packet`)
            // is not wired: other senders' team, latency, game mode and name packets are not
            // put back.
            Out::DisplayNames(entries) => display_names(host, to, entries),
            Out::TeamCreate { name, params, players } => team(host, &name, params.color, Some(players)),
            Out::TeamUpdate { name, params } => team(host, &name, params.color, None),
            Out::TeamRemove { name } => team_remove(host, &name),
            out @ (Out::Objective { .. } | Out::DisplaySlot { .. } | Out::Score { .. } | Out::ResetScore { .. }) => {
                if state().views.entry(to).or_default().apply(out) {
                    changed.insert(to);
                }
            }
        }
    }
    for viewer in changed {
        sidebar(host, viewer);
    }
}

/// What the TAB core sent so far that the engine keeps elsewhere: the world's teams (shared, so a
/// team is set once, not once per viewer), the tab list names set and each viewer's objectives.
#[derive(Default)]
struct State {
    /// Team name -> its colour as last set.
    teams: HashMap<String, Option<i32>>,
    /// Player name -> team they were put on.
    members: HashMap<String, String>,
    /// Player -> list name JSON as last set (None = real name).
    list_names: HashMap<u128, Option<String>>,
    views: HashMap<u128, View>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// One viewer's objectives as the core's packets built them; only the sidebar slot is shown
/// (`player_set_sidebar`). TODO(arismoko-api: objective slots): the list slot (tab list values) and the
/// below-name slot have no API and are dropped.
#[derive(Default)]
struct View {
    titles: HashMap<String, Value>,
    sidebar: Option<String>,
    /// objective -> owner -> (score, display, number format)
    scores: HashMap<String, BTreeMap<String, (i32, Option<Value>, Option<NumberFmt>)>>,
}

impl View {
    /// Applies one packet; true when the sidebar may look different now.
    fn apply(&mut self, out: Out) -> bool {
        match out {
            Out::Objective { name, method, title, .. } => {
                let shown = self.sidebar.as_deref() == Some(name.as_str());
                match method {
                    ObjectiveMethod::Remove => {
                        self.titles.remove(&name);
                        self.scores.remove(&name);
                        if shown {
                            self.sidebar = None;
                        }
                    }
                    _ => {
                        self.titles.insert(name, title);
                    }
                }
                shown
            }
            Out::DisplaySlot { slot, objective } if slot == 1 => {
                self.sidebar = (!objective.is_empty()).then_some(objective);
                true
            }
            Out::DisplaySlot { .. } => false,
            Out::Score { owner, objective, score, display, format } => {
                let shown = self.sidebar.as_deref() == Some(objective.as_str());
                self.scores.entry(objective).or_default().insert(owner, (score, display, format));
                shown
            }
            Out::ResetScore { owner, objective } => {
                let shown = self.sidebar.as_deref() == Some(objective.as_str());
                if let Some(s) = self.scores.get_mut(&objective) {
                    s.remove(&owner);
                }
                shown
            }
            _ => false,
        }
    }
}

impl View {
    /// The sidebar objective's title and rows (text, right-hand value), as vanilla orders them:
    /// highest score first, equal scores by name, at most 15. The text is the line's display (else
    /// its owner); the right side is the number format's text, nothing for `blank`, else the score.
    #[cfg_attr(not(feature = "horizon_main_api"), allow(dead_code))]
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
// HMOD-GAP(set_sidebar): the pinned SDK has no sidebar call; the objectives are still kept by
// `View::apply`. With `horizon_main_api` the sidebar of Horizon main (#17948) shows them.
#[cfg(not(feature = "horizon_main_api"))]
fn sidebar(host: &mut dyn ServerHost, viewer: u128) {
    let _ = (host, viewer);
}

#[cfg(feature = "horizon_main_api")]
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

/// `TeamColor` index (0..15) to the chat colour name `team_set` takes.
const COLORS: [&str; 16] = [
    "black", "dark_blue", "dark_green", "dark_aqua", "dark_red", "dark_purple", "gold", "gray",
    "dark_gray", "blue", "green", "aqua", "red", "light_purple", "yellow", "white",
];

#[cfg(feature = "horizon_main_api")]
fn team_color(index: i32) -> Option<TeamColor> {
    Some(match index {
        0 => TeamColor::Black,
        1 => TeamColor::DarkBlue,
        2 => TeamColor::DarkGreen,
        3 => TeamColor::DarkAqua,
        4 => TeamColor::DarkRed,
        5 => TeamColor::DarkPurple,
        6 => TeamColor::Gold,
        7 => TeamColor::Gray,
        8 => TeamColor::DarkGray,
        9 => TeamColor::Blue,
        10 => TeamColor::Green,
        11 => TeamColor::Aqua,
        12 => TeamColor::Red,
        13 => TeamColor::LightPurple,
        14 => TeamColor::Yellow,
        15 => TeamColor::White,
        _ => return None,
    })
}

/// A TAB team (name tags and sorting) as a team of the world's scoreboard.
// HMOD-GAP(team_prefix_suffix): the pinned SDK has no scoreboard team calls at all, so the core's
// team packets are dropped. With `horizon_main_api` (the teams of PR #17980) a team takes its
// colour and members only: no prefix, suffix, name tag visibility, collision or friendly fire.
#[cfg(not(feature = "horizon_main_api"))]
fn team(host: &mut dyn ServerHost, name: &str, color: Option<i32>, players: Option<Vec<String>>) {
    let _ = (host, name, color, players, COLORS);
}

#[cfg(feature = "horizon_main_api")]
fn team(host: &mut dyn ServerHost, name: &str, color: Option<i32>, players: Option<Vec<String>>) {
    let handle = match host.team(name) {
        Some(handle) => handle,
        None => match host.add_team(name) {
            Ok(handle) => handle,
            Err(_) => return,
        },
    };
    let mut st = state();
    // The core sends each viewer the same team, so only a change is applied.
    if st.teams.get(name) != Some(&color) {
        let _ = host.team_set_color(handle.clone(), color.and_then(team_color));
        st.teams.insert(name.to_owned(), color);
    }
    for player in players.into_iter().flatten() {
        if st.members.get(&player).map(String::as_str) != Some(name) {
            let _ = host.team_join(handle.clone(), ScoreHolder::Name(player.clone()));
            st.members.insert(player, name.to_owned());
        }
    }
}

#[cfg(not(feature = "horizon_main_api"))]
fn team_remove(host: &mut dyn ServerHost, name: &str) {
    let _ = (host, name);
}

#[cfg(feature = "horizon_main_api")]
fn team_remove(host: &mut dyn ServerHost, name: &str) {
    if let Some(handle) = host.team(name) {
        let _ = host.team_remove(handle);
    }
    let mut st = state();
    st.teams.remove(name);
    st.members.retain(|_, team| team != name);
}

/// TAB's tab list names. The core sends them per viewer, `player_set_list_name` is the same for
/// everyone, so only the name a player's own view gets is set (and only when it changed).
// HMOD-GAP(set_list_name): the pinned SDK has no tab list name setter, so the core's names are not
// sent; the `player_tab_list_name_format` hook still sets one name per player.
#[cfg(not(feature = "horizon_main_api"))]
fn display_names(host: &mut dyn ServerHost, viewer: u128, entries: Vec<(u128, Option<Value>)>) {
    let _ = (host, viewer, entries);
}

#[cfg(feature = "horizon_main_api")]
fn display_names(host: &mut dyn ServerHost, viewer: u128, entries: Vec<(u128, Option<Value>)>) {
    for (uuid, name) in entries {
        if uuid != viewer {
            continue;
        }
        let key = name.as_ref().map(Value::to_string);
        if state().list_names.get(&uuid) == Some(&key) {
            continue;
        }
        let text = name.as_ref().map(|json| text_json(&*host, json));
        if let Some(player) = host.player(uuid) {
            let _ = player.set_list_name(host, text);
        }
        state().list_names.insert(uuid, key);
    }
}

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
        #[cfg(feature = "horizon_main_api")]
        log("Shown: sidebar, list names, team colours, ping and game mode. Not shown: team prefixes/suffixes, below-name and list objectives.");
        #[cfg(not(feature = "horizon_main_api"))]
        log("Not shown yet: scoreboard teams (name tags, sorting), sidebar, below-name and list objectives, ping and game mode.");
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        for (usage, level) in api::commands() {
            // A word takes any name (an offline one too), so the usages carry no name completion.
            // TODO(arismoko-api: player_name argument kind): no name completion meanwhile.
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
        // TODO(arismoko-api: command_replace_vanilla): a toggle named like a vanilla root (`/bossbar`
        // by default) is not registered, so it is reached only as `/tab bossbar`.
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
        send(host, packets);
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
}
