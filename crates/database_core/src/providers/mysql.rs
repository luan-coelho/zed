use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::{Column as _, MySqlPool, Row};

use super::PoolManager;
use crate::provider::{
    is_read_query, DatabaseProvider, FormPlaceholders, ProviderCapabilities, ProviderMetadata,
};
use crate::schema::*;

pub struct MysqlProvider {
    pools: PoolManager<MySqlPool>,
}

impl MysqlProvider {
    pub fn new() -> Self {
        Self {
            pools: PoolManager::new(),
        }
    }

    async fn ensure_pool(&self, url: &str) -> Result<MySqlPool> {
        self.pools
            .get_or_create(url, |url| async move {
                MySqlPoolOptions::new()
                    .max_connections(5)
                    .acquire_timeout(std::time::Duration::from_secs(10))
                    .connect(&url)
                    .await
                    .context("failed to connect to MySQL")
            })
            .await
    }
}

impl ProviderMetadata for MysqlProvider {
    fn id(&self) -> &'static str { "mysql" }
    fn display_name(&self) -> &'static str { "MySQL" }
    fn default_port(&self) -> u16 { 3306 }
    fn default_user(&self) -> &'static str { "root" }
    fn url_scheme(&self) -> &'static str { "mysql" }
    fn aliases(&self) -> &'static [&'static str] { &["mariadb"] }
    fn is_file_based(&self) -> bool { false }
    fn form_placeholders(&self) -> FormPlaceholders {
        FormPlaceholders {
            name: "my_connection",
            host: "localhost",
            port: "3306",
            database: "leave empty to list all",
            database_label: "Database (optional)",
            user: "root",
            password: "stored in OS keychain",
        }
    }
}

#[async_trait]
impl DatabaseProvider for MysqlProvider {
    fn metadata(&self) -> &dyn ProviderMetadata {
        self
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            multi_database: true,
            schemas: false,
            roles: false,
            users: true,
            create_database: true,
            create_schema: false,
            create_role: false,
            requires_reconnect_for_database_switch: false,
        }
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
            let columns = get_columns_for_table(&pool, &db_name, &name).await?;
            let indexes = get_indexes_for_table(&pool, &db_name, &name).await?;
            let constraints = get_constraints_for_table(&pool, &db_name, &name).await?;

            tables.push(Table {
                schema: db_name.clone(),
                name,
                columns,
                indexes,
                constraints,
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
            let columns = get_columns_for_table(&pool, &db_name, &name).await?;
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
        get_columns_for_table(&pool, &db_name, table).await
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

    async fn list_databases(&self, url: &str) -> Result<Vec<DatabaseEntry>> {
        let pool = self.ensure_pool(url).await?;
        let current_db: String = sqlx::query_scalar("SELECT DATABASE()")
            .fetch_one(&pool)
            .await?;

        let rows = sqlx::query(
            "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA ORDER BY SCHEMA_NAME",
        )
        .fetch_all(&pool)
        .await?;

        Ok(rows
            .iter()
            .map(|row| {
                let name: String = row.get("SCHEMA_NAME");
                DatabaseEntry {
                    is_current: name == current_db,
                    name,
                }
            })
            .collect())
    }

    async fn list_roles(&self, url: &str) -> Result<Vec<RoleEntry>> {
        let pool = self.ensure_pool(url).await?;
        let rows = sqlx::query("SELECT User, Host FROM mysql.user ORDER BY User")
            .fetch_all(&pool)
            .await?;

        Ok(rows
            .iter()
            .map(|row| {
                let user: String = row.get("User");
                let host: String = row.get("Host");
                RoleEntry {
                    name: format!("{user}@{host}"),
                    is_superuser: false,
                    can_login: true,
                    can_create_db: false,
                    can_create_role: false,
                }
            })
            .collect())
    }

    async fn get_schema_for_database(
        &self,
        base_url: &str,
        database: &str,
    ) -> Result<DatabaseSchema> {
        let url = crate::config::replace_database_in_url(base_url, database)?;
        self.get_schema(&url).await
    }

    async fn create_database(&self, url: &str, name: &str) -> Result<()> {
        let pool = self.ensure_pool(url).await?;
        let sql = format!("CREATE DATABASE `{}`", name.replace('`', "``"));
        sqlx::query(&sql).execute(&pool).await?;
        Ok(())
    }

    async fn create_user(&self, url: &str, name: &str) -> Result<()> {
        let pool = self.ensure_pool(url).await?;
        let sql = format!("CREATE USER '{}'@'%' IDENTIFIED BY 'password'", name.replace('\'', "''"));
        sqlx::query(&sql).execute(&pool).await?;
        Ok(())
    }
}

async fn get_columns_for_table(
    pool: &MySqlPool,
    schema: &str,
    table: &str,
) -> Result<Vec<DbColumn>> {
    let rows = sqlx::query(
        r#"
        SELECT
            c.COLUMN_NAME, c.DATA_TYPE, c.IS_NULLABLE,
            c.COLUMN_DEFAULT, c.COLUMN_KEY, c.COLUMN_COMMENT
        FROM information_schema.COLUMNS c
        WHERE c.TABLE_SCHEMA = ? AND c.TABLE_NAME = ?
        ORDER BY c.ORDINAL_POSITION
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await?;

    // Fetch FK references for this table
    let fk_rows = sqlx::query(
        r#"
        SELECT
            kcu.COLUMN_NAME,
            kcu.REFERENCED_TABLE_NAME,
            kcu.REFERENCED_COLUMN_NAME
        FROM information_schema.KEY_COLUMN_USAGE kcu
        WHERE kcu.TABLE_SCHEMA = ?
          AND kcu.TABLE_NAME = ?
          AND kcu.REFERENCED_TABLE_NAME IS NOT NULL
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await?;

    let fk_map: std::collections::HashMap<String, ForeignKey> = fk_rows
        .iter()
        .map(|r| {
            let col: String = r.get("COLUMN_NAME");
            let ref_table: String = r.get("REFERENCED_TABLE_NAME");
            let ref_col: String = r.get("REFERENCED_COLUMN_NAME");
            (
                col,
                ForeignKey {
                    referenced_table: ref_table,
                    referenced_column: ref_col,
                },
            )
        })
        .collect();

    let mut columns = Vec::new();
    for row in &rows {
        let name: String = row.get("COLUMN_NAME");
        let data_type: String = row.get("DATA_TYPE");
        let is_nullable: String = row.get("IS_NULLABLE");
        let default_value: Option<String> = row.get("COLUMN_DEFAULT");
        let column_key: String = row.get("COLUMN_KEY");
        let comment: String = row.get("COLUMN_COMMENT");

        columns.push(DbColumn {
            name: name.clone(),
            data_type,
            is_nullable: is_nullable == "YES",
            default_value,
            is_primary_key: column_key == "PRI",
            foreign_key: fk_map.get(&name).cloned(),
            comment: if comment.is_empty() {
                None
            } else {
                Some(comment)
            },
        });
    }

    Ok(columns)
}

async fn get_indexes_for_table(
    pool: &MySqlPool,
    schema: &str,
    table: &str,
) -> Result<Vec<Index>> {
    let rows = sqlx::query(
        r#"
        SELECT
            INDEX_NAME,
            NON_UNIQUE,
            GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX) as COLUMNS
        FROM information_schema.STATISTICS
        WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
        GROUP BY INDEX_NAME, NON_UNIQUE
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await?;

    let mut indexes = Vec::new();
    for row in &rows {
        let name: String = row.get("INDEX_NAME");
        let non_unique: i64 = row.try_get::<i64, _>("NON_UNIQUE").unwrap_or(1);
        let columns_str: String = row.try_get::<String, _>("COLUMNS").unwrap_or_default();
        let columns: Vec<String> = columns_str.split(',').map(|s| s.to_string()).collect();

        indexes.push(Index {
            name: name.clone(),
            columns,
            is_unique: non_unique == 0,
            is_primary: name == "PRIMARY",
        });
    }

    Ok(indexes)
}

async fn get_constraints_for_table(
    pool: &MySqlPool,
    schema: &str,
    table: &str,
) -> Result<Vec<Constraint>> {
    let rows = sqlx::query(
        r#"
        SELECT CONSTRAINT_NAME, CONSTRAINT_TYPE
        FROM information_schema.TABLE_CONSTRAINTS
        WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await?;

    let mut constraints = Vec::new();
    for row in &rows {
        let name: String = row.get("CONSTRAINT_NAME");
        let ctype: String = row.get("CONSTRAINT_TYPE");

        let constraint_type = match ctype.as_str() {
            "PRIMARY KEY" => ConstraintType::PrimaryKey,
            "FOREIGN KEY" => ConstraintType::ForeignKey,
            "UNIQUE" => ConstraintType::Unique,
            "CHECK" => ConstraintType::Check,
            _ => continue,
        };

        constraints.push(Constraint {
            name,
            constraint_type,
        });
    }

    Ok(constraints)
}

