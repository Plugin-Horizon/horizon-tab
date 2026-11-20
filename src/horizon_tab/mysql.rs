//! TAB's MySQL storage (the `mysql` section of `config.yml`; `MySQL`, `MySQLConfiguration`,
//! `MySQLGroupConfiguration`, `MySQLUserConfiguration` and `MySQLCommand`, Apache-2.0, see NOTICE).
//! Groups and users live in the tables `tab_groups` and `tab_users`, with TAB's columns.

use crate::horizon_tab::yaml::Yaml;
use std::fmt;

/// The `mysql` section, read as `MySQLConfiguration.fromSection` reads it. The defaults are TAB's
/// own (a placeholder login); the section is off unless `enabled` is true.
#[derive(Clone, PartialEq, Eq)]
pub struct MysqlConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub use_ssl: bool,
}

impl MysqlConfig {
    pub fn from_yaml(y: &Yaml) -> MysqlConfig {
        MysqlConfig {
            enabled: y.get("enabled").bool_or(false),
            host: y.get("host").str_or("127.0.0.1"),
            port: y.get("port").i64_or(3306).clamp(0, u16::MAX as i64) as u16,
            database: y.get("database").str_or("tab"),
            username: y.get("username").str_or("user"),
            password: y.get("password").str_or("password"),
            use_ssl: y.get("useSSL").bool_or(true),
        }
    }
}

impl fmt::Debug for MysqlConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The password never reaches a log or a debug print.
        f.debug_struct("MysqlConfig")
            .field("enabled", &self.enabled)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("use_ssl", &self.use_ssl)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizon_tab::yaml;

    #[test]
    fn defaults_are_tabs_own() {
        let c = MysqlConfig::from_yaml(&Yaml::empty_map());
        assert!(!c.enabled);
        assert_eq!(c.host, "127.0.0.1");
        assert_eq!(c.port, 3306);
        assert_eq!(c.database, "tab");
        assert_eq!(c.username, "user");
        assert_eq!(c.password, "password");
        assert!(c.use_ssl);
    }

    #[test]
    fn reads_the_section() {
        let y = yaml::parse(
            "mysql:\n  enabled: true\n  host: db.internal\n  port: 3307\n  database: tabdb\n  username: tabuser\n  password: s3cret-test\n  useSSL: false\n",
        )
        .unwrap();
        let c = MysqlConfig::from_yaml(y.get("mysql"));
        assert!(c.enabled);
        assert_eq!(c.host, "db.internal");
        assert_eq!(c.port, 3307);
        assert_eq!(c.database, "tabdb");
        assert_eq!(c.username, "tabuser");
        assert_eq!(c.password, "s3cret-test");
        assert!(!c.use_ssl);
    }

    #[test]
    fn debug_print_hides_the_password() {
        let y = yaml::parse("mysql:\n  password: s3cret-test\n").unwrap();
        let text = format!("{:?}", MysqlConfig::from_yaml(y.get("mysql")));
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("s3cret-test"));
    }
}
