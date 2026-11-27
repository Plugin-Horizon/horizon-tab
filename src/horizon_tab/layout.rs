//! The `layout` feature (TAB's `LayoutManagerImpl`, `LayoutConfiguration`, `LayoutPattern`,
//! `PlayerGroup`, `FixedSlot`): up to 80 fake tab-list entries per viewer, laid out in columns or
//! rows, filled with fixed texts and with players chosen by conditions; the real entries are
//! hidden while a layout shows. This module is the configuration and the slot assignment; the
//! engine builds the entries and `lib.rs` sends them (`minecraft:player_info_update`).
//!
//! Skins: `signed_texture:<value>;<signature>` and `player:<online player>` are resolved here;
//! `mineskin:`, `texture:` and `player:` for players who are not online need TAB's web lookups
//! (`api.mineskin.org`, Mojang) and its skin cache, which this port does not call. Such a slot
//! shows the client's default skin for its id, as TAB does when a skin source fails.

use crate::horizon_tab::yaml::Yaml;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Columns,
    Rows,
}

impl Direction {
    /// `Direction.translateSlot`.
    pub fn translate(self, slot: u32) -> u32 {
        match self {
            Direction::Columns => slot,
            Direction::Rows => (slot - 1) % 4 * 20 + (slot - ((slot - 1) % 4)) / 4 + 1,
        }
    }
}

/// The id of a slot's fake entry: `new UUID(0, translated slot)`.
pub fn slot_uuid(direction: Direction, slot: u32) -> u128 {
    u128::from(direction.translate(slot))
}

/// The list order TAB gives a slot (`Integer.MAX_VALUE - translated slot`): higher is listed first.
pub fn list_order(direction: Direction, slot: u32) -> i32 {
    i32::MAX - direction.translate(slot) as i32
}

#[derive(Clone, Debug, PartialEq)]
pub struct FixedSlotDef {
    pub slot: u32,
    pub text: String,
    pub skin: Option<String>,
    pub ping: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupDef {
    pub condition: Option<String>,
    pub slots: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutDef {
    pub name: String,
    pub condition: Option<String>,
    pub default_skin: Option<String>,
    pub slot_count: u32,
    pub fixed: Vec<FixedSlotDef>,
    pub groups: Vec<(String, GroupDef)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutCfg {
    pub direction: Direction,
    pub default_skin: String,
    pub remaining_enabled: bool,
    pub remaining_text: String,
    pub empty_ping: i32,
    pub default_skins: HashMap<u32, String>,
    pub layouts: Vec<LayoutDef>,
}

/// `1-20` or `5` as slot numbers; None for text that is not one.
fn slot_range(line: &str) -> Option<(u32, u32)> {
    let mut parts = line.trim().split('-');
    let from: u32 = parts.next()?.trim().parse().ok()?;
    let to: u32 = match parts.next() {
        Some(t) => t.trim().parse().ok()?,
        None => from,
    };
    Some((from, to))
}

fn opt(y: &Yaml, key: &str) -> Option<String> {
    let v = y.get(key);
    if v.is_null() { None } else { v.as_str() }
}

impl LayoutCfg {
    pub fn from_yaml(y: &Yaml, warnings: &mut Vec<String>) -> LayoutCfg {
        let dir = y.get("direction").str_or("COLUMNS");
        let direction = match dir.as_str() {
            "COLUMNS" => Direction::Columns,
            "ROWS" => Direction::Rows,
            other => {
                warnings.push(format!(
                    "\"{other}\" is not a valid type of layout direction. Valid options are: [COLUMNS, ROWS]. Using COLUMNS"
                ));
                Direction::Columns
            }
        };
        let mut default_skins = HashMap::new();
        for (_, group) in y.get("default-skins").entries() {
            let Some(skin) = opt(group, "skin") else { continue };
            for line in group.get("slots").str_list() {
                if let Some((from, to)) = slot_range(&line) {
                    for s in from..=to.min(80) {
                        default_skins.insert(s, skin.clone());
                    }
                }
            }
        }
        let layouts = y
            .get("layouts")
            .entries()
            .iter()
            .map(|(name, def)| LayoutDef::from_yaml(name, def, warnings))
            .collect();
        LayoutCfg {
            direction,
            default_skin: y.get("default-skin").str_or("mineskin:1753261242"),
            remaining_enabled: y.get("enable-remaining-players-text").bool_or(true),
            remaining_text: y.get("remaining-players-text").str_or("... and %s more"),
            empty_ping: y.get("empty-slot-ping-value").i64_or(1000) as i32,
            default_skins,
            layouts,
        }
    }

    /// `LayoutConfiguration.getDefaultSkin(slot)`.
    pub fn default_skin(&self, slot: u32) -> &str {
        self.default_skins.get(&slot).map(String::as_str).unwrap_or(&self.default_skin)
    }
}

impl LayoutDef {
    fn from_yaml(name: &str, y: &Yaml, warnings: &mut Vec<String>) -> LayoutDef {
        let mut slot_count = if y.get("slot-count").is_null() { 80 } else { y.get("slot-count").i64_or(80) };
        if !(0..=80).contains(&slot_count) {
            warnings.push(format!(
                "Layout \"{name}\" has invalid slot-count value \"{slot_count}\" defined. Slot count must range between 0 - 80. Using 80."
            ));
            slot_count = 80;
        }
        let slot_count = slot_count as u32;
        let mut fixed = Vec::new();
        for line in y.get("fixed-slots").str_list() {
            let parts: Vec<&str> = line.split('|').collect();
            let invalid = || {
                format!(
                    "Layout {name} has invalid fixed slot defined as \"{line}\". Supported values are \"SLOT|TEXT\", \"SLOT|TEXT|SKIN\" and \"SLOT|TEXT|SKIN|PING\", where SLOT is a number from 1 to {slot_count}, TEXT is displayed text, SKIN is skin used for the slot and PING is the latency value (number or placeholder)"
                )
            };
            if parts.len() < 2 {
                warnings.push(invalid());
                continue;
            }
            let Ok(slot) = parts[0].trim().parse::<u32>() else {
                warnings.push(invalid());
                continue;
            };
            if slot < 1 || slot > slot_count {
                warnings.push(format!(
                    "Layout {name} has invalid fixed slot value \"{slot}\" defined. Slots must range between 1 - {slot_count}."
                ));
                continue;
            }
            let skin = parts.get(2).map(|s| s.to_string());
            let mut ping = None;
            if let Some(p) = parts.get(3) {
                let p = p.trim();
                if !p.is_empty() {
                    if !p.contains('%') && p.parse::<f64>().is_err() {
                        warnings.push(format!(
                            "Layout {name} has fixed slot with defined ping \"{p}\", which is not a valid number"
                        ));
                    } else {
                        ping = Some(p.to_owned());
                    }
                }
            }
            fixed.push(FixedSlotDef { slot, text: parts[1].to_owned(), skin, ping });
        }
        let mut groups: Vec<(String, GroupDef)> = Vec::new();
        let mut no_condition: Option<String> = None;
        let mut taken: HashMap<u32, String> = HashMap::new();
        for (gname, g) in y.get("groups").entries() {
            let mut slots: Vec<u32> = Vec::new();
            for line in g.get("slots").str_list() {
                let Some((from, to)) = slot_range(&line) else { continue };
                for i in from..=to {
                    if i < 1 || i > slot_count {
                        warnings.push(format!(
                            "Layout {name}'s player group \"{gname}\" has invalid slot value \"{i}\" defined. Slots must range between 1 - {slot_count}."
                        ));
                        continue;
                    }
                    if slots.contains(&i) {
                        warnings.push(format!("Layout {name}'s player group \"{gname}\" has duplicated slot \"{i}\"."));
                        continue;
                    }
                    slots.push(i);
                }
            }
            let def = GroupDef { condition: opt(g, "display-condition"), slots };
            if let Some(first) = &no_condition {
                warnings.push(format!(
                    "Layout \"{name}\"'s player group \"{gname}\" is unreachable, because it is defined after group \"{first}\", which has no condition requirement."
                ));
            } else if def.condition.is_none() {
                no_condition = Some(gname.clone());
            }
            for s in &def.slots {
                match taken.get(s) {
                    Some(other) => warnings.push(format!(
                        "Layout \"{name}\"'s player group \"{gname}\" defines slot {s}, but this slot is already taken by group \"{other}\", which will take priority."
                    )),
                    None => {
                        taken.insert(*s, gname.clone());
                    }
                }
            }
            groups.push((gname.clone(), def));
        }
        // A fixed slot takes a slot out of the groups (`LayoutPattern.addGroup`).
        for (_, g) in &mut groups {
            g.slots.retain(|s| !fixed.iter().any(|f| f.slot == *s));
        }
        LayoutDef {
            name: name.to_owned(),
            condition: opt(y, "display-condition"),
            default_skin: opt(y, "default-skin"),
            slot_count,
            fixed,
            groups,
        }
    }

    /// The skin text of a fixed slot (`FixedSlot.fromDefinition`).
    pub fn fixed_skin(&self, cfg: &LayoutCfg, f: &FixedSlotDef) -> String {
        match &f.skin {
            Some(s) if !s.is_empty() => s.clone(),
            _ => self.default_skin.clone().unwrap_or_else(|| cfg.default_skin(f.slot).to_owned()),
        }
    }

    /// The skin text of a slot that shows no player and no fixed text.
    pub fn empty_skin(&self, cfg: &LayoutCfg, slot: u32) -> String {
        self.default_skin.clone().unwrap_or_else(|| cfg.default_skin(slot).to_owned())
    }

    /// Slots no group and no fixed slot takes.
    pub fn empty_slots(&self) -> Vec<u32> {
        (1..=self.slot_count)
            .filter(|s| !self.fixed.iter().any(|f| f.slot == *s))
            .filter(|s| !self.groups.iter().any(|(_, g)| g.slots.contains(s)))
            .collect()
    }
}

/// What a group puts in one slot.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Player(usize),
    Text(String),
}

/// `PlayerGroup.tick` for every group of a layout: `players` are the candidates in listing order;
/// `meets(group, player)` is the group's condition for that player. Returns slot -> content.
pub fn assign(
    cfg: &LayoutCfg,
    def: &LayoutDef,
    players: &[usize],
    meets: &mut dyn FnMut(&GroupDef, usize) -> bool,
) -> HashMap<u32, Content> {
    let mut remaining: Vec<usize> = players.to_vec();
    let mut out = HashMap::new();
    for (_, g) in &def.groups {
        let mut meeting = Vec::new();
        remaining.retain(|p| {
            let ok = g.condition.is_none() || meets(g, *p);
            if ok {
                meeting.push(*p);
            }
            !ok
        });
        for (index, slot) in g.slots.iter().enumerate() {
            if cfg.remaining_enabled && index == g.slots.len() - 1 && g.slots.len() < meeting.len() {
                let more = meeting.len() - g.slots.len() + 1;
                out.insert(*slot, Content::Text(cfg.remaining_text.replace("%s", &more.to_string())));
                break;
            }
            match meeting.get(index) {
                Some(p) => {
                    out.insert(*slot, Content::Player(*p));
                }
                None => {
                    out.insert(*slot, Content::Text(String::new()));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizon_tab::yaml;

    fn cfg(text: &str) -> (LayoutCfg, Vec<String>) {
        let mut w = Vec::new();
        let c = LayoutCfg::from_yaml(&yaml::parse(text).unwrap(), &mut w);
        (c, w)
    }

    const SAMPLE: &str = "direction: ROWS\ndefault-skin: 'signed_texture:abc;def'\nempty-slot-ping-value: 5\nlayouts:\n  default:\n    display-condition: '%world%=world'\n    slot-count: 40\n    fixed-slots:\n      - '1|&3Name&f:'\n      - '5|&b%player%|player:Steve|%ping%'\n      - '99|too far'\n      - 'x|bad'\n    groups:\n      staff:\n        display-condition: permission:tab.staff\n        slots:\n          - 1-4\n      players:\n        slots:\n          - 6-9\n      later:\n        slots:\n          - 9\n";

    #[test]
    fn rows_translate_like_tab() {
        assert_eq!(Direction::Rows.translate(1), 1);
        assert_eq!(Direction::Rows.translate(2), 21);
        assert_eq!(Direction::Rows.translate(5), 2);
        assert_eq!(Direction::Rows.translate(80), 80);
        assert_eq!(Direction::Columns.translate(33), 33);
        assert_eq!(slot_uuid(Direction::Rows, 2), 21);
        assert_eq!(list_order(Direction::Columns, 1), i32::MAX - 1);
    }

    #[test]
    fn the_configuration_is_read_with_tabs_warnings() {
        let (c, w) = cfg(SAMPLE);
        assert_eq!(c.direction, Direction::Rows);
        assert_eq!(c.empty_ping, 5);
        let l = &c.layouts[0];
        assert_eq!((l.name.as_str(), l.slot_count), ("default", 40));
        assert_eq!(l.condition.as_deref(), Some("%world%=world"));
        assert_eq!(l.fixed.len(), 2);
        assert_eq!(l.fixed[1], FixedSlotDef { slot: 5, text: "&b%player%".into(), skin: Some("player:Steve".into()), ping: Some("%ping%".into()) });
        // Slot 1 is fixed, so the staff group lost it.
        assert_eq!(l.groups[0].1.slots, vec![2, 3, 4]);
        assert!(w.iter().any(|m| m.contains("invalid fixed slot value \"99\"")), "{w:?}");
        assert!(w.iter().any(|m| m.contains("invalid fixed slot defined as \"x|bad\"")), "{w:?}");
        assert!(w.iter().any(|m| m.contains("\"later\" is unreachable")), "{w:?}");
        assert!(w.iter().any(|m| m.contains("defines slot 9") && m.contains("\"players\"")), "{w:?}");
    }

    #[test]
    fn skins_and_empty_slots() {
        let (c, _) = cfg(SAMPLE);
        let l = &c.layouts[0];
        assert_eq!(l.fixed_skin(&c, &l.fixed[0]), "signed_texture:abc;def");
        assert_eq!(l.fixed_skin(&c, &l.fixed[1]), "player:Steve");
        assert_eq!(l.empty_slots().len(), 40 - 2 - 3 - 4);
        assert!(!l.empty_slots().contains(&1) && !l.empty_slots().contains(&7) && l.empty_slots().contains(&10));
    }

    #[test]
    fn a_default_skin_per_slot_range() {
        let (c, _) = cfg("default-skin: a\ndefault-skins:\n  g:\n    skin: b\n    slots:\n      - 3-4\nlayouts: {}\n");
        assert_eq!(c.default_skin(3), "b");
        assert_eq!(c.default_skin(5), "a");
    }

    #[test]
    fn groups_take_players_in_order_and_the_last_slot_counts_the_rest() {
        let (c, _) = cfg("layouts:\n  l:\n    slot-count: 10\n    groups:\n      a:\n        display-condition: x\n        slots:\n          - 1-2\n      b:\n        slots:\n          - 3-5\n");
        let l = &c.layouts[0];
        // players 0..5; the condition holds for the even ones.
        let got = assign(&c, l, &[0, 1, 2, 3, 4, 5], &mut |_, p| p % 2 == 0);
        assert_eq!(got[&1], Content::Player(0));
        // Three players meet group a, it has two slots: the last shows "... and 2 more".
        assert_eq!(got[&2], Content::Text("... and 2 more".into()));
        assert_eq!(got[&3], Content::Player(1));
        assert_eq!(got[&4], Content::Player(3));
        assert_eq!(got[&5], Content::Player(5));
        // Without enough players the slots stay empty.
        let got = assign(&c, l, &[0], &mut |_, _| true);
        assert_eq!(got[&1], Content::Player(0));
        assert_eq!(got[&2], Content::Text(String::new()));
        assert_eq!(got[&5], Content::Text(String::new()));
    }

    #[test]
    fn bad_direction_falls_back() {
        let (c, w) = cfg("direction: DIAGONAL\nlayouts: {}\n");
        assert_eq!(c.direction, Direction::Columns);
        assert!(w[0].contains("\"DIAGONAL\" is not a valid type of layout direction"));
    }
}
