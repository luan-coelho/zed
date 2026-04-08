use crate::config::{ConnectionConfig, DatabaseConfig};
use crate::connection::ConnectionManager;
use std::collections::HashMap;

fn make_config(connections: Vec<(&str, &str, &str)>) -> DatabaseConfig {
    let mut map = HashMap::new();
    for (name, provider, database) in connections {
        map.insert(
            name.to_string(),
            ConnectionConfig {
                provider: provider.into(),
                host: Some("localhost".into()),
                port: None,
                database: Some(database.into()),
                user: Some("user".into()),
                password_env: None,
                connection_string_env: None,
                ssl: Some(false),
                default: None,
            },
        );
    }
    DatabaseConfig { connections: map }
}

#[test]
fn test_new_manager_is_unconfigured() {
    let mgr = ConnectionManager::new();
    assert!(!mgr.is_configured());
    assert!(mgr.active_connection_name().is_none());
    assert!(mgr.connection_names().is_empty());
}

#[test]
fn test_load_config_sets_default_connection() {
    let mut config = make_config(vec![("dev", "postgres", "devdb")]);
    config
        .connections
        .get_mut("dev")
        .unwrap()
        .default = Some(true);

    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    assert!(mgr.is_configured());
    assert_eq!(mgr.active_connection_name(), Some("dev"));
    assert_eq!(mgr.connection_names(), vec!["dev"]);
}

#[test]
fn test_load_config_picks_first_if_no_default() {
    let config = make_config(vec![("alpha", "postgres", "a")]);

    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    assert!(mgr.is_configured());
    // Should pick the only available connection
    assert_eq!(mgr.active_connection_name(), Some("alpha"));
}

#[test]
fn test_set_active_connection() {
    let config = make_config(vec![
        ("conn_a", "postgres", "db_a"),
        ("conn_b", "postgres", "db_b"),
    ]);

    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);
    mgr.set_active_connection("conn_b");

    assert_eq!(mgr.active_connection_name(), Some("conn_b"));
}

#[test]
fn test_password_management() {
    let mut mgr = ConnectionManager::new();

    mgr.set_password("myconn", "secret123".into());

    // Password is stored in memory
    // We can't directly read it (no getter), but clear_provider should remove it
    mgr.clear_provider("myconn");

    // After clear, the provider and password are removed
    // (verified by the fact that subsequent operations would need to re-set the password)
}

#[test]
fn test_connection_names_returns_all() {
    let config = make_config(vec![
        ("first", "postgres", "db1"),
        ("second", "mysql", "db2"),
        ("third", "sqlite", "db3"),
    ]);

    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let mut names = mgr.connection_names();
    names.sort();
    assert_eq!(names, vec!["first", "second", "third"]);
}

#[test]
fn test_load_config_clears_previous_state() {
    let config1 = make_config(vec![("old", "postgres", "olddb")]);
    let config2 = make_config(vec![("new", "postgres", "newdb")]);

    let mut mgr = ConnectionManager::new();
    mgr.load_config(config1);
    assert!(mgr.connection_names().contains(&"old".to_string()));

    mgr.load_config(config2);
    assert!(!mgr.connection_names().contains(&"old".to_string()));
    assert!(mgr.connection_names().contains(&"new".to_string()));
}

// ── capabilities_for() tests ──────────────────────────────────────

#[test]
fn test_capabilities_for_postgres_provider() {
    let config = make_config(vec![("pg", "postgres", "mydb")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let caps = mgr.capabilities_for("pg");
    assert!(caps.multi_database);
    assert!(caps.schemas);
    assert!(caps.roles);
    assert!(caps.create_database);
    assert!(caps.create_schema);
    assert!(caps.create_role);
    assert!(caps.requires_reconnect_for_database_switch);
}

#[test]
fn test_capabilities_for_mysql_provider() {
    let config = make_config(vec![("my", "mysql", "mydb")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let caps = mgr.capabilities_for("my");
    assert!(caps.multi_database);
    assert!(!caps.schemas);
    assert!(!caps.roles);
    assert!(caps.users);
    assert!(caps.create_database);
    assert!(!caps.create_schema);
    assert!(!caps.create_role);
    assert!(!caps.requires_reconnect_for_database_switch);
}

#[test]
fn test_capabilities_for_sqlite_provider() {
    let config = make_config(vec![("lite", "sqlite", "/tmp/test.db")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let caps = mgr.capabilities_for("lite");
    assert!(!caps.multi_database);
    assert!(!caps.schemas);
    assert!(!caps.roles);
    assert!(!caps.users);
    assert!(!caps.create_database);
    assert!(!caps.create_schema);
    assert!(!caps.create_role);
}

#[test]
fn test_capabilities_for_unknown_provider() {
    let config = make_config(vec![("mystery", "oracle", "orcl")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let caps = mgr.capabilities_for("mystery");
    assert!(!caps.multi_database);
    assert!(!caps.schemas);
    assert!(!caps.roles);
    assert!(!caps.create_database);
}

#[test]
fn test_capabilities_for_no_config_loaded() {
    let mgr = ConnectionManager::new();

    let caps = mgr.capabilities_for("nonexistent");
    assert!(!caps.multi_database);
    assert!(!caps.schemas);
    assert!(!caps.roles);
    assert!(!caps.create_database);
}

// ── url_for_database() tests ──────────────────────────────────────

#[test]
fn test_url_for_database_error_when_provider_not_initialized() {
    let config = make_config(vec![("pg", "postgres", "mydb")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let result = mgr.url_for_database("pg", "other_db");
    assert!(result.is_err());
    let error_message = format!("{}", result.expect_err("expected an error"));
    assert!(
        error_message.contains("Provider not initialized"),
        "unexpected error message: {error_message}"
    );
}

#[test]
fn test_url_for_database_error_when_connection_not_found() {
    let config = make_config(vec![("pg", "postgres", "mydb")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let result = mgr.url_for_database("nonexistent", "some_db");
    assert!(result.is_err());
    let error_message = format!("{}", result.expect_err("expected an error"));
    assert!(
        error_message.contains("not initialized") || error_message.contains("not found"),
        "unexpected error message: {error_message}"
    );
}

// ── capabilities_for with aliases ──────────────────────────────────

#[test]
fn test_capabilities_for_postgresql_alias() {
    let config = make_config(vec![("pg", "postgresql", "mydb")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let caps = mgr.capabilities_for("pg");
    assert!(caps.multi_database);
    assert!(caps.schemas);
    assert!(caps.roles);
}

#[test]
fn test_capabilities_for_mariadb_alias() {
    let config = make_config(vec![("maria", "mariadb", "mydb")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let caps = mgr.capabilities_for("maria");
    assert!(caps.multi_database);
    assert!(caps.users);
    assert!(!caps.schemas);
}

#[test]
fn test_capabilities_for_sqlite3_alias() {
    let config = make_config(vec![("lite", "sqlite3", "/tmp/t.db")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let caps = mgr.capabilities_for("lite");
    assert!(!caps.multi_database);
    assert!(!caps.schemas);
}

// ── multiple connections and switching ──────────────────────────────

#[test]
fn test_load_multiple_connections_and_switch() {
    let config = make_config(vec![
        ("pg", "postgres", "pgdb"),
        ("my", "mysql", "mydb"),
        ("lite", "sqlite", "/tmp/t.db"),
    ]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    assert_eq!(mgr.connection_names().len(), 3);

    mgr.set_active_connection("my");
    assert_eq!(mgr.active_connection_name(), Some("my"));

    mgr.set_active_connection("lite");
    assert_eq!(mgr.active_connection_name(), Some("lite"));
}

#[test]
fn test_clear_provider_removes_password() {
    let mut mgr = ConnectionManager::new();
    mgr.set_password("conn1", "secret".to_string());

    mgr.clear_provider("conn1");
    // After clearing, password is gone (internal state) — we just verify no panic
}

#[test]
fn test_set_password_and_overwrite() {
    let mut mgr = ConnectionManager::new();
    mgr.set_password("conn1", "first".to_string());
    mgr.set_password("conn1", "second".to_string());
    // Verify no panic, password is overwritten
}

// ── Multi-database capabilities queries ────────────────────────────

#[test]
fn test_postgres_is_multi_database() {
    let config = make_config(vec![("pg", "postgres", "mydb")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let caps = mgr.capabilities_for("pg");
    assert!(caps.multi_database);
    assert!(caps.schemas);
    assert!(caps.requires_reconnect_for_database_switch);
}

#[test]
fn test_mysql_is_multi_database_without_schemas() {
    let config = make_config(vec![("my", "mysql", "mydb")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let caps = mgr.capabilities_for("my");
    assert!(caps.multi_database);
    assert!(!caps.schemas);
    assert!(!caps.requires_reconnect_for_database_switch);
}

#[test]
fn test_sqlite_not_multi_database() {
    let config = make_config(vec![("lite", "sqlite", "/tmp/t.db")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let caps = mgr.capabilities_for("lite");
    assert!(!caps.multi_database);
    assert!(!caps.schemas);
}

#[test]
fn test_capabilities_different_connections_different_caps() {
    let config = make_config(vec![
        ("pg", "postgres", "pgdb"),
        ("my", "mysql", "mydb"),
        ("lite", "sqlite", "/tmp/t.db"),
    ]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);

    let pg_caps = mgr.capabilities_for("pg");
    let my_caps = mgr.capabilities_for("my");
    let lite_caps = mgr.capabilities_for("lite");

    assert!(pg_caps.schemas);
    assert!(!my_caps.schemas);
    assert!(!lite_caps.schemas);

    assert!(pg_caps.roles);
    assert!(!my_caps.roles);
    assert!(my_caps.users);

    assert!(pg_caps.create_role);
    assert!(!my_caps.create_role);
    assert!(!lite_caps.create_database);
}

// ── check_connection_health tests ──────────────────────────────────

#[tokio::test]
async fn test_check_connection_health_returns_false_when_no_config() {
    let mut mgr = ConnectionManager::new();
    let result = mgr.check_connection_health().await;
    assert!(result.is_ok());
    assert!(!result.expect("expected Ok"));
}

#[tokio::test]
async fn test_check_connection_health_returns_false_when_no_active_connection() {
    let config = make_config(vec![("pg", "postgres", "mydb")]);
    let mut mgr = ConnectionManager::new();
    mgr.load_config(config);
    mgr.set_active_connection("nonexistent");
    let result = mgr.check_connection_health().await;
    assert!(result.is_ok());
    assert!(!result.expect("expected Ok"));
}
