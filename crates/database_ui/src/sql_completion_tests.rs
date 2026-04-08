use crate::sql_completion::{
    detect_sql_context, extract_word_before_cursor, is_sql_keyword, parse_aliases, SqlContext,
};

// ── detect_sql_context ──────────────────────────────────────────────

#[test]
fn test_context_after_from() {
    let text = "SELECT * FROM ";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterFrom
    ));
}

#[test]
fn test_context_after_join() {
    let text = "SELECT * FROM users JOIN ";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterJoin
    ));
}

#[test]
fn test_context_after_update() {
    let text = "UPDATE ";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterUpdate
    ));
}

#[test]
fn test_context_after_into() {
    let text = "INSERT INTO ";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterInto
    ));
}

#[test]
fn test_context_after_dot() {
    let text = "SELECT u.";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterDot(ref name) if name == "u"
    ));
}

#[test]
fn test_context_after_dot_full_table() {
    let text = "SELECT users.";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterDot(ref name) if name == "users"
    ));
}

#[test]
fn test_context_general_after_select() {
    let text = "SELECT ";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::General
    ));
}

#[test]
fn test_context_general_after_where() {
    let text = "SELECT * FROM users WHERE ";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::General
    ));
}

#[test]
fn test_context_after_table_keyword() {
    let text = "DROP TABLE ";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterFrom
    ));
}

// ── extract_word_before_cursor ──────────────────────────────────────

#[test]
fn test_extract_word_simple() {
    assert_eq!(extract_word_before_cursor("SELECT use", 10), "use");
}

#[test]
fn test_extract_word_after_space() {
    assert_eq!(extract_word_before_cursor("FROM ", 5), "");
}

#[test]
fn test_extract_word_with_underscore() {
    assert_eq!(
        extract_word_before_cursor("SELECT user_na", 14),
        "user_na"
    );
}

#[test]
fn test_extract_word_after_dot() {
    assert_eq!(extract_word_before_cursor("u.nam", 5), "nam");
}

#[test]
fn test_extract_word_start_of_text() {
    assert_eq!(extract_word_before_cursor("SEL", 3), "SEL");
}

// ── parse_aliases ───────────────────────────────────────────────────

#[test]
fn test_alias_simple() {
    let aliases = parse_aliases("SELECT * FROM users u");
    assert_eq!(aliases.get("u"), Some(&"users".to_string()));
}

#[test]
fn test_alias_with_as() {
    let aliases = parse_aliases("SELECT * FROM users AS u");
    assert_eq!(aliases.get("u"), Some(&"users".to_string()));
}

#[test]
fn test_alias_multiple_tables() {
    let aliases = parse_aliases("SELECT * FROM users u JOIN accounts a ON u.id = a.user_id");
    assert_eq!(aliases.get("u"), Some(&"users".to_string()));
    assert_eq!(aliases.get("a"), Some(&"accounts".to_string()));
}

#[test]
fn test_alias_left_join() {
    let aliases =
        parse_aliases("SELECT * FROM orders o LEFT JOIN customers c ON o.customer_id = c.id");
    assert_eq!(aliases.get("o"), Some(&"orders".to_string()));
    assert_eq!(aliases.get("c"), Some(&"customers".to_string()));
}

#[test]
fn test_alias_inner_join() {
    let aliases =
        parse_aliases("SELECT * FROM products p INNER JOIN categories cat ON p.cat_id = cat.id");
    assert_eq!(aliases.get("p"), Some(&"products".to_string()));
    assert_eq!(aliases.get("cat"), Some(&"categories".to_string()));
}

#[test]
fn test_alias_no_alias() {
    let aliases = parse_aliases("SELECT * FROM users WHERE id = 1");
    // "WHERE" should not be interpreted as an alias
    assert!(!aliases.contains_key("where"));
}

#[test]
fn test_alias_case_insensitive() {
    let aliases = parse_aliases("SELECT * FROM Users U");
    assert_eq!(aliases.get("u"), Some(&"users".to_string()));
}

#[test]
fn test_alias_update() {
    let aliases = parse_aliases("UPDATE users u SET u.name = 'test'");
    assert_eq!(aliases.get("u"), Some(&"users".to_string()));
}

#[test]
fn test_alias_empty_query() {
    let aliases = parse_aliases("");
    assert!(aliases.is_empty());
}

#[test]
fn test_alias_no_from() {
    let aliases = parse_aliases("SELECT 1");
    assert!(aliases.is_empty());
}

// ── is_sql_keyword ──────────────────────────────────────────────────

#[test]
fn test_keyword_detection() {
    assert!(is_sql_keyword("SELECT"));
    assert!(is_sql_keyword("FROM"));
    assert!(is_sql_keyword("WHERE"));
    assert!(is_sql_keyword("JOIN"));
    assert!(is_sql_keyword("ON"));
    assert!(is_sql_keyword("AS"));
    assert!(is_sql_keyword("GROUP"));
    assert!(is_sql_keyword("ORDER"));
    assert!(is_sql_keyword("LIMIT"));
}

#[test]
fn test_non_keywords() {
    assert!(!is_sql_keyword("users"));
    assert!(!is_sql_keyword("u"));
    assert!(!is_sql_keyword("my_table"));
    assert!(!is_sql_keyword("id"));
}
