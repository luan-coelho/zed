use std::collections::HashMap;

use anyhow::Result;

use crate::config::{ConnectionConfig, DatabaseConfig};
use crate::provider::DatabaseProvider;
use crate::providers;
use crate::schema::{DatabaseSchema, QueryResult};

pub struct ConnectionManager {
    config: Option<DatabaseConfig>,
    active_connection: Option<String>,
    providers: HashMap<String, Box<dyn DatabaseProvider>>,
    /// Runtime passwords stored securely in memory (from OS keychain).
    /// Key: connection name, Value: plaintext password.
    /// These are never persisted to disk.
    passwords: HashMap<String, String>,
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

    /// Store a password in memory for a named connection.
    /// This password is used instead of `password_env` when connecting.
    pub fn set_password(&mut self, connection_name: &str, password: String) {
        self.passwords.insert(connection_name.to_string(), password);
    }

    /// Clear a cached provider (forces reconnection on next use).
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

    /// Returns (provider, config, optional in-memory password) for the active connection.
    fn active_context(
        &mut self,
    ) -> Result<(&dyn DatabaseProvider, &ConnectionConfig, Option<&str>)> {
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

        Ok((provider.as_ref(), config, password))
    }

    pub async fn test_connection(&mut self) -> Result<()> {
        let (provider, config, password) = self.active_context()?;
        let url = config.connection_url_with_password(password)?;
        provider.test_connection(&url).await
    }

    pub async fn execute_query(&mut self, sql: &str) -> Result<QueryResult> {
        let (provider, config, password) = self.active_context()?;
        let url = config.connection_url_with_password(password)?;
        provider.execute_query(&url, sql).await
    }

    pub async fn execute_query_in_schema(
        &mut self,
        sql: &str,
        schema: Option<&str>,
    ) -> Result<QueryResult> {
        let (provider, config, password) = self.active_context()?;
        let url = config.connection_url_with_password(password)?;
        match schema {
            Some(s) => provider.execute_query_in_schema(&url, sql, s).await,
            None => provider.execute_query(&url, sql).await,
        }
    }

    pub async fn get_schema(&mut self) -> Result<DatabaseSchema> {
        let (provider, config, password) = self.active_context()?;
        let url = config.connection_url_with_password(password)?;
        provider.get_schema(&url).await
    }

    pub async fn explain_query(&mut self, sql: &str) -> Result<String> {
        let (provider, config, password) = self.active_context()?;
        let url = config.connection_url_with_password(password)?;
        provider.explain_query(&url, sql).await
    }

    pub async fn explain_query_in_schema(
        &mut self,
        sql: &str,
        schema: Option<&str>,
    ) -> Result<String> {
        let (provider, config, password) = self.active_context()?;
        let url = config.connection_url_with_password(password)?;
        match schema {
            Some(s) => provider.explain_query_in_schema(&url, sql, s).await,
            None => provider.explain_query(&url, sql).await,
        }
    }
}
