use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Column as _, Row, SqlitePool};

use super::PoolManager;
use crate::provider::{
    is_read_query, DatabaseProvider, FormPlaceholders, ProviderCapabilities, ProviderMetadata,
};
use crate::schema::*;

pub struct SqliteProvider {
    pools: PoolManager<SqlitePool>,
}

impl SqliteProvider {
    pub fn new() -> Self {
        Self {
            pools: PoolManager::new(),
        }
    }

    async fn ensure_pool(&self, url: &str) -> Result<SqlitePool> {
        self.pools
            .get_or_create(url, |url| async move {
                SqlitePoolOptions::new()
                    .max_connections(5)
                    .acquire_timeout(std::time::Duration::from_secs(10))
                    .connect(&url)
                    .await
                    .context("failed to connect to SQLite")
            })
            .await
    }
}

impl ProviderMetadata for SqliteProvider {
    fn id(&self) -> &'static str { "sqlite" }
    fn display_name(&self) -> &'static str { "SQLite" }
    fn default_port(&self) -> u16 { 0 }
    fn default_user(&self) -> &'static str { "" }
    fn url_scheme(&self) -> &'static str { "sqlite" }
    fn aliases(&self) -> &'static [&'static str] { &["sqlite3"] }
    fn is_file_based(&self) -> bool { true }
    fn form_placeholders(&self) -> FormPlaceholders {
        FormPlaceholders {
            name: "my_connection",
            host: "",
            port: "",
            database: "path/to/database.db",
            database_label: "Database File",
            user: "",
            password: "",
        }
    }
}

#[async_trait]
impl DatabaseProvider for SqliteProvider {
    fn metadata(&self) -> &dyn ProviderMetadata {
        self
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    async fn test_connection(&self, url: &str) -> Result<()> {
        let pool = self.ensure_pool(url).await?;
        sqlx::query("SELECT 1").execute(&pool).await?;
        Ok(())
    }

    async fn execute_query(&self, url: &str, sql: &str) -> Result<QueryResult> {
        let pool = self.ensure_pool(url).await?;
        let start = Instant::now();

        if is_read_query(sql) {
            let rows = sqlx::query(sql).fetch_all(&pool).await?;
            let elapsed = start.elapsed().as_millis() as u64;

            let mut result = QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                rows_affected: rows.len() as u64,
                execution_time_ms: elapsed,
            };

            if let Some(first_row) = rows.first() {
                result.columns = first_row.columns().iter().map(|c| c.name().to_string()).collect();
            }

            for row in &rows {
                let mut row_values = Vec::new();
                for i in 0..row.columns().len() {
                    let value: String = row
                        .try_get::<String, _>(i)
                        .or_else(|_| row.try_get::<i64, _>(i).map(|v| v.to_string()))
                        .or_else(|_| row.try_get::<f64, _>(i).map(|v| v.to_string()))
                        .or_else(|_| row.try_get::<bool, _>(i).map(|v| v.to_string()))
                        .unwrap_or_else(|_| "NULL".to_string());
                    row_values.push(value);
                }
                result.rows.push(row_values);
            }

            Ok(result)
        } else {
            let result = sqlx::query(sql).execute(&pool).await?;
            let elapsed = start.elapsed().as_millis() as u64;

            Ok(QueryResult {
                columns: vec!["rows_affected".to_string()],
                rows: vec![vec![result.rows_affected().to_string()]],
                rows_affected: result.rows_affected(),
                execution_time_ms: elapsed,
            })
        }
    }

    async fn get_schema(&self, url: &str) -> Result<DatabaseSchema> {
        let pool = self.ensure_pool(url).await?;

        // Get database name from the file path
        let db_name = url
            .strip_prefix("sqlite://")
            .or_else(|| url.strip_prefix("sqlite:"))
            .unwrap_or(url)
            .rsplit('/')
            .next()
            .unwrap_or("sqlite")
            .to_string();

        let table_rows = sqlx::query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(&pool)
        .await?;

        let mut tables = Vec::new();
        for row in &table_rows {
            let name: String = row.get("name");
            let columns = self.get_table_columns(url, &name).await?;

            // Get indexes
            let idx_rows = sqlx::query(&format!("PRAGMA index_list('{name}')"))
                .fetch_all(&pool)
                .await
                .unwrap_or_default();

            let mut indexes = Vec::new();
            for idx_row in &idx_rows {
                let idx_name: String = idx_row.get("name");
                let is_unique: bool = idx_row.try_get::<i32, _>("unique").map(|v| v != 0).unwrap_or(false);

                let col_rows = sqlx::query(&format!("PRAGMA index_info('{idx_name}')"))
                    .fetch_all(&pool)
                    .await
                    .unwrap_or_default();

                let idx_columns: Vec<String> = col_rows
                    .iter()
                    .filter_map(|r| r.try_get::<String, _>("name").ok())
                    .collect();

                indexes.push(Index {
                    name: idx_name,
                    columns: idx_columns,
                    is_unique,
                    is_primary: false,
                });
            }

            tables.push(Table {
                schema: "main".to_string(),
                name,
                columns,
                indexes,
                constraints: Vec::new(),
                row_count_estimate: None,
            });
        }

        let view_rows = sqlx::query(
            "SELECT name FROM sqlite_master WHERE type='view' ORDER BY name",
        )
        .fetch_all(&pool)
        .await?;

        let mut views = Vec::new();
        for row in &view_rows {
            let name: String = row.get("name");
            let columns = self.get_table_columns(url, &name).await?;
            views.push(View {
                schema: "main".to_string(),
                name,
                columns,
            });
        }

        Ok(DatabaseSchema {
            name: db_name,
            tables,
            views,
        })
    }

    async fn get_table_columns(&self, url: &str, table: &str) -> Result<Vec<DbColumn>> {
        let pool = self.ensure_pool(url).await?;
        let rows = sqlx::query(&format!("PRAGMA table_info('{table}')"))
            .fetch_all(&pool)
            .await?;

        let mut columns = Vec::new();
        for row in &rows {
            let name: String = row.get("name");
            let data_type: String = row.get("type");
            let not_null: bool = row.try_get::<i32, _>("notnull").map(|v| v != 0).unwrap_or(false);
            let default_value: Option<String> = row.try_get("dflt_value").ok();
            let is_pk: bool = row.try_get::<i32, _>("pk").map(|v| v != 0).unwrap_or(false);

            columns.push(DbColumn {
                name,
                data_type,
                is_nullable: !not_null,
                default_value,
                is_primary_key: is_pk,
                foreign_key: None,
                comment: None,
            });
        }

        Ok(columns)
    }

    async fn get_table_row_count(&self, url: &str, table: &str) -> Result<i64> {
        let pool = self.ensure_pool(url).await?;
        let count: i64 =
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&pool)
                .await?;
        Ok(count)
    }

    async fn explain_query(&self, url: &str, sql: &str) -> Result<String> {
        let pool = self.ensure_pool(url).await?;
        let explain_sql = format!("EXPLAIN QUERY PLAN {sql}");
        let rows = sqlx::query(&explain_sql).fetch_all(&pool).await?;

        let mut plan = String::new();
        for row in &rows {
            for i in 0..row.columns().len() {
                let val: String = row
                    .try_get::<String, _>(i)
                    .or_else(|_| row.try_get::<i64, _>(i).map(|v| v.to_string()))
                    .unwrap_or_default();
                plan.push_str(&val);
                plan.push('\t');
            }
            plan.push('\n');
        }

        Ok(plan)
    }
}
