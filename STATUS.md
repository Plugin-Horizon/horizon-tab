# Status

Written blind on 2026-11-06; commands, messages and default files compared line by line with TAB 6.2.2-SNAPSHOT's source on 2026-11-08: the core crate compiles and its unit tests pass (`cargo test` in `core/`); the bundle
(helpers, injections, modules) has **never been applied or built** against Horizon. Base: Horizon `a589a3dd9`; revision of 2026-11-08 switched to the hooks/helpers now in Horizon (list names, player fields, packet hook, permission service, respawn) and is likewise unbuilt.

Legend: **written** = complete as far as can be told without a build; **partly** = works with the limits named;
**not started** = absent.

## Features (TAB 6.2.1, as configured on the servers this replaces)

| Feature | On there? | State | Notes |
|---|---|---|---|
| Reading TAB's own files (`config.yml`, `groups.yml`, `users.yml`, `animations.yml`, `messages.yml`, `playerdata.yml`) from `config/tab/` | yes | written | own YAML subset reader/writer (no YAML crate in Horizon's lock) |
| Tab list names (`tabprefix` + `customtabname` + `tabsuffix`, per world/server, users by name or UUID, `_DEFAULT_`) | yes | written | display name per viewer; see GAPS for the hub injections |
| Name tags via teams (`tagprefix`/`tagsuffix`, team colour from the prefix's last colour, collision, invisible name tags, see friendly invisibles) | yes | written | anti-override through the packet hook (unbuilt, GAPS) |
| Sorting (`GROUPS`, `PERMISSIONS`, `PLACEHOLDER`, `PLACEHOLDER_A_TO_Z`, `_Z_TO_A`, `_LOW_TO_HIGH`, `_HIGH_TO_LOW`, case sensitivity, unique team names) | yes | written | number keys use base U+D7FF instead of U+FFFE (see `sorting.rs`) |
| Player list objective (value, fancy value, INTEGER/HEARTS, title) | yes | written | |
| Disable conditions (`%world%=disabledworld`) | yes | written | |
| Placeholder output replacements (exact, `a-b` ranges, `else`, `%value%`) | yes | written | |
| Conditions (`%condition:name%`, AND/OR, all operators, `permission:`) and short display conditions | yes | written | |
| Animations (`%animation:Name%`) | yes (in disabled texts) | written | |
| Placeholder refresh intervals (per placeholder, default 500 ms, TAB's built-in ones) | yes | written | a per-player cache with TAB's intervals |
| TAB built-in placeholders (`%player%`, `%world%`, `%ping%`, `%online%`, `%staffonline%`, `%worldonline%`, `%group%`, `%date%`, `%time%`, `%displayname%`, `%health%`, `%tps%`, `%mspt%`, `%memory-*%`, `%luckperms-*%`, `%vault-*%`, `%bedrock%`, `%player-version(-id)%`, `%vanished%`, `%gamemode%`, `%uuid%`, `%online_<server>%`, `%viewer:...%`, `%rel_...%`) | yes | partly | `%memory-max%` is the machine's memory (no `-Xmx`); `%player-version%` is the player's protocol from the game; groups/prefixes need a permissions mod (GAPS) |
| Placeholder registry (PlaceholderAPI's role: `%id_params%`, `{id_params}`, `%rel_id_params%`, register/unregister, `player` and `server` expansions) | yes (PlaceholderAPI installed, no expansions) | written | clean room; other mods: `horizon_client::placeholders` |
| TAB's own expansion (`%tab_tabprefix%`... when `register-tab-expansion`) | no | written | |
| Header/footer designs with display conditions | no | written | sent through the game's `native_tab_list` |
| Below-name objective (value, fancy value, default for NPCs) | no | written | |
| Sidebar scoreboard (several boards, display conditions, `||` right-aligned text, title, toggle command, remember toggle, hidden by default, delay on join, `show`, `announce`) | no | written | 1.20.3+ form only (number formats), at most 15 lines |
| Boss bars (style, colour, progress, text, display conditions, announcement bars, toggle, remember toggle, hidden by default, `send`, `announce`) | no | partly | the `/bossbar` toggle command cannot be registered (vanilla has it): `/tab bossbar` works; `/tab bossbar send <player> <bar> <seconds>` shows an announcement bar to one player only while their boss bars are on, as TAB does |
| Spectator fix (`prevent-spectator-effect`) | no | partly | resends every 2 s and rewrites the game's own mode packets through the packet hook (unbuilt) |
| Ping spoof | no | partly | resends every 15 s and rewrites latency packets through the packet hook (unbuilt) |
| `/tab` commands: `reload`, `parse`, `player`/`playeruuid`/`group` `<name> <property> [value] [-w world] [-s server]`, `... remove`, `group <name>` (`tab.groupinfo`), `groups`, `debug` (TAB removed it: prints the same notice), `scoreboard [on/off/toggle/show/announce]`, `bossbar [on/off/toggle/send/announce]`, `nametag show/hide/toggle[view]`, `setcollision`, help (only with `tab.admin`) | — | written | sub-commands, arguments, permission nodes, order of checks and message keys follow TAB's `command/*.java`; `/tab player <name>` alone prints the syntax help, as TAB does |
| `/tab cpu`, `dump`, `mysql` | — | partly | `cpu` and `mysql upload/download` check TAB's nodes (`tab.cpu`, `tab.mysql.upload`, `tab.mysql.download`) and then answer ("not part of this port" / `mysql-fail-not-enabled`); `dump <player>` prints local facts instead of uploading a report |
| Tab completion of `/tab` subcommands, arguments and fixed choices (scoreboard, bossbar, nametag, setcollision, board and bar names from the config) | — | written | one usage per sub-command path (`commands()` in `core/src/api.rs`), so the client completes them itself; player names via `player_name`; all players see the words (the tree is per root, see GAPS); blind: the usage tree is not parsed by Horizon until a build |
| Permission nodes | — | written | taken from `TabConstants.Permission` in the source: `tab.nametag.visibility(.other)`, `tab.nametag.view(.other)`, `tab.bossbar.toggle(.other)`, `tab.scoreboard.toggle/show(.other)`, `tab.announce.bar/scoreboard`, `tab.change.<property>`, `tab.remove`, `tab.groupinfo`, `tab.grouplist`, `tab.reload`, `tab.parse`, `tab.setcollision`, `tab.cpu`, `tab.debug`, `tab.dump`, `tab.mysql.*`, `tab.tabcomplete`; there is no `tab.bossbar.show` or `tab.nametag.toggle`. TAB's own quirks are kept: `bossbar send` is gated by `tab.scoreboard.show`, and `nametag <show/hide/toggle>` without a viewer needs the `.other` node |
| `tab.bypass` (see all players in the per-world list), `tab.seevanished` (see vanished players in the global list/layout) | — | not started | depend on the per-world and global player lists, which are not ported |
| `/tab` `announce` (`scoreboard` and `bossbar`) | — | written | `tab.announce.scoreboard` / `tab.announce.bar` |
| Name tag opacity (`nametag opaque`, `opaqueview`) | no | not started | needs line-of-sight checks from the game (GAPS); the command checks its node, then says it is not ported |
| Layout (fake Tab list slots with skins) | no | not started | |
| Per-world player list | no | not started | needs per-viewer Tab list removal the hub would undo |
| Global player list, proxy support (RedisBungee/Redis/RabbitMQ), MySQL storage | no | not started | |
| Bedrock-specific handling | — | not started | `%bedrock%` asks a registered integration |
| `assign-groups-by-permissions` with `primary-group-finding-list` | no | written | through `tab.group.<name>` |

## Test first, once a build is possible

1. `horizon-mixins apply` with this bundle: the raw target `horizon_client::game::native_hooks::Game<'_>::native_server_packet`
   (a free-standing function, no `self`), the `player_respawned` hook, the three `[[module]]`s, and the crate as a dependency of
   `horizon-client-lib`. The two hub injections of the first revision are gone (`native_set_list_name` replaced them).
2. Compile errors in `helpers/glue.rs` (it uses `pub(super)` game helpers from a new child module of `game`: `native_set_list_name`,
   `native_max_players`, `native_permission_check`, `native_watch_server_packets`).
3. On a throwaway server with two probe clients: teams arrive (`SetPlayerTeam` method 0 with prefix/suffix), the
   display names (`PlayerInfoUpdate` 0x20), the `TAB-PlayerList` objective in slot 0 with scores; sort order in the
   Tab list (a player in a higher group above).
4. A third player joining: the existing clients get their team and name within a tick; the newcomer gets everyone's.
5. A leave: team removed, score reset. `/tab reload` with a changed `groups.yml`.
6. Respawn and dimension change: do teams, objectives and boss bars survive on the client (GAPS, "Unknowns")?
7. Command feedback inside `native_perform_command`; `/sb` registration; `/tab` from the console.
