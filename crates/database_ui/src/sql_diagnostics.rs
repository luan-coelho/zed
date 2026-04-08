use gpui::Entity;
use language::{Buffer, Diagnostic, DiagnosticEntry, DiagnosticSet};
use lsp::LanguageServerId;
use text::PointUtf16;

/// Unique server ID for our SQL diagnostics (high number to avoid conflicts).
const SQL_DIAG_SERVER_ID: LanguageServerId = LanguageServerId(9999);

/// Parse SQL and update diagnostics on the buffer.
/// Shows red underlines for syntax errors.
pub fn update_sql_diagnostics(buffer: &Entity<Buffer>, cx: &mut gpui::App) {
    buffer.update(cx, |buffer, cx| {
        let snapshot = buffer.snapshot();
        let text = snapshot.text();
        let trimmed = text.trim();

        // Don't diagnose empty buffers
        if trimmed.is_empty() {
            let empty =
                DiagnosticSet::new(std::iter::empty::<DiagnosticEntry<PointUtf16>>(), &snapshot);
            buffer.update_diagnostics(SQL_DIAG_SERVER_ID, empty, cx);
            return;
        }

        // Try to parse with sqlparser
        let dialect = sqlparser::dialect::PostgreSqlDialect {};
        let result = sqlparser::parser::Parser::parse_sql(&dialect, trimmed);

        match result {
            Ok(_) => {
                // Valid SQL — clear diagnostics
                let empty = DiagnosticSet::new(
                    std::iter::empty::<DiagnosticEntry<PointUtf16>>(),
                    &snapshot,
                );
                buffer.update_diagnostics(SQL_DIAG_SERVER_ID, empty, cx);
            }
            Err(err) => {
                let (message, line, col) = parse_error_location(&err, trimmed);

                // Underline from the error position to end of that line
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

/// Extract error message and approximate location from sqlparser error.
fn parse_error_location(err: &sqlparser::parser::ParserError, _text: &str) -> (String, u32, u32) {
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
