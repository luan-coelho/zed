use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::{Column as _, MySqlPool, Row};
use tokio::sync::RwLock;

use crate::provider::DatabaseProvider;
use crate::schema::*;

pub struct MysqlProvider {
    pool: RwLock<Option<(String, MySqlPool)>>,
}

impl MysqlProvider {
    pub fn new() -> Self {
        Self {
            pool: RwLock::new(None),
        }
    }

    async fn ensure_pool(&self, url: &str) -> Result<MySqlPool> {
        {
            let guard = self.pool.read().await;
            if let Some((cached_url, pool)) = guard.as_ref() {
                if cached_url == url && !pool.is_closed() {
                    return Ok(pool.clone());
                }
            }
        }

        let pool = MySqlPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect(url)
            .await
            .context("failed to connect to MySQL")?;

        let mut guard = self.pool.write().await;
        *guard = Some((url.to_string(), pool.clone()));
        Ok(pool)
    }
}

#[async_trait]
impl DatabaseProvider for MysqlProvider {
    fn name(&self) -> &str {
        "mysql"
    }

    async fn test_connection(&self, url: &str) -> Result<()> {
        let pool = self.ensure_pool(url).await?;
        sqlx::query("SELECT 1").execute(&pool).await?;
        Ok(())
    }

    async fn execute_query(&self, url: &str, sql: &str) -> Result<QueryResult> {
        let pool = self.ensure_pool(url).await?;
        let start = Instant::now();

        let trimmed = sql.trim_start().to_uppercase();
        let is_query = trimmed.starts_with("SELECT")
            || trimmed.starts_with("WITH")
            || trimmed.starts_with("SHOW")
            || trimmed.starts_with("DESCRIBE")
            || trimmed.starts_with("EXPLAIN");

        if is_query {
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

        let db_name: String = sqlx::query_scalar("SELECT DATABASE()")
            .fetch_one(&pool)
            .await?;

        let table_rows = sqlx::query(
            "SELECT TABLE_NAME FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME",
        )
        .bind(&db_name)
        .fetch_all(&pool)
        .await?;

        let mut tables = Vec::new();
        for row in &table_rows {
            let name: String = row.get("TABLE_NAME");
            let columns = self.get_table_columns(url, &name).await?;

            tables.push(Table {
                schema: db_name.clone(),
                name,
                columns,
                indexes: Vec::new(),
                constraints: Vec::new(),
                row_count_estimate: None,
            });
        }

        let view_rows = sqlx::query(
            "SELECT TABLE_NAME FROM information_schema.VIEWS WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME",
        )
        .bind(&db_name)
        .fetch_all(&pool)
        .await?;

        let mut views = Vec::new();
        for row in &view_rows {
            let name: String = row.get("TABLE_NAME");
            let columns = self.get_table_columns(url, &name).await?;
            views.push(View {
                schema: db_name.clone(),
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
        let db_name: String = sqlx::query_scalar("SELECT DATABASE()")
            .fetch_one(&pool)
            .await?;

        let rows = sqlx::query(
            r#"
            SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, COLUMN_COMMENT
            FROM information_schema.COLUMNS
            WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
            ORDER BY ORDINAL_POSITION
            "#,
        )
        .bind(&db_name)
        .bind(table)
        .fetch_all(&pool)
        .await?;

        let mut columns = Vec::new();
        for row in &rows {
            let name: String = row.get("COLUMN_NAME");
            let data_type: String = row.get("DATA_TYPE");
            let is_nullable: String = row.get("IS_NULLABLE");
            let default_value: Option<String> = row.get("COLUMN_DEFAULT");
            let column_key: String = row.get("COLUMN_KEY");
            let comment: String = row.get("COLUMN_COMMENT");

            columns.push(DbColumn {
                name,
                data_type,
                is_nullable: is_nullable == "YES",
                default_value,
                is_primary_key: column_key == "PRI",
                foreign_key: if column_key == "MUL" {
                    Some(ForeignKey {
                        referenced_table: String::new(),
                        referenced_column: String::new(),
                    })
                } else {
                    None
                },
                comment: if comment.is_empty() { None } else { Some(comment) },
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
        let explain_sql = format!("EXPLAIN {sql}");
        let rows = sqlx::query(&explain_sql).fetch_all(&pool).await?;

        let mut plan = String::new();
        for row in &rows {
            // MySQL EXPLAIN returns multiple columns
            for i in 0..row.columns().len() {
                let val: String = row.try_get::<String, _>(i).unwrap_or_default();
                plan.push_str(&val);
                plan.push('\t');
            }
            plan.push('\n');
        }

        Ok(plan)
    }
}
