use crate::config::{ConnectionConfig, DatabaseConfig};
use std::collections::HashMap;

fn pg_config(host: Option<&str>, port: Option<u16>, db: &str, user: Option<&str>) -> ConnectionConfig {
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

#[test]
fn test_postgres_url_defaults() {
    let config = pg_config(None, None, "mydb", None);
    let url = config.connection_url_with_password(None).unwrap();
    assert_eq!(url, "postgres://postgres@localhost:5432/mydb?sslmode=disable");
}

#[test]
fn test_postgres_url_with_password() {
    let config = pg_config(Some("db.example.com"), Some(5433), "prod", Some("admin"));
    let url = config.connection_url_with_password(Some("s3cret")).unwrap();
    assert_eq!(
        url,
        "postgres://admin:s3cret@db.example.com:5433/prod?sslmode=disable"
    );
}

#[test]
fn test_postgres_url_ssl_enabled() {
    let mut config = pg_config(None, None, "mydb", None);
    config.ssl = Some(true);
    let url = config.connection_url_with_password(None).unwrap();
    assert!(url.contains("sslmode=require"));
}

#[test]
fn test_postgres_url_empty_password_omitted() {
    let config = pg_config(None, None, "mydb", None);
    let url = config.connection_url_with_password(Some("")).unwrap();
    // Empty password should still produce a URL without password in the authority
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
    let url = config.connection_url_with_password(None).unwrap();
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
    let url = config.connection_url_with_password(Some("pw123")).unwrap();
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
    let url = config.connection_url_with_password(None).unwrap();
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
    assert!(config.connection_url_with_password(None).is_err());
}

#[test]
fn test_postgres_requires_database_name() {
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
    assert!(config.connection_url_with_password(None).is_err());
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
