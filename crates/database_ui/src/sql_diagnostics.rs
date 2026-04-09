use std::sync::Arc;

use database_core::DatabaseSchema;
use gpui::Entity;
use language::{Buffer, Diagnostic, DiagnosticEntry, DiagnosticSet};
use lsp::LanguageServerId;
use text::PointUtf16;
use tokio::sync::RwLock;

/// Unique server ID for our SQL diagnostics (high number to avoid conflicts).
const SQL_DIAG_SERVER_ID: LanguageServerId = LanguageServerId(9999);

/// Parse SQL and update diagnostics on the buffer.
/// Shows red underlines for syntax errors and warnings for unknown columns.
pub fn update_sql_diagnostics(
    buffer: &Entity<Buffer>,
    provider: &str,
    schema: &Arc<RwLock<Option<DatabaseSchema>>>,
    cx: &mut gpui::App,
) {
    let schema_snapshot = schema.blocking_read().clone();

    buffer.update(cx, |buffer, cx| {
        let snapshot = buffer.snapshot();
        let text = snapshot.text();
        let trimmed = text.trim();

        if trimmed.is_empty() {
            let empty =
                DiagnosticSet::new(std::iter::empty::<DiagnosticEntry<PointUtf16>>(), &snapshot);
            buffer.update_diagnostics(SQL_DIAG_SERVER_ID, empty, cx);
            return;
        }

        let dialect = dialect_for_provider(provider);
        let result = sqlparser::parser::Parser::parse_sql(dialect.as_ref(), trimmed);

        match result {
            Ok(_) => {
                let mut entries = Vec::new();

                if let Some(ref db_schema) = schema_snapshot {
                    entries = validate_columns(&text, db_schema);
                }

                let diag_set = DiagnosticSet::new(entries, &snapshot);
                buffer.update_diagnostics(SQL_DIAG_SERVER_ID, diag_set, cx);
            }
            Err(err) => {
                let (message, line, col) = parse_error_location(&err, trimmed);

                let lines: Vec<&str> = text.split('\n').collect();
                let end_col = lines
                    .get(line as usize)
                    .map(|l| l.len() as u32)
                    .unwrap_or(col + 1);

                let entries = vec![DiagnosticEntry {
                    range: PointUtf16::new(line, col)..PointUtf16::new(line, end_col),
                    diagnostic: Diagnostic {
                        message,
                        severity: lsp::DiagnosticSeverity::ERROR,
                        source: Some("SQL".to_string()),
                        is_primary: true,
                        ..Default::default()
                    },
                }];

                let diag_set = DiagnosticSet::new(entries, &snapshot);
                buffer.update_diagnostics(SQL_DIAG_SERVER_ID, diag_set, cx);
            }
        }
    });
}

/// Validate qualified column references (e.g., `u.xxxx`) against the schema.
pub(crate) fn validate_columns(
    text: &str,
    schema: &DatabaseSchema,
) -> Vec<DiagnosticEntry<PointUtf16>> {
    let aliases = crate::sql_completion::parse_aliases(text);
    let mut entries = Vec::new();

    for (line_idx, line) in text.split('\n').enumerate() {
        let line_lower = line.to_lowercase();

        // Find all `qualifier.column` patterns in this line
        let mut search_start = 0;
        while let Some(dot_pos) = line[search_start..].find('.') {
            let dot_abs = search_start + dot_pos;

            let qualifier = extract_identifier_before(line, dot_abs);
            let column = extract_identifier_after(line, dot_abs);

            if qualifier.is_empty() || column.is_empty() {
                search_start = dot_abs + 1;
                continue;
            }

            // Skip if column looks like `*`
            if column == "*" {
                search_start = dot_abs + 1;
                continue;
            }

            let qualifier_lower = qualifier.to_lowercase();
            let column_lower = column.to_lowercase();

            // Resolve qualifier: alias → table name, or use directly
            let resolved_table = aliases
                .get(&qualifier_lower)
                .cloned()
                .unwrap_or(qualifier_lower.clone());

            // Check if it's a schema-qualified table name (schema.table) rather than table.column
            let is_schema_name = schema
                .tables
                .iter()
                .any(|t| t.schema.to_lowercase() == qualifier_lower);

            if is_schema_name {
                search_start = dot_abs + 1;
                continue;
            }

            // Find the table in the schema
            let table = schema
                .tables
                .iter()
                .find(|t| t.name.to_lowercase() == resolved_table);

            if let Some(table) = table {
                let column_exists = table
                    .columns
                    .iter()
                    .any(|c| c.name.to_lowercase() == column_lower);

                if !column_exists {
                    let col_start = dot_abs + 1;
                    // Find the actual start in the line (the qualifier start for the range)
                    let col_end = col_start + column.len();

                    // Check the column name matches what we found (avoid false matches from line_lower)
                    let actual = &line[col_start..col_end];
                    if actual.to_lowercase() == column_lower {
                        entries.push(DiagnosticEntry {
                            range: PointUtf16::new(line_idx as u32, col_start as u32)
                                ..PointUtf16::new(line_idx as u32, col_end as u32),
                            diagnostic: Diagnostic {
                                message: format!(
                                    "Column '{}' not found in table '{}'",
                                    column, table.name
                                ),
                                severity: lsp::DiagnosticSeverity::WARNING,
                                source: Some("SQL".to_string()),
                                is_primary: true,
                                ..Default::default()
                            },
                        });
                    }
                }
            }

            search_start = dot_abs + 1;
        }

        let _ = line_lower; // used for context
    }

    entries
}

/// Extract an identifier (word) immediately before the given position.
fn extract_identifier_before(text: &str, pos: usize) -> &str {
    let bytes = text.as_bytes();
    let mut start = pos;
    while start > 0 {
        let ch = bytes[start - 1];
        if ch.is_ascii_alphanumeric() || ch == b'_' {
            start -= 1;
        } else {
            break;
        }
    }
    &text[start..pos]
}

/// Extract an identifier (word) immediately after the given position (skipping the dot).
fn extract_identifier_after(text: &str, dot_pos: usize) -> &str {
    let start = dot_pos + 1;
    if start >= text.len() {
        return "";
    }
    let bytes = text.as_bytes();
    // Allow `*` as a valid column reference
    if bytes[start] == b'*' {
        return &text[start..start + 1];
    }
    let mut end = start;
    while end < text.len() {
        let ch = bytes[end];
        if ch.is_ascii_alphanumeric() || ch == b'_' {
            end += 1;
        } else {
            break;
        }
    }
    &text[start..end]
}

pub(crate) fn dialect_for_provider(provider: &str) -> Box<dyn sqlparser::dialect::Dialect> {
    let registry = database_core::ProviderRegistry::new();
    let dialect_name = registry
        .find_provider_info(provider)
        .map(|p| p.sql_dialect)
        .unwrap_or("postgres");

    match dialect_name {
        "mysql" => Box::new(sqlparser::dialect::MySqlDialect {}),
        "sqlite" => Box::new(sqlparser::dialect::SQLiteDialect {}),
        _ => Box::new(sqlparser::dialect::PostgreSqlDialect {}),
    }
}

/// Extract error message and approximate location from sqlparser error.
pub(crate) fn parse_error_location(err: &sqlparser::parser::ParserError, _text: &str) -> (String, u32, u32) {
    let msg = err.to_string();

    // sqlparser errors look like:
    // "sql parser error: Expected ..., found: xx at Line: 1, Column: 35"
    if let Some(loc_start) = msg.find("Line: ") {
        let after_line = &msg[loc_start + 6..];
        let line_end = after_line.find(',').unwrap_or(after_line.len());
        let line: u32 = after_line[..line_end]
            .trim()
            .parse::<u32>()
            .unwrap_or(1)
            .saturating_sub(1); // 0-indexed

        let col = if let Some(col_start) = msg.find("Column: ") {
            let after_col = &msg[col_start + 8..];
            let col_end = after_col
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after_col.len());
            after_col[..col_end]
                .trim()
                .parse::<u32>()
                .unwrap_or(1)
                .saturating_sub(1) // 0-indexed
        } else {
            0
        };

        // Clean up the message (remove location suffix)
        let clean_msg = if let Some(at_pos) = msg.find(" at Line:") {
            msg[..at_pos].to_string()
        } else {
            msg
        };

        (clean_msg, line, col)
    } else {
        // No location info — put error at start
        (msg, 0, 0)
    }
}
