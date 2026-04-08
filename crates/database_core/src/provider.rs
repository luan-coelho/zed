use anyhow::Result;
use async_trait::async_trait;

use crate::schema::{DatabaseSchema, DbColumn, QueryResult};

#[async_trait]
pub trait DatabaseProvider: Send + Sync {
    fn name(&self) -> &str;

    /// Test the connection using a pre-built connection URL.
    async fn test_connection(&self, url: &str) -> Result<()>;

    /// Execute a query using a pre-built connection URL.
    async fn execute_query(&self, url: &str, sql: &str) -> Result<QueryResult>;

    /// Get the full database schema using a pre-built connection URL.
    async fn get_schema(&self, url: &str) -> Result<DatabaseSchema>;

    /// Get columns for a specific table using a pre-built connection URL.
    async fn get_table_columns(&self, url: &str, table: &str) -> Result<Vec<DbColumn>>;

    /// Get estimated row count for a table using a pre-built connection URL.
    async fn get_table_row_count(&self, url: &str, table: &str) -> Result<i64>;

    /// Execute EXPLAIN on a query using a pre-built connection URL.
    async fn explain_query(&self, url: &str, sql: &str) -> Result<String>;

    /// Execute a query with a specific schema/search_path set.
    /// Default: ignores schema and calls execute_query.
    async fn execute_query_in_schema(
        &self,
        url: &str,
        sql: &str,
        _schema: &str,
    ) -> Result<QueryResult> {
        self.execute_query(url, sql).await
    }

    /// Execute EXPLAIN with a specific schema/search_path set.
    /// Default: ignores schema and calls explain_query.
    async fn explain_query_in_schema(
        &self,
        url: &str,
        sql: &str,
        _schema: &str,
    ) -> Result<String> {
        self.explain_query(url, sql).await
    }
}
