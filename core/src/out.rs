//! What the engine asks the game to send: the vanilla packets behind the Tab list, teams,
//! scoreboard objectives and boss bars, with components as JSON. The mod's helpers turn them into
//! `horizon_protocol` packets (`helpers/tick.rs`); here they stay free of game types for tests.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum NumberFmt {
    Blank,
    Fixed(Value),
}

/// `ClientboundSetPlayerTeamPacket.Parameters`.
#[derive(Clone, Debug, PartialEq)]
pub struct TeamParams {
    pub display_name: Value,
    pub prefix: Value,
    pub suffix: Value,
    /// `Team.Visibility`: 0 always, 1 never, 2 hide for other teams, 3 hide for own team.
    pub visibility: i32,
    /// `Team.CollisionRule`: 0 always, 1 never, 2 push other teams, 3 push own team.
    pub collision: i32,
    /// `TeamColor` 0..15, None for reset.
    pub color: Option<i32>,
    /// 1 friendly fire, 2 see friendly invisibles.
    pub options: i8,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectiveMethod {
    Add,
    Remove,
    Change,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BossOut {
    Add {
        title: Value,
        progress: f32,
        color: i32,
        overlay: i32,
    },
    Remove,
    Progress(f32),
    Title(Value),
    Style {
        color: i32,
        overlay: i32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// Header and footer (sent with the game's `native_tab_list`), with their plain text.
    TabList {
        header: Value,
        footer: Value,
        header_plain: String,
        footer_plain: String,
    },
    /// `PlayerInfoUpdate` with `UPDATE_DISPLAY_NAME` (0x20); None shows the real name.
    DisplayNames(Vec<(u128, Option<Value>)>),
    TeamCreate {
        name: String,
        params: TeamParams,
        players: Vec<String>,
    },
    TeamUpdate {
        name: String,
        params: TeamParams,
    },
    TeamRemove {
        name: String,
    },
    Objective {
        name: String,
        method: ObjectiveMethod,
        title: Value,
        hearts: bool,
        format: Option<NumberFmt>,
    },
    /// `DisplaySlot`: 0 list, 1 sidebar, 2 below name.
    DisplaySlot {
        slot: i32,
        objective: String,
    },
    Score {
        owner: String,
        objective: String,
        score: i32,
        display: Option<Value>,
        format: Option<NumberFmt>,
    },
    ResetScore {
        owner: String,
        objective: String,
    },
    Boss {
        id: u128,
        action: BossOut,
    },
    /// `PlayerInfoUpdate` with `UPDATE_LATENCY` (0x10).
    Latency(Vec<(u128, i32)>),
    /// `PlayerInfoUpdate` with `UPDATE_GAME_MODE` (0x04).
    GameModes(Vec<(u128, i32)>),
    /// A system chat line.
    Chat(Value),
}

/// A packet for one player.
#[derive(Clone, Debug, PartialEq)]
pub struct Packet {
    pub to: u128,
    pub out: Out,
}
