use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::provider::ProviderMetadata;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub connections: HashMap<String, ConnectionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub provider: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub user: Option<String>,
    pub password_env: Option<String>,
    pub connection_string_env: Option<String>,
    pub ssl: Option<bool>,
    pub default: Option<bool>,
}

impl ConnectionConfig {
    pub fn connection_url_with_metadata(
        &self,
        keychain_password: Option<&str>,
        metadata: &dyn ProviderMetadata,
    ) -> Result<String> {
        if let Some(ref env_name) = self.connection_string_env {
            return std::env::var(env_name)
                .with_context(|| format!("environment variable '{env_name}' not set"));
        }

        if metadata.is_file_based() {
            let database = self
                .database
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("database file path required"))?;
            return Ok(format!("{}:{database}", metadata.url_scheme()));
        }

        let host = self.host.as_deref().unwrap_or("localhost");
        let port = self.port.unwrap_or(metadata.default_port());
        let database = self.database.as_deref().unwrap_or("");
        let user = self
            .user
            .as_deref()
            .unwrap_or(metadata.default_user());
        let scheme = metadata.url_scheme();

        let password = if let Some(pw) = keychain_password {
            pw.to_string()
        } else if let Some(ref env_name) = self.password_env {
            std::env::var(env_name)
                .with_context(|| format!("environment variable '{env_name}' not set"))?
        } else {
            String::new()
        };

        let ssl_mode = if self.ssl.unwrap_or(false) {
            "require"
        } else {
            "disable"
        };

        let url = if password.is_empty() {
            format!("{scheme}://{user}@{host}:{port}/{database}?sslmode={ssl_mode}")
        } else {
            format!("{scheme}://{user}:{password}@{host}:{port}/{database}?sslmode={ssl_mode}")
        };

        Ok(url)
    }

    pub fn connection_url_for_database(
        &self,
        database: &str,
        keychain_password: Option<&str>,
        metadata: &dyn ProviderMetadata,
    ) -> Result<String> {
        let mut override_config = self.clone();
        override_config.database = Some(database.to_string());
        override_config.connection_url_with_metadata(keychain_password, metadata)
    }
}

pub fn replace_database_in_url(url: &str, new_db: &str) -> Result<String> {
    let mut parsed = url::Url::parse(url).context("invalid connection URL")?;
    parsed.set_path(&format!("/{new_db}"));
    Ok(parsed.to_string())
}

impl DatabaseConfig {
    pub fn load_from_workspace(workspace_path: &str) -> Result<Self> {
        let config_path = Path::new(workspace_path).join(".database/connections.toml");
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;

        let config: DatabaseConfig =
            toml::from_str(&content).context("failed to parse .database/connections.toml")?;

        Ok(config)
    }
}
