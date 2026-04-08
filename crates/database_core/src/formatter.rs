use crate::schema::{DatabaseSchema, QueryResult};

pub fn format_query_result(result: &QueryResult) -> String {
    let mut md = String::new();

    md.push_str("## Query Results\n\n");
    md.push_str(&format!(
        "**Rows:** {} | **Time:** {}ms\n\n",
        result.rows_affected, result.execution_time_ms
    ));

    if result.columns.is_empty() {
        md.push_str("*No columns returned*\n");
        return md;
    }

    let mut widths: Vec<usize> = result.columns.iter().map(|c| c.len()).collect();
    for row in &result.rows {
        for (i, val) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(val.len());
            }
        }
    }

    // Header
    md.push('|');
    for (i, col) in result.columns.iter().enumerate() {
        md.push_str(&format!(" {:<width$} |", col, width = widths[i]));
    }
    md.push('\n');

    // Separator
    md.push('|');
    for width in &widths {
        md.push_str(&format!("-{}-|", "-".repeat(*width)));
    }
    md.push('\n');

    // Rows
    for row in &result.rows {
        md.push('|');
        for (i, val) in row.iter().enumerate() {
            let width = widths.get(i).copied().unwrap_or(val.len());
            md.push_str(&format!(" {:<width$} |", val, width = width));
        }
        md.push('\n');
    }

    md
}

pub fn format_schema(schema: &DatabaseSchema) -> String {
    let mut md = String::new();

    md.push_str(&format!("## Database: `{}`\n\n", schema.name));

    if !schema.tables.is_empty() {
        md.push_str(&format!("### Tables ({})\n\n", schema.tables.len()));

        for table in &schema.tables {
            md.push_str(&format!("#### `{}.{}`", table.schema, table.name));
            if let Some(count) = table.row_count_estimate {
                md.push_str(&format!(" (~{count} rows)"));
            }
            md.push_str("\n\n");

            md.push_str("| Column | Type | Nullable | Default | Key |\n");
            md.push_str("|--------|------|----------|---------|-----|\n");

            for col in &table.columns {
                let key = if col.is_primary_key {
                    "PK".to_string()
                } else if let Some(ref fk) = col.foreign_key {
                    format!("FK -> {}.{}", fk.referenced_table, fk.referenced_column)
                } else {
                    "-".to_string()
                };

                md.push_str(&format!(
                    "| `{}` | {} | {} | {} | {} |\n",
                    col.name,
                    col.data_type,
                    if col.is_nullable { "YES" } else { "NO" },
                    col.default_value.as_deref().unwrap_or("-"),
                    key,
                ));
            }

            if !table.indexes.is_empty() {
                md.push_str("\n**Indexes:**\n");
                for idx in &table.indexes {
                    let unique = if idx.is_unique { " UNIQUE" } else { "" };
                    let primary = if idx.is_primary { " PRIMARY" } else { "" };
                    md.push_str(&format!(
                        "- `{}`{}{}: ({})\n",
                        idx.name,
                        primary,
                        unique,
                        idx.columns.join(", ")
                    ));
                }
            }

            md.push('\n');
        }
    }

    if !schema.views.is_empty() {
        md.push_str(&format!("### Views ({})\n\n", schema.views.len()));

        for view in &schema.views {
            md.push_str(&format!("#### `{}.{}`\n\n", view.schema, view.name));
            md.push_str("| Column | Type | Nullable |\n");
            md.push_str("|--------|------|----------|\n");

            for col in &view.columns {
                md.push_str(&format!(
                    "| `{}` | {} | {} |\n",
                    col.name,
                    col.data_type,
                    if col.is_nullable { "YES" } else { "NO" },
                ));
            }

            md.push('\n');
        }
    }

    md
}
