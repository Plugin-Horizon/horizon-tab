# horizon-tab

## Port status

Item list: audit/rec100/tab.md (round 5, 2026-11-25: 41 of 44 items work, 2 are built up to the transport (engine), 1 is n/a (removed); nothing is open; the code is written against the combined next SDK and was not compiled yet). Earlier estimates below are superseded.

> **Unofficial Rust port of TAB for the Horizon server (https://github.com/arismoko/horizon). Not affiliated with or endorsed by the TAB authors.**

**Not affiliated with TAB (NEZNAMY) or PlaceholderAPI (HelpChat).**

> **Built for and working only with [arismoko/horizon](https://github.com/arismoko/horizon)** (the Horizon fork that carries the mod API). [horizon-rust/horizon](https://github.com/horizon-rust/horizon) is experimental for mods and not supported: the earlier version for its native hooks / horizon-mixins is frozen on the `horizon-plain` branch (work stopped on 2026-11-12).

## Which Horizon

This is a native trait mod: `horizon-mod.toml`, a `Cargo.toml` that depends on `horizon-mod-api`, `src/lib.rs` with the hooks, and the plugin logic in the `core/` crate (plain Rust, no engine needed for its tests).

- Pin: `[native] horizon = "54090ee36c4e2c7b43b66db864220c577b94b7e0"` in `horizon-mod.toml` (game `26.3`). That is a development build of arismoko/horizon `main` plus mod API additions that are not all merged upstream yet, so a stock `main` may lack some host methods this mod calls until they land.
- It compiled and loaded on that build and was smoke-tested there; it has not seen a long run with real players. Treat it as experimental.
- Feature descriptions below were written for the earlier horizon-rust version; where they name engine calls (`native_*`) or old file paths, read them as descriptions only. The Known gaps list is current.

## What it does

Only part of the list below is visible on Horizon today: header/footer, one tab list name per player (the same for
every viewer), boss bars, chat lines, commands and placeholders work. Name tags and sorting (scoreboard teams), the
sidebar, the below-name and list objectives, ping bars and game-mode updates compute their data but nothing is sent
to players yet; see Known gaps.

- Reads TAB's own files from `config/tab/` (beside `server.properties`): `config.yml`, `groups.yml`, `users.yml`,
  `animations.yml`, `messages.yml`, `playerdata.yml`. A Bukkit server's `plugins/TAB/` folder can be copied there as it
  is; `/tab player|group ...` writes the same files back. Missing files are created with TAB's defaults.
- With `mysql.enabled: true` groups and users are kept in the MySQL tables `tab_groups` and `tab_users` instead (TAB's
  layout); `/tab mysql upload|download` copy between the tables and the two files. Not compiled yet (see Known gaps).
- Tab list names: `tabprefix` + `customtabname` + `tabsuffix`, looked up as TAB does (per world, per server, user by
  name, user by UUID, group, `_DEFAULT_`).
- Name tags: one team per player with `tagprefix` / `tagsuffix`, the team colour taken from the prefix's last colour,
  collision, invisible name tags; sorting with all of TAB's sorting types.
- Player list objective (the number beside the name, with a fancy value), below-name objective.
- Header/footer designs and sidebar scoreboards with display conditions, boss bars, announcements, toggles.
- Placeholders: TAB's built-ins (`%player%`, `%ping%`, `%online%`, `%staffonline%`, `%world%`, `%date%`, ...),
  animations, conditions, output replacements and refresh intervals; anything else (`%player_name%`,
  `%server_tps_1_colored%`, a mod's `%coins_balance%`) comes from the registry.
- Formatting: `&` codes, `&#RRGGBB`, `{#RRGGBB}`, `#<RRGGBB>`, `&x&R&R&G&G&B&B`, TAB's gradients
  (`<#RRGGBB>text</#RRGGBB>`, `{#RRGGBB>}text{#RRGGBB<}`, `<$#RRGGBB>text<$#RRGGBB>`,
  `<gradient:#RRGGBB>text</gradient:#RRGGBB>`) and, with `components.minimessage-support`, MiniMessage: colours,
  decorations, `<reset>`, `<newline>`, `<gradient>`, `<rainbow>`, `<transition>`, `<pride>`, `<shadow>`, `<font>`,
  `<insertion>`, `<click>`, `<hover>`, `<key>`, `<lang>`, `<selector>`, `<score>`, `<nbt>` and `\<` escapes (`mm.rs`).
- Features of round 4: `layout` (fake tab-list entries per viewer, fixed slots, groups, COLUMNS/ROWS), `per-world-playerlist`,
  opaque name tags (`/tab nametag opaque|opaqueview`, line of sight), nick-plugin compatibility (a renamed entry moves the
  team member and the scores), `placeholders.locale` (en, de, fr, es, it, nl, pt, sv, pl).
- Commands, as TAB 6.2.2 has them: `/tab reload | parse | player | playeruuid | group | groups | debug | dump | cpu |
  mysql | scoreboard | bossbar | nametag | setcollision` and the scoreboard toggle command (`/sb`), with TAB's
  permission nodes (`tab.admin` allows all). `/tab cpu` times the engine's stages and the placeholders for ten seconds and prints TAB's report; `/tab dump <player>` writes `plugins/TAB/dump/<player>-<time>.txt` (nothing is uploaded).

What is ported and what is not: the Known gaps section. What Horizon would need to do it without internals:
the Known gaps section.

## For other mods

Once this bundle is installed, other native mods reach two modules:

```rust
// A placeholder expansion: %coins_balance% for a player.
horizon_client::placeholders::register_fn("coins", |player, params| match params {
    "balance" => player.map(|p| balance_of(p.uuid).to_string()),
    _ => None,
});
let line = horizon_client::placeholders::set_placeholders_for(Some(uuid), "You have %coins_balance% coins");

// Permissions, groups, prefixes, vanish, Bedrock: a permissions mod answers TAB.
struct MyPerms;
impl horizon_client::tab::Integration for MyPerms {
    fn name(&self) -> &str { "myperms" }
    fn has_permission(&self, player: u128, node: &str) -> Option<bool> { /* ... */ None }
    fn primary_group(&self, player: u128) -> Option<String> { /* ... */ None }
    fn prefix(&self, player: u128) -> Option<String> { /* ... */ None }
}
horizon_client::tab::register_integration(Box::new(MyPerms));
```

Without an integration, operators (level 1 and up) have every TAB permission and everyone's group is `default`.

## Build

Build it as a native trait mod with arismoko's launcher (see the "native mods" how-to in the arismoko/horizon docs):

```sh
horizon-launcher native-build <path to this folder> --server-output <dir> --version <tag> --trust-native-code
```

`--trust-native-code` is your consent to compile and run this code inside the server. The logic crate tests run without the engine: `(cd core && cargo test)`.

## Known gaps

Round 4 (2026-11-25) closed the list of open items; what stays out, and why:

- `proxy-support`, `global-playerlist`, `%online_<server>%` and the `tab:bridge-6` bridge (round 5): the logic is in the mod and tested (`proxy.rs`: TAB's ProxySupport messages, the remote-player directory, server groups and `shouldSee`; `bridge.rs` and `Engine::bridge_*`: the backend half of the proxy bridge). The transports (round 8, uncompiled until the queue builds them): `tab:bridge-6` and the `PLUGIN` support channel (`tab:<TAB_4_suffix>` lower-cased) run on `register_plugin_channel`, the `plugin_message` hook and `send_plugin_message` (the carrier is the first online player; messages wait while nobody is online); `REDIS` runs on the redis crate (`transport.rs`, pub/sub on the `TAB_4_<suffix>` channel with the Base64 line as payload). `RABBITMQ` runs on lapin (`transport.rs`: a fanout exchange named by `rabbitmq.exchange`, an exclusive queue per instance, the Base64 line as body). Neither broker link was checked against a second TAB instance (delivery4j's framing is not in the source tree).
- Unlimited name tags (armor stands): removed from TAB before 6.2.2 (its LegacyConverter deletes the option); nothing to port.
- `/tab dump` writes a local file; the upload to a paste service is not done (the dump holds player names, ids and the config).
- The help menu has no hover or click events in TAB 6.2.2; a `help-menu` in `messages.yml` that uses `<click>` or `<hover>` works through the MiniMessage module.
- Layout skins: `signed_texture:<value>;<signature>` and `player:<online name>` are resolved; `mineskin:`, `texture:` and `player:` of players who are not online need TAB's web lookups and skin cache, so those slots show the client's default skin (TAB's own result when a source fails).
- `per-world-playerlist` regex world patterns support literals, `.`, classes, `\d \w \s`, `* + ?` and `|`; a pattern with groups or counted repeats is compared as a literal (TAB's own fallback for an invalid regex).
- MiniMessage: Adventure's source is not in the repository; gradient, rainbow and transition follow its documentation (negative phase included). `pride` knows pride, trans, bi, pan, nb and ace.
- The LuckPerms bridge gives the primary group only; prefixes and suffixes are not published to mods.
- Player-name arguments of `/tab` complete online names, only for senders with `tab.tabcomplete`; the literal sub-command words are not filtered per sender. An offline name is still accepted as typed.
- MySQL storage (`mysql.enabled`) is blind-written. Its database calls run on the server thread. Rows with unknown properties get no `checkProperty` warning.

## Licence and origin

Derived from TAB (https://github.com/NEZNAMY/TAB, Apache-2.0). Licensed under the Apache License 2.0, see
[LICENSE](LICENSE) and [NOTICE](NOTICE). The placeholder registry is original work, also released under Apache-2.0.

- **TAB** is Apache-2.0. Its source was read; the sorting, the property lookup order, the condition syntax, the
  placeholder names and intervals, the formats and the command set follow it. No Java was copied. Its default
  config files are included unchanged as `core/defaults/` (`core/testdata/` holds the sample configuration the unit tests use).
- **PlaceholderAPI** is GPL-3.0: **clean room**. Its source and its expansions' sources were never opened. The
  registry (`core/src/placeholders/`) follows only the placeholder syntax users write (`%identifier_params%`,
  `{...}`, `%rel_...%`) and expansion names users know (`player`, `server`).
