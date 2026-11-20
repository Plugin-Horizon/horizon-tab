# hmod port: tab
Parity: 37% -> 40% now (42 -> 46 of 115 items) / ~53% -> ~57% with our PRs (MySQL adds 4 items: `/tab mysql upload`, `/tab mysql download`, the storage feature, the `mysql` section's keys; hand count, as before). Blind round 100, NOT compiled. Earlier rounds below.

Round 100 (HUNDRED, blind, not compiled): MySQL storage
- `mysql` section of `config.yml` read as TAB's `MySQLConfiguration.fromSection` (enabled, host, port, database, username, password, useSSL; defaults 127.0.0.1, 3306, tab, user, password, true). The password is redacted in `Debug` output. Ported in `src/horizon_tab/mysql.rs`.
- The crate is the `mysql` 28 used by husksync and luckperms (28.0.4 in perm-sdk's lock); no new DB crate. `Db::connect` makes the pool and creates `tab_groups` / `tab_users` with TAB's columns, as TAB's constructors do.
- With `mysql.enabled`, groups are read at start and users are read per name and per UUID when the player joins (and for the players online after `/tab reload`). Any failure at start keeps the YAML files, with TAB's "Failed to connect" warning. The empty-table warning is TAB's.
- Writes of `/tab player|group ... set|remove` go to the table as TAB's statements (delete of that scope, then insert; remove deletes the name's rows). Worlds and servers match by exact name; users have no `_DEFAULT_` fallback (TAB's MySQL rules). Names are lower case in the table, except `_DEFAULT_` of groups.
- `/tab mysql upload` empties both tables, then writes the rows of `groups.yml` and `users.yml` (TAB's defaults if a file is missing) and the running configuration uses them. `/tab mysql download` writes the rows into the two files. Both answer with TAB's messages; without a connection, `mysql-fail-not-enabled`; a failure prints to the console and answers `mysql-fail-error`.
- Unit tests with literal values: the schema and select text, the set/remove/upload texts, row mapping, the upload rows of a file, exact scope, the users' no-default rule, the redacted Debug, the config defaults.
- Not done, and why: the database calls run on the server thread (TAB uses its own thread), so a join or an upload waits for them; `/tab dump` does not show the section (no dump); `checkProperty` warnings for rows are not printed.

Round 4 (HUNDRED, blind, not compiled)
- Permission nodes of `permission:<node>` / `!permission:<node>` conditions in `config/tab/config.yml` are declared at level 1 (`api::condition_permission_nodes`, `permission_nodes_in`; unit-tested). Before, typed queries of these nodes were refused, so every such condition was false for everyone.
- `/tab` name completion (`tab_complete` in `src/lib.rs`) is only offered to senders with `tab.admin` or `tab.tabcomplete`, as TAB's `TabCommand.complete` does. The literal sub-command words are offered by the host's command tree and are not filtered per sender here (GAPS_UPSTREAM `tab:` line on literal completion).
- New Cargo feature `horizon_protocol_api` (off by default; branch `api-player-protocol`, issue #18797): `player.protocol_version(host)` feeds `%player-version%` and `%player-version-id%` through the existing protocol-to-name table. Without it both show protocol 0 as before.
- New Cargo feature `horizon_team_options_api` (off by default, needs `horizon_main_api`; branch `api-scoreboard-team-options`, issue #18817, stacked on PR #17980): every team packet's prefix and suffix (`team_set_prefix` / `team_set_suffix`), name tag visibility, collision rule and friendly fire (bit 1 of the options byte) are applied, once per change (`State::team_options`), and dropped on team removal. `visibility_of` and `collision_of` map TAB's numbers to the vocabulary, unit-tested. Still missing: can-see-friendly-invisibles (bit 2 of the options byte, no setter).
- Not done in this round, and why: `mysql` storage (round 100 added it, see above); `/tab cpu` and `/tab dump` upload (no CPU-time accounting per feature, no upload); `nametag opaque` (per-viewer entity data, GAPS_UPSTREAM `tab:`); `layout`, global and per-world playerlist (per-viewer tab-list entries, GAPS_UPSTREAM `tab:`); `%online_<server>%` for other servers (proxy bridge, GAPS_UPSTREAM `tab:`); `locale` (`placeholders.locale`) is read but only en-US date and time text is produced.
- Checked and already matching TAB: `/tab` sub-command set, `groups` and `debug` answers, the permission nodes of the commands (`tab.change.<property>` included), the built-in placeholder names (`TabConstants.Placeholder`), the messages of `messages.yml`, the default `config.yml` shipped unchanged.

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
