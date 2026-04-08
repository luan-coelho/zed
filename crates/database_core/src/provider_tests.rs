use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::config::replace_database_in_url;
use crate::provider::{is_read_query, ProviderCapabilities};
use crate::providers::PoolManager;
use crate::providers::ProviderRegistry;

// ── is_read_query tests ───────────────────────────────────────────

#[test]
fn test_select_is_read_query() {
    assert!(is_read_query("SELECT * FROM users"));
}

#[test]
fn test_with_is_read_query() {
    assert!(is_read_query("WITH cte AS (SELECT 1) SELECT * FROM cte"));
}

#[test]
fn test_explain_is_read_query() {
    assert!(is_read_query("EXPLAIN SELECT * FROM users"));
}

#[test]
fn test_show_is_read_query() {
    assert!(is_read_query("SHOW TABLES"));
}

#[test]
fn test_table_is_read_query() {
    assert!(is_read_query("TABLE users"));
}

#[test]
fn test_describe_is_read_query() {
    assert!(is_read_query("DESCRIBE users"));
}

#[test]
fn test_pragma_is_read_query() {
    assert!(is_read_query("PRAGMA table_info('users')"));
}

#[test]
fn test_insert_is_not_read_query() {
    assert!(!is_read_query("INSERT INTO users VALUES (1, 'a')"));
}

#[test]
fn test_update_is_not_read_query() {
    assert!(!is_read_query("UPDATE users SET name = 'b'"));
}

#[test]
fn test_delete_is_not_read_query() {
    assert!(!is_read_query("DELETE FROM users WHERE id = 1"));
}

#[test]
fn test_create_is_not_read_query() {
    assert!(!is_read_query("CREATE TABLE users (id INT)"));
}

#[test]
fn test_alter_is_not_read_query() {
    assert!(!is_read_query("ALTER TABLE users ADD COLUMN age INT"));
}

#[test]
fn test_drop_is_not_read_query() {
    assert!(!is_read_query("DROP TABLE users"));
}

#[test]
fn test_is_read_query_case_insensitive() {
    assert!(is_read_query("select * from users"));
    assert!(is_read_query("Select * From users"));
    assert!(is_read_query("sElEcT 1"));
}

#[test]
fn test_is_read_query_leading_whitespace() {
    assert!(is_read_query("   SELECT 1"));
    assert!(is_read_query("\t\n  SELECT 1"));
    assert!(!is_read_query("   INSERT INTO t VALUES (1)"));
}

#[test]
fn test_is_read_query_empty_string() {
    assert!(!is_read_query(""));
    assert!(!is_read_query("   "));
}

// ── ProviderRegistry tests ────────────────────────────────────────

#[test]
fn test_registry_new_has_three_providers() {
    let registry = ProviderRegistry::new();
    let providers = registry.available_providers();
    assert_eq!(providers.len(), 3);
}

#[test]
fn test_registry_create_postgres() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("postgres");
    assert!(provider.is_some());
    assert_eq!(provider.as_ref().map(|p| p.metadata().id()), Some("postgres"));
}

#[test]
fn test_registry_create_postgresql_alias() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("postgresql");
    assert!(provider.is_some());
    assert_eq!(provider.as_ref().map(|p| p.metadata().id()), Some("postgres"));
}

#[test]
fn test_registry_create_mysql() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("mysql");
    assert!(provider.is_some());
    assert_eq!(provider.as_ref().map(|p| p.metadata().id()), Some("mysql"));
}

#[test]
fn test_registry_create_mariadb_alias() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("mariadb");
    assert!(provider.is_some());
    assert_eq!(provider.as_ref().map(|p| p.metadata().id()), Some("mysql"));
}

#[test]
fn test_registry_create_sqlite() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("sqlite");
    assert!(provider.is_some());
    assert_eq!(provider.as_ref().map(|p| p.metadata().id()), Some("sqlite"));
}

#[test]
fn test_registry_create_sqlite3_alias() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("sqlite3");
    assert!(provider.is_some());
    assert_eq!(provider.as_ref().map(|p| p.metadata().id()), Some("sqlite"));
}

#[test]
fn test_registry_unknown_provider_returns_none() {
    let registry = ProviderRegistry::new();
    assert!(registry.create_provider("oracle").is_none());
    assert!(registry.create_provider("mongodb").is_none());
    assert!(registry.create_provider("").is_none());
}

#[test]
fn test_registry_create_provider_case_insensitive() {
    let registry = ProviderRegistry::new();
    assert!(registry.create_provider("POSTGRES").is_some());
    assert!(registry.create_provider("Postgres").is_some());
    assert!(registry.create_provider("MySQL").is_some());
    assert!(registry.create_provider("SQLITE").is_some());
}

#[test]
fn test_registry_available_providers_metadata() {
    let registry = ProviderRegistry::new();
    let providers = registry.available_providers();

    let ids: Vec<&str> = providers.iter().map(|p| p.id).collect();
    assert!(ids.contains(&"postgres"));
    assert!(ids.contains(&"mysql"));
    assert!(ids.contains(&"sqlite"));

    let postgres_info = providers.iter().find(|p| p.id == "postgres");
    assert!(postgres_info.is_some());
    let postgres_info = postgres_info.expect("postgres provider should exist");
    assert_eq!(postgres_info.display_name, "PostgreSQL");
    assert_eq!(postgres_info.default_port, 5432);
    assert!(!postgres_info.is_file_based);

    let sqlite_info = providers.iter().find(|p| p.id == "sqlite");
    assert!(sqlite_info.is_some());
    let sqlite_info = sqlite_info.expect("sqlite provider should exist");
    assert!(sqlite_info.is_file_based);
}

// ── Provider metadata tests ───────────────────────────────────────

#[test]
fn test_postgres_metadata() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("postgres").expect("postgres provider should exist");
    let meta = provider.metadata();

    assert_eq!(meta.id(), "postgres");
    assert_eq!(meta.display_name(), "PostgreSQL");
    assert_eq!(meta.default_port(), 5432);
    assert_eq!(meta.default_user(), "postgres");
    assert_eq!(meta.url_scheme(), "postgres");
}

#[test]
fn test_postgres_aliases() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("postgres").expect("postgres provider should exist");
    let aliases = provider.metadata().aliases();
    assert!(aliases.contains(&"postgresql"));
}

#[test]
fn test_postgres_not_file_based() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("postgres").expect("postgres provider should exist");
    assert!(!provider.metadata().is_file_based());
}

#[test]
fn test_postgres_form_placeholders() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("postgres").expect("postgres provider should exist");
    let placeholders = provider.metadata().form_placeholders();

    assert!(!placeholders.name.is_empty());
    assert!(!placeholders.host.is_empty());
    assert!(!placeholders.port.is_empty());
    assert!(!placeholders.database.is_empty());
    assert!(!placeholders.database_label.is_empty());
    assert!(!placeholders.user.is_empty());
    assert!(!placeholders.password.is_empty());
}

#[test]
fn test_mysql_metadata() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("mysql").expect("mysql provider should exist");
    let meta = provider.metadata();

    assert_eq!(meta.id(), "mysql");
    assert_eq!(meta.display_name(), "MySQL");
    assert_eq!(meta.default_port(), 3306);
    assert_eq!(meta.default_user(), "root");
    assert_eq!(meta.url_scheme(), "mysql");
}

#[test]
fn test_mysql_aliases() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("mysql").expect("mysql provider should exist");
    let aliases = provider.metadata().aliases();
    assert!(aliases.contains(&"mariadb"));
}

#[test]
fn test_mysql_not_file_based() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("mysql").expect("mysql provider should exist");
    assert!(!provider.metadata().is_file_based());
}

#[test]
fn test_mysql_form_placeholders() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("mysql").expect("mysql provider should exist");
    let placeholders = provider.metadata().form_placeholders();

    assert!(!placeholders.name.is_empty());
    assert!(!placeholders.host.is_empty());
    assert!(!placeholders.port.is_empty());
    assert!(!placeholders.database.is_empty());
    assert!(!placeholders.database_label.is_empty());
    assert!(!placeholders.user.is_empty());
    assert!(!placeholders.password.is_empty());
}

#[test]
fn test_sqlite_metadata() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("sqlite").expect("sqlite provider should exist");
    let meta = provider.metadata();

    assert_eq!(meta.id(), "sqlite");
    assert_eq!(meta.display_name(), "SQLite");
    assert_eq!(meta.default_port(), 0);
    assert_eq!(meta.default_user(), "");
    assert_eq!(meta.url_scheme(), "sqlite");
}

#[test]
fn test_sqlite_aliases() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("sqlite").expect("sqlite provider should exist");
    let aliases = provider.metadata().aliases();
    assert!(aliases.contains(&"sqlite3"));
}

#[test]
fn test_sqlite_is_file_based() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("sqlite").expect("sqlite provider should exist");
    assert!(provider.metadata().is_file_based());
}

#[test]
fn test_sqlite_form_placeholders_file_specific() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("sqlite").expect("sqlite provider should exist");
    let placeholders = provider.metadata().form_placeholders();

    assert!(!placeholders.name.is_empty());
    assert!(!placeholders.database.is_empty());
    assert_eq!(placeholders.database_label, "Database File");
    // File-based providers have empty host/port/user/password placeholders
    assert!(placeholders.host.is_empty());
    assert!(placeholders.port.is_empty());
    assert!(placeholders.user.is_empty());
    assert!(placeholders.password.is_empty());
}

#[test]
fn test_all_providers_have_non_empty_name_placeholder() {
    let registry = ProviderRegistry::new();
    for name in &["postgres", "mysql", "sqlite"] {
        let provider = registry.create_provider(name).expect("provider should exist");
        let placeholders = provider.metadata().form_placeholders();
        assert!(
            !placeholders.name.is_empty(),
            "Provider {name} should have a non-empty name placeholder"
        );
    }
}

// ── Provider capabilities tests ───────────────────────────────────

#[test]
fn test_postgres_capabilities() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("postgres").expect("postgres provider should exist");
    let capabilities = provider.capabilities();

    assert!(capabilities.multi_database);
    assert!(capabilities.schemas);
    assert!(capabilities.roles);
    assert!(!capabilities.users);
    assert!(capabilities.create_database);
    assert!(capabilities.create_schema);
    assert!(capabilities.create_role);
    assert!(capabilities.requires_reconnect_for_database_switch);
}

#[test]
fn test_mysql_capabilities() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("mysql").expect("mysql provider should exist");
    let capabilities = provider.capabilities();

    assert!(capabilities.multi_database);
    assert!(!capabilities.schemas);
    assert!(!capabilities.roles);
    assert!(capabilities.users);
    assert!(capabilities.create_database);
    assert!(!capabilities.create_schema);
    assert!(!capabilities.create_role);
    assert!(!capabilities.requires_reconnect_for_database_switch);
}

#[test]
fn test_sqlite_capabilities() {
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("sqlite").expect("sqlite provider should exist");
    let capabilities = provider.capabilities();

    assert!(!capabilities.multi_database);
    assert!(!capabilities.schemas);
    assert!(!capabilities.roles);
    assert!(!capabilities.users);
    assert!(!capabilities.create_database);
    assert!(!capabilities.create_schema);
    assert!(!capabilities.create_role);
    assert!(!capabilities.requires_reconnect_for_database_switch);
}

#[test]
fn test_sqlite_capabilities_match_default() {
    let default_capabilities = ProviderCapabilities::default();
    let registry = ProviderRegistry::new();
    let provider = registry.create_provider("sqlite").expect("sqlite provider should exist");
    let capabilities = provider.capabilities();

    assert_eq!(capabilities.multi_database, default_capabilities.multi_database);
    assert_eq!(capabilities.schemas, default_capabilities.schemas);
    assert_eq!(capabilities.roles, default_capabilities.roles);
    assert_eq!(capabilities.users, default_capabilities.users);
    assert_eq!(capabilities.create_database, default_capabilities.create_database);
    assert_eq!(capabilities.create_schema, default_capabilities.create_schema);
    assert_eq!(capabilities.create_role, default_capabilities.create_role);
    assert_eq!(
        capabilities.requires_reconnect_for_database_switch,
        default_capabilities.requires_reconnect_for_database_switch
    );
}

#[test]
fn test_default_capabilities_are_all_false() {
    let capabilities = ProviderCapabilities::default();

    assert!(!capabilities.multi_database);
    assert!(!capabilities.schemas);
    assert!(!capabilities.roles);
    assert!(!capabilities.users);
    assert!(!capabilities.create_database);
    assert!(!capabilities.create_schema);
    assert!(!capabilities.create_role);
    assert!(!capabilities.requires_reconnect_for_database_switch);
}

#[test]
fn test_postgres_and_mysql_differ_on_schemas_and_roles() {
    let registry = ProviderRegistry::new();
    let pg = registry.create_provider("postgres").expect("postgres provider should exist");
    let my = registry.create_provider("mysql").expect("mysql provider should exist");
    let pg_caps = pg.capabilities();
    let my_caps = my.capabilities();

    assert!(pg_caps.schemas);
    assert!(!my_caps.schemas);
    assert!(pg_caps.roles);
    assert!(!my_caps.roles);
    assert!(!pg_caps.users);
    assert!(my_caps.users);
}

// ── replace_database_in_url tests ─────────────────────────────────

#[test]
fn test_replace_database_in_postgres_url() {
    let result = replace_database_in_url("postgres://user:pass@localhost:5432/olddb", "newdb");
    assert!(result.is_ok());
    let url = result.expect("should parse valid URL");
    assert!(url.contains("/newdb"));
    assert!(!url.contains("/olddb"));
}

#[test]
fn test_replace_database_in_mysql_url() {
    let result = replace_database_in_url("mysql://root@localhost:3306/mydb", "otherdb");
    assert!(result.is_ok());
    let url = result.expect("should parse valid URL");
    assert!(url.contains("/otherdb"));
    assert!(!url.contains("/mydb"));
}

#[test]
fn test_replace_database_preserves_query_params() {
    let result = replace_database_in_url(
        "postgres://user@localhost:5432/olddb?sslmode=require",
        "newdb",
    );
    assert!(result.is_ok());
    let url = result.expect("should parse valid URL");
    assert!(url.contains("/newdb"));
    assert!(url.contains("sslmode=require"));
}

#[test]
fn test_replace_database_invalid_url_returns_error() {
    let result = replace_database_in_url("not-a-valid-url", "newdb");
    assert!(result.is_err());
}

#[test]
fn test_replace_database_empty_name() {
    let result = replace_database_in_url("postgres://user@localhost:5432/olddb", "");
    assert!(result.is_ok());
    let url = result.expect("should parse valid URL");
    assert!(url.contains("localhost"));
}

#[test]
fn test_replace_database_special_characters() {
    let result = replace_database_in_url("postgres://user@localhost:5432/olddb", "my-db_2");
    assert!(result.is_ok());
    let url = result.expect("should parse valid URL");
    assert!(url.contains("/my-db_2"));
}

#[test]
fn test_replace_database_preserves_credentials() {
    let result = replace_database_in_url("postgres://admin:s3cret@host:5432/old", "new");
    assert!(result.is_ok());
    let url = result.expect("should parse valid URL");
    assert!(url.contains("admin"));
    assert!(url.contains("s3cret"));
    assert!(url.contains("/new"));
}

#[test]
fn test_replace_database_preserves_host_and_port() {
    let result = replace_database_in_url("postgres://user@myhost:9999/old", "new");
    assert!(result.is_ok());
    let url = result.expect("should parse valid URL");
    assert!(url.contains("myhost"));
    assert!(url.contains("9999"));
    assert!(url.contains("/new"));
}

// ── PoolManager tests ─────────────────────────────────────────────

#[tokio::test]
async fn test_pool_manager_new_is_empty() {
    let manager: PoolManager<String> = PoolManager::new();
    // Verify the manager can be created without error. There is no `len()` method,
    // so we confirm it works by successfully calling get_or_create later.
    let result = manager
        .get_or_create("test://url", |url| async move { Ok(url) })
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_pool_manager_get_or_create_returns_value() {
    let manager: PoolManager<String> = PoolManager::new();
    let result = manager
        .get_or_create("postgres://localhost/db", |url| async move {
            Ok(format!("pool-for-{url}"))
        })
        .await;
    assert!(result.is_ok());
    assert_eq!(
        result.expect("should return created pool"),
        "pool-for-postgres://localhost/db"
    );
}

#[tokio::test]
async fn test_pool_manager_caches_result() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let manager: PoolManager<String> = PoolManager::new();

    let first = {
        let call_count = call_count.clone();
        manager
            .get_or_create("postgres://localhost/db", |url| {
                let call_count = call_count.clone();
                async move {
                    call_count.fetch_add(1, Ordering::SeqCst);
                    Ok(format!("pool-{url}"))
                }
            })
            .await
    };
    assert!(first.is_ok());

    let second = {
        let call_count = call_count.clone();
        manager
            .get_or_create("postgres://localhost/db", |url| {
                let call_count = call_count.clone();
                async move {
                    call_count.fetch_add(1, Ordering::SeqCst);
                    Ok(format!("pool-{url}"))
                }
            })
            .await
    };
    assert!(second.is_ok());

    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        first.expect("first call should succeed"),
        second.expect("second call should succeed")
    );
}

#[tokio::test]
async fn test_pool_manager_different_urls_get_different_pools() {
    let manager: PoolManager<String> = PoolManager::new();

    let pool_a = manager
        .get_or_create("postgres://localhost/db_a", |url| async move {
            Ok(format!("pool-{url}"))
        })
        .await;

    let pool_b = manager
        .get_or_create("postgres://localhost/db_b", |url| async move {
            Ok(format!("pool-{url}"))
        })
        .await;

    assert!(pool_a.is_ok());
    assert!(pool_b.is_ok());
    assert_ne!(
        pool_a.expect("pool_a should succeed"),
        pool_b.expect("pool_b should succeed")
    );
}

#[tokio::test]
async fn test_pool_manager_factory_error_propagates() {
    let manager: PoolManager<String> = PoolManager::new();

    let result = manager
        .get_or_create("postgres://localhost/db", |_url| async move {
            Err(anyhow::anyhow!("connection refused"))
        })
        .await;

    assert!(result.is_err());
    let error_message = format!("{}", result.expect_err("should be an error"));
    assert!(error_message.contains("connection refused"));
}

// ── ProviderRegistry available_providers form_placeholders ─────────

#[test]
fn test_registry_postgres_form_placeholders() {
    let registry = ProviderRegistry::new();
    let providers = registry.available_providers();
    let pg = providers.iter().find(|p| p.id == "postgres").expect("postgres exists");
    assert_eq!(pg.form_placeholders.port, "5432");
    assert_eq!(pg.form_placeholders.user, "postgres");
    assert_eq!(pg.form_placeholders.database, "mydb");
    assert_eq!(pg.form_placeholders.database_label, "Database");
    assert!(!pg.is_file_based);
}

#[test]
fn test_registry_mysql_form_placeholders() {
    let registry = ProviderRegistry::new();
    let providers = registry.available_providers();
    let my = providers.iter().find(|p| p.id == "mysql").expect("mysql exists");
    assert_eq!(my.form_placeholders.port, "3306");
    assert_eq!(my.form_placeholders.user, "root");
    assert!(my.form_placeholders.database_label.contains("optional"));
    assert!(!my.is_file_based);
}

#[test]
fn test_registry_sqlite_form_placeholders() {
    let registry = ProviderRegistry::new();
    let providers = registry.available_providers();
    let lite = providers.iter().find(|p| p.id == "sqlite").expect("sqlite exists");
    assert!(lite.is_file_based);
    assert!(lite.form_placeholders.database.contains("database"));
    assert_eq!(lite.form_placeholders.user, "");
    assert_eq!(lite.form_placeholders.port, "");
}

#[test]
fn test_registry_all_providers_have_display_name() {
    let registry = ProviderRegistry::new();
    for info in registry.available_providers() {
        assert!(!info.display_name.is_empty(), "provider {} has empty display name", info.id);
    }
}

// ── ProviderCapabilities edge cases ────────────────────────────────

#[test]
fn test_capabilities_default_clone() {
    let caps = ProviderCapabilities::default();
    let cloned = caps.clone();
    assert!(!cloned.multi_database);
    assert!(!cloned.schemas);
}

#[test]
fn test_capabilities_debug_format() {
    let caps = ProviderCapabilities::default();
    let debug = format!("{caps:?}");
    assert!(debug.contains("multi_database"));
    assert!(debug.contains("false"));
}
