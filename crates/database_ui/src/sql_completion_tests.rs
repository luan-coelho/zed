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

// ── detect_sql_context for schema-qualified queries ────────────────

#[test]
fn test_context_after_dot_schema_public() {
    let text = "SELECT * FROM public.";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterDot(ref name) if name == "public"
    ));
}

#[test]
fn test_context_after_dot_schema_audit() {
    let text = "SELECT * FROM audit.";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterDot(ref name) if name == "audit"
    ));
}

#[test]
fn test_context_after_dot_schema_in_join() {
    let text = "SELECT * FROM users JOIN audit.";
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterDot(ref name) if name == "audit"
    ));
}

#[test]
fn test_context_after_dot_schema_with_partial_table() {
    let text = "SELECT * FROM public.us";
    // Cursor is at end; the dot is not at the end, so context should not be AfterDot.
    // The word before cursor is "us", and the last relevant keyword is FROM.
    // Actually, detect_sql_context checks if before ends with '.', which it doesn't here.
    // So this should be AfterFrom since FROM is the last keyword token.
    assert!(matches!(
        detect_sql_context(text, text.len()),
        SqlContext::AfterFrom
    ));
}

// ── build_completions tests (schema-aware) ─────────────────────────

use crate::sql_completion::SqlCompletionProvider;
use database_core::{DatabaseSchema, DbColumn, Table, View};
use std::sync::Arc;
use tokio::sync::RwLock;

fn make_test_column(name: &str, data_type: &str) -> DbColumn {
    DbColumn {
        name: name.to_string(),
        data_type: data_type.to_string(),
        is_nullable: true,
        default_value: None,
        is_primary_key: false,
        foreign_key: None,
        comment: None,
    }
}

fn make_test_schema() -> DatabaseSchema {
    DatabaseSchema {
        name: "test_db".to_string(),
        tables: vec![
            Table {
                schema: "public".to_string(),
                name: "users".to_string(),
                columns: vec![
                    make_test_column("id", "integer"),
                    make_test_column("name", "text"),
                    make_test_column("email", "text"),
                ],
                indexes: vec![],
                constraints: vec![],
                row_count_estimate: None,
            },
            Table {
                schema: "public".to_string(),
                name: "orders".to_string(),
                columns: vec![
                    make_test_column("id", "integer"),
                    make_test_column("user_id", "integer"),
                    make_test_column("total", "numeric"),
                ],
                indexes: vec![],
                constraints: vec![],
                row_count_estimate: None,
            },
            Table {
                schema: "audit".to_string(),
                name: "logs".to_string(),
                columns: vec![
                    make_test_column("id", "integer"),
                    make_test_column("action", "text"),
                    make_test_column("timestamp", "timestamptz"),
                ],
                indexes: vec![],
                constraints: vec![],
                row_count_estimate: None,
            },
            Table {
                schema: "audit".to_string(),
                name: "changes".to_string(),
                columns: vec![
                    make_test_column("id", "integer"),
                    make_test_column("table_name", "text"),
                    make_test_column("old_value", "jsonb"),
                ],
                indexes: vec![],
                constraints: vec![],
                row_count_estimate: None,
            },
        ],
        views: vec![View {
            schema: "public".to_string(),
            name: "active_users".to_string(),
            columns: vec![
                make_test_column("id", "integer"),
                make_test_column("name", "text"),
            ],
        }],
    }
}

fn make_test_provider(
    schema: DatabaseSchema,
    active_schema: Option<String>,
) -> SqlCompletionProvider {
    SqlCompletionProvider::new(
        Arc::new(RwLock::new(Some(schema))),
        Arc::new(RwLock::new(active_schema)),
    )
}

fn dummy_replace_range() -> std::ops::Range<language::Anchor> {
    let buffer_id = text::BufferId::new(1).unwrap();
    let anchor = text::Anchor::min_for_buffer(buffer_id);
    anchor..anchor
}

fn completion_labels(completions: &[project::Completion]) -> Vec<String> {
    completions
        .iter()
        .map(|c| c.new_text.clone())
        .collect()
}

// ── AfterDot: schema name resolves to tables in that schema ────────

#[test]
fn test_after_dot_schema_suggests_tables_from_public() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT * FROM public.";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"users".to_string()));
    assert!(labels.contains(&"orders".to_string()));
    assert!(labels.contains(&"active_users".to_string()));
    // Should NOT contain tables from the audit schema
    assert!(!labels.contains(&"logs".to_string()));
    assert!(!labels.contains(&"changes".to_string()));
}

#[test]
fn test_after_dot_schema_suggests_tables_from_audit() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT * FROM audit.";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"logs".to_string()));
    assert!(labels.contains(&"changes".to_string()));
    // Should NOT contain tables from the public schema
    assert!(!labels.contains(&"users".to_string()));
    assert!(!labels.contains(&"orders".to_string()));
    assert!(!labels.contains(&"active_users".to_string()));
}

#[test]
fn test_after_dot_schema_filters_by_prefix() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT * FROM audit.lo";
    let completions = provider.build_completions("lo", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"logs".to_string()));
    assert!(!labels.contains(&"changes".to_string()));
}

#[test]
fn test_after_dot_table_name_suggests_columns() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT users.";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    // "users" is a table name, not a schema name, so we get columns
    assert!(labels.contains(&"id".to_string()));
    assert!(labels.contains(&"name".to_string()));
    assert!(labels.contains(&"email".to_string()));
    // Should NOT contain table names
    assert!(!labels.contains(&"users".to_string()));
    assert!(!labels.contains(&"orders".to_string()));
}

#[test]
fn test_after_dot_table_name_suggests_columns_with_active_schema() {
    let provider = make_test_provider(make_test_schema(), Some("public".to_string()));
    let text = "SELECT users.";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"id".to_string()));
    assert!(labels.contains(&"name".to_string()));
    assert!(labels.contains(&"email".to_string()));
}

#[test]
fn test_after_dot_unknown_name_gives_no_completions() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT nonexistent.";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());

    assert!(completions.is_empty());
}

#[test]
fn test_after_dot_alias_resolves_to_table_columns() {
    let provider = make_test_provider(make_test_schema(), Some("public".to_string()));
    let text = "SELECT u. FROM users u";
    // Cursor at position 9 (after "u.")
    let offset = "SELECT u.".len();
    let completions = provider.build_completions("", text, offset, dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"id".to_string()));
    assert!(labels.contains(&"name".to_string()));
    assert!(labels.contains(&"email".to_string()));
}

// ── AfterFrom: schema names appear as completions ──────────────────

#[test]
fn test_after_from_includes_schema_names() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT * FROM ";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"public".to_string()));
    assert!(labels.contains(&"audit".to_string()));
}

#[test]
fn test_after_from_schema_name_filtered_by_prefix() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT * FROM pub";
    let completions = provider.build_completions("pub", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"public".to_string()));
    assert!(!labels.contains(&"audit".to_string()));
}

#[test]
fn test_after_from_includes_tables_and_schemas() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT * FROM ";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    // Both schemas and tables should be present
    assert!(labels.contains(&"public".to_string()));
    assert!(labels.contains(&"audit".to_string()));
    assert!(labels.contains(&"users".to_string()));
    assert!(labels.contains(&"orders".to_string()));
    assert!(labels.contains(&"logs".to_string()));
    assert!(labels.contains(&"changes".to_string()));
    assert!(labels.contains(&"active_users".to_string()));
}

#[test]
fn test_after_from_with_active_schema_filters_tables() {
    let provider = make_test_provider(make_test_schema(), Some("public".to_string()));
    let text = "SELECT * FROM ";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    // Schema names should still all be present (they are unfiltered)
    assert!(labels.contains(&"public".to_string()));
    assert!(labels.contains(&"audit".to_string()));
    // Only public-schema tables should appear (active schema filter)
    assert!(labels.contains(&"users".to_string()));
    assert!(labels.contains(&"orders".to_string()));
    assert!(labels.contains(&"active_users".to_string()));
    // audit-schema tables should be filtered out by active_schema
    assert!(!labels.contains(&"logs".to_string()));
    assert!(!labels.contains(&"changes".to_string()));
}

// ── AfterJoin: schema names appear as completions ──────────────────

#[test]
fn test_after_join_includes_schema_names() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT * FROM users JOIN ";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"public".to_string()));
    assert!(labels.contains(&"audit".to_string()));
}

#[test]
fn test_after_join_includes_tables_and_schemas() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT * FROM users JOIN ";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"public".to_string()));
    assert!(labels.contains(&"audit".to_string()));
    assert!(labels.contains(&"orders".to_string()));
    assert!(labels.contains(&"logs".to_string()));
}

// ── General context: schema names appear as completions ────────────

#[test]
fn test_general_context_includes_schema_names() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT ";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"public".to_string()));
    assert!(labels.contains(&"audit".to_string()));
}

#[test]
fn test_general_context_schema_filtered_by_prefix() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT aud";
    let completions = provider.build_completions("aud", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"audit".to_string()));
    assert!(!labels.contains(&"public".to_string()));
}

#[test]
fn test_general_context_includes_keywords_tables_and_schemas() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT ";
    let completions = provider.build_completions("", text, text.len(), dummy_replace_range());
    let labels = completion_labels(&completions);

    // Keywords
    assert!(labels.contains(&"SELECT".to_string()));
    assert!(labels.contains(&"FROM".to_string()));
    // Schemas
    assert!(labels.contains(&"public".to_string()));
    assert!(labels.contains(&"audit".to_string()));
    // Tables
    assert!(labels.contains(&"users".to_string()));
    assert!(labels.contains(&"orders".to_string()));
}

// ── Schema-qualified alias resolution ──────────────────────────────

#[test]
fn test_alias_with_schema_qualified_table() {
    let aliases = parse_aliases("SELECT * FROM audit.action_plan_aud AS a WHERE");
    assert_eq!(
        aliases.get("a").map(|s| s.as_str()),
        Some("audit.action_plan_aud")
    );
}

#[test]
fn test_alias_with_schema_qualified_implicit() {
    let aliases = parse_aliases("SELECT * FROM audit.logs l WHERE");
    assert_eq!(
        aliases.get("l").map(|s| s.as_str()),
        Some("audit.logs")
    );
}

#[test]
fn test_after_dot_alias_resolves_schema_qualified_columns() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT * FROM audit.logs AS a WHERE a.";
    let offset = text.len();
    let range = dummy_replace_range();
    let completions = provider.build_completions("", text, offset, range);
    let labels = completion_labels(&completions);
    assert!(
        labels.contains(&"action".to_string()),
        "Expected audit.logs columns (id, action, timestamp), got: {labels:?}"
    );
    assert!(labels.contains(&"id".to_string()));
    assert!(labels.contains(&"timestamp".to_string()));
}

#[test]
fn test_after_dot_alias_resolves_schema_qualified_implicit_alias() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT l. FROM audit.logs l";
    let offset = "SELECT l.".len();
    let range = dummy_replace_range();
    let completions = provider.build_completions("", text, offset, range);
    let labels = completion_labels(&completions);
    assert!(
        labels.contains(&"action".to_string()),
        "Expected audit.logs columns via implicit alias, got: {labels:?}"
    );
}

#[test]
fn test_after_dot_schema_qualified_no_alias() {
    // Directly using schema.table.column without alias
    // "audit.logs" is the table itself, so typing "logs." should give columns
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT logs. FROM audit.logs";
    let offset = "SELECT logs.".len();
    let range = dummy_replace_range();
    let completions = provider.build_completions("", text, offset, range);
    let labels = completion_labels(&completions);
    assert!(
        labels.contains(&"action".to_string()),
        "Expected logs columns when using table name directly, got: {labels:?}"
    );
}

// ── parse_aliases with schema-qualified tables ─────────────────────

#[test]
fn test_alias_schema_qualified_with_as() {
    let aliases = parse_aliases("SELECT * FROM audit.action_plan_aud AS a");
    assert_eq!(aliases.get("a").map(|s| s.as_str()), Some("audit.action_plan_aud"));
}

#[test]
fn test_alias_schema_qualified_implicit() {
    let aliases = parse_aliases("SELECT * FROM audit.logs l WHERE l.id = 1");
    assert_eq!(aliases.get("l").map(|s| s.as_str()), Some("audit.logs"));
}

#[test]
fn test_alias_schema_qualified_join() {
    let aliases = parse_aliases(
        "SELECT * FROM public.users u JOIN audit.logs l ON u.id = l.user_id"
    );
    assert_eq!(aliases.get("u").map(|s| s.as_str()), Some("public.users"));
    assert_eq!(aliases.get("l").map(|s| s.as_str()), Some("audit.logs"));
}

#[test]
fn test_alias_schema_qualified_no_alias() {
    let aliases = parse_aliases("SELECT * FROM audit.logs WHERE id = 1");
    assert!(aliases.is_empty());
}

#[test]
fn test_alias_mixed_qualified_and_unqualified() {
    let aliases = parse_aliases("SELECT * FROM users u JOIN audit.logs l ON u.id = l.user_id");
    assert_eq!(aliases.get("u").map(|s| s.as_str()), Some("users"));
    assert_eq!(aliases.get("l").map(|s| s.as_str()), Some("audit.logs"));
}

// ── Completion with active_schema filtering + schema-qualified alias ──

#[test]
fn test_alias_column_completion_with_active_schema_public() {
    let provider = make_test_provider(make_test_schema(), Some("public".to_string()));
    let text = "SELECT a. FROM audit.changes AS a";
    let offset = "SELECT a.".len();
    let range = dummy_replace_range();
    let completions = provider.build_completions("", text, offset, range);
    let labels = completion_labels(&completions);
    // Should resolve alias "a" to audit.changes, not filtered by active schema
    assert!(labels.contains(&"table_name".to_string()), "Expected audit.changes columns, got: {labels:?}");
    assert!(labels.contains(&"old_value".to_string()));
}

#[test]
fn test_after_dot_alias_with_multiple_schema_qualified_joins() {
    let provider = make_test_provider(make_test_schema(), None);
    let text = "SELECT a.id, c. FROM audit.logs AS a JOIN audit.changes AS c ON a.id = c.id";
    let offset = "SELECT a.id, c.".len();
    let range = dummy_replace_range();
    let completions = provider.build_completions("", text, offset, range);
    let labels = completion_labels(&completions);
    // "c" resolves to audit.changes which has columns: id, table_name, old_value
    assert!(labels.contains(&"table_name".to_string()), "Expected audit.changes columns, got: {labels:?}");
    assert!(labels.contains(&"old_value".to_string()));
}
