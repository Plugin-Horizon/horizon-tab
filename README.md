# horizon-tab

> **Unofficial Rust port of TAB for the Horizon server (https://github.com/horizon-rust/horizon). Not affiliated with or endorsed by the TAB authors.**

TAB's features for [Horizon](https://github.com/horizon-rust/horizon) servers, as a native mod (a horizon-mixins
bundle): the Tab list's header and footer, formatted player names in the Tab list, name tags and sorting through
teams, the player list and below-name objectives, the sidebar scoreboard and boss bars, all driven by placeholders.
It comes with a small placeholder registry that other mods register into, in the role PlaceholderAPI plays on
Bukkit servers.

**Not affiliated with TAB (NEZNAMY) or PlaceholderAPI (HelpChat).**

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
  permission nodes (`tab.admin` allows all). `cpu`, `dump` and `mysql` need parts that are not ported, see STATUS.md.

What is ported and what is not: [STATUS.md](STATUS.md). What Horizon would need to do it without internals:
[GAPS.md](GAPS.md).

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

The bundle is this folder (`horizon-mixin.toml`, `helpers/`, the crate `core/`), pinned to Horizon
`a589a3dd9be04fa7f17667c0175711aaa92e1147`:

```sh
horizon-mixins apply --workspace <horizon source> --mods <folder holding this one> --out <stage>
cd <stage> && cargo build --profile fast -p horizon-client --no-default-features
cd core && cargo test          # the logic alone, no engine needed
```

The core uses only `serde_json` and `chrono` from Horizon's `Cargo.lock`.

## How it hooks in

| Injection | Target | Why |
|---|---|---|
| `start` | hook point `server_about_to_start` v1 | default files, load |
| `tick` | hook point `tick` v1 | players and server to the core, its packets out |
| `stopping` | hook point `server_stopping` v1 | take away what was shown |
| `register-commands`, `perform-command`, `console-command` | hook points v1 | `/tab`, `/sb` |
| `player-info` (RETURN) | `lan::hub::Hub::player_info` | the game's own Tab list entries carry the formatted name |
| `set-nickname` (HEAD) | `lan::hub::Hub::set_nickname` | a nickname change overwrites names; resend |
| modules `tab`, `placeholders` | `horizon_client::tab`, `horizon_client::placeholders` | the API, for the game and other mods |
| module `glue` | `crate::game::tab_glue` | reads hub state (latency, nicknames, hidden players), sends packets |

The header and footer go through the game's `native_tab_list`; everything else is sent as vanilla packets
(`SetPlayerTeam`, `PlayerInfoUpdate`, `SetObjective`, `SetDisplayObjective`, `SetScore`, `ResetScore`, `BossEvent`).

## Licence and origin

Derived from TAB (https://github.com/NEZNAMY/TAB, Apache-2.0). Licensed under the Apache License 2.0, see
[LICENSE](LICENSE) and [NOTICE](NOTICE). The placeholder registry is original work, also released under Apache-2.0.

- **TAB** is Apache-2.0. Its source was read; the sorting, the property lookup order, the condition syntax, the
  placeholder names and intervals, the formats and the command set follow it. No Java was copied. Its default
  config files are included unchanged as `core/defaults/` (`core/testdata/` holds the sample configuration the unit tests use).
- **PlaceholderAPI** is GPL-3.0: **clean room**. Its source and its expansions' sources were never opened. The
  registry (`core/src/placeholders/`) follows only the placeholder syntax users write (`%identifier_params%`,
  `{...}`, `%rel_...%`) and expansion names users know (`player`, `server`).

## Untested

Everything that touches the engine: written against the Horizon source without a build (see STATUS.md, "Test first").

## Contributing

Pull requests and improvements are always welcome! Open an issue or a PR -- direct pushes to main are disabled, every change goes through review.
