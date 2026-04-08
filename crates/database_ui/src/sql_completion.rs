use database_core::DatabaseSchema;
use editor::CompletionProvider;
use gpui::{Context, Task, Window};
use language::Buffer;
use project::{Completion, CompletionDisplayOptions, CompletionResponse, CompletionSource};
use std::sync::Arc;
use text::ToOffset as _;
use tokio::sync::RwLock;

/// SQL completion provider that provides completions based on the database schema.
pub struct SqlCompletionProvider {
    schema: Arc<RwLock<Option<DatabaseSchema>>>,
    active_schema_name: Arc<RwLock<Option<String>>>,
}

impl SqlCompletionProvider {
    pub fn new(
        schema: Arc<RwLock<Option<DatabaseSchema>>>,
        active_schema_name: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            schema,
            active_schema_name,
        }
    }

    fn build_completions(
        &self,
        prefix: &str,
        buffer_text: &str,
        offset: usize,
        replace_range: std::ops::Range<language::Anchor>,
    ) -> Vec<Completion> {
        let schema = self.schema.blocking_read();
        let Some(schema) = schema.as_ref() else {
            return self.keyword_completions(prefix, replace_range);
        };

        let active_schema = self.active_schema_name.blocking_read();
        let schema_filter = active_schema.as_deref();

        let prefix_lower = prefix.to_lowercase();
        let context = detect_sql_context(buffer_text, offset);

        let mut completions = Vec::new();

        // Filter tables and views by active schema (if set)
        let tables: Vec<_> = schema
            .tables
            .iter()
            .filter(|t| schema_filter.is_none_or(|s| t.schema == s))
            .collect();
        let views: Vec<_> = schema
            .views
            .iter()
            .filter(|v| schema_filter.is_none_or(|s| v.schema == s))
            .collect();

        // Parse aliases to resolve alias.column completions
        let aliases = parse_aliases(buffer_text);

        match context {
            SqlContext::AfterDot(ref name) => {
                // Resolve alias → table name, or use directly as table name
                let resolved = aliases
                    .get(&name.to_lowercase())
                    .cloned()
                    .unwrap_or_else(|| name.to_lowercase());

                // Column completions for the resolved table (or view)
                let target_table = tables
                    .iter()
                    .find(|t| t.name.to_lowercase() == resolved);
                let target_view = views
                    .iter()
                    .find(|v| v.name.to_lowercase() == resolved);

                let columns = target_table
                    .map(|t| t.columns.as_slice())
                    .or_else(|| target_view.map(|v| v.columns.as_slice()));

                if let Some(cols) = columns {
                    for col in cols {
                        if prefix.is_empty() || col.name.to_lowercase().starts_with(&prefix_lower)
                        {
                            completions.push(make_completion(
                                replace_range.clone(),
                                &col.name,
                                &format!("{} ({})", col.name, col.data_type),
                                &col.data_type,
                            ));
                        }
                    }
                }
            }
            SqlContext::AfterFrom | SqlContext::AfterJoin | SqlContext::AfterUpdate | SqlContext::AfterInto => {
                // Table name completions
                for table in &tables {
                    if prefix.is_empty() || table.name.to_lowercase().starts_with(&prefix_lower) {
                        let cols: Vec<_> = table.columns.iter().map(|c| c.name.as_str()).collect();
                        completions.push(make_completion(replace_range.clone(),
                            &table.name,
                            &format!("{} ({})", table.name, cols.join(", ")),
                            "table",
                        ));
                    }
                }
                for view in &views {
                    if prefix.is_empty() || view.name.to_lowercase().starts_with(&prefix_lower) {
                        completions.push(make_completion(replace_range.clone(),
                            &view.name,
                            &format!("{} (view)", view.name),
                            "view",
                        ));
                    }
                }
            }
            SqlContext::General => {
                // Keywords + table names + functions
                completions.extend(self.keyword_completions(&prefix_lower, replace_range.clone()));

                for table in &tables {
                    if table.name.to_lowercase().starts_with(&prefix_lower) {
                        completions.push(make_completion(replace_range.clone(), &table.name, &table.name, "table"));
                    }
                }

                completions.extend(self.function_completions(&prefix_lower, replace_range.clone()));
            }
        }

        completions
    }

    fn keyword_completions(&self, prefix: &str, replace_range: std::ops::Range<language::Anchor>) -> Vec<Completion> {
        let keywords = [
            "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IN", "EXISTS",
            "INSERT INTO", "VALUES", "UPDATE", "SET", "DELETE FROM",
            "CREATE TABLE", "ALTER TABLE", "DROP TABLE",
            "JOIN", "LEFT JOIN", "RIGHT JOIN", "INNER JOIN", "FULL JOIN",
            "ON", "AS", "GROUP BY", "ORDER BY", "HAVING",
            "LIMIT", "OFFSET", "DISTINCT", "UNION", "UNION ALL",
            "CASE", "WHEN", "THEN", "ELSE", "END",
            "IS NULL", "IS NOT NULL", "BETWEEN", "LIKE", "ILIKE",
            "ASC", "DESC", "NULLS FIRST", "NULLS LAST",
            "BEGIN", "COMMIT", "ROLLBACK",
            "WITH", "RECURSIVE", "RETURNING",
            "EXPLAIN", "EXPLAIN ANALYZE",
            "TRUE", "FALSE", "NULL",
        ];

        keywords
            .iter()
            .filter(|k| {
                prefix.is_empty()
                    || k.to_lowercase().starts_with(&prefix.to_lowercase())
            })
            .map(|k| make_completion(replace_range.clone(),k, k, "keyword"))
            .collect()
    }

    fn function_completions(&self, prefix: &str, replace_range: std::ops::Range<language::Anchor>) -> Vec<Completion> {
        let functions = [
            ("COUNT(*)", "COUNT(expression) → bigint"),
            ("SUM()", "SUM(expression) → numeric"),
            ("AVG()", "AVG(expression) → numeric"),
            ("MIN()", "MIN(expression) → same as input"),
            ("MAX()", "MAX(expression) → same as input"),
            ("COALESCE()", "COALESCE(value1, value2, ...) → first non-null"),
            ("NULLIF()", "NULLIF(value1, value2) → null if equal"),
            ("CAST()", "CAST(expression AS type)"),
            ("NOW()", "NOW() → timestamp with time zone"),
            ("CURRENT_DATE", "CURRENT_DATE → date"),
            ("CURRENT_TIMESTAMP", "CURRENT_TIMESTAMP → timestamp"),
            ("EXTRACT()", "EXTRACT(field FROM source)"),
            ("DATE_TRUNC()", "DATE_TRUNC(field, source)"),
            ("UPPER()", "UPPER(string) → string"),
            ("LOWER()", "LOWER(string) → string"),
            ("LENGTH()", "LENGTH(string) → integer"),
            ("TRIM()", "TRIM(string) → string"),
            ("SUBSTRING()", "SUBSTRING(string FROM start FOR length)"),
            ("CONCAT()", "CONCAT(str1, str2, ...) → string"),
            ("REPLACE()", "REPLACE(string, from, to) → string"),
            ("ARRAY_AGG()", "ARRAY_AGG(expression) → array"),
            ("STRING_AGG()", "STRING_AGG(expression, delimiter) → string"),
            ("JSON_AGG()", "JSON_AGG(expression) → json"),
            ("JSONB_AGG()", "JSONB_AGG(expression) → jsonb"),
            ("ROW_NUMBER()", "ROW_NUMBER() OVER (...)"),
            ("RANK()", "RANK() OVER (...)"),
            ("DENSE_RANK()", "DENSE_RANK() OVER (...)"),
            ("LAG()", "LAG(value, offset, default) OVER (...)"),
            ("LEAD()", "LEAD(value, offset, default) OVER (...)"),
            ("GENERATE_SERIES()", "GENERATE_SERIES(start, stop, step)"),
            ("TO_CHAR()", "TO_CHAR(timestamp, format)"),
            ("TO_DATE()", "TO_DATE(text, format)"),
            ("TO_NUMBER()", "TO_NUMBER(text, format)"),
        ];

        functions
            .iter()
            .filter(|(name, _)| {
                prefix.is_empty()
                    || name.to_lowercase().starts_with(&prefix.to_lowercase())
            })
            .map(|(name, desc)| make_completion(replace_range.clone(),name, desc, "function"))
            .collect()
    }
}

impl CompletionProvider for SqlCompletionProvider {
    fn completions(
        &self,
        buffer: &gpui::Entity<Buffer>,
        buffer_position: text::Anchor,
        _trigger: editor::CompletionContext,
        _window: &mut Window,
        cx: &mut Context<editor::Editor>,
    ) -> Task<anyhow::Result<Vec<CompletionResponse>>> {
        let buffer = buffer.read(cx);
        let snapshot = buffer.snapshot();
        let offset = buffer_position.to_offset(&snapshot);
        let text = snapshot.text();

        // Extract the current word being typed
        let prefix = extract_word_before_cursor(&text, offset);

        // Calculate replace range for the word being typed
        let word_start = offset - prefix.len();
        let start_anchor = snapshot.anchor_before(word_start);
        let end_anchor = snapshot.anchor_after(offset);
        let replace_range = start_anchor..end_anchor;

        let completions = self.build_completions(&prefix, &text, offset, replace_range);

        Task::ready(Ok(vec![CompletionResponse {
            completions,
            display_options: CompletionDisplayOptions::default(),
            is_incomplete: false,
        }]))
    }

    fn is_completion_trigger(
        &self,
        _buffer: &gpui::Entity<Buffer>,
        _position: language::Anchor,
        text: &str,
        trigger_in_words: bool,
        _cx: &mut Context<editor::Editor>,
    ) -> bool {
        text == "." || text == " " || trigger_in_words
    }

    fn sort_completions(&self) -> bool {
        true
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn make_completion(
    replace_range: std::ops::Range<language::Anchor>,
    label: &str,
    _detail: &str,
    _kind: &str,
) -> Completion {
    use language::CodeLabel;

    Completion {
        replace_range,
        new_text: label.to_string(),
        label: CodeLabel {
            text: label.to_string(),
            runs: vec![],
            filter_range: 0..label.len(),
        },
        documentation: None,
        source: CompletionSource::Custom,
        icon_path: None,
        match_start: None,
        snippet_deduplication_key: None,
        insert_text_mode: None,
        confirm: None,
    }
}

#[derive(Debug)]
pub(crate) enum SqlContext {
    AfterFrom,
    AfterJoin,
    AfterUpdate,
    AfterInto,
    AfterDot(String),
    General,
}

pub(crate) fn detect_sql_context(text: &str, offset: usize) -> SqlContext {
    let before = &text[..offset];

    // Check if we're after a dot (table.column or alias.column)
    if before.ends_with('.') {
        let word_before_dot = extract_word_before_cursor(before, offset - 1);
        if !word_before_dot.is_empty() {
            return SqlContext::AfterDot(word_before_dot);
        }
    }

    // Check the last keyword before cursor
    let before_upper = before.to_uppercase();
    let tokens: Vec<&str> = before_upper.split_whitespace().collect();

    for token in tokens.iter().rev() {
        match *token {
            "FROM" | "TABLE" => return SqlContext::AfterFrom,
            "JOIN" => return SqlContext::AfterJoin,
            "UPDATE" => return SqlContext::AfterUpdate,
            "INTO" => return SqlContext::AfterInto,
            _ => {
                if ["SELECT", "WHERE", "AND", "OR", "ON", "SET", "VALUES", "HAVING", "CASE", "WHEN", "THEN", "ELSE"]
                    .contains(token)
                {
                    return SqlContext::General;
                }
            }
        }
    }

    SqlContext::General
}

/// Parse SQL aliases from the query text.
/// Detects patterns: `FROM table alias`, `FROM table AS alias`,
/// `JOIN table alias ON`, `JOIN table AS alias ON`, etc.
/// Returns a map of alias → table_name (both lowercase).
pub(crate) fn parse_aliases(text: &str) -> std::collections::HashMap<String, String> {
    let mut aliases = std::collections::HashMap::new();

    // Tokenize preserving original case for table names
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let upper_tokens: Vec<String> = tokens.iter().map(|t| t.to_uppercase()).collect();

    let table_keywords = ["FROM", "JOIN", "INNER", "LEFT", "RIGHT", "FULL", "CROSS", "UPDATE"];

    let mut i = 0;
    while i < tokens.len() {
        let upper = &upper_tokens[i];

        // Skip JOIN modifiers to find the actual JOIN keyword
        if ["INNER", "LEFT", "RIGHT", "FULL", "CROSS", "NATURAL", "OUTER"].contains(&upper.as_str())
        {
            i += 1;
            continue;
        }

        if table_keywords.contains(&upper.as_str()) {
            // Next token should be table name
            let table_idx = i + 1;
            if table_idx >= tokens.len() {
                break;
            }
            let table_name = tokens[table_idx];

            // Skip if table_name looks like a keyword or subquery
            if table_name.starts_with('(') || upper_tokens[table_idx] == "SELECT" {
                i += 2;
                continue;
            }

            // Check for alias: FROM table AS alias or FROM table alias
            let next_idx = table_idx + 1;
            if next_idx < tokens.len() {
                let next_upper = &upper_tokens[next_idx];

                if next_upper == "AS" {
                    // FROM table AS alias
                    let alias_idx = next_idx + 1;
                    if alias_idx < tokens.len() {
                        let alias = tokens[alias_idx].trim_end_matches(',');
                        if !is_sql_keyword(&upper_tokens[alias_idx].trim_end_matches(',')) {
                            aliases.insert(
                                alias.to_lowercase(),
                                table_name.to_lowercase(),
                            );
                        }
                    }
                } else if !is_sql_keyword(next_upper.trim_end_matches(','))
                    && !next_upper.starts_with('(')
                    && next_upper != "ON"
                    && next_upper != "WHERE"
                    && next_upper != "SET"
                    && next_upper != "GROUP"
                    && next_upper != "ORDER"
                    && next_upper != "LIMIT"
                    && next_upper != "HAVING"
                    && next_upper != "UNION"
                    && next_upper != ";"
                    && !next_upper.contains(',')
                    && !next_upper.contains('.')
                {
                    // FROM table alias (implicit alias)
                    let alias = tokens[next_idx].trim_end_matches(',');
                    aliases.insert(
                        alias.to_lowercase(),
                        table_name.to_lowercase(),
                    );
                }
            }

            i = table_idx + 1;
        } else {
            i += 1;
        }
    }

    aliases
}

pub(crate) fn is_sql_keyword(word: &str) -> bool {
    [
        "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IN", "EXISTS", "INSERT", "INTO",
        "VALUES", "UPDATE", "SET", "DELETE", "CREATE", "ALTER", "DROP", "JOIN", "INNER",
        "LEFT", "RIGHT", "FULL", "CROSS", "NATURAL", "OUTER", "ON", "AS", "GROUP", "BY",
        "ORDER", "HAVING", "LIMIT", "OFFSET", "DISTINCT", "UNION", "ALL", "CASE", "WHEN",
        "THEN", "ELSE", "END", "IS", "NULL", "BETWEEN", "LIKE", "ILIKE", "ASC", "DESC",
        "BEGIN", "COMMIT", "ROLLBACK", "WITH", "RECURSIVE", "RETURNING", "EXPLAIN", "TABLE",
        "TRUE", "FALSE",
    ]
    .contains(&word)
}

pub(crate) fn extract_word_before_cursor(text: &str, offset: usize) -> String {
    let before = &text[..offset];
    let start = before
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);
    before[start..].to_string()
}
