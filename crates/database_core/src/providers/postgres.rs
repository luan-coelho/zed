use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Column as _, PgPool, Row};
use tokio::sync::RwLock;

use super::PoolManager;
use crate::provider::{
    is_read_query, DatabaseProvider, FormPlaceholders, ProviderCapabilities, ProviderMetadata,
};
use crate::schema::*;

pub struct PostgresProvider {
    pools: PoolManager<PgPool>,
    pinned_conn: RwLock<Option<sqlx::pool::PoolConnection<sqlx::Postgres>>>,
}

impl PostgresProvider {
    pub fn new() -> Self {
        Self {
            pools: PoolManager::new(),
            pinned_conn: RwLock::new(None),
        }
    }

    async fn ensure_pool(&self, url: &str) -> Result<PgPool> {
        self.pools
            .get_or_create(url, |url| async move {
                PgPoolOptions::new()
                    .max_connections(5)
                    .acquire_timeout(std::time::Duration::from_secs(10))
                    .connect(&url)
                    .await
                    .context("failed to connect to PostgreSQL")
            })
            .await
    }
}

impl ProviderMetadata for PostgresProvider {
    fn id(&self) -> &'static str { "postgres" }
    fn display_name(&self) -> &'static str { "PostgreSQL" }
    fn default_port(&self) -> u16 { 5432 }
    fn default_user(&self) -> &'static str { "postgres" }
    fn url_scheme(&self) -> &'static str { "postgres" }
    fn aliases(&self) -> &'static [&'static str] { &["postgresql"] }
    fn is_file_based(&self) -> bool { false }
    fn form_placeholders(&self) -> FormPlaceholders {
        FormPlaceholders {
            name: "my_connection",
            host: "localhost",
            port: "5432",
            database: "mydb",
            database_label: "Database",
            user: "postgres",
            password: "stored in OS keychain",
        }
    }
}

#[async_trait]
impl DatabaseProvider for PostgresProvider {
    fn metadata(&self) -> &dyn ProviderMetadata {
        self
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            multi_database: true,
            schemas: true,
            roles: true,
            users: false,
            create_database: true,
            create_schema: true,
            create_role: true,
            requires_reconnect_for_database_switch: true,
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
                result.columns = first_row
                    .columns()
                    .iter()
                    .map(|c| c.name().to_string())
                    .collect();
            }

            for row in &rows {
                let mut row_values = Vec::new();
                for i in 0..row.columns().len() {
                    let value = get_column_value_as_string(row, i);
                    row_values.push(value);
                }
                result.rows.push(row_values);
            }

            Ok(result)
        } else {
            let pg_result = sqlx::query(sql).execute(&pool).await?;
            let elapsed = start.elapsed().as_millis() as u64;

            Ok(QueryResult {
                columns: vec!["rows_affected".to_string()],
                rows: vec![vec![pg_result.rows_affected().to_string()]],
                rows_affected: pg_result.rows_affected(),
                execution_time_ms: elapsed,
            })
        }
    }

    async fn get_schema(&self, url: &str) -> Result<DatabaseSchema> {
        let pool = self.ensure_pool(url).await?;

        let table_rows = sqlx::query(
            r#"
            SELECT t.table_schema, t.table_name
            FROM information_schema.tables t
            WHERE t.table_schema NOT IN ('pg_catalog', 'information_schema')
              AND t.table_type = 'BASE TABLE'
            ORDER BY t.table_schema, t.table_name
            "#,
        )
        .fetch_all(&pool)
        .await?;

        let mut tables = Vec::new();
        for row in &table_rows {
            let schema: String = row.get("table_schema");
            let name: String = row.get("table_name");

            let columns = get_columns_for_table(&pool, &schema, &name).await?;
            let indexes = get_indexes_for_table(&pool, &schema, &name).await?;
            let constraints = get_constraints_for_table(&pool, &schema, &name).await?;

            tables.push(Table {
                schema,
                name,
                columns,
                indexes,
                constraints,
                row_count_estimate: None,
            });
        }

        let view_rows = sqlx::query(
            r#"
            SELECT table_schema, table_name
            FROM information_schema.views
            WHERE table_schema NOT IN ('pg_catalog', 'information_schema')
            ORDER BY table_schema, table_name
            "#,
        )
        .fetch_all(&pool)
        .await?;

        let mut views = Vec::new();
        for row in &view_rows {
            let schema: String = row.get("table_schema");
            let name: String = row.get("table_name");
            let columns = get_columns_for_table(&pool, &schema, &name).await?;
            views.push(View {
                schema,
                name,
                columns,
            });
        }

        let db_name: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&pool)
            .await?;

        Ok(DatabaseSchema {
            name: db_name,
            tables,
            views,
        })
    }

    async fn get_table_columns(&self, url: &str, table: &str) -> Result<Vec<DbColumn>> {
        let pool = self.ensure_pool(url).await?;
        get_columns_for_table(&pool, "public", table).await
    }

    async fn get_table_row_count(&self, url: &str, table: &str) -> Result<i64> {
        let pool = self.ensure_pool(url).await?;
        let count: i64 =
            sqlx::query_scalar("SELECT reltuples::bigint FROM pg_class WHERE relname = $1")
                .bind(table)
                .fetch_one(&pool)
                .await?;
        Ok(count)
    }

    async fn explain_query(&self, url: &str, sql: &str) -> Result<String> {
        let pool = self.ensure_pool(url).await?;
        let explain_sql = format!("EXPLAIN (ANALYZE, FORMAT TEXT) {sql}");
        let rows = sqlx::query(&explain_sql).fetch_all(&pool).await?;

        let mut plan = String::new();
        for row in &rows {
            let line: String = row.get(0);
            plan.push_str(&line);
            plan.push('\n');
        }

        Ok(plan)
    }

    async fn execute_query_in_schema(
        &self,
        url: &str,
        sql: &str,
        schema: &str,
    ) -> Result<QueryResult> {
        let pool = self.ensure_pool(url).await?;
        let mut conn = pool.acquire().await?;

        // Set search_path for this connection
        let set_path = format!("SET search_path TO \"{}\"", schema);
        sqlx::query(&set_path)
            .execute(&mut *conn)
            .await
            .context("failed to set search_path")?;

        let start = Instant::now();
        if is_read_query(sql) {
            let rows = sqlx::query(sql).fetch_all(&mut *conn).await?;
            let elapsed = start.elapsed().as_millis() as u64;

            let mut result = QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                rows_affected: rows.len() as u64,
                execution_time_ms: elapsed,
            };

            if let Some(first_row) = rows.first() {
                result.columns = first_row
                    .columns()
                    .iter()
                    .map(|c| c.name().to_string())
                    .collect();
            }

            for row in &rows {
                let mut row_values = Vec::new();
                for i in 0..row.columns().len() {
                    let value = get_column_value_as_string(row, i);
                    row_values.push(value);
                }
                result.rows.push(row_values);
            }

            Ok(result)
        } else {
            let pg_result = sqlx::query(sql).execute(&mut *conn).await?;
            let elapsed = start.elapsed().as_millis() as u64;

            Ok(QueryResult {
                columns: vec!["rows_affected".to_string()],
                rows: vec![vec![pg_result.rows_affected().to_string()]],
                rows_affected: pg_result.rows_affected(),
                execution_time_ms: elapsed,
            })
        }
    }

    async fn explain_query_in_schema(
        &self,
        url: &str,
        sql: &str,
        schema: &str,
    ) -> Result<String> {
        let pool = self.ensure_pool(url).await?;
        let mut conn = pool.acquire().await?;

        let set_path = format!("SET search_path TO \"{}\"", schema);
        sqlx::query(&set_path)
            .execute(&mut *conn)
            .await
            .context("failed to set search_path")?;

        let explain_sql = format!("EXPLAIN (ANALYZE, FORMAT TEXT) {sql}");
        let rows = sqlx::query(&explain_sql).fetch_all(&mut *conn).await?;

        let mut plan = String::new();
        for row in &rows {
            let line: String = row.get(0);
            plan.push_str(&line);
            plan.push('\n');
        }

        Ok(plan)
    }

    async fn list_databases(&self, url: &str) -> Result<Vec<DatabaseEntry>> {
        let pool = self.ensure_pool(url).await?;
        let current_db: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&pool)
            .await?;

        let rows = sqlx::query(
            "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
        )
        .fetch_all(&pool)
        .await?;

        Ok(rows
            .iter()
            .map(|row| {
                let name: String = row.get("datname");
                DatabaseEntry {
                    is_current: name == current_db,
                    name,
                }
            })
            .collect())
    }

    async fn list_schemas(&self, url: &str) -> Result<Vec<SchemaEntry>> {
        let pool = self.ensure_pool(url).await?;
        let rows = sqlx::query(
            "SELECT schema_name FROM information_schema.schemata ORDER BY schema_name",
        )
        .fetch_all(&pool)
        .await?;

        Ok(rows
            .iter()
            .map(|row| {
                let name: String = row.get("schema_name");
                SchemaEntry {
                    is_system: matches!(
                        name.as_str(),
                        "pg_catalog" | "information_schema" | "pg_toast"
                    ),
                    name,
                }
            })
            .collect())
    }

    async fn list_roles(&self, url: &str) -> Result<Vec<RoleEntry>> {
        let pool = self.ensure_pool(url).await?;
        let rows = sqlx::query(
            "SELECT rolname, rolsuper, rolcreaterole, rolcreatedb, rolcanlogin \
             FROM pg_roles WHERE rolname NOT LIKE 'pg_%' ORDER BY rolname",
        )
        .fetch_all(&pool)
        .await?;

        Ok(rows
            .iter()
            .map(|row| RoleEntry {
                name: row.get("rolname"),
                is_superuser: row.get("rolsuper"),
                can_login: row.get("rolcanlogin"),
                can_create_db: row.get("rolcreatedb"),
                can_create_role: row.get("rolcreaterole"),
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
        let sql = format!("CREATE DATABASE \"{}\"", name.replace('"', "\"\""));
        sqlx::query(&sql).execute(&pool).await?;
        Ok(())
    }

    async fn create_schema(&self, url: &str, name: &str) -> Result<()> {
        let pool = self.ensure_pool(url).await?;
        let sql = format!("CREATE SCHEMA \"{}\"", name.replace('"', "\"\""));
        sqlx::query(&sql).execute(&pool).await?;
        Ok(())
    }

    async fn create_role(&self, url: &str, name: &str) -> Result<()> {
        let pool = self.ensure_pool(url).await?;
        let sql = format!("CREATE ROLE \"{}\"", name.replace('"', "\"\""));
        sqlx::query(&sql).execute(&pool).await?;
        Ok(())
    }

    async fn begin_transaction(&self, url: &str, isolation: Option<&str>) -> Result<()> {
        let pool = self.ensure_pool(url).await?;
        let mut conn = pool.acquire().await.context("failed to acquire connection for transaction")?;

        if let Some(level) = isolation {
            let set_isolation = format!("SET TRANSACTION ISOLATION LEVEL {level}");
            sqlx::query(&set_isolation).execute(&mut *conn).await?;
        }
        sqlx::query("BEGIN").execute(&mut *conn).await?;

        let mut guard = self.pinned_conn.write().await;
        *guard = Some(conn);
        Ok(())
    }

    async fn commit_transaction(&self, _url: &str) -> Result<()> {
        let mut guard = self.pinned_conn.write().await;
        let mut conn = guard.take().ok_or_else(|| anyhow::anyhow!("No active transaction"))?;
        sqlx::query("COMMIT").execute(&mut *conn).await?;
        Ok(())
    }

    async fn rollback_transaction(&self, _url: &str) -> Result<()> {
        let mut guard = self.pinned_conn.write().await;
        let mut conn = guard.take().ok_or_else(|| anyhow::anyhow!("No active transaction"))?;
        sqlx::query("ROLLBACK").execute(&mut *conn).await?;
        Ok(())
    }

    async fn execute_in_transaction(&self, _url: &str, sql: &str) -> Result<QueryResult> {
        let mut guard = self.pinned_conn.write().await;
        let conn = guard.as_mut().ok_or_else(|| anyhow::anyhow!("No active transaction"))?;

        let start = Instant::now();

        if is_read_query(sql) {
            let rows = sqlx::query(sql).fetch_all(&mut **conn).await?;
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
                    row_values.push(get_column_value_as_string(row, i));
                }
                result.rows.push(row_values);
            }
            Ok(result)
        } else {
            let pg_result = sqlx::query(sql).execute(&mut **conn).await?;
            let elapsed = start.elapsed().as_millis() as u64;
            Ok(QueryResult {
                columns: vec!["rows_affected".to_string()],
                rows: vec![vec![pg_result.rows_affected().to_string()]],
                rows_affected: pg_result.rows_affected(),
                execution_time_ms: elapsed,
            })
        }
    }

    async fn has_active_transaction(&self, _url: &str) -> bool {
        self.pinned_conn.read().await.is_some()
    }
}

fn get_column_value_as_string(row: &sqlx::postgres::PgRow, index: usize) -> String {
    row.try_get::<String, _>(index)
        .or_else(|_| row.try_get::<i32, _>(index).map(|v| v.to_string()))
        .or_else(|_| row.try_get::<i64, _>(index).map(|v| v.to_string()))
        .or_else(|_| row.try_get::<i16, _>(index).map(|v| v.to_string()))
        .or_else(|_| row.try_get::<f64, _>(index).map(|v| v.to_string()))
        .or_else(|_| row.try_get::<f32, _>(index).map(|v| v.to_string()))
        .or_else(|_| row.try_get::<bool, _>(index).map(|v| v.to_string()))
        .or_else(|_| {
            row.try_get::<chrono::NaiveDateTime, _>(index)
                .map(|v| v.to_string())
        })
        .or_else(|_| {
            row.try_get::<chrono::NaiveDate, _>(index)
                .map(|v| v.to_string())
        })
        .or_else(|_| {
            row.try_get::<chrono::DateTime<chrono::Utc>, _>(index)
                .map(|v| v.to_string())
        })
        .or_else(|_| {
            row.try_get::<serde_json::Value, _>(index)
                .map(|v| v.to_string())
        })
        .or_else(|_| {
            row.try_get::<uuid::Uuid, _>(index)
                .map(|v| v.to_string())
        })
        .unwrap_or_else(|_| "NULL".to_string())
}

async fn get_columns_for_table(
    pool: &PgPool,
    schema: &str,
    table: &str,
) -> Result<Vec<DbColumn>> {
    let rows = sqlx::query(
        r#"
        SELECT
            c.column_name,
            c.data_type,
            c.is_nullable,
            c.column_default,
            COALESCE(
                (SELECT true FROM information_schema.table_constraints tc
                 JOIN information_schema.key_column_usage kcu
                   ON tc.constraint_name = kcu.constraint_name
                  AND tc.table_schema = kcu.table_schema
                 WHERE tc.constraint_type = 'PRIMARY KEY'
                   AND tc.table_schema = c.table_schema
                   AND tc.table_name = c.table_name
                   AND kcu.column_name = c.column_name
                 LIMIT 1),
                false
            ) as is_pk,
            (SELECT ccu.table_name || '.' || ccu.column_name
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
              AND tc.table_schema = kcu.table_schema
             JOIN information_schema.constraint_column_usage ccu
               ON tc.constraint_name = ccu.constraint_name
              AND tc.table_schema = ccu.table_schema
             WHERE tc.constraint_type = 'FOREIGN KEY'
               AND tc.table_schema = c.table_schema
               AND tc.table_name = c.table_name
               AND kcu.column_name = c.column_name
             LIMIT 1
            ) as fk_ref,
            pgd.description as column_comment
        FROM information_schema.columns c
        LEFT JOIN pg_catalog.pg_statio_all_tables st
          ON c.table_schema = st.schemaname AND c.table_name = st.relname
        LEFT JOIN pg_catalog.pg_description pgd
          ON pgd.objoid = st.relid AND pgd.objsubid = c.ordinal_position
        WHERE c.table_schema = $1 AND c.table_name = $2
        ORDER BY c.ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await?;

    let mut columns = Vec::new();
    for row in &rows {
        let name: String = row.get("column_name");
        let data_type: String = row.get("data_type");
        let is_nullable: String = row.get("is_nullable");
        let default_value: Option<String> = row.get("column_default");
        let is_pk: bool = row.get("is_pk");
        let fk_ref: Option<String> = row.get("fk_ref");
        let comment: Option<String> = row.get("column_comment");

        let foreign_key = fk_ref.and_then(|ref_str| {
            let parts: Vec<&str> = ref_str.splitn(2, '.').collect();
            if parts.len() == 2 {
                Some(ForeignKey {
                    referenced_table: parts[0].to_string(),
                    referenced_column: parts[1].to_string(),
                })
            } else {
                None
            }
        });

        columns.push(DbColumn {
            name,
            data_type,
            is_nullable: is_nullable == "YES",
            default_value,
            is_primary_key: is_pk,
            foreign_key,
            comment,
        });
    }

    Ok(columns)
}

async fn get_indexes_for_table(pool: &PgPool, schema: &str, table: &str) -> Result<Vec<Index>> {
    let rows = sqlx::query(
        r#"
        SELECT
            i.relname as index_name,
            ix.indisunique as is_unique,
            ix.indisprimary as is_primary,
            array_agg(a.attname ORDER BY array_position(ix.indkey, a.attnum)) as columns
        FROM pg_index ix
        JOIN pg_class t ON t.oid = ix.indrelid
        JOIN pg_class i ON i.oid = ix.indexrelid
        JOIN pg_namespace n ON n.oid = t.relnamespace
        JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey)
        WHERE n.nspname = $1 AND t.relname = $2
        GROUP BY i.relname, ix.indisunique, ix.indisprimary
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await?;

    let mut indexes = Vec::new();
    for row in &rows {
        indexes.push(Index {
            name: row.get("index_name"),
            columns: row.get("columns"),
            is_unique: row.get("is_unique"),
            is_primary: row.get("is_primary"),
        });
    }

    Ok(indexes)
}

async fn get_constraints_for_table(
    pool: &PgPool,
    schema: &str,
    table: &str,
) -> Result<Vec<Constraint>> {
    let rows = sqlx::query(
        r#"
        SELECT constraint_name, constraint_type
        FROM information_schema.table_constraints
        WHERE table_schema = $1 AND table_name = $2
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await?;

    let mut constraints = Vec::new();
    for row in &rows {
        let name: String = row.get("constraint_name");
        let ctype: String = row.get("constraint_type");

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
