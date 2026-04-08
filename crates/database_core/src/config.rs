use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
    /// Environment variable name containing the password
    pub password_env: Option<String>,
    /// Environment variable name containing the full connection string
    pub connection_string_env: Option<String>,
    pub ssl: Option<bool>,
    pub default: Option<bool>,
}

impl ConnectionConfig {
    pub fn connection_url(&self) -> Result<String> {
        self.connection_url_with_password(None)
    }

    /// Build the connection URL, optionally using a password from the keychain
    /// instead of the environment variable.
    pub fn connection_url_with_password(&self, keychain_password: Option<&str>) -> Result<String> {
        if let Some(ref env_name) = self.connection_string_env {
            return std::env::var(env_name)
                .with_context(|| format!("environment variable '{env_name}' not set"));
        }

        // SQLite uses file path, not host/port
        if self.provider == "sqlite" || self.provider == "sqlite3" {
            let database = self
                .database
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("database file path required"))?;
            return Ok(format!("sqlite:{database}"));
        }

        let host = self.host.as_deref().unwrap_or("localhost");
        let default_port = match self.provider.as_str() {
            "mysql" | "mariadb" => 3306,
            _ => 5432,
        };
        let port = self.port.unwrap_or(default_port);
        let database = self
            .database
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("database name required"))?;
        let default_user = match self.provider.as_str() {
            "mysql" | "mariadb" => "root",
            _ => "postgres",
        };
        let user = self.user.as_deref().unwrap_or(default_user);

        let password = if let Some(pw) = keychain_password {
            pw.to_string()
        } else if let Some(ref env_name) = self.password_env {
            std::env::var(env_name)
                .with_context(|| format!("environment variable '{env_name}' not set"))?
        } else {
            String::new()
        };

        let scheme = match self.provider.as_str() {
            "mysql" | "mariadb" => "mysql",
            _ => "postgres",
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
