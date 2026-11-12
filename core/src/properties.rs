//! `groups.yml` and `users.yml`: a player's `tabprefix`, `customtabname`, `tabsuffix`, `tagprefix`,
//! `tagsuffix` (and any other property), looked up as TAB does
//! (`YamlPropertyConfigurationFile.getProperty`, Apache-2.0, see NOTICE):
//! per-world entry, per-world `_DEFAULT_`, per-server entry, per-server `_DEFAULT_`, the entry,
//! `_DEFAULT_`; users by name, then by UUID, then the player's group in `groups.yml`.
//! Sections and keys are matched without case. `/tab player|group` edits write the same files.

use crate::yaml::{self, Yaml};
use std::path::{Path, PathBuf};

pub const DEFAULT_GROUP: &str = "_DEFAULT_";
pub const VALID_PROPERTIES: [&str; 5] = ["tabprefix", "customtabname", "tabsuffix", "tagprefix", "tagsuffix"];

#[derive(Clone, Debug, Default)]
pub struct PropertyFile {
    pub root: Yaml,
    pub path: Option<PathBuf>,
    /// "group" or "user", for `/tab debug`'s source text.
    pub category: &'static str,
    /// Set when the file exists but could not be read: it is then never saved, so an edit
    /// cannot replace its content with an empty tree (TAB marks such a file broken too).
    pub broken: Option<String>,
}

/// A found value and where it came from (`group=Player, world=world1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub value: String,
    pub source: String,
}

impl PropertyFile {
    pub fn load(path: &Path, category: &'static str) -> Result<PropertyFile, String> {
        let root = match std::fs::read_to_string(path) {
            Ok(text) => yaml::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Yaml::empty_map(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(PropertyFile {
            root,
            path: Some(path.to_owned()),
            category,
            broken: None,
        })
    }

    pub fn from_str(text: &str, category: &'static str) -> Result<PropertyFile, String> {
        Ok(PropertyFile {
            root: yaml::parse(text).map_err(|e| e.to_string())?,
            path: None,
            category,
            broken: None,
        })
    }

    pub fn save(&self) -> Result<(), String> {
        if let Some(why) = &self.broken {
            return Err(format!("the file could not be read ({why}), not saving"));
        }
        let Some(path) = &self.path else { return Ok(()) };
        let tmp = path.with_extension("yml.tmp");
        std::fs::write(&tmp, yaml::write(&self.root)).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    /// The world group `world` belongs to: a key of `per-world` that names it, alone or in a
    /// `;`-separated list (TAB's world groups), or None.
    fn world_group(&self, world: Option<&str>) -> Option<String> {
        let world = world?;
        self.root
            .get("per-world")
            .keys()
            .into_iter()
            .find(|k| group_matches(k, world))
    }

    fn server_group(&self, server: Option<&str>) -> Option<String> {
        let server = server?;
        self.root
            .get("per-server")
            .keys()
            .into_iter()
            .find(|k| group_matches(k, server))
    }

    pub fn get(&self, name: &str, property: &str, world: Option<&str>, server: Option<&str>) -> Option<Found> {
        let cat = self.category;
        let try_at = |node: &Yaml, entry: &str, source: String| -> Option<Found> {
            let v = node.get(entry).get(property);
            let value = match v {
                // TAB joins a list-valued property with line breaks.
                Yaml::List(items) => Some(items.iter().filter_map(Yaml::as_str).collect::<Vec<_>>().join("\n")),
                other => other.as_str(),
            };
            value.map(|value| Found { value, source })
        };
        if let Some(wg) = self.world_group(world) {
            let node = self.root.at(&["per-world", &wg]);
            let w = world.unwrap_or_default();
            if let Some(f) = try_at(node, name, format!("{cat}={name}, world={w}")) {
                return Some(f);
            }
            if let Some(f) = try_at(node, DEFAULT_GROUP, format!("{cat}={DEFAULT_GROUP}, world={w}")) {
                return Some(f);
            }
        }
        if let Some(sg) = self.server_group(server) {
            let node = self.root.at(&["per-server", &sg]);
            let s = server.unwrap_or_default();
            if let Some(f) = try_at(node, name, format!("{cat}={name}, server={s}")) {
                return Some(f);
            }
            if let Some(f) = try_at(node, DEFAULT_GROUP, format!("{cat}={DEFAULT_GROUP}, server={s}")) {
                return Some(f);
            }
        }
        if let Some(f) = try_at(&self.root, name, format!("{cat}={name}")) {
            return Some(f);
        }
        try_at(&self.root, DEFAULT_GROUP, format!("{cat}={DEFAULT_GROUP}"))
    }

    /// Sets (or with None removes) a property of `name`, globally or for one world/server.
    pub fn set(&mut self, name: &str, property: &str, value: Option<&str>, world: Option<&str>, server: Option<&str>) {
        let section = if let Some(w) = world {
            self.root.section_mut("per-world").section_mut(w).section_mut(name)
        } else if let Some(s) = server {
            self.root.section_mut("per-server").section_mut(s).section_mut(name)
        } else {
            self.root.section_mut(name)
        };
        match value {
            // A value with line breaks is stored as a list, as TAB's `fromString` does.
            Some(v) if v.contains('\n') => {
                section.set(property, Yaml::List(v.split('\n').map(|l| Yaml::Str(l.to_owned())).collect()))
            }
            Some(v) => section.set(property, Yaml::Str(v.to_owned())),
            None => {
                section.remove(property);
            }
        }
        // Leave no empty entries behind.
        let empty = |y: &Yaml| matches!(y, Yaml::Map(m) if m.is_empty());
        if world.is_none() && server.is_none() && empty(self.root.get(name)) {
            self.root.remove(name);
        }
    }

    /// Removes everything about `name`, also per world and per server.
    pub fn remove_all(&mut self, name: &str) {
        self.root.remove(name);
        for section in ["per-world", "per-server"] {
            if let Some(Yaml::Map(groups)) = self.root.get_mut(section) {
                for (_, g) in groups.iter_mut() {
                    g.remove(name);
                }
            }
        }
    }

    /// The names that have any entry (for `/tab groups`).
    pub fn entries(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .root
            .keys()
            .into_iter()
            .filter(|k| !k.eq_ignore_ascii_case("per-world") && !k.eq_ignore_ascii_case("per-server"))
            .collect();
        for section in ["per-world", "per-server"] {
            for (_, g) in self.root.get(section).entries() {
                for k in g.keys() {
                    if !out.iter().any(|o| o.eq_ignore_ascii_case(&k)) {
                        out.push(k);
                    }
                }
            }
        }
        out
    }
}

/// Both files, with TAB's order: user by name, user by UUID, group.
#[derive(Clone, Debug, Default)]
pub struct Properties {
    pub groups: PropertyFile,
    pub users: PropertyFile,
}

impl Properties {
    pub fn lookup(
        &self,
        name: &str,
        uuid: &str,
        group: &str,
        property: &str,
        world: Option<&str>,
        server: Option<&str>,
    ) -> Option<Found> {
        self.users
            .get(name, property, world, server)
            .or_else(|| self.users.get(uuid, property, world, server))
            .or_else(|| self.groups.get(group, property, world, server))
    }
}

/// TAB's `Configs.getGroup` match: `key` is one name or a `;`-separated list; an entry ending
/// in `*` matches by prefix, one starting with `*` by suffix, any other one whole; no case.
fn group_matches(key: &str, name: &str) -> bool {
    let name = name.to_lowercase();
    key.split(';').map(|p| p.trim().to_lowercase()).any(|p| {
        if p.len() > 1 && p.ends_with('*') {
            name.starts_with(&p[..p.len() - 1])
        } else if p.len() > 1 && p.starts_with('*') {
            name.ends_with(&p[1..])
        } else {
            p == name
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_world_groups() {
        let f = PropertyFile::from_str(
            "per-world:\n  'world*':\n    Steve:\n      tabprefix: 'W '\n  '*_end;lobby':\n    Steve:\n      tabprefix: 'E '\n",
            "user",
        )
        .unwrap();
        assert_eq!(f.get("Steve", "tabprefix", Some("world_nether"), None).unwrap().value, "W ");
        assert_eq!(f.get("Steve", "tabprefix", Some("World"), None).unwrap().value, "W ");
        assert_eq!(f.get("Steve", "tabprefix", Some("the_end"), None).unwrap().value, "E ");
        assert_eq!(f.get("Steve", "tabprefix", Some("LOBBY"), None).unwrap().value, "E ");
        assert!(f.get("Steve", "tabprefix", Some("other"), None).is_none());
    }

    #[test]
    fn list_valued_property() {
        let mut f = PropertyFile::from_str("Steve:\n  tabsuffix:\n  - 'a'\n  - 'b'\n", "user").unwrap();
        assert_eq!(f.get("Steve", "tabsuffix", None, None).unwrap().value, "a\nb");
        f.set("Alex", "tabsuffix", Some("x\ny"), None, None);
        assert!(matches!(f.root.at(&["Alex", "tabsuffix"]), Yaml::List(l) if l.len() == 2));
        assert_eq!(f.get("Alex", "tabsuffix", None, None).unwrap().value, "x\ny");
    }

    #[test]
    fn broken_file_is_never_saved() {
        let dir = std::env::temp_dir().join(format!("horizon-tab-broken-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let original = "admin:\n  tabprefix: 'unterminated\nother:\n  tabprefix: 'x'\n";
        std::fs::write(dir.join("groups.yml"), original).unwrap();
        let mut e = crate::engine::Engine::load(&dir);
        assert!(e.props.groups.broken.is_some());
        e.props.groups.set("admin", "tabprefix", Some("&c"), None, None);
        assert!(e.props.groups.save().is_err());
        assert_eq!(std::fs::read_to_string(dir.join("groups.yml")).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn props() -> Properties {
        Properties {
            groups: PropertyFile::from_str(include_str!("../testdata/groups.yml"), "group").unwrap(),
            users: PropertyFile::from_str(
                "Steve:\n  tabprefix: '&c&l[OP] &f'\n01234567-89ab-cdef-0123-456789abcdef:\n  tabsuffix: ' &bVerified'\nper-world:\n  nether:\n    Steve:\n      tabprefix: '&cHot '\n",
                "user",
            )
            .unwrap(),
        }
    }

    #[test]
    fn order_of_lookup() {
        let p = props();
        let uuid = "01234567-89ab-cdef-0123-456789abcdef";
        let f = p
            .lookup("Steve", uuid, "default", "tabprefix", Some("world"), None)
            .unwrap();
        assert_eq!(f.value, "&c&l[OP] &f");
        assert_eq!(f.source, "user=Steve");
        let f = p
            .lookup("Steve", uuid, "default", "tabprefix", Some("nether"), None)
            .unwrap();
        assert_eq!(f.value, "&cHot ");
        let f = p.lookup("Steve", uuid, "default", "tabsuffix", None, None).unwrap();
        assert_eq!(f.value, " &bVerified");
        // Group, case-insensitive, then _DEFAULT_.
        let f = p.lookup("Alex", "x", "player", "tabprefix", None, None).unwrap();
        assert_eq!(f.value, "&0&l[&7&lPlayer&0&l] &3");
        let f = p.lookup("Alex", "x", "player", "customtabname", None, None).unwrap();
        assert_eq!(
            (f.value.as_str(), f.source.as_str()),
            ("%displayname%", "group=_DEFAULT_")
        );
        let f = p
            .lookup("Alex", "x", "Owner", "tagprefix", Some("world1"), None)
            .unwrap();
        assert_eq!(f.value, "&2&lOwner &a");
    }

    #[test]
    fn edit_and_write() {
        let mut p = props();
        p.users.set("Alex", "tagprefix", Some("&a"), None, None);
        p.users.set("Steve", "tabprefix", None, None, None);
        assert!(p.users.get("Steve", "tabprefix", None, None).is_none());
        let text = crate::yaml::write(&p.users.root);
        let back = PropertyFile::from_str(&text, "user").unwrap();
        assert_eq!(back.get("Alex", "tagprefix", None, None).unwrap().value, "&a");
        p.users.remove_all("Steve");
        assert!(p.users.get("Steve", "tabprefix", Some("nether"), None).is_none());
    }
}
