//! TAB as an arismoko native mod (trait mod on `mod_api`). The logic is the unchanged
//! `horizon-tab-core` crate (`core/`); this file maps its hooks, its host calls and the packets it
//! answers onto arismoko's API. Gaps are marked `TODO(arismoko-api: <name>)`; PORT_NOTES.md lists them.
//!
//! Ported: tab list header and footer (`player_tab_list`), chat lines (`player_tell`), the
//! `/tab` family and the toggle commands (`register_commands` / `run_mod_command`), boss bars
//! (host boss bar calls, one bar per viewer), the tab list name (`player_tab_list_name_format`),
//! the tick (`server_tick`), respawns and the stop.
//!
//! Packets: latency and game mode per viewer go out through `player_send_packet`. Permission
//! nodes are asked of the game (`player_permission_value`) each tick, for the nodes the core asked
//! about. Not ported yet: teams (name tags), objectives (tab list ping and health, below name,
//! sidebar), scores, per-viewer display names (their bodies need NBT text components), the packet
//! guard, and the LuckPerms bridge (its placeholders and prefixes).

use horizon_tab::api;
use horizon_tab::engine::PlayerIn;
use horizon_tab::out::{BossOut, Out, Packet};
use horizon_tab::placeholders::Server;
use mod_api::serde_json::Value;
use mod_api::{
    BossBarColor, BossBarOverlay, ClientboundPlayPacket, Command, CommandArgumentValue,
    CommandSource, Flow, Hooks, Identifier, LoopContext, PlayerRef, PlayerRespawnedEvent,
    PlayerTabListNameFormatEvent, RegisterCommandsEvent, RunModCommandEvent, ServerAboutToStartEvent,
    ServerHost, ServerStoppingEvent, ServerTickEvent, ServerTickPhase, Text,
};
use horizon_tab::out::{NumberFmt, ObjectiveMethod};
use mod_api::{Sidebar, SidebarLine};
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
        ping: player.latency(host).unwrap_or(0),
        nickname: player.nickname(host).flatten(),
        // A player hidden by `player_hide` (a level set) is vanished.
        vanished: player.hidden(host).flatten().is_some(),
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

/// Sends one player-info action to `viewer` alone, through `player_send_packet`.
fn send_player_info(host: &mut dyn ServerHost, viewer: u128, action: u8, entries: &[(u128, i32)]) {
    let Some(player) = host.player(viewer) else {
        return;
    };
    let Ok(id) = Identifier::parse("minecraft:player_info_update") else {
        return;
    };
    let _ = host.player_send_packet(player, ClientboundPlayPacket::new(id, player_info_body(action, entries)));
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
            Out::DisplayNames(entries) => display_names(host, entries),
            Out::TeamCreate { name, params, players } => team(host, &name, params.color, Some(players)),
            Out::TeamUpdate { name, params } => team(host, &name, params.color, None),
            Out::TeamRemove { name } => {
                let mut st = state();
                if st.teams.remove(&name).is_some() {
                    drop(st);
                    host.team_remove(&name);
                }
            }
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

/// The viewer's sidebar from their objectives: lines by score, highest first (as vanilla sorts),
/// the line's text its display (else the owner), the right side its number format (else the score).
fn sidebar(host: &mut dyn ServerHost, viewer: u128) {
    let built = {
        let st = state();
        st.views.get(&viewer).and_then(|v| {
            let name = v.sidebar.as_ref()?;
            let title = v.titles.get(name).cloned().unwrap_or(Value::String(String::new()));
            let mut rows: Vec<_> = v.scores.get(name).map(|m| m.iter().map(|(o, r)| (o.clone(), r.clone())).collect()).unwrap_or_default();
            rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
            Some((title, rows))
        })
    };
    let Some(player) = host.player(viewer) else { return };
    let sidebar = built.map(|(title, rows)| {
        let lines = rows
            .into_iter()
            .take(15)
            .map(|(owner, (score, display, format))| {
                let text = display.map(|d| text_json(&*host, &d)).unwrap_or_else(|| Text::literal(owner.as_str()));
                let value = match format {
                    Some(NumberFmt::Blank) => None,
                    Some(NumberFmt::Fixed(v)) => Some(text_json(&*host, &v)),
                    None => Some(Text::literal(score.to_string().as_str())),
                };
                SidebarLine::new(text, value)
            })
            .collect();
        Sidebar::new(text_json(&*host, &title), lines)
    });
    if let Err(e) = player.set_sidebar(host, sidebar) {
        log(&format!("sidebar not set: {e:?}"));
    }
}

/// `TeamColor` index (0..15) to the chat colour name `team_set` takes.
const COLORS: [&str; 16] = [
    "black", "dark_blue", "dark_green", "dark_aqua", "dark_red", "dark_purple", "gold", "gray",
    "dark_gray", "blue", "green", "aqua", "red", "light_purple", "yellow", "white",
];

/// A TAB team (name tags and sorting) as a team of the world's scoreboard (`team_set`/`team_join`).
/// The core sends the same team to every viewer; the engine's teams are shared, so each change is
/// made once. TODO(arismoko-api: team prefix/suffix/visibility/collision): `team_set` takes the colour
/// only, so TAB's tag prefix and suffix are not shown (the colour is).
fn team(host: &mut dyn ServerHost, name: &str, color: Option<i32>, players: Option<Vec<String>>) {
    let mut st = state();
    if st.teams.get(name) != Some(&color) {
        let c = color.and_then(|i| usize::try_from(i).ok()).and_then(|i| COLORS.get(i).copied());
        if host.team_set(name, c) {
            st.teams.insert(name.to_owned(), color);
        } else {
            log(&format!("team {name} not set"));
            return;
        }
    }
    for player in players.into_iter().flatten() {
        if st.members.get(&player).map(String::as_str) != Some(name) && host.team_join(name, &player) {
            st.members.insert(player, name.to_owned());
        }
    }
}

/// TAB's tab list names. The core sends them per viewer; `set_list_name` is the same for everyone,
/// so a name is set when it differs from the last one set (TODO(arismoko-api: per-viewer list names)).
fn display_names(host: &mut dyn ServerHost, entries: Vec<(u128, Option<Value>)>) {
    for (target, value) in entries {
        let key = value.as_ref().map(Value::to_string);
        if state().list_names.get(&target) == Some(&key) {
            continue;
        }
        let text = value.map(|v| text_json(&*host, &v));
        if let Some(player) = host.player(target) {
            if matches!(player.set_list_name(host, text), Ok(true)) {
                state().list_names.insert(target, key);
            }
        }
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
    fn server_about_to_start(&self, _: &mut dyn ServerHost, _: &mut ServerAboutToStartEvent) {
        if api::started() {
            return;
        }
        // The core reads `plugins/TAB/` relative to the server folder, as the Paper plugin did.
        for line in api::start(Path::new("config")) {
            log(&line);
        }
        log("ported to arismoko: tab list, chat, boss bars, commands, name format, latency, game mode,");
        log("teams (colour only), list names and the sidebar; tag prefix/suffix, below-name and list objectives are not");
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
        // Rebuild the line: literal path, then the greedy text argument (`tab [arguments:text]`).
        let mut line = event.command.literals.join(" ");
        for argument in &event.command.arguments {
            if let CommandArgumentValue::String(text) = &argument.value {
                line.push(' ');
                line.push_str(text);
            }
        }
        let answer = match &event.source {
            CommandSource::Player { player, permission } => {
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
            let answer = host.player(uuid).and_then(|p| p.permission_value(&*host, &node));
            api::host_permission_answer(uuid, &node, answer, now);
        }
        let present: HashSet<u128> = online.iter().map(|p| p.uuid()).collect();
        horizon_tab::integration::host_permissions_retain(&present);
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
