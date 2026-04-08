use crate::config::{ConnectionConfig, DatabaseConfig};
use crate::providers;
use std::collections::HashMap;

fn pg_config(
    host: Option<&str>,
    port: Option<u16>,
    db: &str,
    user: Option<&str>,
) -> ConnectionConfig {
    ConnectionConfig {
        provider: "postgres".into(),
        host: host.map(|h| h.into()),
        port,
        database: Some(db.into()),
        user: user.map(|u| u.into()),
        password_env: None,
        connection_string_env: None,
        ssl: Some(false),
        default: None,
    }
}

fn build_url(config: &ConnectionConfig, password: Option<&str>) -> anyhow::Result<String> {
    let provider = providers::get_provider(&config.provider)
        .ok_or_else(|| anyhow::anyhow!("unknown provider"))?;
    config.connection_url_with_metadata(password, provider.metadata())
}

#[test]
fn test_postgres_url_defaults() {
    let config = pg_config(None, None, "mydb", None);
    let url = build_url(&config, None).unwrap();
    assert_eq!(url, "postgres://postgres@localhost:5432/mydb?sslmode=disable");
}

#[test]
fn test_postgres_url_with_password() {
    let config = pg_config(Some("db.example.com"), Some(5433), "prod", Some("admin"));
    let url = build_url(&config, Some("s3cret")).unwrap();
    assert_eq!(
        url,
        "postgres://admin:s3cret@db.example.com:5433/prod?sslmode=disable"
    );
}

#[test]
fn test_postgres_url_ssl_enabled() {
    let mut config = pg_config(None, None, "mydb", None);
    config.ssl = Some(true);
    let url = build_url(&config, None).unwrap();
    assert!(url.contains("sslmode=require"));
}

#[test]
fn test_postgres_url_empty_password_omitted() {
    let config = pg_config(None, None, "mydb", None);
    let url = build_url(&config, Some("")).unwrap();
    assert!(!url.contains(":@"));
}

#[test]
fn test_mysql_url_defaults() {
    let config = ConnectionConfig {
        provider: "mysql".into(),
        host: None,
        port: None,
        database: Some("mydb".into()),
        user: None,
        password_env: None,
        connection_string_env: None,
        ssl: Some(false),
        default: None,
    };
    let url = build_url(&config, None).unwrap();
    assert_eq!(url, "mysql://root@localhost:3306/mydb?sslmode=disable");
}

#[test]
fn test_mysql_url_with_password() {
    let config = ConnectionConfig {
        provider: "mysql".into(),
        host: Some("mysql.local".into()),
        port: Some(3307),
        database: Some("app".into()),
        user: Some("appuser".into()),
        password_env: None,
        connection_string_env: None,
        ssl: Some(false),
        default: None,
    };
    let url = build_url(&config, Some("pw123")).unwrap();
    assert_eq!(
        url,
        "mysql://appuser:pw123@mysql.local:3307/app?sslmode=disable"
    );
}

#[test]
fn test_sqlite_url() {
    let config = ConnectionConfig {
        provider: "sqlite".into(),
        host: None,
        port: None,
        database: Some("/tmp/test.db".into()),
        user: None,
        password_env: None,
        connection_string_env: None,
        ssl: None,
        default: None,
    };
    let url = build_url(&config, None).unwrap();
    assert_eq!(url, "sqlite:/tmp/test.db");
}

#[test]
fn test_sqlite_requires_database_path() {
    let config = ConnectionConfig {
        provider: "sqlite".into(),
        host: None,
        port: None,
        database: None,
        user: None,
        password_env: None,
        connection_string_env: None,
        ssl: None,
        default: None,
    };
    assert!(build_url(&config, None).is_err());
}

#[test]
fn test_postgres_empty_database_allowed() {
    let config = ConnectionConfig {
        provider: "postgres".into(),
        host: None,
        port: None,
        database: None,
        user: None,
        password_env: None,
        connection_string_env: None,
        ssl: None,
        default: None,
    };
    // Non-file-based providers now allow empty database (shows as empty path)
    let url = build_url(&config, None).unwrap();
    assert!(url.contains("postgres://"));
}

#[test]
fn test_config_serialization_roundtrip() {
    let mut connections = HashMap::new();
    connections.insert(
        "dev".into(),
        pg_config(Some("localhost"), Some(5432), "devdb", Some("dev")),
    );

    let config = DatabaseConfig { connections };
    let toml_str = toml::to_string_pretty(&config).unwrap();
    let parsed: DatabaseConfig = toml::from_str(&toml_str).unwrap();

    assert!(parsed.connections.contains_key("dev"));
    let conn = &parsed.connections["dev"];
    assert_eq!(conn.provider, "postgres");
    assert_eq!(conn.database.as_deref(), Some("devdb"));
    assert_eq!(conn.host.as_deref(), Some("localhost"));
    assert_eq!(conn.port, Some(5432));
}

#[test]
fn test_config_load_from_toml_string() {
    let toml = r#"
[connections.production]
provider = "postgres"
host = "db.prod.internal"
port = 5432
database = "app_production"
user = "deploy"
password_env = "DB_PASSWORD"
ssl = true
default = true
"#;

    let config: DatabaseConfig = toml::from_str(toml).unwrap();
    assert!(config.connections.contains_key("production"));
    let conn = &config.connections["production"];
    assert_eq!(conn.provider, "postgres");
    assert_eq!(conn.host.as_deref(), Some("db.prod.internal"));
    assert_eq!(conn.ssl, Some(true));
    assert_eq!(conn.default, Some(true));
    assert_eq!(conn.password_env.as_deref(), Some("DB_PASSWORD"));
}

#[test]
fn test_config_multiple_connections() {
    let toml = r#"
[connections.pg]
provider = "postgres"
database = "pgdb"

[connections.my]
provider = "mysql"
database = "mydb"

[connections.lite]
provider = "sqlite"
database = "/tmp/lite.db"
"#;

    let config: DatabaseConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.connections.len(), 3);
    assert_eq!(config.connections["pg"].provider, "postgres");
    assert_eq!(config.connections["my"].provider, "mysql");
    assert_eq!(config.connections["lite"].provider, "sqlite");
}

// ── connection_url_for_database ────────────────────────────────────

#[test]
fn test_connection_url_for_database_postgres() {
    let config = pg_config(Some("db.host"), Some(5432), "original", Some("user1"));
    let provider = providers::get_provider("postgres").expect("provider exists");
    let url = config
        .connection_url_for_database("newdb", None, provider.metadata())
        .expect("url builds");
    assert!(url.contains("/newdb"));
    assert!(!url.contains("/original"));
}

#[test]
fn test_connection_url_for_database_preserves_password() {
    let config = pg_config(Some("host"), Some(5432), "old", Some("admin"));
    let provider = providers::get_provider("postgres").expect("provider exists");
    let url = config
        .connection_url_for_database("new", Some("secret"), provider.metadata())
        .expect("url builds");
    assert!(url.contains("secret"));
    assert!(url.contains("/new"));
}

#[test]
fn test_connection_url_for_database_mysql() {
    let config = ConnectionConfig {
        provider: "mysql".into(),
        host: Some("myhost".into()),
        port: Some(3306),
        database: Some("olddb".into()),
        user: Some("root".into()),
        password_env: None,
        connection_string_env: None,
        ssl: Some(false),
        default: None,
    };
    let provider = providers::get_provider("mysql").expect("provider exists");
    let url = config
        .connection_url_for_database("newdb", None, provider.metadata())
        .expect("url builds");
    assert!(url.contains("mysql://"));
    assert!(url.contains("/newdb"));
}

// ── connection_url_with_metadata edge cases ────────────────────────

#[test]
fn test_url_with_metadata_no_host_uses_localhost() {
    let config = ConnectionConfig {
        provider: "postgres".into(),
        host: None,
        port: None,
        database: Some("db".into()),
        user: None,
        password_env: None,
        connection_string_env: None,
        ssl: None,
        default: None,
    };
    let url = build_url(&config, None).expect("url builds");
    assert!(url.contains("localhost"));
}

#[test]
fn test_url_with_metadata_mysql_defaults() {
    let config = ConnectionConfig {
        provider: "mysql".into(),
        host: None,
        port: None,
        database: Some("test".into()),
        user: None,
        password_env: None,
        connection_string_env: None,
        ssl: None,
        default: None,
    };
    let url = build_url(&config, None).expect("url builds");
    assert!(url.contains("mysql://"));
    assert!(url.contains("root@"));
    assert!(url.contains(":3306/"));
}

#[test]
fn test_url_with_metadata_sqlite_file_based() {
    let config = ConnectionConfig {
        provider: "sqlite".into(),
        host: None,
        port: None,
        database: Some("/data/app.db".into()),
        user: None,
        password_env: None,
        connection_string_env: None,
        ssl: None,
        default: None,
    };
    let url = build_url(&config, None).expect("url builds");
    assert_eq!(url, "sqlite:/data/app.db");
}

#[test]
fn test_url_connection_string_env_takes_precedence() {
    let config = ConnectionConfig {
        provider: "postgres".into(),
        host: Some("host".into()),
        port: Some(5432),
        database: Some("db".into()),
        user: Some("user".into()),
        password_env: None,
        connection_string_env: Some("TEST_DB_URL_NONEXISTENT_12345".into()),
        ssl: None,
        default: None,
    };
    let result = build_url(&config, None);
    assert!(result.is_err());
}

#[test]
fn test_url_empty_database_for_non_file_provider() {
    let config = ConnectionConfig {
        provider: "postgres".into(),
        host: None,
        port: None,
        database: None,
        user: None,
        password_env: None,
        connection_string_env: None,
        ssl: None,
        default: None,
    };
    let url = build_url(&config, None).expect("url builds");
    assert!(url.contains("postgres://postgres@localhost:5432/"));
}
