//! TAB for Horizon: the logic of the TAB plugin (tab list header/footer, player list names, name
//! tags and sorting through teams, player list and below-name objectives, sidebar scoreboard,
//! boss bars) and a placeholder registry other mods register into. Free of game types, so it is
//! unit tested on its own; the bundle's helpers feed it the game state and send what it answers.
//!
//! Reads and writes TAB's own files (`config.yml`, `groups.yml`, `users.yml`, `animations.yml`,
//! `messages.yml`, `playerdata.yml`), so a server's `plugins/TAB/` folder works as it is.

pub mod api;
pub mod commands;
pub mod conditions;
pub mod config;
pub mod datefmt;
pub mod engine;
pub mod integration;
pub mod out;
pub mod placeholders;
pub mod properties;
pub mod sorting;
pub mod tabph;
pub mod text;
pub mod yaml;
