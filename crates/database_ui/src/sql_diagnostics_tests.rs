use crate::sql_diagnostics::parse_error_location;
use sqlparser::parser::{Parser, ParserError};

fn parse_err(sql: &str) -> ParserError {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    Parser::parse_sql(&dialect, sql).unwrap_err()
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
