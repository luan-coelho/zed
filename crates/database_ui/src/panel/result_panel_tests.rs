// Tests for result panel logic: sorting and filtering of query results.
// These test the pure data transformation logic without needing GPUI context.

fn sort_rows(
    rows: &[Vec<String>],
    sort_column: Option<usize>,
    ascending: bool,
) -> Vec<Vec<String>> {
    if let Some(col) = sort_column {
        let mut sorted = rows.to_vec();
        sorted.sort_by(|a, b| {
            let val_a = a.get(col).map(|s| s.as_str()).unwrap_or("");
            let val_b = b.get(col).map(|s| s.as_str()).unwrap_or("");
            let cmp = val_a.cmp(val_b);
            if ascending { cmp } else { cmp.reverse() }
        });
        sorted
    } else {
        rows.to_vec()
    }
}

fn filter_rows(rows: &[Vec<String>], filter: &str) -> Vec<Vec<String>> {
    if filter.is_empty() {
        return rows.to_vec();
    }
    let filter_lower = filter.to_lowercase();
    rows.iter()
        .filter(|row| {
            row.iter()
                .any(|val| val.to_lowercase().contains(&filter_lower))
        })
        .cloned()
        .collect()
}

fn sample_rows() -> Vec<Vec<String>> {
    vec![
        vec!["1".into(), "Alice".into(), "alice@example.com".into()],
        vec!["2".into(), "Bob".into(), "bob@example.com".into()],
        vec!["3".into(), "Charlie".into(), "charlie@test.com".into()],
        vec!["4".into(), "Alice".into(), "alice2@example.com".into()],
        vec!["5".into(), "Dave".into(), "dave@other.org".into()],
    ]
}

// ── Sort tests ─────────────────────────────────────────────────────

#[test]
fn test_sort_no_column() {
    let rows = sample_rows();
    let sorted = sort_rows(&rows, None, true);
    assert_eq!(sorted, rows);
}

#[test]
fn test_sort_by_first_column_ascending() {
    let rows = vec![
        vec!["3".into(), "C".into()],
        vec!["1".into(), "A".into()],
        vec!["2".into(), "B".into()],
    ];
    let sorted = sort_rows(&rows, Some(0), true);
    assert_eq!(sorted[0][0], "1");
    assert_eq!(sorted[1][0], "2");
    assert_eq!(sorted[2][0], "3");
}

#[test]
fn test_sort_by_first_column_descending() {
    let rows = vec![
        vec!["1".into(), "A".into()],
        vec!["3".into(), "C".into()],
        vec!["2".into(), "B".into()],
    ];
    let sorted = sort_rows(&rows, Some(0), false);
    assert_eq!(sorted[0][0], "3");
    assert_eq!(sorted[1][0], "2");
    assert_eq!(sorted[2][0], "1");
}

#[test]
fn test_sort_by_name_column() {
    let rows = sample_rows();
    let sorted = sort_rows(&rows, Some(1), true);
    assert_eq!(sorted[0][1], "Alice");
    assert_eq!(sorted[1][1], "Alice");
    assert_eq!(sorted[2][1], "Bob");
    assert_eq!(sorted[3][1], "Charlie");
    assert_eq!(sorted[4][1], "Dave");
}

#[test]
fn test_sort_preserves_row_count() {
    let rows = sample_rows();
    let sorted = sort_rows(&rows, Some(0), true);
    assert_eq!(sorted.len(), rows.len());
}

#[test]
fn test_sort_empty_rows() {
    let rows: Vec<Vec<String>> = vec![];
    let sorted = sort_rows(&rows, Some(0), true);
    assert!(sorted.is_empty());
}

#[test]
fn test_sort_column_out_of_bounds() {
    let rows = sample_rows();
    let sorted = sort_rows(&rows, Some(99), true);
    assert_eq!(sorted.len(), rows.len());
}

// ── Filter tests ───────────────────────────────────────────────────

#[test]
fn test_filter_empty_string_returns_all() {
    let rows = sample_rows();
    let filtered = filter_rows(&rows, "");
    assert_eq!(filtered.len(), rows.len());
}

#[test]
fn test_filter_by_name() {
    let rows = sample_rows();
    let filtered = filter_rows(&rows, "alice");
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().all(|r| r[1] == "Alice"));
}

#[test]
fn test_filter_case_insensitive() {
    let rows = sample_rows();
    let filtered = filter_rows(&rows, "ALICE");
    assert_eq!(filtered.len(), 2);
}

#[test]
fn test_filter_by_email_domain() {
    let rows = sample_rows();
    let filtered = filter_rows(&rows, "example.com");
    assert_eq!(filtered.len(), 3);
}

#[test]
fn test_filter_by_id() {
    let rows = sample_rows();
    let filtered = filter_rows(&rows, "3");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0][1], "Charlie");
}

#[test]
fn test_filter_no_match() {
    let rows = sample_rows();
    let filtered = filter_rows(&rows, "zzzzz");
    assert!(filtered.is_empty());
}

#[test]
fn test_filter_partial_match() {
    let rows = sample_rows();
    let filtered = filter_rows(&rows, "ob");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0][1], "Bob");
}

#[test]
fn test_filter_matches_any_column() {
    let rows = sample_rows();
    let filtered = filter_rows(&rows, "other.org");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0][1], "Dave");
}

#[test]
fn test_filter_empty_rows() {
    let rows: Vec<Vec<String>> = vec![];
    let filtered = filter_rows(&rows, "test");
    assert!(filtered.is_empty());
}

// ── Combined sort + filter tests ───────────────────────────────────

#[test]
fn test_filter_then_sort() {
    let rows = sample_rows();
    let filtered = filter_rows(&rows, "example.com");
    let sorted = sort_rows(&filtered, Some(1), true);
    assert_eq!(sorted.len(), 3);
    assert_eq!(sorted[0][1], "Alice");
    assert_eq!(sorted[1][1], "Alice");
    assert_eq!(sorted[2][1], "Bob");
}

#[test]
fn test_sort_then_filter() {
    let rows = sample_rows();
    let sorted = sort_rows(&rows, Some(1), false);
    let filtered = filter_rows(&sorted, "alice");
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().all(|r| r[1] == "Alice"));
}

// ── Export format tests ────────────────────────────────────────────

use crate::result_panel::{format_as_csv, format_as_json, format_as_tsv};

fn sample_columns() -> Vec<String> {
    vec!["id".into(), "name".into(), "email".into()]
}

fn sample_export_rows() -> Vec<Vec<String>> {
    vec![
        vec!["1".into(), "Alice".into(), "alice@example.com".into()],
        vec!["2".into(), "Bob".into(), "bob@example.com".into()],
    ]
}

// ── TSV ────────────────────────────────────────────────────────────

#[test]
fn test_tsv_header() {
    let tsv = format_as_tsv(&sample_columns(), &sample_export_rows());
    let first_line = tsv.lines().next().expect("should have header");
    assert_eq!(first_line, "id\tname\temail");
}

#[test]
fn test_tsv_rows() {
    let tsv = format_as_tsv(&sample_columns(), &sample_export_rows());
    let lines: Vec<&str> = tsv.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1], "1\tAlice\talice@example.com");
    assert_eq!(lines[2], "2\tBob\tbob@example.com");
}

#[test]
fn test_tsv_empty_rows() {
    let tsv = format_as_tsv(&sample_columns(), &[]);
    let lines: Vec<&str> = tsv.lines().collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], "id\tname\temail");
}

// ── CSV ────────────────────────────────────────────────────────────

#[test]
fn test_csv_header() {
    let csv = format_as_csv(&sample_columns(), &sample_export_rows());
    let first_line = csv.lines().next().expect("should have header");
    assert_eq!(first_line, "id,name,email");
}

#[test]
fn test_csv_rows() {
    let csv = format_as_csv(&sample_columns(), &sample_export_rows());
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1], "1,Alice,alice@example.com");
}

#[test]
fn test_csv_escapes_commas() {
    let cols = vec!["name".into()];
    let rows = vec![vec!["hello, world".into()]];
    let csv = format_as_csv(&cols, &rows);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[1], "\"hello, world\"");
}

#[test]
fn test_csv_escapes_quotes() {
    let cols = vec!["name".into()];
    let rows = vec![vec!["say \"hi\"".into()]];
    let csv = format_as_csv(&cols, &rows);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[1], "\"say \"\"hi\"\"\"");
}

#[test]
fn test_csv_escapes_newlines() {
    let cols = vec!["bio".into()];
    let rows = vec![vec!["line1\nline2".into()]];
    let csv = format_as_csv(&cols, &rows);
    assert!(csv.contains("\"line1\nline2\""));
}

#[test]
fn test_csv_empty_rows() {
    let csv = format_as_csv(&sample_columns(), &[]);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 1);
}

// ── JSON ───────────────────────────────────────────────────────────

#[test]
fn test_json_structure() {
    let json = format_as_json(&sample_columns(), &sample_export_rows()).expect("should produce JSON");
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).expect("should parse");
    assert_eq!(parsed.len(), 2);
}

#[test]
fn test_json_keys_match_columns() {
    let json = format_as_json(&sample_columns(), &sample_export_rows()).expect("should produce JSON");
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).expect("should parse");
    let obj = parsed[0].as_object().expect("should be object");
    assert!(obj.contains_key("id"));
    assert!(obj.contains_key("name"));
    assert!(obj.contains_key("email"));
}

#[test]
fn test_json_values() {
    let json = format_as_json(&sample_columns(), &sample_export_rows()).expect("should produce JSON");
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).expect("should parse");
    assert_eq!(parsed[0]["id"], "1");
    assert_eq!(parsed[0]["name"], "Alice");
    assert_eq!(parsed[1]["name"], "Bob");
}

#[test]
fn test_json_null_values() {
    let cols = vec!["id".into(), "val".into()];
    let rows = vec![vec!["1".into(), "NULL".into()]];
    let json = format_as_json(&cols, &rows).expect("should produce JSON");
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).expect("should parse");
    assert!(parsed[0]["val"].is_null());
}

#[test]
fn test_json_empty_rows() {
    let json = format_as_json(&sample_columns(), &[]).expect("should produce JSON");
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).expect("should parse");
    assert!(parsed.is_empty());
}

#[test]
fn test_json_is_pretty_printed() {
    let json = format_as_json(&sample_columns(), &sample_export_rows()).expect("should produce JSON");
    assert!(json.contains('\n'));
    assert!(json.contains("  "));
}
