pub mod config;
pub mod connection;
pub mod formatter;
pub mod provider;
pub mod providers;
pub mod schema;

pub use config::{ConnectionConfig, DatabaseConfig};
pub use connection::ConnectionManager;
pub use provider::{
    DatabaseProvider, FormPlaceholders, ProviderCapabilities, ProviderMetadata,
};
pub use providers::{ProviderInfo, ProviderRegistry};
pub use schema::{
    DatabaseEntry, DatabaseSchema, DbColumn, QueryResult, RoleEntry, SchemaEntry, Table, View,
};

#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod connection_tests;
#[cfg(test)]
mod formatter_tests;
#[cfg(test)]
mod provider_tests;
#[cfg(test)]
mod schema_tests;
