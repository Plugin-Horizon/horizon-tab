# hmod port: tab
Parity: before 35% -> after 36% default build / ~43% with feature `horizon_main_api` (~40/115 -> ~41/115 items; ~48/115 -> ~49/115 with the feature; method: commands+sub-commands, permission nodes, config keys, message keys and listed features counted against TAB 6.2.2-SNAPSHOT; the feature items count only once the pinned SDK is moved to a main that has the calls)

Round 2 (FIX2)
- New Cargo feature `horizon_main_api` (off by default). The pinned SDK (perm-sdk) has none of the following calls; they exist on Horizon main / PR #17980 and are coded behind the feature in `src/lib.rs`. Written blind from the merged signatures and the paper ledger; neither the feature code nor the default-build edits (sidebar_rows, its tests, cfg stubs) were compiled (owner order 2026-11-20).
  - `player.latency(host)` -> `%ping%` (#17878); `player.nickname(host)`; `player.hidden(host)` -> `%vanished%` / `%online%` (#17878).
  - `player.set_sidebar(host, Some(Sidebar))` (#17948): TAB's sidebar scoreboards, built from the core's objective packets by `View::sidebar_rows` (vanilla order: score desc, name, max 15; unit-tested, also in the default build).
  - `player.set_list_name(host, Option<Text>)` (#17884): the name the player's OWN view receives (list names are per player, not per viewer); the `player_tab_list_name_format` hook keeps the last word.
  - Scoreboard teams (`host.team/add_team/team_set_color/team_join/team_leave/team_remove`, PR #17980, branch tab-teams): team colour and members only; the team API has no prefix, suffix, name tag visibility, collision or friendly fire.
  - `host.player_send_packet` (main): per-viewer latency and game mode (`minecraft:player_info_update`).
- Not closable even with the feature: team prefix/suffix/visibility/collision (needs `set_player_team` packet body or team setters), list and below-name objectives (no slot API), per-viewer list names, `%player-version%` (no protocol read), anti-override guard (no packet watch), `nametag opaque`, layout, global/per-world playerlist, mysql, proxy bridge.

Round 1
- Fixed: src/lib.rs imports: removed `ClientboundPlayPacket`, `Identifier`, `Sidebar`, `SidebarLine` (no such API); added typed permission imports (13 errors).
- Fixed: src/lib.rs `player_in`: `ping`, `nickname`, `vanished` read values replaced by `0`, `None`, `false` (no getters on main).
- Fixed: src/lib.rs permissions: string `permission_value` queries (tick and `/tab` command) replaced by `host_answer`, a typed Boolean `permission_bool` query; any refusal or unknown player is a denial.
- Fixed: src/lib.rs new `permission_gather_nodes` hook declares the nodes below once.
- Fixed: src/lib.rs packet, team, sidebar and list-name paths reduced to no-op bodies with HMOD-GAP comments (their logic was built only to feed the missing calls).
- Audit round: `tab.send.bar` now guards `/tab bossbar send` and is declared; `tab.bypass`, `tab.seevanished` and `tab.group.<name>` (from `primary-group-finding-list` and `groups.yml`, read at gathering) are declared at level 1; `tab.spectatorbypass` default is now level 1 (the original's `default: op`). Placeholders `%deaths%` (stat `minecraft:deaths`), `%invisible%` and `%collision%` added. `/tab` prints `6.2.2-SNAPSHOT`. README and start log no longer claim features that send nothing.
- Permission nodes: every name from `api::command_permission_nodes()` plus `tab.staff` -> Level 1 (operator level 1, as the old fallback); `tab.spectatorbypass` -> Level 4. All declared in `src/lib.rs` `permission_gather_nodes`.
- Gaps: HMOD-GAP(player_latency): `%ping%` is 0. HMOD-GAP(player_nickname_read): no nickname read. HMOD-GAP(player_hidden_read): nobody is vanished. HMOD-GAP(player_send_packet): latency and game mode per viewer not sent. HMOD-GAP(team_set/team_join), HMOD-GAP(team_remove): no scoreboard teams, name tag colours not set. HMOD-GAP(set_sidebar): no sidebar. HMOD-GAP(set_list_name): per-viewer tab list names not sent (the `player_tab_list_name_format` hook still sets names).
- Unsure:
  - Config- and integration-driven nodes (`tab.group.<name>`, conditions' `perm` nodes, `tab.staff` variants) are not declared; typed queries of undeclared nodes are refused, so they are denied where the old code fell back to operator level.
  - `tab.spectatorbypass` default level 4: the old code asked with fallback 0 (nobody); no level denies everyone.
  - Assumed `permission_gather_nodes` runs before the first permission query.
  - `vanished` is always false; `ping` is always 0.

Round 3 (blind, not compiled)
- `/tab <player|playeruuid|group> <name> ...` and the other `<name:player_name>` slots of `/tab` complete ONLINE player names
  (`tab_complete` in `src/lib.rs`, the slot test `api::player_name_expected`, unit tests in `src/horizon_tab/api.rs`). The
  registered usage still takes a word, so an offline name is accepted as before. Names are not offered for `group` (a word).
- Not done, and why (all need host APIs, see GAPS_UPSTREAM `tab:` lines): `layout` (per-viewer fake list entries), global and
  per-world playerlist (per-viewer tab list removal), `%online_<server>%` for other servers (proxy bridge), `nametag opaque`
  (per-viewer entity data and team options; still answers that it is not part of this port), `%player-version%`.
- Not done: `mysql` storage. It needs a database-backed user and group source in place of `users.yml`/`groups.yml` (TAB's
  `MySQLUserConfiguration`/`MySQLGroupConfiguration`) and the upload/download commands; the port's config is file-only, and
  the work was too large to write blind. `/tab mysql` stays the message-only answer (`mysql-fail-not-enabled`). The `mysql`
  crate is already used by husksync, so a later round can build on it.
