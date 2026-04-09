use crate::sql_diagnostics::{dialect_for_provider, parse_error_location, validate_columns};
use database_core::{DatabaseSchema, DbColumn, Table};
use sqlparser::parser::{Parser, ParserError};

fn parse_err(sql: &str) -> ParserError {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    Parser::parse_sql(&dialect, sql).unwrap_err()
}

fn col(name: &str, data_type: &str, nullable: bool, pk: bool) -> DbColumn {
    DbColumn {
        name: name.to_string(),
        data_type: data_type.to_string(),
        is_nullable: nullable,
        default_value: None,
        is_primary_key: pk,
        foreign_key: None,
        comment: None,
    }
}

fn test_schema() -> DatabaseSchema {
    DatabaseSchema {
        name: "testdb".to_string(),
        tables: vec![
            Table {
                schema: "public".to_string(),
                name: "users".to_string(),
                columns: vec![
                    col("id", "integer", false, true),
                    col("name", "text", false, false),
                    col("email", "text", true, false),
                    col("active", "boolean", false, false),
                ],
                indexes: Vec::new(),
                constraints: Vec::new(),
                row_count_estimate: None,
            },
            Table {
                schema: "public".to_string(),
                name: "orders".to_string(),
                columns: vec![
                    col("id", "integer", false, true),
                    col("user_id", "integer", false, false),
                    col("total", "numeric", false, false),
                ],
                indexes: Vec::new(),
                constraints: Vec::new(),
                row_count_estimate: None,
            },
        ],
        views: Vec::new(),
    }
}

// ── parse_error_location ────────────────────────────────────────────

#[test]
fn test_error_location_with_line_and_column() {
    let err = parse_err("SELECT * FROM users WHERE =");
    let (msg, line, col) = parse_error_location(&err, "SELECT * FROM users WHERE =");

    assert!(!msg.is_empty(), "msg should not be empty");
    assert_eq!(line, 0, "should be line 0 (first line)");
    assert!(col > 0, "should have a column offset");
}

#[test]
fn test_error_location_incomplete_statement() {
    let err = parse_err("SELECT * FROM");
    let (msg, _line, _col) = parse_error_location(&err, "SELECT * FROM");

    assert!(!msg.is_empty());
}

#[test]
fn test_error_location_multiline() {
    let sql = "SELECT *\nFROM users\nWHERE = 1";
    let err = parse_err(sql);
    let (msg, line, _col) = parse_error_location(&err, sql);

    assert!(!msg.is_empty());
    // Error should be on line 2 (0-indexed) where "WHERE = 1" is
    assert!(line >= 2, "error should be on line 2+, got {line}");
}

#[test]
fn test_error_message_cleaned() {
    let err = parse_err("SELECT * FROM users WHERE =");
    let (msg, _, _) = parse_error_location(&err, "SELECT * FROM users WHERE =");

    // Message should not contain " at Line:" suffix
    assert!(
        !msg.contains(" at Line:"),
        "message should be cleaned: {msg}"
    );
}

// ── sqlparser validation ────────────────────────────────────────────

#[test]
fn test_valid_select() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(&dialect, "SELECT 1").is_ok());
}

#[test]
fn test_valid_select_from() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(&dialect, "SELECT * FROM users").is_ok());
}

#[test]
fn test_valid_select_with_alias() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(
        Parser::parse_sql(&dialect, "SELECT u.id FROM users u WHERE u.active = true").is_ok()
    );
}

#[test]
fn test_valid_insert() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(
        Parser::parse_sql(&dialect, "INSERT INTO users (name) VALUES ('Alice')").is_ok()
    );
}

#[test]
fn test_valid_update() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(
        Parser::parse_sql(&dialect, "UPDATE users SET name = 'Bob' WHERE id = 1").is_ok()
    );
}

#[test]
fn test_valid_delete() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(&dialect, "DELETE FROM users WHERE id = 1").is_ok());
}

#[test]
fn test_valid_create_table() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(
        &dialect,
        "CREATE TABLE test (id SERIAL PRIMARY KEY, name TEXT NOT NULL)"
    )
    .is_ok());
}

#[test]
fn test_valid_join() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(
        &dialect,
        "SELECT u.name, o.total FROM users u JOIN orders o ON u.id = o.user_id"
    )
    .is_ok());
}

#[test]
fn test_valid_multiple_statements() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(&dialect, "SELECT 1; SELECT 2;").is_ok());
}

#[test]
fn test_invalid_syntax() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(&dialect, "SELEC * FROM users").is_err());
}

#[test]
fn test_invalid_missing_table() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(&dialect, "SELECT * FROM").is_err());
}

#[test]
fn test_invalid_dangling_operator() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(&dialect, "SELECT * FROM t ORDER BY ,").is_err());
}

#[test]
fn test_invalid_where_incomplete() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(&dialect, "SELECT * FROM t WHERE").is_err());
}

#[test]
fn test_invalid_unmatched_paren() {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(&dialect, "SELECT COUNT(* FROM t").is_err());
}

#[test]
fn test_empty_string_is_valid() {
    // Empty string parses as 0 statements, which is Ok
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    assert!(Parser::parse_sql(&dialect, "").is_ok());
}

// ── dialect_for_provider ───────────────────────────────────────────

#[test]
fn test_dialect_postgres() {
    let dialect = dialect_for_provider("postgres");
    assert!(Parser::parse_sql(dialect.as_ref(), "SELECT 1").is_ok());
}

#[test]
fn test_dialect_postgresql_alias() {
    let dialect = dialect_for_provider("postgresql");
    assert!(Parser::parse_sql(dialect.as_ref(), "SELECT 1").is_ok());
}

#[test]
fn test_dialect_mysql() {
    let dialect = dialect_for_provider("mysql");
    // MySQL backtick-quoted identifiers
    assert!(Parser::parse_sql(dialect.as_ref(), "SELECT `id` FROM `users`").is_ok());
}

#[test]
fn test_dialect_mariadb_alias() {
    let dialect = dialect_for_provider("mariadb");
    assert!(Parser::parse_sql(dialect.as_ref(), "SELECT `id` FROM `users`").is_ok());
}

#[test]
fn test_dialect_sqlite() {
    let dialect = dialect_for_provider("sqlite");
    assert!(Parser::parse_sql(dialect.as_ref(), "SELECT 1").is_ok());
}

#[test]
fn test_dialect_unknown_defaults_to_postgres() {
    let dialect = dialect_for_provider("oracle");
    assert!(Parser::parse_sql(dialect.as_ref(), "SELECT 1").is_ok());
}

// ── validate_columns ───────────────────────────────────────────────

#[test]
fn test_valid_column_with_alias() {
    let schema = test_schema();
    let sql = "SELECT u.id, u.name FROM users u";
    let entries = validate_columns(sql, &schema);
    assert!(entries.is_empty(), "valid columns should produce no diagnostics");
}

#[test]
fn test_invalid_column_with_alias() {
    let schema = test_schema();
    let sql = "SELECT u.xxxx FROM users u";
    let entries = validate_columns(sql, &schema);
    assert_eq!(entries.len(), 1, "should detect one invalid column");
    assert!(entries[0].diagnostic.message.contains("xxxx"));
    assert!(entries[0].diagnostic.message.contains("users"));
}

#[test]
fn test_invalid_column_in_where_clause() {
    let schema = test_schema();
    let sql = "SELECT * FROM users AS u WHERE u.nonexistent = 1";
    let entries = validate_columns(sql, &schema);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].diagnostic.message.contains("nonexistent"));
}

#[test]
fn test_multiple_invalid_columns() {
    let schema = test_schema();
    let sql = "SELECT u.foo, u.bar FROM users u";
    let entries = validate_columns(sql, &schema);
    assert_eq!(entries.len(), 2, "should detect two invalid columns");
}

#[test]
fn test_mixed_valid_and_invalid_columns() {
    let schema = test_schema();
    let sql = "SELECT u.id, u.missing FROM users u";
    let entries = validate_columns(sql, &schema);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].diagnostic.message.contains("missing"));
}

#[test]
fn test_valid_column_with_table_name() {
    let schema = test_schema();
    let sql = "SELECT users.id FROM users";
    let entries = validate_columns(sql, &schema);
    assert!(entries.is_empty(), "direct table.column should be valid");
}

#[test]
fn test_invalid_column_with_table_name() {
    let schema = test_schema();
    let sql = "SELECT users.nonexistent FROM users";
    let entries = validate_columns(sql, &schema);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].diagnostic.message.contains("nonexistent"));
}

#[test]
fn test_star_column_is_not_flagged() {
    let schema = test_schema();
    let sql = "SELECT u.* FROM users u";
    let entries = validate_columns(sql, &schema);
    assert!(entries.is_empty(), "u.* should not be flagged");
}

#[test]
fn test_schema_qualified_table_not_flagged() {
    let schema = test_schema();
    let sql = "SELECT * FROM public.users";
    let entries = validate_columns(sql, &schema);
    assert!(entries.is_empty(), "public.users should not flag 'users' as a bad column");
}

#[test]
fn test_unknown_table_not_flagged() {
    let schema = test_schema();
    let sql = "SELECT x.id FROM unknown_table x";
    let entries = validate_columns(sql, &schema);
    assert!(entries.is_empty(), "unknown table should not produce diagnostics");
}

#[test]
fn test_join_with_valid_columns() {
    let schema = test_schema();
    let sql = "SELECT u.name, o.total FROM users u JOIN orders o ON u.id = o.user_id";
    let entries = validate_columns(sql, &schema);
    assert!(entries.is_empty(), "valid join columns should produce no diagnostics");
}

#[test]
fn test_join_with_invalid_column() {
    let schema = test_schema();
    let sql = "SELECT u.name, o.bad_col FROM users u JOIN orders o ON u.id = o.user_id";
    let entries = validate_columns(sql, &schema);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].diagnostic.message.contains("bad_col"));
    assert!(entries[0].diagnostic.message.contains("orders"));
}

#[test]
fn test_multiline_column_validation() {
    let schema = test_schema();
    let sql = "SELECT\n  u.id,\n  u.missing_col\nFROM users u";
    let entries = validate_columns(sql, &schema);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].diagnostic.message.contains("missing_col"));
    // Should point to line 2 (0-indexed)
    assert_eq!(entries[0].range.start.row, 2);
}

#[test]
fn test_case_insensitive_column_validation() {
    let schema = test_schema();
    let sql = "SELECT u.ID, u.Name FROM users u";
    let entries = validate_columns(sql, &schema);
    assert!(entries.is_empty(), "column validation should be case-insensitive");
}

#[test]
fn test_diagnostic_range_points_to_column_name() {
    let schema = test_schema();
    let sql = "SELECT u.badcol FROM users u";
    let entries = validate_columns(sql, &schema);
    assert_eq!(entries.len(), 1);
    // "SELECT u.badcol" — dot is at position 8, column starts at 9
    assert_eq!(entries[0].range.start.column, 9);
    assert_eq!(entries[0].range.end.column, 15); // "badcol" is 6 chars
}

#[test]
fn test_quoted_table_name_with_invalid_column() {
    let schema = test_schema();
    let sql = "SELECT * FROM 'users' AS u WHERE u.xxx";
    let entries = validate_columns(sql, &schema);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].diagnostic.message.contains("xxx"));
    assert!(entries[0].diagnostic.message.contains("users"));
}

#[test]
fn test_double_quoted_table_name_with_valid_column() {
    let schema = test_schema();
    let sql = "SELECT a.id FROM \"users\" a";
    let entries = validate_columns(sql, &schema);
    assert!(entries.is_empty(), "valid column on double-quoted table should work");
}

#[test]
fn test_backtick_quoted_table_name_with_invalid_column() {
    let schema = test_schema();
    let sql = "SELECT a.nope FROM `users` a";
    let entries = validate_columns(sql, &schema);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].diagnostic.message.contains("nope"));
}
