//! `/tab` and the toggle commands. Sub-commands, arguments, permission nodes and messages follow
//! TAB's `TabCommand` and its sub-commands (`command/` in the shared module) and `messages.yml`
//! (Apache-2.0, see NOTICE). `cpu` and `dump` print no report/upload, and `mysql` always answers
//! that MySQL is not enabled: those parts of TAB are not in this port.

use crate::engine::Engine;
use crate::integration;
use crate::out::{Out, Packet};
use crate::properties::VALID_PROPERTIES;

/// The TAB version whose commands this follows (shown by `/tab`).
const TAB_VERSION: &str = "6.2.2";

/// `PropertyConfiguration.VALID_PROPERTIES` in TAB's order.
pub const COMMAND_PROPERTIES: [&str; 5] = ["tagprefix", "tagsuffix", "tabprefix", "customtabname", "tabsuffix"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sender {
    Console,
    Player { uuid: u128, op_level: u8 },
}

/// One piece of a line that mixes formatted text with text shown exactly as it is.
#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    /// TAB-formatted text (`&` codes).
    Fmt(String),
    /// Verbatim text in the named colour (`gold`, `yellow`, `white`, `dark_aqua`).
    Lit(String, &'static str),
}

#[derive(Debug, Default, PartialEq)]
pub struct Output {
    /// TAB-formatted lines for the sender (`&` codes).
    pub replies: Vec<String>,
    /// Lines (by index in `replies`) that mix formatted and verbatim pieces; `replies[i]` holds the
    /// pieces joined, for tests.
    pub rich: Vec<(usize, Vec<Seg>)>,
    pub packets: Vec<Packet>,
}

impl Output {
    /// TAB's `sendMessage`: nothing for an empty message, one line per line break.
    fn say(&mut self, line: impl Into<String>) {
        let line = line.into();
        if line.is_empty() {
            return;
        }
        for l in line.split('\n') {
            self.replies.push(l.to_owned());
        }
    }

    fn say_rich(&mut self, segs: Vec<Seg>) {
        let joined: String = segs
            .iter()
            .map(|s| match s {
                Seg::Fmt(t) | Seg::Lit(t, _) => t.as_str(),
            })
            .collect();
        self.rich.push((self.replies.len(), segs));
        self.replies.push(joined);
    }
}

/// `TabPlayer.hasPermission` for a command: the console and `tab.admin` may do everything.
fn has(sender: Sender, node: &str) -> bool {
    match sender {
        Sender::Console => true,
        Sender::Player { uuid, op_level } => {
            integration::permission(uuid, "tab.admin", op_level) || integration::permission(uuid, node, op_level)
        }
    }
}

/// Splits arguments at spaces.
fn words(args: &str) -> Vec<String> {
    args.split_whitespace().map(str::to_owned).collect()
}

fn no_permission(e: &Engine, o: &mut Output) {
    o.say(e.message(
        "no-permission",
        "&cI'm sorry, but you do not have permission to perform this command. Please contact the server administrators if you believe that this is in error.",
    ));
}

fn player_not_online(e: &Engine, o: &mut Output, name: &str) {
    o.say(
        e.message("player-not-online", "&cNo online player found with the name \"%player%\"")
            .replace("%player%", name),
    );
}

fn invalid_number(e: &Engine, o: &mut Output, input: &str) {
    o.say(
        e.message("invalid-number", "\"%input%\" is not a number!")
            .replace("%input%", input),
    );
}

fn only_from_game(e: &Engine, o: &mut Output) {
    o.say(e.message("command-only-from-game", "&cThis command must be ran from the game"));
}

/// `player.sendMessage(...)` to someone who may not be the sender.
fn tell(e: &Engine, sender: Sender, o: &mut Output, uuid: u128, line: String) {
    if line.is_empty() {
        return;
    }
    if matches!(sender, Sender::Player { uuid: u, .. } if u == uuid) {
        return o.say(line);
    }
    for l in line.split('\n') {
        o.packets.push(Packet {
            to: uuid,
            out: Out::Chat(crate::text::component(l, e.cfg.minimessage)),
        });
    }
}

fn sender_index(e: &Engine, sender: Sender) -> Option<usize> {
    match sender {
        Sender::Player { uuid, .. } => e.index_of(uuid),
        Sender::Console => None,
    }
}

fn is_same(sender: Sender, uuid: u128) -> bool {
    matches!(sender, Sender::Player { uuid: u, .. } if u == uuid)
}

/// The permission a `/tab` sub-command needs before it runs (`SubCommand.permission`); None for
/// the ones that check inside.
fn node_of(sub: &str) -> Option<&'static str> {
    Some(match sub {
        "reload" => "tab.reload",
        "cpu" => "tab.cpu",
        "debug" => "tab.debug",
        "dump" => "tab.dump",
        "groups" => "tab.grouplist",
        "parse" => "tab.parse",
        "setcollision" => "tab.setcollision",
        _ => return None,
    })
}

/// The command line after `/tab ` (empty for `/tab`).
pub fn tab(e: &mut Engine, sender: Sender, args: &str) -> Output {
    run(e, sender, words(args))
}

fn run(e: &mut Engine, sender: Sender, a: Vec<String>) -> Output {
    let mut o = Output::default();
    let Some(first) = a.first() else {
        tab_help(e, sender, &mut o);
        return o;
    };
    let sub = first.to_lowercase();
    if !matches!(
        sub.as_str(),
        "bossbar"
            | "cpu"
            | "debug"
            | "dump"
            | "group"
            | "groups"
            | "mysql"
            | "nametag"
            | "parse"
            | "player"
            | "playeruuid"
            | "reload"
            | "setcollision"
            | "scoreboard"
    ) {
        tab_help(e, sender, &mut o);
        return o;
    }
    if let Some(node) = node_of(&sub) {
        if !has(sender, node) {
            no_permission(e, &mut o);
            return o;
        }
    }
    let rest = &a[1..];
    match sub.as_str() {
        "reload" => reload(e, &mut o),
        "parse" => parse(e, sender, rest, &mut o),
        "player" | "playeruuid" | "group" => property(e, sender, &sub, rest, &mut o),
        "groups" => {
            o.say("&3Configured groups:");
            let groups = e.props.groups.entries();
            o.say(format!("&9{}", groups.join(", &9")));
        }
        "debug" => o.say("&cThe debug command was removed in favor of a new \"/tab dump <player>\" command."),
        "dump" => dump(e, rest, &mut o),
        "cpu" => o.say("&cThis command is not part of this port."),
        "mysql" => mysql(e, sender, rest, &mut o),
        "scoreboard" => scoreboard(e, sender, rest, &mut o),
        "bossbar" => bossbar(e, sender, rest, &mut o),
        "nametag" => nametag(e, sender, rest, &mut o),
        "setcollision" => {
            if !e.cfg.teams.enabled {
                o.say(e.message(
                    "team-feature-required",
                    "This command requires scoreboard teams feature enabled",
                ));
            } else if rest.len() == 2 {
                if let Some(i) = e.find(&rest[0]) {
                    e.set_collision(i, rest[1].eq_ignore_ascii_case("true"));
                } else {
                    player_not_online(e, &mut o, &rest[0]);
                }
            } else {
                o.say(e.message(
                    "collision-command-usage",
                    "Usage: /tab setcollision <player> <true/false>",
                ));
            }
        }
        _ => {}
    }
    o
}

/// `/tab`, `/tab help` and any unknown word: the menu only for those with `tab.admin`.
fn tab_help(e: &Engine, sender: Sender, o: &mut Output) {
    if has(sender, "tab.admin") {
        o.say(format!("&3TAB v{TAB_VERSION}"));
        o.say(e.message(
            "help-menu",
            "&m                                                                                \n &8>> &3&l/tab reload\n      - &7Reloads plugin and config\n &8>> &3&l/tab &9group&3/&9player &3<name> &9<property> &3<value...>\n      - &7Do &8/tab group/player &7to show properties\n &8>> &3&l/tab parse <player> <placeholder> \n      - &7Test if a placeholder works\n &8>> &3&l/tab dump <player>\n      - &7Dumps information about player and server\n &8>> &3&l/tab cpu\n      - &7shows CPU usage of the plugin\n &8>> &3&l/tab group/player <name> remove\n      - &7Clears all data about player/group\n&m                                                                                ",
        ));
    }
}

/// The scoreboard toggle command (`toggle-command`, `/sb` by default): `/tab scoreboard toggle <args>`.
pub fn scoreboard_toggle(e: &mut Engine, sender: Sender, args: &str) -> Output {
    let mut a = vec!["scoreboard".to_owned(), "toggle".to_owned()];
    a.extend(words(args));
    run(e, sender, a)
}

/// The boss bar toggle command: `/tab bossbar toggle <args>`.
pub fn bossbar_toggle(e: &mut Engine, sender: Sender, args: &str) -> Output {
    let mut a = vec!["bossbar".to_owned(), "toggle".to_owned()];
    a.extend(words(args));
    run(e, sender, a)
}

/// `ReloadCommand`: TAB's answer is `reload-success`, or `reload-fail-file` naming a broken file.
fn reload(e: &mut Engine, o: &mut Output) {
    o.packets = e.reload();
    let broken = e.warnings.iter().find_map(|w| {
        let (path, _) = w.split_once(".yml: ")?;
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        Some(format!("{name}.yml"))
    });
    for w in &e.warnings {
        o.say(format!("&c[TAB] {w}"));
    }
    match broken {
        Some(file) => o.say(
            e.message(
                "reload-fail-file",
                "&3[TAB] &4Failed to reload, file %file% has broken syntax. Check console for more info.",
            )
            .replace("%file%", &file),
        ),
        None => o.say(e.message("reload-success", "&3[TAB] Successfully reloaded")),
    }
}

/// `ParseCommand`.
fn parse(e: &mut Engine, sender: Sender, a: &[String], o: &mut Output) {
    if a.len() < 2 {
        return o.say(e.message("parse-command-usage", "Usage: /tab parse <player> <placeholder>"));
    }
    let i = if a[0] == "me" {
        match sender_index(e, sender) {
            Some(i) => i,
            None => {
                return o.say("&cThe \"me\" argument instead of player name is only available in-game and parses the placeholder for player who ran the command. If you wish to use the parse command from the console, use name of an online player instead of \"me\".");
            }
        }
    } else {
        match e.find(&a[0]) {
            Some(i) => i,
            None => return player_not_online(e, o, &a[0]),
        }
    };
    let text = a[1..].join(" ");
    let name = e.tab_player(i).p.name.clone();
    let parsed = e.parse_for(i, &text);
    o.say_rich(vec![
        Seg::Lit("Replacing placeholder ".into(), "gold"),
        Seg::Lit(text, "yellow"),
        Seg::Lit(" for player ".into(), "gold"),
        Seg::Lit(name, "yellow"),
    ]);
    o.say_rich(vec![
        Seg::Lit("Colored output: ".into(), "dark_aqua"),
        Seg::Lit("\"".into(), "yellow"),
        Seg::Fmt(parsed.clone()),
        Seg::Lit("\"".into(), "yellow"),
    ]);
    o.say_rich(vec![
        Seg::Lit("Raw colors: ".into(), "dark_aqua"),
        Seg::Lit("\"".into(), "yellow"),
        Seg::Lit(parsed.replace('§', "&"), "white"),
        Seg::Lit("\"".into(), "yellow"),
    ]);
    o.say(format!("&3Output length: &e{} &3characters", parsed.encode_utf16().count()));
}

/// `DumpCommand` without the upload: the facts it collects about the player, printed here.
fn dump(e: &mut Engine, a: &[String], o: &mut Output) {
    let Some(name) = a.first() else {
        return o.say("&cUsage: /tab dump <player>");
    };
    let Some(i) = e.find(name) else {
        return player_not_online(e, o, name);
    };
    o.say("&eDump (uploading is not part of this port, the data is printed instead):");
    o.say(format!("&6Integrations: &f{}", {
        let n = integration::names();
        if n.is_empty() {
            "none (operators have all permissions)".to_owned()
        } else {
            n.join(", ")
        }
    }));
    o.say(format!(
        "&6Placeholder expansions: &f{}",
        crate::placeholders::identifiers().join(", ")
    ));
    let tp = e.tab_player(i).clone();
    o.say(format!(
        "&6Player: &f{}  &6group: &f{}  &6world: &f{}",
        tp.p.name, tp.group, tp.p.world
    ));
    if e.cfg.teams.enabled {
        o.say(format!(
            "&6Team name: &f{}",
            e.team_of(i).unwrap_or_default().replace('\0', "\\0")
        ));
        for (line, v) in e.sort_values(i) {
            o.say(format!("&6Sorting {line}: &f{v}"));
        }
    }
    for p in COMMAND_PROPERTIES {
        let (v, source) = e.property_of(i, p);
        o.say(format!("&6{p}: &r{v} &8(source: {source})"));
    }
}

/// `MySQLCommand`: this port has no MySQL storage, so the answer after the permission check is
/// TAB's "not enabled".
fn mysql(e: &mut Engine, sender: Sender, a: &[String], o: &mut Output) {
    let help = |e: &Engine, o: &mut Output| {
        o.say(e.message(
            "mysql-help-menu",
            "&6/tab mysql upload - uploads data from files to mysql\n&6/tab mysql download - downloads data from mysql to files",
        ))
    };
    let Some(first) = a.first() else {
        return help(e, o);
    };
    let node = if first.eq_ignore_ascii_case("download") {
        "tab.mysql.download"
    } else if first.eq_ignore_ascii_case("upload") {
        "tab.mysql.upload"
    } else {
        return help(e, o);
    };
    if has(sender, node) {
        o.say(e.message(
            "mysql-fail-not-enabled",
            "&cCannot download/upload data from/to MySQL, because it's disabled.",
        ));
    } else {
        no_permission(e, o);
    }
}

/// `PropertyCommand` and its three children (`player`, `playeruuid`, `group`).
fn property(e: &mut Engine, sender: Sender, kind: &str, a: &[String], o: &mut Output) {
    let group = kind == "group";
    if group && a.is_empty() {
        return property_help(o);
    }
    if group && a.len() == 1 {
        if has(sender, "tab.groupinfo") {
            group_info(e, &a[0], o);
        } else {
            no_permission(e, o);
        }
        return;
    }
    if !group && a.len() <= 1 {
        return property_help(o);
    }
    if kind == "playeruuid" {
        let Some(i) = e.find(&a[0]) else {
            return player_not_online(e, o, &a[0]);
        };
        if a[1].eq_ignore_ascii_case("remove") {
            if !has(sender, "tab.remove") {
                return no_permission(e, o);
            }
            let (uuid, pname) = {
                let t = e.tab_player(i);
                (t.p.uuid_string(), t.p.name.clone())
            };
            return remove_entry(e, kind, &uuid, &format!("{pname}({uuid})"), o);
        }
    } else if a[1].eq_ignore_ascii_case("remove") {
        if !has(sender, "tab.remove") {
            return no_permission(e, o);
        }
        return remove_entry(e, kind, &a[0], &a[0], o);
    }
    try_save(e, sender, kind, a, o);
}

fn remove_entry(e: &mut Engine, kind: &str, key: &str, shown: &str, o: &mut Output) {
    let group = kind == "group";
    let file = if group { &mut e.props.groups } else { &mut e.props.users };
    if let Some(why) = &file.broken {
        return o.say(format!("&c[TAB] Could not read the file ({why}), not changing it"));
    }
    file.remove_all(key);
    if let Err(err) = file.save() {
        o.say(format!("&c[TAB] Could not save: {err}"));
    }
    let (k, default, who) = if group {
        (
            "group-data-removed",
            "&3[TAB] All data has been successfully removed from group &e%group%",
            "%group%",
        )
    } else {
        (
            "user-data-removed",
            "&3[TAB] All data has been successfully removed from player &e%player%",
            "%player%",
        )
    };
    o.say(e.message(k, default).replace(who, shown));
}

/// `PropertyCommand.trySaveEntity` (args: name, property, value..., optionally `-w world` / `-s server`).
fn try_save(e: &mut Engine, sender: Sender, kind: &str, a: &[String], o: &mut Output) {
    let mut value = a[2..].join(" ");
    let mut world = None;
    let mut server = None;
    let flag = a[a.len() - 2].as_str();
    let last = a[a.len() - 1].as_str();
    if flag == "-w" || flag == "-s" {
        let target = last.to_owned();
        value = if value.starts_with(flag) {
            String::new()
        } else {
            value
                .len()
                .checked_sub(target.len() + 4)
                .and_then(|n| value.get(..n))
                .unwrap_or("")
                .to_owned()
        };
        if flag == "-w" {
            world = Some(target);
        } else {
            server = Some(target);
        }
    }
    if value.len() > 1
        && ((value.starts_with('"') && value.ends_with('"')) || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value = value[1..value.len() - 1].to_owned();
    }
    let property = a[1].to_lowercase();
    if !VALID_PROPERTIES.contains(&property.as_str()) {
        return property_help(o);
    }
    if !has(sender, &format!("tab.change.{property}")) {
        return no_permission(e, o);
    }
    save_entity(e, kind, &a[0], &property, &value, server.as_deref(), world.as_deref(), o);
}

#[allow(clippy::too_many_arguments)]
fn save_entity(
    e: &mut Engine,
    kind: &str,
    name: &str,
    property: &str,
    value: &str,
    server: Option<&str>,
    world: Option<&str>,
    o: &mut Output,
) {
    let group = kind == "group";
    // The file key and the name shown in the answer.
    let (key, shown) = if kind == "playeruuid" {
        let Some(i) = e.find(name) else {
            return player_not_online(e, o, name);
        };
        let uuid = e.tab_player(i).p.uuid_string();
        (uuid.clone(), format!("{name}({uuid})"))
    } else {
        (name.to_owned(), name.to_owned())
    };
    let removing = value.is_empty();
    let (k, default, who) = match (group, removing) {
        (true, false) => (
            "group-value-assigned",
            "&3[TAB] %property% '&r%value%&r&3' has been successfully assigned to group &e%group%",
            "%group%",
        ),
        (true, true) => (
            "group-value-removed",
            "&3[TAB] %property% has been successfully removed from group &e%group%",
            "%group%",
        ),
        (false, false) => (
            "user-value-assigned",
            "&3[TAB] %property% '&r%value%&r&3' has been successfully assigned to player &e%player%",
            "%player%",
        ),
        (false, true) => (
            "user-value-removed",
            "&3[TAB] %property% has been successfully removed from player &e%player%",
            "%player%",
        ),
    };
    o.say(
        e.message(k, default)
            .replace("%property%", property)
            .replace("%value%", value)
            .replace(who, &shown),
    );
    let file = if group { &mut e.props.groups } else { &mut e.props.users };
    if let Some(why) = &file.broken {
        return o.say(format!("&c[TAB] Could not read the file ({why}), not changing it"));
    }
    if !removing {
        if let Some(found) = file.get(&key, property, world, server) {
            if found.value == value {
                return;
            }
        }
    }
    file.set(&key, property, (!removing).then_some(value), world, server);
    if let Err(err) = file.save() {
        o.say(format!("&c[TAB] Could not save: {err}"));
    }
}

fn property_help(o: &mut Output) {
    o.say("&cSyntax&8: &3&l/tab &9group&3/&9player &3<name> &9<property> &3<value...>");
    o.say("&7Valid Properties are:");
    o.say(" - &9tabprefix&3/&9customtabname&3/&9tabsuffix");
    o.say(" - &9tagprefix&3/&9tagsuffix");
}

/// A YAML value the way Java's `toString` writes it (lists as `[a, b]`).
fn java_string(v: &crate::yaml::Yaml) -> String {
    use crate::yaml::Yaml;
    match v {
        Yaml::List(items) => format!("[{}]", items.iter().map(java_string).collect::<Vec<_>>().join(", ")),
        Yaml::Map(entries) => format!(
            "{{{}}}",
            entries
                .iter()
                .map(|(k, v)| format!("{k}={}", java_string(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Yaml::Null => "null".to_owned(),
        other => other.as_str().unwrap_or_default(),
    }
}

/// `GroupCommand.sendGroupInfo`.
fn group_info(e: &Engine, group: &str, o: &mut Output) {
    o.say(format!("&f=== Group &9{group}&f ==="));
    let file = &e.props.groups.root;
    for (k, v) in file.get(group).entries() {
        o.say(format!("  {k}: {}", java_string(v)));
    }
    for (section, header) in [("per-world", "&6World {}:&e"), ("per-server", "&3Server {}:&b")] {
        for (name, groups) in file.get(section).entries() {
            let settings = groups.get(group);
            if !matches!(settings, crate::yaml::Yaml::Map(_)) {
                continue;
            }
            o.say(header.replace("{}", name));
            for (k, v) in settings.entries() {
                o.say(format!("  {k}: {}", java_string(v)));
            }
        }
    }
}

/// The part of `[on|off|toggle] [player] [-s]` shared by the scoreboard and boss bar commands:
/// the target (the sender, or the named player with the `.other` node) and the `-s` flag.
fn toggle_target(
    e: &Engine,
    sender: Sender,
    a: &[String],
    other_node: &str,
    o: &mut Output,
) -> Option<(usize, bool)> {
    let silent = a.last().is_some_and(|s| s == "-s");
    let provided = a.len() - usize::from(silent);
    if provided > 0 {
        if !has(sender, other_node) {
            no_permission(e, o);
            return None;
        }
        match e.find(&a[0]) {
            Some(i) => Some((i, silent)),
            None => {
                player_not_online(e, o, &a[0]);
                None
            }
        }
    } else {
        match sender_index(e, sender) {
            Some(i) => Some((i, silent)),
            None => {
                only_from_game(e, o);
                None
            }
        }
    }
}

/// `scoreboard [on|off|toggle] [player] [-s]`, `show <name> [player]`, `announce <name> <seconds>`.
fn scoreboard(e: &mut Engine, sender: Sender, args: &[String], o: &mut Output) {
    let args: Vec<String> = if args.is_empty() {
        vec!["toggle".to_owned()]
    } else {
        args.to_vec()
    };
    let sub = args[0].to_lowercase();
    let node = match sub.as_str() {
        "announce" => "tab.announce.scoreboard",
        "show" => "tab.scoreboard.show",
        "toggle" | "on" | "off" => "tab.scoreboard.toggle",
        _ => {
            return o.say(e.message(
                "scoreboard-help-menu",
                "/tab scoreboard [on/off/toggle] [player] [options]\n/tab scoreboard show <name> [player]\n/tab scoreboard announce <name> <length>",
            ));
        }
    };
    if !has(sender, node) {
        return no_permission(e, o);
    }
    let a = &args[1..];
    if !e.cfg.scoreboard.enabled {
        return o.say(e.message(
            "scoreboard-feature-not-enabled",
            "&4This command requires the scoreboard feature to be enabled.",
        ));
    }
    let board_name = |e: &Engine, n: &str| e.cfg.scoreboard.boards.iter().find(|b| b.name == n).map(|b| b.name.clone());
    let not_found = |e: &Engine, o: &mut Output, n: &str| {
        o.say(
            e.message("scoreboard-not-found", "&cNo scoreboard found with the name \"%name%\"")
                .replace("%name%", n),
        )
    };
    match sub.as_str() {
        "announce" => {
            if a.len() != 2 {
                return o.say(e.message(
                    "scoreboard-announce-command-usage",
                    "Usage: /tab scoreboard announce <scoreboard name> <length>",
                ));
            }
            let Ok(secs) = a[1].parse::<i32>() else {
                return invalid_number(e, o, &a[1]);
            };
            match board_name(e, &a[0]) {
                Some(name) => e.announce_board(name, i64::from(secs)),
                None => not_found(e, o, &a[0]),
            }
        }
        "show" => {
            if a.is_empty() || a.len() > 2 {
                return o.say(e.message(
                    "scoreboard-show-usage",
                    "Usage: /tab scoreboard show <scoreboard> [player]",
                ));
            }
            let Some(name) = board_name(e, &a[0]) else {
                return not_found(e, o, &a[0]);
            };
            let target = if a.len() == 1 {
                match sender_index(e, sender) {
                    Some(i) => i,
                    None => return only_from_game(e, o),
                }
            } else {
                if !has(sender, "tab.scoreboard.show.other") {
                    return no_permission(e, o);
                }
                match e.find(&a[1]) {
                    Some(i) => i,
                    None => return player_not_online(e, o, &a[1]),
                }
            };
            e.show_board(target, Some(name));
        }
        _ => {
            let Some((i, silent)) = toggle_target(e, sender, a, "tab.scoreboard.toggle.other", o) else {
                return;
            };
            let before = e.scoreboard_shown(i);
            let mode = match sub.as_str() {
                "on" => Some(true),
                "off" => Some(false),
                _ => None,
            };
            if mode == Some(before) {
                return;
            }
            let shown = e.set_scoreboard(i, mode, false);
            if !silent {
                let uuid = e.tab_player(i).p.uuid;
                let msg = if shown {
                    e.message("scoreboard-toggle-on", "&2Scoreboard enabled")
                } else {
                    e.message("scoreboard-toggle-off", "&7Scoreboard disabled")
                };
                tell(e, sender, o, uuid, msg);
            }
        }
    }
}

/// `bossbar [on|off|toggle] [player] [-s]`, `send <player> <bar> <seconds>`, `announce <bar> <seconds>`.
fn bossbar(e: &mut Engine, sender: Sender, args: &[String], o: &mut Output) {
    let args: Vec<String> = if args.is_empty() {
        vec!["toggle".to_owned()]
    } else {
        args.to_vec()
    };
    let sub = args[0].to_lowercase();
    // TAB's own `BossBarSendCommand` is registered with the scoreboard `show` node.
    let node = match sub.as_str() {
        "announce" => "tab.announce.bar",
        "send" => "tab.scoreboard.show",
        "toggle" | "on" | "off" => "tab.bossbar.toggle",
        _ => {
            return o.say(e.message(
                "bossbar-help-menu",
                "/tab bossbar [on/off/toggle] [player] [options]\n/tab bossbar send <name> [player]\n/tab bossbar announce <name> <length>",
            ));
        }
    };
    if !has(sender, node) {
        return no_permission(e, o);
    }
    let a = &args[1..];
    if !e.cfg.bossbar.enabled {
        return o.say(e.message(
            "bossbar-feature-not-enabled",
            "&cThis command requires the bossbar feature to be enabled.",
        ));
    }
    let bar_of = |e: &Engine, n: &str| e.cfg.bossbar.bars.iter().find(|b| b.name == n).cloned();
    let not_found = |e: &Engine, o: &mut Output, n: &str| {
        o.say(
            e.message("bossbar-not-found", "&cNo bossbar found with the name \"%name%\"")
                .replace("%name%", n),
        )
    };
    let not_announcement = |e: &Engine, o: &mut Output| {
        o.say(e.message(
            "bossbar-not-marked-as-announcement",
            "&cThis bossbar is not marked as an announcement bar and is therefore already displayed permanently (if display condition is met)",
        ))
    };
    match sub.as_str() {
        "announce" => {
            if a.len() != 2 {
                return o.say(e.message(
                    "bossbar-announce-command-usage",
                    "Usage: /tab bossbar announce <bar name> <length>",
                ));
            }
            let Ok(secs) = a[1].parse::<i32>() else {
                return invalid_number(e, o, &a[1]);
            };
            let Some(bar) = bar_of(e, &a[0]) else {
                return not_found(e, o, &a[0]);
            };
            if !bar.announcement {
                return not_announcement(e, o);
            }
            if e.bar_announced(&bar.name) {
                return o.say(e.message("bossbar-already-announced", "&cThis bossbar is already being announced"));
            }
            e.announce_bar(bar.name.clone(), i64::from(secs));
            o.say(
                e.message(
                    "bossbar-announcement-success",
                    "&aAnnouncing bossbar &6%bossbar% &afor %length% seconds.",
                )
                .replace("%bossbar%", &bar.name)
                .replace("%length%", &secs.to_string()),
            );
        }
        "send" => {
            if a.len() != 3 {
                return o.say(e.message(
                    "send-bar-command-usage",
                    "Usage: /tab send bar <player> <bar name> <length>",
                ));
            }
            let Some(i) = e.find(&a[0]) else {
                return player_not_online(e, o, &a[0]);
            };
            let Ok(secs) = a[2].parse::<i32>() else {
                // TAB reports the bar name here, not the length.
                return invalid_number(e, o, &a[1]);
            };
            let Some(bar) = bar_of(e, &a[1]) else {
                return not_found(e, o, &a[1]);
            };
            if !bar.announcement {
                return not_announcement(e, o);
            }
            let pname = e.tab_player(i).p.name.clone();
            e.send_bar(i, bar.name.clone(), i64::from(secs));
            o.say(
                e.message(
                    "bossbar-send-success",
                    "&aSending bossbar &6%bossbar% &ato player &6%player% &afor %length% seconds.",
                )
                .replace("%player%", &pname)
                .replace("%bossbar%", &bar.name)
                .replace("%length%", &secs.to_string()),
            );
        }
        _ => {
            let Some((i, silent)) = toggle_target(e, sender, a, "tab.bossbar.toggle.other", o) else {
                return;
            };
            let before = e.bossbar_shown(i);
            let mode = match sub.as_str() {
                "on" => Some(true),
                "off" => Some(false),
                _ => None,
            };
            if mode == Some(before) {
                return;
            }
            let shown = e.set_bossbar(i, mode);
            if !silent {
                let uuid = e.tab_player(i).p.uuid;
                let msg = if shown {
                    e.message("bossbar-toggle-on", "&2Bossbar is now visible")
                } else {
                    e.message("bossbar-toggle-off", "&7Bossbar is no longer visible. Magic!")
                };
                tell(e, sender, o, uuid, msg);
            }
        }
    }
}

/// `NameTagCommand`: `<show|opaque|hide|toggle> [player] [viewer] [-s]` and
/// `<showview|opaqueview|hideview|toggleview> [viewer] [-s]`.
fn nametag(e: &mut Engine, sender: Sender, a: &[String], o: &mut Output) {
    let no_arg = |e: &Engine, o: &mut Output| {
        o.say(e.message(
            "nametag.no-arg-from-console",
            "&cYou need to specify player if running this command from the console",
        ))
    };
    let help = |e: &Engine, o: &mut Output| {
        o.say(e.message(
            "nametag.help-menu",
            "/tab nametag <show/opaque/hide/toggle> [player] [viewer] [-s] - Toggles nametag of specified player\n/tab nametag <showview/opaqueview/hideview/toggleview> [viewer] [-s] - Toggles nametag VIEW of specified player",
        ))
    };
    if a.len() == 1 && matches!(sender, Sender::Console) {
        return no_arg(e, o);
    }
    if a.is_empty() || a.len() > 4 {
        return help(e, o);
    }
    if !e.cfg.teams.enabled {
        return o.say(e.message(
            "nametag.feature-not-enabled",
            "&cThis command requires nametag feature to be enabled.",
        ));
    }
    let action = a[0].to_lowercase();
    let args = &a[1..];
    let silent = args.last().is_some_and(|s| s == "-s");
    let eff = if silent { &args[..args.len() - 1] } else { args };
    let unsupported = "&cNametag opacity (line-of-sight checks) is not part of this port.";
    match action.as_str() {
        "show" | "opaque" | "hide" | "toggle" => {
            let player = if let Some(n) = eff.first() {
                e.find(n)
            } else {
                sender_index(e, sender)
            };
            let Some(player) = player else {
                return match eff.first() {
                    None => no_arg(e, o),
                    Some(n) => player_not_online(e, o, n),
                };
            };
            let viewer = if let Some(n) = eff.get(1) {
                match e.find(n) {
                    Some(v) => Some(v),
                    None => return player_not_online(e, o, n),
                }
            } else {
                None
            };
            // TAB compares the sender with the *viewer* (null when none is named), so a player who
            // names no viewer needs the `.other` node.
            let same = match (sender, viewer) {
                (Sender::Console, None) => true,
                (Sender::Player { uuid, .. }, Some(v)) => e.tab_player(v).p.uuid == uuid,
                _ => false,
            };
            let node = if same {
                "tab.nametag.visibility"
            } else {
                "tab.nametag.visibility.other"
            };
            if !has(sender, node) {
                return no_permission(e, o);
            }
            if action == "opaque" {
                return o.say(unsupported);
            }
            let hidden = e.nametag_target(player, viewer, &action);
            if !silent {
                let uuid = e.tab_player(player).p.uuid;
                let msg = if hidden {
                    e.message("nametag.player-hidden", "&aYour nametag was hidden")
                } else {
                    e.message("nametag.player-shown", "&aYour nametag was shown")
                };
                tell(e, sender, o, uuid, msg);
            }
        }
        "showview" | "opaqueview" | "hideview" | "toggleview" => {
            let viewer = if let Some(n) = eff.first() {
                e.find(n)
            } else {
                sender_index(e, sender)
            };
            let Some(viewer) = viewer else {
                return match eff.first() {
                    None => no_arg(e, o),
                    Some(n) => player_not_online(e, o, n),
                };
            };
            let uuid = e.tab_player(viewer).p.uuid;
            let node = if is_same(sender, uuid) {
                "tab.nametag.view"
            } else {
                "tab.nametag.view.other"
            };
            if !has(sender, node) {
                return no_permission(e, o);
            }
            let hidden_now = e.nametag_view_is_hidden(viewer);
            let want_hidden = match action.as_str() {
                "showview" => false,
                "hideview" => true,
                "toggleview" => !hidden_now,
                _ => return o.say(unsupported),
            };
            // Nothing happens (and nothing is said) when it already is that way.
            if want_hidden == hidden_now {
                return;
            }
            e.set_nametag_view_hidden(viewer, Some(want_hidden));
            if !silent {
                let msg = if want_hidden {
                    e.message("nametag.view-hidden", "&aNametags of all players were hidden to you")
                } else {
                    e.message("nametag.view-shown", "&aNametags of all players were shown to you")
                };
                tell(e, sender, o, uuid, msg);
            }
        }
        _ => help(e, o),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::engine::PlayerIn;
    use crate::placeholders::Server;
    use crate::properties::{Properties, PropertyFile};
    use crate::yaml::{self, Yaml};

    fn engine() -> Engine {
        let cfg = Config::from_yaml(&yaml::parse(include_str!("../testdata/config.yml")).unwrap());
        let props = Properties {
            groups: PropertyFile::from_str(include_str!("../testdata/groups.yml"), "group").unwrap(),
            users: PropertyFile::from_str("", "user").unwrap(),
        };
        let messages = yaml::parse(include_str!("../defaults/messages.yml")).unwrap();
        let mut e = Engine::new(
            "/nonexistent".into(),
            cfg,
            props,
            Default::default(),
            messages,
            Yaml::empty_map(),
            Vec::new(),
        );
        let p = PlayerIn {
            uuid: 1,
            name: "Steve".into(),
            dimension: "minecraft:overworld".into(),
            ping: 9,
            ..PlayerIn::default()
        };
        e.tick(
            0,
            Server {
                level_name: "world".into(),
                ..Server::default()
            },
            vec![p],
        );
        e
    }

    #[test]
    fn property_set_and_remove() {
        let mut e = engine();
        let o = tab(&mut e, Sender::Console, "player Steve tabprefix \"&c[Admin] \"");
        assert_eq!(
            o.replies,
            vec!["&3[TAB] tabprefix '&r&c[Admin] &r&3' has been successfully assigned to player &eSteve"]
        );
        assert_eq!(
            e.props.users.get("Steve", "tabprefix", None, None).unwrap().value,
            "&c[Admin] "
        );
        tab(&mut e, Sender::Console, "player Steve tagprefix &a -w world_nether");
        assert_eq!(
            e.props
                .users
                .root
                .at(&["per-world", "world_nether", "Steve", "tagprefix"])
                .as_str()
                .unwrap(),
            "&a"
        );
        let o = tab(&mut e, Sender::Console, "player Steve tabprefix");
        assert!(
            o.replies[0].contains("has been successfully removed from player"),
            "{:?}",
            o.replies
        );
        assert!(e.props.users.root.get("Steve").get("tabprefix").is_null());
    }

    #[test]
    fn permissions_and_parse() {
        let mut e = engine();
        let guest = Sender::Player { uuid: 1, op_level: 0 };
        let o = tab(&mut e, guest, "reload");
        assert!(o.replies[0].starts_with("&cI'm sorry"));
        let op = Sender::Player { uuid: 1, op_level: 2 };
        let o = tab(&mut e, op, "parse me %player% has %ping% ms");
        assert_eq!(o.replies[0], "Replacing placeholder %player% has %ping% ms for player Steve");
        assert_eq!(o.replies[1], "Colored output: \"Steve has 9 ms\"");
        assert_eq!(o.replies[2], "Raw colors: \"Steve has 9 ms\"");
        assert_eq!(o.replies[3], "&3Output length: &e14 &3characters");
        assert_eq!(o.rich.len(), 3);
        let o = tab(&mut e, op, "parse Nobody x");
        assert!(o.replies[0].contains("Nobody"));
    }

    #[test]
    fn disabled_features_say_so() {
        let mut e = engine();
        let o = scoreboard_toggle(&mut e, Sender::Player { uuid: 1, op_level: 4 }, "");
        assert_eq!(
            o.replies,
            vec!["&4This command requires the scoreboard feature to be enabled."]
        );
        // The answer goes to the viewer, not to the console.
        let o = tab(&mut e, Sender::Console, "nametag hideview Steve");
        assert!(o.replies.is_empty());
        assert_eq!(o.packets.len(), 1);
        assert_eq!(o.packets[0].to, 1);
        // Already hidden: nothing happens.
        let o = tab(&mut e, Sender::Console, "nametag hideview Steve");
        assert!(o.packets.is_empty());
    }
}
