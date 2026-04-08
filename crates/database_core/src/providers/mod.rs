pub mod mysql;
pub mod postgres;
pub mod sqlite;

use crate::provider::DatabaseProvider;

pub fn get_provider(name: &str) -> Option<Box<dyn DatabaseProvider>> {
    match name {
        "postgres" | "postgresql" => Some(Box::new(postgres::PostgresProvider::new())),
        "mysql" | "mariadb" => Some(Box::new(mysql::MysqlProvider::new())),
        "sqlite" | "sqlite3" => Some(Box::new(sqlite::SqliteProvider::new())),
        _ => None,
    }
}
