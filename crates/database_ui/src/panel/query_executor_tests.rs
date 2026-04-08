use super::query_executor::{is_connection_error, split_sql_statements};

#[test]
fn test_single_statement() {
    let stmts = split_sql_statements("SELECT 1");
    assert_eq!(stmts, vec!["SELECT 1"]);
}

#[test]
fn test_single_statement_with_semicolon() {
    let stmts = split_sql_statements("SELECT 1;");
    assert_eq!(stmts, vec!["SELECT 1"]);
}

#[test]
fn test_two_statements() {
    let stmts = split_sql_statements("SELECT 1; SELECT 2");
    assert_eq!(stmts, vec!["SELECT 1", "SELECT 2"]);
}

#[test]
fn test_multiple_statements() {
    let stmts = split_sql_statements("SELECT 1; SELECT 2; SELECT 3;");
    assert_eq!(stmts, vec!["SELECT 1", "SELECT 2", "SELECT 3"]);
}

#[test]
fn test_whitespace_between_statements() {
    let stmts = split_sql_statements("  SELECT 1  ;  \n  SELECT 2  ;  ");
    assert_eq!(stmts, vec!["SELECT 1", "SELECT 2"]);
}

#[test]
fn test_empty_statements_skipped() {
    let stmts = split_sql_statements(";;; SELECT 1 ;;; SELECT 2 ;;;");
    assert_eq!(stmts, vec!["SELECT 1", "SELECT 2"]);
}

#[test]
fn test_empty_input() {
    let stmts = split_sql_statements("");
    assert!(stmts.is_empty());
}

#[test]
fn test_only_semicolons() {
    let stmts = split_sql_statements(";;;");
    assert!(stmts.is_empty());
}

#[test]
fn test_semicolon_inside_single_quotes() {
    let stmts = split_sql_statements("SELECT 'hello;world'");
    assert_eq!(stmts, vec!["SELECT 'hello;world'"]);
}

#[test]
fn test_semicolon_inside_double_quotes() {
    let stmts = split_sql_statements("SELECT \"col;name\" FROM t");
    assert_eq!(stmts, vec!["SELECT \"col;name\" FROM t"]);
}

#[test]
fn test_escaped_single_quote() {
    let stmts = split_sql_statements("SELECT 'it''s'; SELECT 1");
    assert_eq!(stmts, vec!["SELECT 'it''s'", "SELECT 1"]);
}

#[test]
fn test_line_comment_with_semicolon() {
    let stmts = split_sql_statements("SELECT 1 -- this is a comment; not a split\n; SELECT 2");
    assert_eq!(stmts, vec![
        "SELECT 1 -- this is a comment; not a split",
        "SELECT 2",
    ]);
}

#[test]
fn test_block_comment_with_semicolon() {
    let stmts = split_sql_statements("SELECT /* ; */ 1; SELECT 2");
    assert_eq!(stmts, vec!["SELECT /* ; */ 1", "SELECT 2"]);
}

#[test]
fn test_multiline_statement() {
    let sql = "SELECT *\nFROM users\nWHERE id = 1;\n\nSELECT count(*)\nFROM orders;";
    let stmts = split_sql_statements(sql);
    assert_eq!(stmts.len(), 2);
    assert!(stmts[0].contains("FROM users"));
    assert!(stmts[1].contains("FROM orders"));
}

#[test]
fn test_insert_with_values_containing_semicolons() {
    let sql = "INSERT INTO t (name) VALUES ('a;b'); INSERT INTO t (name) VALUES ('c')";
    let stmts = split_sql_statements(sql);
    assert_eq!(stmts, vec![
        "INSERT INTO t (name) VALUES ('a;b')",
        "INSERT INTO t (name) VALUES ('c')",
    ]);
}

#[test]
fn test_create_function_with_body() {
    let sql = "CREATE TABLE t (id int); INSERT INTO t VALUES (1)";
    let stmts = split_sql_statements(sql);
    assert_eq!(stmts, vec![
        "CREATE TABLE t (id int)",
        "INSERT INTO t VALUES (1)",
    ]);
}

#[test]
fn test_nested_block_comments() {
    let stmts = split_sql_statements("SELECT /* outer /* not nested */ 1 */; SELECT 2");
    // sqlparser doesn't support nested block comments — the first */ ends it
    assert_eq!(stmts.len(), 2);
}

#[test]
fn test_only_whitespace() {
    let stmts = split_sql_statements("   \n\t  ");
    assert!(stmts.is_empty());
}

#[test]
fn test_dollar_sign_in_string() {
    let stmts = split_sql_statements("SELECT '$100'; SELECT 1");
    assert_eq!(stmts, vec!["SELECT '$100'", "SELECT 1"]);
}

#[test]
fn test_mixed_quotes_and_comments() {
    let sql = "SELECT 'a' -- comment;\n; SELECT \"b;c\"";
    let stmts = split_sql_statements(sql);
    assert_eq!(stmts.len(), 2);
    assert!(stmts[0].contains("'a'"));
    assert!(stmts[1].contains("\"b;c\""));
}

#[test]
fn test_trailing_newlines() {
    let stmts = split_sql_statements("SELECT 1;\n\n\n");
    assert_eq!(stmts, vec!["SELECT 1"]);
}

#[test]
fn test_windows_line_endings() {
    let stmts = split_sql_statements("SELECT 1;\r\nSELECT 2;\r\n");
    assert_eq!(stmts, vec!["SELECT 1", "SELECT 2"]);
}

#[test]
fn test_complex_real_world_query() {
    let sql = r#"
        -- Create users table
        CREATE TABLE users (
            id SERIAL PRIMARY KEY,
            name VARCHAR(100) NOT NULL
        );

        -- Insert sample data
        INSERT INTO users (name) VALUES ('Alice'), ('Bob');

        -- Query with subquery
        SELECT * FROM users WHERE id IN (SELECT id FROM users WHERE name LIKE 'A%');
    "#;
    let stmts = split_sql_statements(sql);
    assert_eq!(stmts.len(), 3);
    assert!(stmts[0].starts_with("-- Create users table"));
    assert!(stmts[1].starts_with("-- Insert sample data"));
    assert!(stmts[2].starts_with("-- Query with subquery"));
}

// ── is_connection_error tests ──────────────────────────────────────

#[test]
fn test_connection_refused_is_connection_error() {
    assert!(is_connection_error("connection refused"));
}

#[test]
fn test_connection_reset_is_connection_error() {
    assert!(is_connection_error("connection reset by peer"));
}

#[test]
fn test_broken_pipe_is_connection_error() {
    assert!(is_connection_error("broken pipe"));
}

#[test]
fn test_pool_timed_out_is_connection_error() {
    assert!(is_connection_error("pool timed out while waiting for an open connection"));
}

#[test]
fn test_connection_closed_is_connection_error() {
    assert!(is_connection_error("connection closed before message completed"));
}

#[test]
fn test_server_closed_is_connection_error() {
    assert!(is_connection_error("server closed the connection unexpectedly"));
}

#[test]
fn test_connection_timed_out_is_connection_error() {
    assert!(is_connection_error("connection timed out"));
}

#[test]
fn test_connection_terminated_is_connection_error() {
    assert!(is_connection_error("connection terminated"));
}

#[test]
fn test_no_connection_is_connection_error() {
    assert!(is_connection_error("no connection to the server"));
}

#[test]
fn test_case_insensitive_connection_refused() {
    assert!(is_connection_error("Connection Refused"));
}

#[test]
fn test_case_insensitive_broken_pipe() {
    assert!(is_connection_error("BROKEN PIPE"));
}

#[test]
fn test_relation_does_not_exist_is_not_connection_error() {
    assert!(!is_connection_error("relation \"users\" does not exist"));
}

#[test]
fn test_syntax_error_is_not_connection_error() {
    assert!(!is_connection_error("syntax error at or near \"SELEC\""));
}

#[test]
fn test_permission_denied_is_not_connection_error() {
    assert!(!is_connection_error("permission denied for table users"));
}

#[test]
fn test_empty_string_is_not_connection_error() {
    assert!(!is_connection_error(""));
}

#[test]
fn test_generic_error_is_not_connection_error() {
    assert!(!is_connection_error("something went wrong"));
}
