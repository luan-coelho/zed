pub mod mysql;
pub mod postgres;
pub mod sqlite;

use std::collections::HashMap;
use std::future::Future;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::provider::{DatabaseProvider, FormPlaceholders};

// ── Generic Pool Manager ───────────────────────────────────────────

pub struct PoolManager<P: Clone> {
    pools: RwLock<HashMap<String, P>>,
}

impl<P: Clone> PoolManager<P> {
    pub fn new() -> Self {
        Self {
            pools: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get_or_create<F, Fut>(&self, url: &str, create: F) -> Result<P>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<P>>,
    {
        {
            let guard = self.pools.read().await;
            if let Some(pool) = guard.get(url) {
                return Ok(pool.clone());
            }
        }

        log_connection_attempt(url);
        let url_owned = url.to_string();
        let pool = create(url_owned).await?;
        let mut guard = self.pools.write().await;
        guard.insert(url.to_string(), pool.clone());
        Ok(pool)
    }
}

fn log_connection_attempt(url: &str) {
    let masked = url
        .find("://")
        .and_then(|start| {
            let after = &url[start + 3..];
            after.find('@').map(|at| {
                let user_end = after.find(':').unwrap_or(at);
                format!(
                    "{}://{}:***@{}",
                    &url[..start],
                    &after[..user_end],
                    &after[at + 1..]
                )
            })
        })
        .unwrap_or_else(|| url.to_string());
    eprintln!("[database_core] Connecting to: {masked}");
}

// ── Provider Registry ──────────────────────────────────────────────

type ProviderFactory = fn() -> Box<dyn DatabaseProvider>;

struct ProviderEntry {
    id: &'static str,
    display_name: &'static str,
    aliases: &'static [&'static str],
    factory: ProviderFactory,
}

pub struct ProviderRegistry {
    entries: Vec<ProviderEntry>,
}

pub struct ProviderInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub default_port: u16,
    pub is_file_based: bool,
    pub sql_dialect: &'static str,
    pub form_placeholders: FormPlaceholders,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            entries: Vec::new(),
        };
        registry.register(
            "postgres",
            "PostgreSQL",
            &["postgresql"],
            || Box::new(postgres::PostgresProvider::new()),
        );
        registry.register(
            "mysql",
            "MySQL",
            &["mariadb"],
            || Box::new(mysql::MysqlProvider::new()),
        );
        registry.register(
            "sqlite",
            "SQLite",
            &["sqlite3"],
            || Box::new(sqlite::SqliteProvider::new()),
        );
        registry
    }

    fn register(
        &mut self,
        id: &'static str,
        display_name: &'static str,
        aliases: &'static [&'static str],
        factory: ProviderFactory,
    ) {
        self.entries.push(ProviderEntry {
            id,
            display_name,
            aliases,
            factory,
        });
    }

    pub fn find_provider_info(&self, name: &str) -> Option<ProviderInfo> {
        let lower = name.to_lowercase();
        self.entries
            .iter()
            .find(|e| e.id == lower || e.aliases.contains(&lower.as_str()))
            .map(|e| {
                let provider = (e.factory)();
                let meta = provider.metadata();
                ProviderInfo {
                    id: e.id,
                    display_name: e.display_name,
                    default_port: meta.default_port(),
                    is_file_based: meta.is_file_based(),
                    sql_dialect: meta.sql_dialect(),
                    form_placeholders: meta.form_placeholders(),
                }
            })
    }

    pub fn create_provider(&self, name: &str) -> Option<Box<dyn DatabaseProvider>> {
        let lower = name.to_lowercase();
        self.entries
            .iter()
            .find(|e| e.id == lower || e.aliases.contains(&lower.as_str()))
            .map(|e| (e.factory)())
    }

    pub fn available_providers(&self) -> Vec<ProviderInfo> {
        self.entries
            .iter()
            .map(|e| {
                let provider = (e.factory)();
                let meta = provider.metadata();
                ProviderInfo {
                    id: e.id,
                    display_name: e.display_name,
                    default_port: meta.default_port(),
                    is_file_based: meta.is_file_based(),
                    sql_dialect: meta.sql_dialect(),
                    form_placeholders: meta.form_placeholders(),
                }
            })
            .collect()
    }
}

/// Convenience function for backwards compatibility.
pub fn get_provider(name: &str) -> Option<Box<dyn DatabaseProvider>> {
    ProviderRegistry::new().create_provider(name)
}
