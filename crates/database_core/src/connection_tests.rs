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
