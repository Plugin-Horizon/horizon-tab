// Support module `crate::game::tab_glue` (horizon-client, beside the game's own modules): reads the
// game state the TAB core needs and sends what it answers. A child of `game`, so the game's
// private fields and its `pub(super)` helpers are in reach.
use super::Game;
use horizon_client::lan::data::component_nbt;
use horizon_client::tab::{self, BossOut, NumberFmt, ObjectiveMethod, Out, Packet, PlayerIn, Server, TeamParams};
use horizon_client::text::Text;
use horizon_protocol::packets::play::{self, Clientbound as C};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Starts the core once (at server start; again lazily if that hook was missed).
pub(crate) fn ensure_started() {
    if tab::started() {
        return;
    }
    match Game::native_config_dir() {
        Ok(dir) => {
            for line in tab::start(&dir) {
                eprintln!("[tab] {line}");
            }
        }
        Err(e) => eprintln!("[tab] no config folder: {e}"),
    }
}

/// The players and the server, as the core reads them. Empty on a client.
pub(super) fn snapshot(game: &Game<'_>) -> Option<(Server, Vec<PlayerIn>)> {
    // None while the hub is out of reach and on a client.
    let max_players = game.native_max_players()?;
    let (rate, mspt) = game.native_tick_stats();
    let tps = if mspt > 0.0 {
        (rate as f64).min(1000.0 / mspt)
    } else {
        rate as f64
    };
    let level_name = game
        .native_world_dir()
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "world".to_owned());
    let (used, max) = tab::process_memory_mb();
    let server = Server {
        now_ms: now_ms(),
        started_ms: tab::started_ms(),
        tps,
        mspt,
        max_players: max_players as i32,
        online: 0,
        memory_used_mb: used,
        memory_max_mb: max,
        version: "26.3".to_owned(),
        level_name,
    };
    let players = game
        .native_server_players()
        .into_iter()
        .map(|p| PlayerIn {
            uuid: p.uuid,
            name: p.name,
            dimension: p.dimension,
            position: p.position,
            yaw: p.yaw,
            pitch: p.pitch,
            health: p.health,
            game_mode: p.game_mode,
            op_level: p.permission,
            ping: p.latency,
            nickname: p.nickname,
            vanished: p.hidden.is_some(),
            entity_id: p.entity_id,
            protocol: p.protocol,
            fake: p.fake,
        })
        .collect();
    Some((server, players))
}

/// The Tab list names the game itself puts in its entries for a player (a join, a vanish ending, a
/// nickname change), kept by the hub until set again: `native_set_list_name`.
fn list_names_sent() -> &'static std::sync::Mutex<std::collections::HashMap<u128, serde_json::Value>> {
    static S: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<u128, serde_json::Value>>> =
        std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

/// Gives the game the names the core formats, where they changed since the last call. `uuids` are
/// the players who are on.
pub(super) fn sync_list_names(game: &mut Game<'_>, uuids: &[u128]) {
    let mut sent = list_names_sent().lock().unwrap_or_else(|e| e.into_inner());
    for &uuid in uuids {
        let wanted = tab::list_name(uuid);
        if sent.get(&uuid) == wanted.as_ref() {
            continue;
        }
        let name = wanted.as_ref().map(|v| text(v, &tab::component_plain(v)));
        if game.native_set_list_name(uuid, name.as_ref()) {
            match wanted {
                Some(v) => sent.insert(uuid, v),
                None => sent.remove(&uuid),
            };
        }
    }
    sent.retain(|u, _| uuids.contains(u));
}

/// Takes back every name set with `sync_list_names` (the server stops, or TAB is reloaded off).
pub(super) fn clear_list_names(game: &mut Game<'_>) {
    let mut sent = list_names_sent().lock().unwrap_or_else(|e| e.into_inner());
    for (uuid, _) in sent.drain() {
        game.native_set_list_name(uuid, None);
    }
}

/// Asks the game's permission service about one node, as the core's answer: `Some` when a
/// permissions mod decides, `None` for the operator fallback.
fn ask_permission(game: &Game<'_>, uuid: u128, node: &str) -> Option<bool> {
    let mut answer = None;
    game.native_permission_check(uuid, node, 1, &mut answer);
    answer
}

/// Answers the nodes the core asked about (`integration::permission`) and those whose answer is old.
pub(super) fn resolve_permissions(game: &Game<'_>) {
    let now = now_ms();
    for (uuid, node) in tab::host_permission_requests(now) {
        tab::host_permission_answer(uuid, &node, ask_permission(game, uuid, &node), now);
    }
}

/// The answers for every node a `/tab` command can ask, ahead of the command.
pub(super) fn resolve_command_permissions(game: &Game<'_>, uuid: u128) {
    let now = now_ms();
    for node in tab::command_permission_nodes() {
        tab::host_permission_answer(uuid, &node, ask_permission(game, uuid, &node), now);
    }
}

/// A clientbound play packet on its way to `viewer` (the hook is heard for every packet of every
/// guest after `native_watch_server_packets`): put TAB's values back where another sender's
/// packet would replace them.
pub(crate) fn guard_packet(viewer: u128, packet: &mut C, cancel: &mut bool) {
    if !tab::started() {
        return;
    }
    match packet {
        C::SetPlayerTeam(t) => {
            // 0 creates a team with players, 3 adds players.
            if matches!(t.method, 0 | 3) && tab::guard_team(viewer, &t.name, &t.players) {
                *cancel = true;
            }
        }
        C::PlayerInfoUpdate(u) => {
            let (names, latency, modes) = (u.actions & 0x20 != 0, u.actions & 0x10 != 0, u.actions & 0x04 != 0);
            if !(names || latency || modes) {
                return;
            }
            let spoof = if latency { tab::guard_latency() } else { None };
            for e in &mut u.entries {
                if let (Some(ms), Some(value)) = (&mut e.latency, spoof) {
                    *ms = value;
                }
                if let Some(mode) = &mut e.game_mode {
                    if let Some(shown) = tab::guard_game_mode(viewer, e.uuid, *mode) {
                        *mode = shown;
                    }
                }
                if names {
                    if let Some(ours) = tab::guard_display_name(viewer, e.uuid) {
                        e.display_name = Some(Some(component_nbt(&ours)));
                    }
                }
            }
        }
        _ => {}
    }
}

fn text(v: &serde_json::Value, plain: &str) -> Text {
    Text::literal(plain.to_owned()).with_component(v.clone())
}

fn number_format(f: Option<NumberFmt>) -> Option<play::NumberFormat> {
    f.map(|f| match f {
        NumberFmt::Blank => play::NumberFormat::Blank,
        NumberFmt::Fixed(v) => play::NumberFormat::Fixed(component_nbt(&v)),
    })
}

fn team(p: &TeamParams) -> play::TeamParameters {
    play::TeamParameters {
        display_name: component_nbt(&p.display_name),
        prefix: component_nbt(&p.prefix),
        suffix: component_nbt(&p.suffix),
        name_tag_visibility: p.visibility,
        collision_rule: p.collision,
        color: p.color,
        options: p.options,
    }
}

fn info(actions: u8, entries: Vec<play::PlayerInfo>) -> C {
    C::PlayerInfoUpdate(play::PlayerInfoUpdate { actions, entries })
}

/// Sends the core's packets.
pub(super) fn send(game: &mut Game<'_>, packets: Vec<Packet>) {
    for Packet { to, out } in packets {
        let packet = match out {
            Out::TabList {
                header,
                footer,
                header_plain,
                footer_plain,
            } => {
                // The game's own helper (API 49).
                game.native_tab_list(to, &text(&header, &header_plain), &text(&footer, &footer_plain));
                continue;
            }
            Out::Chat(line) => {
                let plain = tab::component_plain(&line);
                game.native_tell(to, text(&line, &plain));
                continue;
            }
            Out::DisplayNames(entries) => info(
                0x20,
                entries
                    .into_iter()
                    .map(|(uuid, name)| play::PlayerInfo {
                        uuid,
                        display_name: Some(name.map(|v| component_nbt(&v))),
                        ..Default::default()
                    })
                    .collect(),
            ),
            Out::Latency(entries) => info(
                0x10,
                entries
                    .into_iter()
                    .map(|(uuid, ms)| play::PlayerInfo {
                        uuid,
                        latency: Some(ms),
                        ..Default::default()
                    })
                    .collect(),
            ),
            Out::GameModes(entries) => info(
                0x04,
                entries
                    .into_iter()
                    .map(|(uuid, mode)| play::PlayerInfo {
                        uuid,
                        game_mode: Some(mode),
                        ..Default::default()
                    })
                    .collect(),
            ),
            Out::TeamCreate { name, params, players } => C::SetPlayerTeam(play::SetPlayerTeam {
                name,
                method: 0,
                parameters: Some(team(&params)),
                players,
            }),
            Out::TeamUpdate { name, params } => C::SetPlayerTeam(play::SetPlayerTeam {
                name,
                method: 2,
                parameters: Some(team(&params)),
                players: Vec::new(),
            }),
            Out::TeamRemove { name } => C::SetPlayerTeam(play::SetPlayerTeam {
                name,
                method: 1,
                parameters: None,
                players: Vec::new(),
            }),
            Out::Objective {
                name,
                method,
                title,
                hearts,
                format,
            } => {
                let method = match method {
                    ObjectiveMethod::Add => 0,
                    ObjectiveMethod::Remove => 1,
                    ObjectiveMethod::Change => 2,
                };
                let display = (method != 1).then(|| (component_nbt(&title), i32::from(hearts), number_format(format)));
                C::SetObjective(play::SetObjective { name, method, display })
            }
            Out::DisplaySlot { slot, objective } => {
                C::SetDisplayObjective(play::SetDisplayObjective { slot, objective })
            }
            Out::Score {
                owner,
                objective,
                score,
                display,
                format,
            } => C::SetScore(play::SetScore {
                owner,
                objective,
                score,
                display: display.map(|v| component_nbt(&v)),
                number_format: number_format(format),
            }),
            Out::ResetScore { owner, objective } => C::ResetScore(play::ResetScore {
                owner,
                objective: Some(objective),
            }),
            Out::Boss { id, action } => {
                let action = match action {
                    BossOut::Add {
                        title,
                        progress,
                        color,
                        overlay,
                    } => play::BossAction::Add {
                        name: component_nbt(&title),
                        progress,
                        color,
                        overlay,
                        darken: false,
                        music: false,
                        fog: false,
                    },
                    BossOut::Remove => play::BossAction::Remove,
                    BossOut::Progress(progress) => play::BossAction::UpdateProgress { progress },
                    BossOut::Title(title) => play::BossAction::UpdateName {
                        name: component_nbt(&title),
                    },
                    BossOut::Style { color, overlay } => play::BossAction::UpdateStyle { color, overlay },
                };
                C::BossEvent(play::BossEvent { id, action })
            }
        };
        game.native_send(to, packet);
    }
}
