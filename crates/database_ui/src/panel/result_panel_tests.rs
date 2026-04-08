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
