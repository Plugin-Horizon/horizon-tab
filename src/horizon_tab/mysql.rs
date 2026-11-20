//! TAB's MySQL storage (the `mysql` section of `config.yml`; `MySQL`, `MySQLConfiguration`,
//! `MySQLGroupConfiguration`, `MySQLUserConfiguration` and `MySQLCommand`, Apache-2.0, see NOTICE).
//! Groups and users live in the tables `tab_groups` and `tab_users`, with TAB's columns.

use crate::horizon_tab::properties::{PropertyFile, Properties};
use crate::horizon_tab::yaml::Yaml;
use mysql::prelude::Queryable;
use mysql::{OptsBuilder, Params, Pool, SslOpts, Value};
use std::fmt;
use std::sync::Arc;

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

pub const GROUPS_TABLE: &str = "tab_groups";
pub const USERS_TABLE: &str = "tab_users";

/// TAB's `CREATE TABLE` statements (`MySQLGroupConfiguration`, `MySQLUserConfiguration`).
pub const CREATE_GROUPS: &str = "create table if not exists tab_groups (`group` varchar(64), `property` varchar(16), `value` varchar(1024), world varchar(64), server varchar(64))";
pub const CREATE_USERS: &str = "create table if not exists tab_users (`user` varchar(64), `property` varchar(16), `value` varchar(1024), world varchar(64), server varchar(64))";

/// One statement with its positional parameters (`None` is SQL NULL).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stmt {
    pub sql: String,
    pub params: Vec<Option<String>>,
}

/// One stored setting: TAB's row (name, property, value, world, server). A NULL value
/// stands for no value, as TAB's `put(property, null)` does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub property: String,
    pub value: Option<String>,
    pub world: Option<String>,
    pub server: Option<String>,
}

/// The table and its name column for a category (`group` or `user`).
pub fn table_of(category: &str) -> (&'static str, &'static str) {
    if category == "group" {
        (GROUPS_TABLE, "group")
    } else {
        (USERS_TABLE, "user")
    }
}

/// Reads every row of the category's table.
pub fn select_all(category: &str) -> Stmt {
    let (table, column) = table_of(category);
    Stmt {
        sql: format!("SELECT `{column}`, `property`, `value`, `world`, `server` FROM `{table}`"),
        params: Vec::new(),
    }
}

/// Reads the rows of one name (`MySQLUserConfiguration.load` reads by name, then by UUID).
pub fn select_name(category: &str, name: &str) -> Stmt {
    let (table, column) = table_of(category);
    Stmt {
        sql: format!("SELECT `{column}`, `property`, `value`, `world`, `server` FROM `{table}` WHERE `{column}` = ?"),
        params: vec![Some(name.to_owned())],
    }
}

/// A scope condition: `= ?` with its value, or `IS NULL` (TAB's `querySymbol`; the JDBC driver
/// sends `is NULL` for a null parameter, which a server-side prepared statement cannot bind).
fn scope(column: &str, value: Option<&str>, params: &mut Vec<Option<String>>) -> String {
    match value {
        Some(v) => {
            params.push(Some(v.to_owned()));
            format!("{column} = ?")
        }
        None => format!("{column} IS NULL"),
    }
}

/// `setProperty` of the MySQL configurations: the row of that property in that scope goes, then
/// the new value is inserted (none for a removal).
pub fn set_statements(
    category: &str,
    name: &str,
    property: &str,
    value: Option<&str>,
    world: Option<&str>,
    server: Option<&str>,
) -> Vec<Stmt> {
    let (table, column) = table_of(category);
    let mut params = vec![Some(name.to_owned()), Some(property.to_owned())];
    let world_cond = scope("world", world, &mut params);
    let server_cond = scope("server", server, &mut params);
    let mut out = vec![Stmt {
        sql: format!("delete from `{table}` where `{column}` = ? and `property` = ? and {world_cond} and {server_cond}"),
        params,
    }];
    if let Some(v) = value {
        out.push(Stmt {
            sql: format!("insert into `{table}` (`{column}`, `property`, `value`, `world`, `server`) values (?, ?, ?, ?, ?)"),
            params: vec![
                Some(name.to_owned()),
                Some(property.to_owned()),
                Some(v.to_owned()),
                world.map(str::to_owned),
                server.map(str::to_owned),
            ],
        });
    }
    out
}

/// `remove` of the MySQL configurations: every row of the name.
pub fn remove_statement(category: &str, name: &str) -> Stmt {
    let (table, column) = table_of(category);
    Stmt {
        sql: format!("delete from `{table}` where `{column}` = ?"),
        params: vec![Some(name.to_owned())],
    }
}

/// The first step of `/tab mysql upload`.
pub fn delete_all(category: &str) -> Stmt {
    let (table, _) = table_of(category);
    Stmt {
        sql: format!("DELETE FROM {table}"),
        params: Vec::new(),
    }
}

/// A row as the table returns it (name, property, value, world, server). A row without a name or
/// a property is skipped.
pub fn row_of(cols: (Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)) -> Option<Row> {
    let (name, property, value, world, server) = cols;
    Some(Row {
        name: name?,
        property: property?,
        value,
        world,
        server,
    })
}

/// The text TAB stores for a value: scalars as Java prints them, a list joined with line breaks.
/// Null and mappings have none.
fn value_text(value: &Yaml) -> Option<String> {
    match value {
        Yaml::List(items) => Some(items.iter().filter_map(Yaml::as_str).collect::<Vec<_>>().join("\n")),
        Yaml::Map(_) | Yaml::Null => None,
        scalar => scalar.as_str(),
    }
}

fn push_settings(out: &mut Vec<Row>, name: &str, section: &Yaml, world: Option<&str>, server: Option<&str>) {
    for (property, value) in section.entries() {
        if let Some(text) = value_text(value) {
            out.push(Row {
                name: name.to_owned(),
                property: property.clone(),
                value: Some(text),
                world: world.map(str::to_owned),
                server: server.map(str::to_owned),
            });
        }
    }
}

/// The rows `/tab mysql upload` writes for a file: per name, the global settings, then the
/// per-world ones, then the per-server ones (`MySQLCommand.upload`).
pub fn rows_of(file: &PropertyFile) -> Vec<Row> {
    let mut out = Vec::new();
    for name in file.entries() {
        push_settings(&mut out, &name, file.root.get(&name), None, None);
        for (world, sections) in file.root.get("per-world").entries() {
            push_settings(&mut out, &name, sections.get(&name), Some(world.as_str()), None);
        }
        for (server, sections) in file.root.get("per-server").entries() {
            push_settings(&mut out, &name, sections.get(&name), None, Some(server.as_str()));
        }
    }
    out
}

/// `MySQLGroupConfiguration`'s startup warning for an empty `tab_groups`.
const EMPTY_WARNING: &str = "[MySQL] Using MySQL to store groups and users, however, the database is empty. You can get started by uploading existing data in files using \"/tab mysql upload\". Further modifications can be done using property commands (/tab <group / player> <name> <property> <value...>).";

/// A pool of connections to TAB's database (the `mysql` crate, as husksync and luckperms use it).
pub struct Db {
    pool: Pool,
}

impl fmt::Debug for Db {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Db")
    }
}

/// The columns of a row read by `Db::rows`, in the order of `select_all` and `select_name`.
type Columns = (Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);

/// The `mysql` crate's errors do not carry the password.
fn db_err(e: impl fmt::Display) -> String {
    format!("database: {e}")
}

fn params_of(params: &[Option<String>]) -> Params {
    if params.is_empty() {
        return Params::Empty;
    }
    Params::Positional(
        params
            .iter()
            .map(|p| match p {
                Some(s) => Value::Bytes(s.clone().into_bytes()),
                None => Value::NULL,
            })
            .collect(),
    )
}

impl Db {
    /// Opens the pool (the connection is made now) and creates both tables, as TAB's constructors do.
    pub fn connect(cfg: &MysqlConfig) -> Result<Db, String> {
        let mut opts = OptsBuilder::new()
            .ip_or_hostname(Some(cfg.host.clone()))
            .tcp_port(cfg.port)
            .user(Some(cfg.username.clone()))
            .pass(Some(cfg.password.clone()))
            .db_name(Some(cfg.database.clone()));
        if cfg.use_ssl {
            opts = opts.ssl_opts(Some(SslOpts::default()));
        }
        let pool = Pool::new(opts).map_err(db_err)?;
        let db = Db { pool };
        for sql in [CREATE_GROUPS, CREATE_USERS] {
            db.exec(&Stmt {
                sql: sql.to_owned(),
                params: Vec::new(),
            })?;
        }
        Ok(db)
    }

    pub fn exec(&self, stmt: &Stmt) -> Result<(), String> {
        let mut conn = self.pool.get_conn().map_err(db_err)?;
        conn.exec_drop(stmt.sql.as_str(), params_of(&stmt.params)).map_err(db_err)
    }

    pub fn rows(&self, stmt: &Stmt) -> Result<Vec<Row>, String> {
        let mut conn = self.pool.get_conn().map_err(db_err)?;
        let found: Vec<Columns> = conn.exec(stmt.sql.as_str(), params_of(&stmt.params)).map_err(db_err)?;
        Ok(found.into_iter().filter_map(row_of).collect())
    }
}

/// `Configs`: with `mysql.enabled`, the groups and the users come from the database and the files
/// are not read. A failure (connecting, the tables, reading the groups) keeps the files, as TAB does.
pub fn attach(cfg: &MysqlConfig, props: &mut Properties, warnings: &mut Vec<String>) {
    if !cfg.enabled {
        return;
    }
    let db = match Db::connect(cfg) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            warnings.push(format!("Failed to connect to MySQL: {e}"));
            return;
        }
    };
    let rows = match db.rows(&select_all("group")) {
        Ok(rows) => rows,
        Err(e) => {
            warnings.push(format!("Failed to connect to MySQL: {e}"));
            return;
        }
    };
    if rows.is_empty() {
        warnings.push(EMPTY_WARNING.to_owned());
    }
    let mut groups = PropertyFile::database("group", db.clone());
    for row in &rows {
        groups.put_row(row);
    }
    props.groups = groups;
    props.users = PropertyFile::database("user", db);
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

    fn stmt(sql: &str, params: &[Option<&str>]) -> Stmt {
        Stmt {
            sql: sql.to_owned(),
            params: params.iter().map(|p| p.map(str::to_owned)).collect(),
        }
    }

    #[test]
    fn schema_text() {
        assert_eq!(
            CREATE_GROUPS,
            "create table if not exists tab_groups (`group` varchar(64), `property` varchar(16), `value` varchar(1024), world varchar(64), server varchar(64))"
        );
        assert_eq!(
            CREATE_USERS,
            "create table if not exists tab_users (`user` varchar(64), `property` varchar(16), `value` varchar(1024), world varchar(64), server varchar(64))"
        );
    }

    #[test]
    fn select_texts() {
        assert_eq!(
            select_all("group"),
            stmt("SELECT `group`, `property`, `value`, `world`, `server` FROM `tab_groups`", &[])
        );
        assert_eq!(
            select_all("user"),
            stmt("SELECT `user`, `property`, `value`, `world`, `server` FROM `tab_users`", &[])
        );
        assert_eq!(
            select_name("user", "steve"),
            stmt(
                "SELECT `user`, `property`, `value`, `world`, `server` FROM `tab_users` WHERE `user` = ?",
                &[Some("steve")]
            )
        );
    }

    #[test]
    fn set_global_user_value() {
        assert_eq!(
            set_statements("user", "steve", "tabprefix", Some("&c"), None, None),
            vec![
                stmt(
                    "delete from `tab_users` where `user` = ? and `property` = ? and world IS NULL and server IS NULL",
                    &[Some("steve"), Some("tabprefix")]
                ),
                stmt(
                    "insert into `tab_users` (`user`, `property`, `value`, `world`, `server`) values (?, ?, ?, ?, ?)",
                    &[Some("steve"), Some("tabprefix"), Some("&c"), None, None]
                ),
            ]
        );
    }

    #[test]
    fn set_scoped_group_value_and_removal() {
        assert_eq!(
            set_statements("group", "admin", "tagprefix", Some("&4"), Some("world"), Some("lobby")),
            vec![
                stmt(
                    "delete from `tab_groups` where `group` = ? and `property` = ? and world = ? and server = ?",
                    &[Some("admin"), Some("tagprefix"), Some("world"), Some("lobby")]
                ),
                stmt(
                    "insert into `tab_groups` (`group`, `property`, `value`, `world`, `server`) values (?, ?, ?, ?, ?)",
                    &[Some("admin"), Some("tagprefix"), Some("&4"), Some("world"), Some("lobby")]
                ),
            ]
        );
        assert_eq!(
            set_statements("group", "admin", "tagprefix", None, None, Some("lobby")),
            vec![stmt(
                "delete from `tab_groups` where `group` = ? and `property` = ? and world IS NULL and server = ?",
                &[Some("admin"), Some("tagprefix"), Some("lobby")]
            )]
        );
    }

    #[test]
    fn remove_and_upload_texts() {
        assert_eq!(
            remove_statement("group", "admin"),
            stmt("delete from `tab_groups` where `group` = ?", &[Some("admin")])
        );
        assert_eq!(remove_statement("user", "Steve"), stmt("delete from `tab_users` where `user` = ?", &[Some("Steve")]));
        assert_eq!(delete_all("group"), stmt("DELETE FROM tab_groups", &[]));
        assert_eq!(delete_all("user"), stmt("DELETE FROM tab_users", &[]));
    }

    #[test]
    fn row_mapping_skips_rows_without_name_or_property() {
        assert_eq!(
            row_of((Some("Admin".into()), Some("tabprefix".into()), None, Some("world1".into()), None)),
            Some(Row {
                name: "Admin".into(),
                property: "tabprefix".into(),
                value: None,
                world: Some("world1".into()),
                server: None,
            })
        );
        assert_eq!(row_of((None, Some("tabprefix".into()), None, None, None)), None);
        assert_eq!(row_of((Some("Admin".into()), None, None, None, None)), None);
    }

    #[test]
    fn upload_rows_of_a_file() {
        let file = PropertyFile::from_str(
            "default:\n  tabprefix: '&7'\n  tabsuffix:\n  - 'a'\n  - 'b'\n  tagprefix:\nper-world:\n  nether:\n    default:\n      tabprefix: '&c'\nper-server:\n  lobby:\n    default:\n      customtabname: 5\n",
            "group",
        )
        .unwrap();
        let row = |name: &str, property: &str, value: &str, world: Option<&str>, server: Option<&str>| Row {
            name: name.into(),
            property: property.into(),
            value: Some(value.into()),
            world: world.map(str::to_owned),
            server: server.map(str::to_owned),
        };
        assert_eq!(
            rows_of(&file),
            vec![
                row("default", "tabprefix", "&7", None, None),
                row("default", "tabsuffix", "a\nb", None, None),
                row("default", "tabprefix", "&c", Some("nether"), None),
                row("default", "customtabname", "5", None, Some("lobby")),
            ]
        );
    }
}
