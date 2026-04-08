pub mod config;
pub mod connection;
pub mod formatter;
pub mod provider;
pub mod providers;
pub mod schema;

pub use config::{ConnectionConfig, DatabaseConfig};
pub use connection::ConnectionManager;
pub use provider::DatabaseProvider;
pub use schema::{DatabaseSchema, DbColumn, QueryResult, Table, View};

#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod connection_tests;
#[cfg(test)]
mod formatter_tests;
