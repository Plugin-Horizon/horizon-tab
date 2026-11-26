//! TAB's `config.yml`, read as the Java plugin writes it (config-version 7, TAB 5.x/6.x layout).
//! Only the keys this mod acts on are modelled; [`Config::unsupported`] lists switched-on features
//! it does not implement, for a warning at start.

use crate::horizon_tab::mysql::MysqlConfig;
use crate::horizon_tab::yaml::Yaml;
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Design {
    pub name: String,
    pub display_condition: Option<String>,
    pub header: Vec<String>,
    pub footer: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeaderFooter {
    pub enabled: bool,
    pub disable_condition: Option<String>,
    pub designs: Vec<Design>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TablistNames {
    pub enabled: bool,
    pub disable_condition: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Teams {
    pub enabled: bool,
    pub enable_collision: bool,
    pub invisible_nametags: bool,
    pub sorting_types: Vec<String>,
    pub case_sensitive_sorting: bool,
    pub can_see_friendly_invisibles: bool,
    pub disable_condition: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListObjective {
    pub enabled: bool,
    pub value: String,
    pub fancy_value: String,
    pub title: String,
    pub hearts: bool,
    pub disable_condition: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BelowName {
    pub enabled: bool,
    pub value: String,
    pub fancy_value: String,
    pub fancy_value_default: String,
    pub title: String,
    pub disable_condition: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bar {
    pub name: String,
    pub style: String,
    pub color: String,
    pub progress: String,
    pub text: String,
    pub display_condition: Option<String>,
    pub announcement: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BossBars {
    pub enabled: bool,
    pub toggle_command: String,
    pub remember_toggle_choice: bool,
    pub hidden_by_default: bool,
    pub bars: Vec<Bar>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    pub name: String,
    pub title: String,
    pub display_condition: Option<String>,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scoreboards {
    pub enabled: bool,
    pub toggle_command: String,
    pub remember_toggle_choice: bool,
    pub hidden_by_default: bool,
    pub delay_on_join_ms: i64,
    pub boards: Vec<Board>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConditionDef {
    pub name: String,
    pub conditions: Vec<String>,
    /// AND (true) or OR (false).
    pub and: bool,
    pub yes: String,
    pub no: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub header_footer: HeaderFooter,
    pub tablist_names: TablistNames,
    pub teams: Teams,
    pub list_objective: ListObjective,
    pub belowname: BelowName,
    pub prevent_spectator_effect: bool,
    pub bossbar: BossBars,
    pub scoreboard: Scoreboards,
    pub ping_spoof: Option<i32>,
    pub date_format: String,
    pub time_format: String,
    /// `placeholders.locale`, a language tag for month and day names.
    pub locale: String,
    pub time_offset_hours: f64,
    pub register_tab_expansion: bool,
    /// `placeholder-output-replacements`: placeholder -> (output -> replacement), in order.
    pub replacements: Vec<(String, Vec<(String, String)>)>,
    pub conditions: Vec<ConditionDef>,
    pub default_refresh_ms: i64,
    pub refresh_ms: HashMap<String, i64>,
    pub assign_groups_by_permissions: bool,
    pub primary_group_finding_list: Vec<String>,
    pub permission_refresh_ms: i64,
    pub minimessage: bool,
    pub debug: bool,
    pub mysql: MysqlConfig,
    pub unsupported: Vec<String>,
    /// `per-world-playerlist` when enabled.
    pub per_world: Option<crate::horizon_tab::perworld::PerWorld>,
    /// `global-playerlist` when enabled.
    pub global: Option<crate::horizon_tab::proxy::GlobalCfg>,
    /// `proxy-support` when enabled.
    pub proxy_support: Option<ProxySupportCfg>,
    /// `layout` when enabled.
    pub layout: Option<crate::horizon_tab::layout::LayoutCfg>,
    /// Warnings the feature sections gave while reading (invalid slots, duplicate worlds...).
    pub config_warnings: Vec<String>,
    /// Proxy-only keys that are switched on (`proxy-support.enabled`, `use-*`): read and ignored on a
    /// backend, so no warning, only a note at start.
    pub proxy_only_on: Vec<String>,
}

/// Keys of TAB's file that only a proxy reads. The port reads them and does nothing with them.
pub const PROXY_ONLY_KEYS: &[&str] = &["use-bukkit-permissions-manager", "use-online-uuid-in-tablist"];

/// `proxy-support` (TAB's `ProxySupportConfiguration`): the instances' own channel.
#[derive(Clone, Debug, PartialEq)]
pub struct ProxySupportCfg {
    /// `PLUGIN`, `REDIS` or `RABBITMQ`.
    pub kind: String,
    /// `TAB_4_<channel-name-suffix>`.
    pub channel: String,
    pub plugin_name: String,
    /// `proxy-support.redis.url`.
    pub redis_url: String,
}

/// Key paths (`*` = any key) of the files TAB keeps that the port reads. `check_original` compares the
/// original files against this list. Keep it in step with `Config::from_yaml`, `properties.rs` and `tabph.rs`.
pub const KEYS_READ: &[&str] = &[
    "header-footer.enabled",
    "header-footer.designs.*.display-condition",
    "header-footer.designs.*.header",
    "header-footer.designs.*.footer",
    "tablist-name-formatting.enabled",
    "tablist-name-formatting.disable-condition",
    "scoreboard-teams.enabled",
    "scoreboard-teams.enable-collision",
    "scoreboard-teams.invisible-nametags",
    "scoreboard-teams.sorting-types",
    "scoreboard-teams.case-sensitive-sorting",
    "scoreboard-teams.can-see-friendly-invisibles",
    "scoreboard-teams.disable-condition",
    "playerlist-objective.enabled",
    "playerlist-objective.value",
    "playerlist-objective.fancy-value",
    "playerlist-objective.title",
    "playerlist-objective.render-type",
    "playerlist-objective.disable-condition",
    "belowname-objective.enabled",
    "belowname-objective.value",
    "belowname-objective.fancy-value",
    "belowname-objective.fancy-value-default",
    "belowname-objective.title",
    "belowname-objective.disable-condition",
    "prevent-spectator-effect.enabled",
    "bossbar.enabled",
    "bossbar.toggle-command",
    "bossbar.remember-toggle-choice",
    "bossbar.hidden-by-default",
    "bossbar.bars.*.style",
    "bossbar.bars.*.color",
    "bossbar.bars.*.progress",
    "bossbar.bars.*.text",
    "bossbar.bars.*.display-condition",
    "bossbar.bars.*.announcement-bar",
    "scoreboard.enabled",
    "scoreboard.toggle-command",
    "scoreboard.remember-toggle-choice",
    "scoreboard.hidden-by-default",
    "scoreboard.delay-on-join-milliseconds",
    "scoreboard.scoreboards.*.title",
    "scoreboard.scoreboards.*.display-condition",
    "scoreboard.scoreboards.*.lines",
    "ping-spoof.enabled",
    "ping-spoof.value",
    "placeholders.date-format",
    "placeholders.time-format",
    "placeholders.time-offset",
    "placeholders.register-tab-expansion",
    "placeholder-output-replacements.*",
    "conditions.*.type",
    "conditions.*.conditions",
    "conditions.*.true",
    "conditions.*.yes",
    "conditions.*.false",
    "conditions.*.no",
    "placeholder-refresh-intervals.*",
    "assign-groups-by-permissions",
    "primary-group-finding-list",
    "permission-refresh-interval",
    "debug",
    "components.minimessage-support",
    "placeholders.locale",
    "layout.enabled",
    "layout.direction",
    "layout.default-skin",
    "layout.default-skins.*.skin",
    "layout.default-skins.*.slots",
    "layout.enable-remaining-players-text",
    "layout.remaining-players-text",
    "layout.empty-slot-ping-value",
    "layout.layouts.*.display-condition",
    "layout.layouts.*.default-skin",
    "layout.layouts.*.slot-count",
    "layout.layouts.*.fixed-slots",
    "layout.layouts.*.groups.*.display-condition",
    "layout.layouts.*.groups.*.slots",
    "global-playerlist.enabled",
    "per-world-playerlist.enabled",
    "per-world-playerlist.allow-bypass-permission",
    "per-world-playerlist.ignore-effect-in-worlds",
    "per-world-playerlist.shared-playerlist-world-groups.*",
    "mysql.enabled",
    "mysql.host",
    "mysql.port",
    "mysql.database",
    "mysql.username",
    "mysql.password",
    "mysql.useSSL",
    "proxy-support.enabled",
    "proxy-support.channel-name-suffix",
    "proxy-support.type",
    "proxy-support.plugin.name",
    "proxy-support.redis.url",
    "proxy-support.rabbitmq.exchange",
    "proxy-support.rabbitmq.url",
    "global-playerlist.display-others-as-spectators",
    "global-playerlist.display-vanished-players-as-spectators",
    "global-playerlist.isolate-unlisted-servers",
    "global-playerlist.update-latency",
    "global-playerlist.spy-servers",
    "global-playerlist.server-groups.*",
    "use-bukkit-permissions-manager",
    "use-online-uuid-in-tablist",
    // groups.yml and users.yml (properties.rs, VALID_PROPERTIES)
    "*.tabprefix",
    "*.customtabname",
    "*.tabsuffix",
    "*.tagprefix",
    "*.tagsuffix",
    "per-world.*.*",
];

impl Default for Config {
    fn default() -> Self {
        Config::from_yaml(&Yaml::empty_map())
    }
}

fn opt_str(y: &Yaml) -> Option<String> {
    y.as_str().filter(|s| !s.trim().is_empty())
}

impl Config {
    pub fn from_yaml(y: &Yaml) -> Config {
        let hf = y.get("header-footer");
        let mut designs: Vec<Design> = hf
            .get("designs")
            .entries()
            .iter()
            .map(|(name, d)| Design {
                name: name.clone(),
                display_condition: opt_str(d.get("display-condition")),
                header: d.get("header").str_list(),
                footer: d.get("footer").str_list(),
            })
            .collect();
        // The older layout: header/footer directly under header-footer.
        if designs.is_empty() && (!hf.get("header").is_null() || !hf.get("footer").is_null()) {
            designs.push(Design {
                name: "default".into(),
                display_condition: None,
                header: hf.get("header").str_list(),
                footer: hf.get("footer").str_list(),
            });
        }
        let st = y.get("scoreboard-teams");
        let po = y.get("playerlist-objective");
        let bn = y.get("belowname-objective");
        let bb = y.get("bossbar");
        let sb = y.get("scoreboard");
        let ph = y.get("placeholders");
        let refresh = y.get("placeholder-refresh-intervals");
        let mut refresh_ms = HashMap::new();
        let mut default_refresh_ms = 500;
        for (k, v) in refresh.entries() {
            if k.eq_ignore_ascii_case("default-refresh-interval") {
                default_refresh_ms = v.i64_or(500);
            } else if let Some(ms) = v.as_i64() {
                refresh_ms.insert(k.clone(), ms);
            }
        }
        let conditions = y
            .get("conditions")
            .entries()
            .iter()
            .map(|(name, c)| {
                let ty = c.get("type").str_or("AND");
                ConditionDef {
                    name: name.clone(),
                    conditions: c.get("conditions").str_list(),
                    and: !ty.eq_ignore_ascii_case("OR"),
                    yes: c
                        .get("true")
                        .as_str()
                        .or_else(|| c.get("yes").as_str())
                        .unwrap_or_else(|| "true".into()),
                    no: c
                        .get("false")
                        .as_str()
                        .or_else(|| c.get("no").as_str())
                        .unwrap_or_else(|| "false".into()),
                }
            })
            .collect();
        let replacements = y
            .get("placeholder-output-replacements")
            .entries()
            .iter()
            .map(|(ph, map)| {
                (
                    ph.clone(),
                    map.entries()
                        .iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default()))
                        .collect(),
                )
            })
            .collect();
        let mut cfg = Config {
            header_footer: HeaderFooter {
                enabled: hf.get("enabled").bool_or(false),
                disable_condition: opt_str(hf.get("disable-condition")),
                designs,
            },
            tablist_names: TablistNames {
                enabled: y.at(&["tablist-name-formatting", "enabled"]).bool_or(true),
                disable_condition: opt_str(y.at(&["tablist-name-formatting", "disable-condition"])),
            },
            teams: Teams {
                enabled: st.get("enabled").bool_or(true),
                enable_collision: st.get("enable-collision").bool_or(true),
                invisible_nametags: st.get("invisible-nametags").bool_or(false),
                sorting_types: st.get("sorting-types").str_list(),
                case_sensitive_sorting: st.get("case-sensitive-sorting").bool_or(true),
                can_see_friendly_invisibles: st.get("can-see-friendly-invisibles").bool_or(false),
                disable_condition: opt_str(st.get("disable-condition")),
            },
            list_objective: ListObjective {
                enabled: po.get("enabled").bool_or(false),
                value: po.get("value").str_or("%ping%"),
                fancy_value: po.get("fancy-value").str_or(""),
                title: po.get("title").str_or("TAB"),
                hearts: po.get("render-type").str_or("INTEGER").eq_ignore_ascii_case("HEARTS"),
                disable_condition: opt_str(po.get("disable-condition")),
            },
            belowname: BelowName {
                enabled: bn.get("enabled").bool_or(false),
                value: bn.get("value").str_or("%health%"),
                fancy_value: bn.get("fancy-value").str_or(""),
                fancy_value_default: bn.get("fancy-value-default").str_or(""),
                title: bn.get("title").str_or(""),
                disable_condition: opt_str(bn.get("disable-condition")),
            },
            prevent_spectator_effect: y.at(&["prevent-spectator-effect", "enabled"]).bool_or(false),
            bossbar: BossBars {
                enabled: bb.get("enabled").bool_or(false),
                toggle_command: bb.get("toggle-command").str_or("/bossbar"),
                remember_toggle_choice: bb.get("remember-toggle-choice").bool_or(false),
                hidden_by_default: bb.get("hidden-by-default").bool_or(false),
                bars: bb
                    .get("bars")
                    .entries()
                    .iter()
                    .map(|(name, b)| Bar {
                        name: name.clone(),
                        style: b.get("style").str_or("PROGRESS"),
                        color: b.get("color").str_or("WHITE"),
                        progress: b.get("progress").str_or("100"),
                        text: b.get("text").str_or(""),
                        display_condition: opt_str(b.get("display-condition")),
                        announcement: b.get("announcement-bar").bool_or(false),
                    })
                    .collect(),
            },
            scoreboard: Scoreboards {
                enabled: sb.get("enabled").bool_or(false),
                toggle_command: sb.get("toggle-command").str_or("/sb"),
                remember_toggle_choice: sb.get("remember-toggle-choice").bool_or(false),
                hidden_by_default: sb.get("hidden-by-default").bool_or(false),
                delay_on_join_ms: sb.get("delay-on-join-milliseconds").i64_or(0),
                boards: sb
                    .get("scoreboards")
                    .entries()
                    .iter()
                    .map(|(name, b)| Board {
                        name: name.clone(),
                        title: b.get("title").str_or(""),
                        display_condition: opt_str(b.get("display-condition")),
                        lines: b.get("lines").str_list(),
                    })
                    .collect(),
            },
            ping_spoof: y
                .at(&["ping-spoof", "enabled"])
                .bool_or(false)
                .then(|| y.at(&["ping-spoof", "value"]).i64_or(0) as i32),
            date_format: ph.get("date-format").str_or("dd.MM.yyyy"),
            time_format: ph.get("time-format").str_or("[HH:mm:ss / h:mm a]"),
            locale: ph.get("locale").str_or("en-US"),
            time_offset_hours: ph.get("time-offset").as_f64().unwrap_or(0.0),
            register_tab_expansion: ph.get("register-tab-expansion").bool_or(false),
            replacements,
            conditions,
            default_refresh_ms,
            refresh_ms,
            assign_groups_by_permissions: y.get("assign-groups-by-permissions").bool_or(false),
            primary_group_finding_list: y.get("primary-group-finding-list").str_list(),
            permission_refresh_ms: y.get("permission-refresh-interval").i64_or(1000),
            minimessage: y.at(&["components", "minimessage-support"]).bool_or(true),
            debug: y.get("debug").bool_or(false),
            mysql: MysqlConfig::from_yaml(y.get("mysql")),
            unsupported: Vec::new(),
            per_world: None,
            global: None,
            proxy_support: None,
            layout: None,
            config_warnings: Vec::new(),
            proxy_only_on: Vec::new(),
        };
        for key in PROXY_ONLY_KEYS {
            let path: Vec<&str> = key.split('.').collect();
            let on = match y.at(&path) {
                Yaml::Bool(b) => *b,
                _ => false,
            };
            if on {
                cfg.proxy_only_on.push((*key).to_owned());
            }
        }
        if y.at(&["layout", "enabled"]).bool_or(false) {
            let mut w = Vec::new();
            cfg.layout = Some(crate::horizon_tab::layout::LayoutCfg::from_yaml(y.get("layout"), &mut w));
            cfg.config_warnings.extend(w);
        }
        if y.at(&["per-world-playerlist", "enabled"]).bool_or(false) {
            let mut w = Vec::new();
            cfg.per_world = Some(crate::horizon_tab::perworld::PerWorld::from_yaml(y.get("per-world-playerlist"), &mut w));
            cfg.config_warnings.extend(w);
        }
        if y.at(&["global-playerlist", "enabled"]).bool_or(false) {
            let mut w = Vec::new();
            cfg.global = Some(crate::horizon_tab::proxy::GlobalCfg::from_yaml(y.get("global-playerlist"), &mut w));
            cfg.config_warnings.extend(w);
        }
        if y.at(&["proxy-support", "enabled"]).bool_or(false) {
            let ps = y.get("proxy-support");
            let kind = ps.get("type").str_or("PLUGIN").to_ascii_uppercase();
            let kind = if ["PLUGIN", "REDIS", "RABBITMQ"].contains(&kind.as_str()) {
                kind
            } else {
                cfg.config_warnings.push(format!(
                    "Proxy support type \"{kind}\" is not supported. Supported types are: PLUGIN, REDIS, RABBITMQ. Defaulting to PLUGIN."
                ));
                "PLUGIN".to_owned()
            };
            cfg.proxy_support = Some(ProxySupportCfg {
                kind,
                channel: crate::horizon_tab::proxy::channel_name(&ps.get("channel-name-suffix").str_or("custom")),
                plugin_name: ps.at(&["plugin", "name"]).str_or("RedisBungee"),
                redis_url: ps.at(&["redis", "url"]).str_or("redis://:password@localhost:6379/0"),
            });
        }
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_our_shape() {
        let y = crate::horizon_tab::yaml::parse(include_str!("../../data/tab/testdata/config.yml")).unwrap();
        let c = Config::from_yaml(&y);
        assert!(!c.header_footer.enabled);
        assert_eq!(c.header_footer.designs.len(), 2);
        assert_eq!(
            c.header_footer.designs[0].display_condition.as_deref(),
            Some("%world%=test")
        );
        assert!(c.tablist_names.enabled);
        assert_eq!(
            c.tablist_names.disable_condition.as_deref(),
            Some("%world%=disabledworld")
        );
        assert!(c.teams.enabled);
        assert_eq!(
            c.teams.sorting_types,
            vec!["GROUPS:owner,admin,op,vip,guest,default", "PLACEHOLDER_A_TO_Z:%player%"]
        );
        assert!(c.list_objective.enabled && !c.list_objective.hearts);
        assert_eq!(c.list_objective.fancy_value, "&7Ping: %ping%");
        assert_eq!(c.scoreboard.boards.len(), 2);
        assert_eq!(
            c.scoreboard.boards[0].display_condition.as_deref(),
            Some("%player-version-id%>=765;%bedrock%=false")
        );
        assert_eq!(c.bossbar.bars[0].color, "%animation:barcolors%");
        assert_eq!(c.conditions[0].name, "nick");
        assert_eq!(c.conditions[0].no, "~%essentials_nickname%");
        assert_eq!(c.default_refresh_ms, 500);
        assert_eq!(c.refresh_ms.get("%player_health%"), Some(&200));
        assert_eq!(
            c.replacements[0].1,
            vec![
                ("yes".to_owned(), "&7| Vanished".to_owned()),
                ("no".to_owned(), String::new())
            ]
        );
        assert!(c.unsupported.is_empty());
        assert!(!c.mysql.enabled);
        assert_eq!(c.mysql.port, 3306);
        assert!(c.minimessage);
    }

    #[test]
    fn proxy_only_keys_are_read_and_ignored() {
        let y = crate::horizon_tab::yaml::parse("proxy-support:\n  enabled: true\nuse-online-uuid-in-tablist: false\nuse-bukkit-permissions-manager: true\n")
            .unwrap();
        let c = Config::from_yaml(&y);
        assert_eq!(c.proxy_only_on, vec!["use-bukkit-permissions-manager"]);
        // Inert on a backend: no "not implemented" warning.
        assert!(c.unsupported.is_empty());
        // The shipped test config has proxy-support on: it is read and used, not ignored.
        let c = Config::from_yaml(&crate::horizon_tab::yaml::parse(include_str!("../../data/tab/testdata/config.yml")).unwrap());
        let ps = c.proxy_support.expect("proxy-support is on in the test config");
        assert_eq!((ps.kind.as_str(), ps.channel.as_str(), ps.plugin_name.as_str()), ("PLUGIN", "TAB_4_custom", "RedisBungee"));
        assert!(c.global.is_none(), "global-playerlist is off there");
        assert!(c.unsupported.iter().all(|u| u != "proxy-support"));
    }

    #[test]
    fn global_playerlist_and_proxy_support_are_read() {
        let y = crate::horizon_tab::yaml::parse(
            "global-playerlist:\n  enabled: true\n  isolate-unlisted-servers: true\nproxy-support:\n  enabled: true\n  type: nonsense\n  channel-name-suffix: net\n",
        )
        .unwrap();
        let c = Config::from_yaml(&y);
        assert!(c.global.as_ref().is_some_and(|g| g.isolate_unlisted));
        assert_eq!(c.proxy_support.as_ref().map(|p| (p.kind.as_str(), p.channel.as_str())), Some(("PLUGIN", "TAB_4_net")));
        assert!(c.config_warnings.iter().any(|w| w.contains("\"NONSENSE\" is not supported")));
    }
}
