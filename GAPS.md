# What Horizon lacks for this mod

Checked against Horizon `a589a3dd9` (native hooks API 411). Each line: the hook or helper that would do it, what it
is for, and what the mod does today.

## Tab list and players

- [x] (used) `Game::native_set_list_name(uuid, Option<&Text>)`: a formatted Tab list name the hub keeps per player and puts in every
  entry it sends (join, a vanish ending, a nickname change). `helpers/glue.rs` `sync_list_names` hands it the name the core formats each
  tick; `stopping.rs` takes them back. Both raw injections on `Hub::player_info` / `Hub::set_nickname` are gone. Names that depend on the
  viewer (`%rel_...%`, `%viewer:...%`) are still sent per viewer as `UPDATE_DISPLAY_NAME`, and the packet hook below puts them back if
  another sender overwrites them.
- [x] (used) `NativeServerPlayer.latency`, `.protocol`, `.nickname`, `.hidden`: read in `glue.rs` `snapshot`, no hub access left.
- [x] (used) `Game::native_max_players()`: `%server_max_players%`; its `None` also says "no hub / client" to `snapshot`.
- [ ] `NativeServerPlayer.xp_level` / `.on_ground` exist now, but no TAB placeholder uses them, so nothing reads them.
- [ ] `Game::native_level_name() -> String` (server.properties `level-name`) — `%world%` takes the world folder's file name from
  `native_world_dir()` as before.

## Packets the game sends

- [x] (used, never run) `native_server_packet(player, packet, cancel)` + `native_watch_server_packets()`: `helpers/server_packet.rs` ->
  `tab_glue::guard_packet` -> `tab::guard_*`. Anti-override: a `SetPlayerTeam` (create/add players) that would move a player out of
  TAB's team is cancelled; `UPDATE_LATENCY` entries get the ping spoof value; `UPDATE_GAME_MODE` shows spectators as survival (the
  spectator fix, per packet instead of every 2 s); `UPDATE_DISPLAY_NAME` gets TAB's name for that viewer. The periodic resends stay as
  the first send and as a safety net. The core part (`engine.rs` `guard_*`) is unit tested; the hook wiring is not built.
- [ ] `native_tab_list` and `native_tell` take `&Text`; a `Text::from_component(serde_json::Value)` constructor would
  make that explicit — today: `Text::literal(plain).with_component(json)`, which works.

## Commands

- [ ] `NativeCommand { replace_vanilla: bool }` (or a documented way to take over a vanilla root such as `/bossbar`) —
  needed for TAB's default boss bar `toggle-command: /bossbar` — today: `command_tree` refuses names vanilla has, so
  the toggle works only as `/tab bossbar` (or with another `toggle-command`).
- [x] (used, never run) The permission service: the core cannot call the game, so `integration::permission` asks a cache that
  `tab_glue::resolve_permissions` fills from `Game::native_permission_check` (a mod's answer, or none = operator fallback) for every
  node TAB asked about, refreshed every 5 s; `/tab` commands get all their nodes answered first (`resolve_command_permissions`). An
  integration registered through `horizon_client::tab::register_integration` still wins. A node asked for the first time answers
  with the operator fallback until the next tick.

## Between mods

- [ ] Optional dependency between bundles (`[dependencies] tab = { version, optional = true }` in a permissions, vanish or nickname
  mod, with the Cargo feature named `tab`): it needs this bundle to have an `[api]` trait package, which it does not have (it publishes
  `horizon_client::tab` / `placeholders` through `[[module]]`s). Not converted.

## Proxy

- [ ] `Game::native_send_custom_payload_to_player`, `native_custom_payload_from_client` and `native_player_register_channels` exist now, but
  the `tab:bridge-*` message format of TAB's proxy plugin is not documented in this repo and the original's code was not read for it, so
  the bridge is not written.

## Unknowns to settle with a build

- [ ] Whether Horizon's respawn or dimension change clears a client's boss bars, objectives or teams (vanilla keeps
  them). `native_player_respawned` is injected (`helpers/respawned.rs`) and makes the core resend everything to that viewer; a
  dimension change is not covered (no hook in the catalog). Check on a build whether a resend after a respawn shows duplicate-team
  warnings on a vanilla client.

## Command completion

- [ ] Per-sub-command visibility: TAB's `tab.tabcomplete` decides who sees `/tab` completions, but a native command's tree is per root (`NativeCommand::permission` gates the whole `/tab`), and usages only filter by operator level. Every player sees the sub-command words; running them is still checked. Needed: a per-usage permission node in `NativeCommand`, or a hook that filters the words of a usage per player (`native_player_commands_sent_v2` works on root names only).

## Commands not fully portable

- [ ] A line-of-sight query (`Game::native_line_of_sight(viewer: u128, target: u128) -> bool`, the check TAB's `requestLineOfSight` makes) — needed for `/tab nametag opaque` and `opaqueview` (name tags hidden behind walls) — today: the two actions check their permission and answer that they are not ported.
- [ ] A way to run work off the tick thread and reach a MySQL server (`mysql` is in Horizon's lock, but no mod-facing connection pool) — needed for TAB's `mysql:` storage and `/tab mysql upload|download` — today: the files are the only storage and the command answers `mysql-fail-not-enabled`.
- [ ] An HTTP client for mods — needed for `/tab dump` (TAB uploads a report to a paste service) — today: the facts are printed in chat.
- [ ] CPU timing per feature and placeholder — needed for `/tab cpu` — today: not measured.
