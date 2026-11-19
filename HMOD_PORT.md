# hmod port: tab
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
