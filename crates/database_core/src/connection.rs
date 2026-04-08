use std::collections::HashMap;

use anyhow::Result;

use crate::config::DatabaseConfig;
use crate::provider::{DatabaseProvider, ProviderCapabilities};
use crate::providers;
use crate::schema::{DatabaseEntry, DatabaseSchema, QueryResult, RoleEntry, SchemaEntry};

pub struct ConnectionManager {
    config: Option<DatabaseConfig>,
    active_connection: Option<String>,
    providers: HashMap<String, Box<dyn DatabaseProvider>>,
    passwords: HashMap<String, String>,
}

struct ActiveContext<'a> {
    provider: &'a dyn DatabaseProvider,
    url: String,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            config: None,
            active_connection: None,
            providers: HashMap::new(),
            passwords: HashMap::new(),
        }
    }

    pub fn load_config(&mut self, config: DatabaseConfig) {
        let default_name = config
            .connections
            .iter()
            .find(|(_, c)| c.default.unwrap_or(false))
            .or_else(|| config.connections.iter().next())
            .map(|(name, _)| name.clone());

        self.active_connection = default_name;
        self.providers.clear();
        self.config = Some(config);
    }

    pub fn set_active_connection(&mut self, name: &str) {
        self.active_connection = Some(name.to_string());
    }

    pub fn active_connection_name(&self) -> Option<&str> {
        self.active_connection.as_deref()
    }

    pub fn connection_names(&self) -> Vec<String> {
        self.config
            .as_ref()
            .map(|c| c.connections.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn is_configured(&self) -> bool {
        self.config.is_some() && self.active_connection.is_some()
    }

    pub fn set_password(&mut self, connection_name: &str, password: String) {
        self.passwords.insert(connection_name.to_string(), password);
    }

    pub fn clear_provider(&mut self, connection_name: &str) {
        self.providers.remove(connection_name);
        self.passwords.remove(connection_name);
    }

    fn ensure_provider(&mut self) -> Result<()> {
        let conn_name = self
            .active_connection
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No active connection"))?
            .clone();

        if self.providers.contains_key(&conn_name) {
            return Ok(());
        }

        let config = self
            .config
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No configuration loaded"))?;

        let conn_config = config
            .connections
            .get(&conn_name)
            .ok_or_else(|| anyhow::anyhow!("Connection '{conn_name}' not found"))?;

        let provider = providers::get_provider(&conn_config.provider)
            .ok_or_else(|| anyhow::anyhow!("Unknown provider: {}", conn_config.provider))?;

        self.providers.insert(conn_name, provider);
        Ok(())
    }

    fn active_context(&mut self) -> Result<ActiveContext<'_>> {
        self.ensure_provider()?;

        let conn_name = self.active_connection.as_ref().unwrap();
        let provider = self.providers.get(conn_name.as_str()).unwrap();
        let config = self
            .config
            .as_ref()
            .unwrap()
            .connections
            .get(conn_name.as_str())
            .unwrap();
        let password = self.passwords.get(conn_name.as_str()).map(|s| s.as_str());
        let url = config.connection_url_with_metadata(password, provider.metadata())?;

        Ok(ActiveContext {
            provider: provider.as_ref(),
            url,
        })
    }

    pub fn capabilities_for(&self, conn_name: &str) -> ProviderCapabilities {
        if let Some(provider) = self.providers.get(conn_name) {
            return provider.capabilities();
        }
        let provider_name = self
            .config
            .as_ref()
            .and_then(|c| c.connections.get(conn_name))
            .map(|c| c.provider.as_str());
        if let Some(name) = provider_name {
            if let Some(provider) = providers::get_provider(name) {
                return provider.capabilities();
            }
        }
        ProviderCapabilities::default()
    }

    pub async fn test_connection(&mut self) -> Result<()> {
        let ctx = self.active_context()?;
        ctx.provider.test_connection(&ctx.url).await
    }

    pub async fn execute_query(&mut self, sql: &str) -> Result<QueryResult> {
        let ctx = self.active_context()?;
        ctx.provider.execute_query(&ctx.url, sql).await
    }

    pub async fn execute_query_in_schema(
        &mut self,
        sql: &str,
        schema: Option<&str>,
    ) -> Result<QueryResult> {
        let ctx = self.active_context()?;
        match schema {
            Some(s) => ctx.provider.execute_query_in_schema(&ctx.url, sql, s).await,
            None => ctx.provider.execute_query(&ctx.url, sql).await,
        }
    }

    pub async fn get_schema(&mut self) -> Result<DatabaseSchema> {
        let ctx = self.active_context()?;
        ctx.provider.get_schema(&ctx.url).await
    }

    pub async fn explain_query(&mut self, sql: &str) -> Result<String> {
        let ctx = self.active_context()?;
        ctx.provider.explain_query(&ctx.url, sql).await
    }

    pub async fn explain_query_in_schema(
        &mut self,
        sql: &str,
        schema: Option<&str>,
    ) -> Result<String> {
        let ctx = self.active_context()?;
        match schema {
            Some(s) => ctx.provider.explain_query_in_schema(&ctx.url, sql, s).await,
            None => ctx.provider.explain_query(&ctx.url, sql).await,
        }
    }

    pub async fn execute_query_in_database(
        &mut self,
        sql: &str,
        database: &str,
        schema: Option<&str>,
    ) -> Result<QueryResult> {
        let ctx = self.active_context()?;
        let db_url = crate::config::replace_database_in_url(&ctx.url, database)?;
        match schema {
            Some(s) => ctx.provider.execute_query_in_schema(&db_url, sql, s).await,
            None => ctx.provider.execute_query(&db_url, sql).await,
        }
    }

    pub async fn explain_query_in_database(
        &mut self,
        sql: &str,
        database: &str,
        schema: Option<&str>,
    ) -> Result<String> {
        let ctx = self.active_context()?;
        let db_url = crate::config::replace_database_in_url(&ctx.url, database)?;
        match schema {
            Some(s) => ctx.provider.explain_query_in_schema(&db_url, sql, s).await,
            None => ctx.provider.explain_query(&db_url, sql).await,
        }
    }

    pub async fn list_databases(&mut self) -> Result<Vec<DatabaseEntry>> {
        let ctx = self.active_context()?;
        ctx.provider.list_databases(&ctx.url).await
    }

    pub async fn list_schemas(&mut self) -> Result<Vec<SchemaEntry>> {
        let ctx = self.active_context()?;
        ctx.provider.list_schemas(&ctx.url).await
    }

    pub async fn list_roles(&mut self) -> Result<Vec<RoleEntry>> {
        let ctx = self.active_context()?;
        ctx.provider.list_roles(&ctx.url).await
    }

    pub async fn get_schema_for_database(&mut self, database: &str) -> Result<DatabaseSchema> {
        let ctx = self.active_context()?;
        ctx.provider
            .get_schema_for_database(&ctx.url, database)
            .await
    }

    pub fn url_for_database(&mut self, conn_name: &str, database: &str) -> Result<String> {
        let provider = self
            .providers
            .get(conn_name)
            .ok_or_else(|| anyhow::anyhow!("Provider not initialized for '{conn_name}'"))?;

        let config = self
            .config
            .as_ref()
            .and_then(|c| c.connections.get(conn_name))
            .ok_or_else(|| anyhow::anyhow!("Connection '{conn_name}' not found"))?;

        let password = self.passwords.get(conn_name).map(|s| s.as_str());
        config.connection_url_for_database(database, password, provider.metadata())
    }

    pub async fn create_database(&mut self, name: &str) -> Result<()> {
        let ctx = self.active_context()?;
        ctx.provider.create_database(&ctx.url, name).await
    }

    pub async fn create_schema(&mut self, name: &str) -> Result<()> {
        let ctx = self.active_context()?;
        ctx.provider.create_schema(&ctx.url, name).await
    }

    pub async fn create_role(&mut self, name: &str) -> Result<()> {
        let ctx = self.active_context()?;
        ctx.provider.create_role(&ctx.url, name).await
    }

    pub async fn begin_transaction(&mut self, isolation: Option<&str>) -> Result<()> {
        let ctx = self.active_context()?;
        ctx.provider.begin_transaction(&ctx.url, isolation).await
    }

    pub async fn commit_transaction(&mut self) -> Result<()> {
        let ctx = self.active_context()?;
        ctx.provider.commit_transaction(&ctx.url).await
    }

    pub async fn rollback_transaction(&mut self) -> Result<()> {
        let ctx = self.active_context()?;
        ctx.provider.rollback_transaction(&ctx.url).await
    }

    pub async fn execute_in_transaction(&mut self, sql: &str) -> Result<QueryResult> {
        let ctx = self.active_context()?;
        ctx.provider.execute_in_transaction(&ctx.url, sql).await
    }

    pub async fn has_active_transaction(&mut self) -> bool {
        match self.active_context() {
            Ok(ctx) => ctx.provider.has_active_transaction(&ctx.url).await,
            Err(_) => false,
        }
    }

    pub async fn check_connection_health(&mut self) -> Result<bool> {
        match self.active_context() {
            Ok(ctx) => Ok(ctx.provider.is_connection_alive(&ctx.url).await),
            Err(_) => Ok(false),
        }
    }
}
