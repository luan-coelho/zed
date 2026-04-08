use anyhow::Result;
use async_trait::async_trait;

use crate::schema::{DatabaseEntry, DatabaseSchema, DbColumn, QueryResult, RoleEntry, SchemaEntry};

// ── Provider Metadata ──────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct FormPlaceholders {
    pub name: &'static str,
    pub host: &'static str,
    pub port: &'static str,
    pub database: &'static str,
    pub database_label: &'static str,
    pub user: &'static str,
    pub password: &'static str,
}

pub trait ProviderMetadata {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn default_port(&self) -> u16;
    fn default_user(&self) -> &'static str;
    fn url_scheme(&self) -> &'static str;
    fn aliases(&self) -> &'static [&'static str];
    fn is_file_based(&self) -> bool;
    fn form_placeholders(&self) -> FormPlaceholders;
    /// SQL dialect identifier for sqlparser (e.g., "postgres", "mysql", "sqlite").
    fn sql_dialect(&self) -> &'static str {
        self.id()
    }
}

// ── Provider Capabilities ──────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ProviderCapabilities {
    pub multi_database: bool,
    pub schemas: bool,
    pub roles: bool,
    pub users: bool,
    pub create_database: bool,
    pub create_schema: bool,
    pub create_role: bool,
    pub requires_reconnect_for_database_switch: bool,
}

// ── Utility ────────────────────────────────────────────────────────

pub fn is_read_query(sql: &str) -> bool {
    let trimmed = sql.trim_start().to_uppercase();
    trimmed.starts_with("SELECT")
        || trimmed.starts_with("WITH")
        || trimmed.starts_with("EXPLAIN")
        || trimmed.starts_with("SHOW")
        || trimmed.starts_with("TABLE")
        || trimmed.starts_with("DESCRIBE")
        || trimmed.starts_with("PRAGMA")
}

// ── DatabaseProvider Trait ──────────────────────────────────────────

#[async_trait]
pub trait DatabaseProvider: Send + Sync {
    fn metadata(&self) -> &dyn ProviderMetadata;

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    async fn test_connection(&self, url: &str) -> Result<()>;
    async fn execute_query(&self, url: &str, sql: &str) -> Result<QueryResult>;
    async fn get_schema(&self, url: &str) -> Result<DatabaseSchema>;
    async fn get_table_columns(&self, url: &str, table: &str) -> Result<Vec<DbColumn>>;
    async fn get_table_row_count(&self, url: &str, table: &str) -> Result<i64>;
    async fn explain_query(&self, url: &str, sql: &str) -> Result<String>;

    async fn execute_query_in_schema(
        &self,
        url: &str,
        sql: &str,
        _schema: &str,
    ) -> Result<QueryResult> {
        self.execute_query(url, sql).await
    }

    async fn explain_query_in_schema(
        &self,
        url: &str,
        sql: &str,
        _schema: &str,
    ) -> Result<String> {
        self.explain_query(url, sql).await
    }

    async fn list_databases(&self, _url: &str) -> Result<Vec<DatabaseEntry>> {
        Err(anyhow::anyhow!(
            "{}: list_databases not supported",
            self.metadata().id()
        ))
    }

    async fn list_schemas(&self, _url: &str) -> Result<Vec<SchemaEntry>> {
        Err(anyhow::anyhow!(
            "{}: list_schemas not supported",
            self.metadata().id()
        ))
    }

    async fn list_roles(&self, _url: &str) -> Result<Vec<RoleEntry>> {
        Err(anyhow::anyhow!(
            "{}: list_roles not supported",
            self.metadata().id()
        ))
    }

    async fn get_schema_for_database(
        &self,
        base_url: &str,
        _database: &str,
    ) -> Result<DatabaseSchema> {
        self.get_schema(base_url).await
    }

    async fn create_database(&self, _url: &str, _name: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "{}: create_database not supported",
            self.metadata().id()
        ))
    }

    async fn create_schema(&self, _url: &str, _name: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "{}: create_schema not supported",
            self.metadata().id()
        ))
    }

    async fn create_role(&self, _url: &str, _name: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "{}: create_role not supported",
            self.metadata().id()
        ))
    }

    async fn begin_transaction(&self, _url: &str, _isolation: Option<&str>) -> Result<()> {
        Err(anyhow::anyhow!(
            "{}: transactions not supported",
            self.metadata().id()
        ))
    }

    async fn commit_transaction(&self, _url: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "{}: transactions not supported",
            self.metadata().id()
        ))
    }

    async fn rollback_transaction(&self, _url: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "{}: transactions not supported",
            self.metadata().id()
        ))
    }

    async fn execute_in_transaction(&self, _url: &str, _sql: &str) -> Result<QueryResult> {
        Err(anyhow::anyhow!(
            "{}: transactions not supported",
            self.metadata().id()
        ))
    }

    async fn has_active_transaction(&self, _url: &str) -> bool {
        false
    }

    async fn is_connection_alive(&self, url: &str) -> bool {
        self.test_connection(url).await.is_ok()
    }
}
