//! `per-world-playerlist` (TAB's `PerWorldPlayerListConfiguration` and the Bukkit
//! `PerWorldPlayerList`): a player sees only the players of the worlds in their own world group,
//! unless they bypass (`tab.bypass`, when `allow-bypass-permission` is on) or stand in an ignored
//! world. TAB hides the others with `hidePlayer`; the port hides their entity from the viewer and
//! takes them out of the viewer's tab list (`Out::EntityVisible`).

use crate::horizon_tab::yaml::Yaml;

#[derive(Clone, Debug, PartialEq)]
pub struct PerWorld {
    pub allow_bypass: bool,
    pub ignored_worlds: Vec<String>,
    /// Group name -> world patterns, in file order.
    pub groups: Vec<(String, Vec<String>)>,
}

impl PerWorld {
    /// Reads the section; a world listed in two groups is warned about and stays in the first.
    pub fn from_yaml(y: &Yaml, warnings: &mut Vec<String>) -> PerWorld {
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        let mut taken: Vec<(String, String)> = Vec::new();
        for (name, v) in y.get("shared-playerlist-world-groups").entries() {
            let worlds = v.str_list();
            for w in &worlds {
                if let Some((_, first)) = taken.iter().find(|(world, _)| world == w) {
                    warnings.push(format!(
                        "World \"{w}\" is defined in per world playerlist groups \"{first}\" and \"{name}\", but it can only be a part of one group."
                    ));
                } else {
                    taken.push((w.clone(), name.clone()));
                }
            }
            groups.push((name.clone(), worlds));
        }
        let ignored = if y.get("ignore-effect-in-worlds").is_null() {
            vec!["ignoredworld".to_owned(), "build".to_owned()]
        } else {
            y.get("ignore-effect-in-worlds").str_list()
        };
        PerWorld {
            allow_bypass: y.get("allow-bypass-permission").bool_or(false),
            ignored_worlds: ignored,
            groups,
        }
    }

    /// `computeWorldGroup`: the group whose pattern matches, else the world's own default group.
    pub fn group_of(&self, world: &str) -> String {
        for (group, worlds) in &self.groups {
            if worlds.iter().any(|p| matches_pattern(world, p)) {
                return group.clone();
            }
        }
        format!("{world}-default")
    }

    /// `shouldSee` for the world part: the viewer sees the target's list entry and entity.
    pub fn sees(&self, viewer_world: &str, target_world: &str, viewer_bypasses: bool) -> bool {
        if (self.allow_bypass && viewer_bypasses) || self.ignored_worlds.iter().any(|w| w == viewer_world) {
            return true;
        }
        self.group_of(viewer_world) == self.group_of(target_world)
    }
}

/// `DataManager.matchesPattern`: exact, `prefix*`, `*suffix` or `regex:<pattern>`. A regex that does
/// not parse is compared as a literal, as TAB does.
pub fn matches_pattern(name: &str, pattern: &str) -> bool {
    if let Some(re) = pattern.strip_prefix("regex:") {
        return match Regex::compile(re) {
            Some(r) => r.matches_whole(name),
            None => name == pattern,
        };
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else {
        name == pattern
    }
}

// ---- a small regular expression matcher (the crate set has no regex) ------------------------
// Supported: literals, `.`, classes `[a-z0-9_]` / `[^...]`, escapes `\d \w \s \D \W \S \.` and so on,
// the quantifiers `* + ?` and top-level `|`. Anything else (groups, counted repeats) makes the
// pattern not compile, which falls back to the literal comparison.

#[derive(Clone, Debug)]
enum Atom {
    Any,
    Lit(char),
    Class(Vec<(char, char)>, bool),
}

#[derive(Clone, Debug)]
struct Piece {
    atom: Atom,
    min: usize,
    max: usize,
}

struct Regex {
    alternatives: Vec<Vec<Piece>>,
}

fn class_of(c: char) -> Option<Atom> {
    Some(match c {
        'd' => Atom::Class(vec![('0', '9')], false),
        'D' => Atom::Class(vec![('0', '9')], true),
        'w' => Atom::Class(vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], false),
        'W' => Atom::Class(vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], true),
        's' => Atom::Class(vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')], false),
        'S' => Atom::Class(vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')], true),
        _ => return None,
    })
}

impl Regex {
    fn compile(src: &str) -> Option<Regex> {
        let chars: Vec<char> = src.chars().collect();
        let mut alternatives = vec![Vec::new()];
        let mut i = 0;
        while i < chars.len() {
            let atom = match chars[i] {
                '|' => {
                    alternatives.push(Vec::new());
                    i += 1;
                    continue;
                }
                '(' | ')' | '{' | '}' | '^' | '$' => return None,
                '*' | '+' | '?' => return None,
                '.' => {
                    i += 1;
                    Atom::Any
                }
                '\\' => {
                    let c = *chars.get(i + 1)?;
                    i += 2;
                    class_of(c).unwrap_or(Atom::Lit(c))
                }
                '[' => {
                    i += 1;
                    let negate = chars.get(i) == Some(&'^');
                    if negate {
                        i += 1;
                    }
                    let mut ranges = Vec::new();
                    loop {
                        let c = *chars.get(i)?;
                        if c == ']' && !ranges.is_empty() {
                            i += 1;
                            break;
                        }
                        let c = if c == '\\' {
                            i += 1;
                            *chars.get(i)?
                        } else {
                            c
                        };
                        if chars.get(i + 1) == Some(&'-') && chars.get(i + 2).is_some_and(|e| *e != ']') {
                            ranges.push((c, chars[i + 2]));
                            i += 3;
                        } else {
                            ranges.push((c, c));
                            i += 1;
                        }
                    }
                    Atom::Class(ranges, negate)
                }
                c => {
                    i += 1;
                    Atom::Lit(c)
                }
            };
            let (min, max) = match chars.get(i) {
                Some('*') => (0, usize::MAX),
                Some('+') => (1, usize::MAX),
                Some('?') => (0, 1),
                _ => (1, 1),
            };
            if (min, max) != (1, 1) {
                i += 1;
            }
            alternatives.last_mut()?.push(Piece { atom, min, max });
        }
        Some(Regex { alternatives })
    }

    fn matches_whole(&self, text: &str) -> bool {
        let t: Vec<char> = text.chars().collect();
        self.alternatives.iter().any(|alt| Self::run(alt, &t))
    }

    fn atom_matches(atom: &Atom, c: char) -> bool {
        match atom {
            Atom::Any => true,
            Atom::Lit(l) => *l == c,
            Atom::Class(ranges, negate) => ranges.iter().any(|(a, b)| *a <= c && c <= *b) != *negate,
        }
    }

    fn run(pieces: &[Piece], text: &[char]) -> bool {
        let Some((first, rest)) = pieces.split_first() else {
            return text.is_empty();
        };
        let mut n = 0;
        while n < text.len() && n < first.max && Self::atom_matches(&first.atom, text[n]) {
            n += 1;
        }
        // Greedy with backtracking.
        let mut take = n;
        loop {
            if take >= first.min && Self::run(rest, &text[take..]) {
                return true;
            }
            if take == 0 || take <= first.min {
                return false;
            }
            take -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizon_tab::yaml;

    fn cfg() -> PerWorld {
        let y = yaml::parse(
            "enabled: true\nallow-bypass-permission: true\nignore-effect-in-worlds:\n  - build\nshared-playerlist-world-groups:\n  lobby:\n    - lobby1\n    - lobby2\n  games:\n    - game*\n    - lobby2\n",
        )
        .unwrap();
        let mut w = Vec::new();
        let c = PerWorld::from_yaml(&y, &mut w);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("\"lobby2\"") && w[0].contains("\"lobby\"") && w[0].contains("\"games\""));
        c
    }

    #[test]
    fn worlds_in_one_group_see_each_other() {
        let c = cfg();
        assert_eq!(c.group_of("lobby1"), "lobby");
        assert_eq!(c.group_of("game7"), "games");
        assert_eq!(c.group_of("nether"), "nether-default");
        assert!(c.sees("lobby1", "lobby2", false));
        assert!(!c.sees("lobby1", "game7", false));
        assert!(!c.sees("nether", "world", false));
        assert!(c.sees("world", "world", false));
    }

    #[test]
    fn bypass_and_ignored_worlds() {
        let c = cfg();
        assert!(c.sees("lobby1", "game7", true));
        assert!(c.sees("build", "game7", false));
        let mut off = c.clone();
        off.allow_bypass = false;
        assert!(!off.sees("lobby1", "game7", true));
    }

    #[test]
    fn defaults_when_the_keys_are_missing() {
        let mut w = Vec::new();
        let c = PerWorld::from_yaml(&yaml::parse("enabled: true\n").unwrap(), &mut w);
        assert!(!c.allow_bypass);
        assert_eq!(c.ignored_worlds, vec!["ignoredworld", "build"]);
        assert!(c.groups.is_empty());
    }

    #[test]
    fn patterns() {
        assert!(matches_pattern("lobby", "lobby"));
        assert!(matches_pattern("lobby-2", "lobby*"));
        assert!(matches_pattern("my_lobby", "*lobby"));
        assert!(!matches_pattern("lobby2", "lobby"));
        assert!(matches_pattern("lobby-12", "regex:lobby-[0-9]+"));
        assert!(!matches_pattern("lobby-", "regex:lobby-[0-9]+"));
        assert!(!matches_pattern("lobby-12x", "regex:lobby-[0-9]+"));
        assert!(matches_pattern("pvp", "regex:pvp|arena"));
        // A group is not compiled: the pattern is compared as a literal, which is false here.
        assert!(!matches_pattern("world_nether", "regex:world(_nether)?"));
        assert!(matches_pattern("regex:world(_nether)?", "regex:world(_nether)?"));
    }
}
