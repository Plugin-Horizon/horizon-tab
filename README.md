# horizon-tab

> **Unofficial Rust port of TAB for the Horizon server (https://github.com/arismoko/horizon). Not affiliated with or endorsed by the TAB authors.**

**Not affiliated with TAB (NEZNAMY) or PlaceholderAPI (HelpChat).**

> **Built for and working only with [arismoko/horizon](https://github.com/arismoko/horizon)** (the Horizon fork that carries the mod API). [horizon-rust/horizon](https://github.com/horizon-rust/horizon) is experimental for mods and not supported: the earlier version for its native hooks / horizon-mixins is frozen on the `horizon-plain` branch (work stopped on 2026-11-12).

## Which Horizon

This is a native trait mod: `horizon-mod.toml`, a `Cargo.toml` that depends on `horizon-mod-api`, `src/lib.rs` with the hooks, and the plugin logic in the `core/` crate (plain Rust, no engine needed for its tests).

- Pin: `[native] horizon = "54090ee36c4e2c7b43b66db864220c577b94b7e0"` in `horizon-mod.toml` (game `26.3`). That is a development build of arismoko/horizon `main` plus mod API additions that are not all merged upstream yet, so a stock `main` may lack some host methods this mod calls until they land.
- It compiled and loaded on that build and was smoke-tested there; it has not seen a long run with real players. Treat it as experimental.
- Feature descriptions below were written for the earlier horizon-rust version; where they name engine calls (`native_*`) or old file paths, read them as descriptions only. The Known gaps list is current.

## What it does

- Reads TAB's own files from `config/tab/` (beside `server.properties`): `config.yml`, `groups.yml`, `users.yml`,
  `animations.yml`, `messages.yml`, `playerdata.yml`. A Bukkit server's `plugins/TAB/` folder can be copied there as it
  is; `/tab player|group ...` writes the same files back. Missing files are created with TAB's defaults.
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
  `<gradient:#RRGGBB>text</gradient:#RRGGBB>`) and, with `components.minimessage-support`, common MiniMessage tags.
- Commands, as TAB 6.2.2 has them: `/tab reload | parse | player | playeruuid | group | groups | debug | dump | cpu |
  mysql | scoreboard | bossbar | nametag | setcollision` and the scoreboard toggle command (`/sb`), with TAB's
  permission nodes (`tab.admin` allows all). `cpu`, `dump` and `mysql` need parts that are not ported, see the Known gaps section.

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

What does not work yet on arismoko/horizon (each is marked `TODO(arismoko-api: ...)` in `src/lib.rs`):

- Team prefixes and suffixes are not shown (only the colour); the tab-list and below-name objective slots are dropped.
- The anti-override guard is off (outgoing packets cannot be watched), so other senders' team, latency or name packets are not put back.
- `%player-version%` is wrong (no protocol read); name completion falls back to plain words.
- A toggle named like a vanilla command (`/bossbar`) is only reachable as `/tab bossbar`.
- List names are per player, not per viewer.

## Licence and origin

Derived from TAB (https://github.com/NEZNAMY/TAB, Apache-2.0). Licensed under the Apache License 2.0, see
[LICENSE](LICENSE) and [NOTICE](NOTICE). The placeholder registry is original work, also released under Apache-2.0.

- **TAB** is Apache-2.0. Its source was read; the sorting, the property lookup order, the condition syntax, the
  placeholder names and intervals, the formats and the command set follow it. No Java was copied. Its default
  config files are included unchanged as `core/defaults/` (`core/testdata/` holds the sample configuration the unit tests use).
- **PlaceholderAPI** is GPL-3.0: **clean room**. Its source and its expansions' sources were never opened. The
  registry (`core/src/placeholders/`) follows only the placeholder syntax users write (`%identifier_params%`,
  `{...}`, `%rel_...%`) and expansion names users know (`player`, `server`).
