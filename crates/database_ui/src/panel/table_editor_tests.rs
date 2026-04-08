use super::table_editor::{generate_alter_ddl, ColumnState, IndexState};
use crate::database_panel::{OriginalColumn, OriginalIndex};

fn col(name: &str, data_type: &str, nullable: bool, default: &str, pk: bool) -> OriginalColumn {
    OriginalColumn {
        name: name.into(),
        data_type: data_type.into(),
        is_nullable: nullable,
        default_value: if default.is_empty() {
            None
        } else {
            Some(default.into())
        },
        is_primary_key: pk,
    }
}

fn col_state(
    original_name: Option<&str>,
    name: &str,
    data_type: &str,
    nullable: bool,
    default: &str,
    pk: bool,
    deleted: bool,
) -> ColumnState {
    ColumnState {
        original_name: original_name.map(|s| s.into()),
        name: name.into(),
        data_type: data_type.into(),
        nullable,
        default_value: default.into(),
        is_primary_key: pk,
        marked_for_deletion: deleted,
    }
}

fn idx(name: &str, columns: &[&str], unique: bool) -> OriginalIndex {
    OriginalIndex {
        name: name.into(),
        columns: columns.iter().map(|s| s.to_string()).collect(),
        is_unique: unique,
    }
}

fn idx_state(
    original_name: Option<&str>,
    name: &str,
    columns: &str,
    unique: bool,
    deleted: bool,
) -> IndexState {
    IndexState {
        original_name: original_name.map(|s| s.into()),
        name: name.into(),
        columns: columns.into(),
        is_unique: unique,
        marked_for_deletion: deleted,
    }
}

// ── No changes ─────────────────────────────────────────────────────

#[test]
fn test_no_changes_produces_empty_ddl() {
    let originals = vec![col("id", "integer", false, "", true)];
    let current = vec![col_state(Some("id"), "id", "integer", false, "", true, false)];
    let ddl = generate_alter_ddl("public", "users", "users", &originals, &current, &[], &[]);
    assert!(ddl.is_empty(), "Expected no DDL, got: {ddl:?}");
}

// ── Table rename ───────────────────────────────────────────────────

#[test]
fn test_table_rename() {
    let originals = vec![col("id", "integer", false, "", true)];
    let current = vec![col_state(Some("id"), "id", "integer", false, "", true, false)];
    let ddl = generate_alter_ddl("public", "users", "accounts", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("RENAME TO \"accounts\""), "Got: {}", ddl[0]);
}

// ── Add column ─────────────────────────────────────────────────────

#[test]
fn test_add_column() {
    let originals = vec![col("id", "integer", false, "", true)];
    let current = vec![
        col_state(Some("id"), "id", "integer", false, "", true, false),
        col_state(None, "email", "text", true, "", false, false),
    ];
    let ddl = generate_alter_ddl("public", "users", "users", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("ADD COLUMN \"email\" text"), "Got: {}", ddl[0]);
}

#[test]
fn test_add_column_not_null() {
    let originals = vec![];
    let current = vec![col_state(None, "name", "varchar", false, "", false, false)];
    let ddl = generate_alter_ddl("public", "t", "t", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("NOT NULL"), "Got: {}", ddl[0]);
}

#[test]
fn test_add_column_with_default() {
    let originals = vec![];
    let current = vec![col_state(None, "status", "int", true, "0", false, false)];
    let ddl = generate_alter_ddl("public", "t", "t", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("DEFAULT 0"), "Got: {}", ddl[0]);
}

// ── Drop column ────────────────────────────────────────────────────

#[test]
fn test_drop_column() {
    let originals = vec![
        col("id", "integer", false, "", true),
        col("name", "text", true, "", false),
    ];
    let current = vec![
        col_state(Some("id"), "id", "integer", false, "", true, false),
        col_state(Some("name"), "name", "text", true, "", false, true),
    ];
    let ddl = generate_alter_ddl("public", "users", "users", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("DROP COLUMN \"name\""), "Got: {}", ddl[0]);
}

#[test]
fn test_drop_new_column_no_ddl() {
    let originals = vec![];
    let current = vec![col_state(None, "temp", "text", true, "", false, true)];
    let ddl = generate_alter_ddl("public", "t", "t", &originals, &current, &[], &[]);
    assert!(ddl.is_empty(), "Dropping a new column should not generate DDL, got: {ddl:?}");
}

// ── Rename column ──────────────────────────────────────────────────

#[test]
fn test_rename_column() {
    let originals = vec![col("name", "text", true, "", false)];
    let current = vec![col_state(Some("name"), "full_name", "text", true, "", false, false)];
    let ddl = generate_alter_ddl("public", "users", "users", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(
        ddl[0].contains("RENAME COLUMN \"name\" TO \"full_name\""),
        "Got: {}",
        ddl[0]
    );
}

// ── Change type ────────────────────────────────────────────────────

#[test]
fn test_change_column_type() {
    let originals = vec![col("age", "integer", true, "", false)];
    let current = vec![col_state(Some("age"), "age", "bigint", true, "", false, false)];
    let ddl = generate_alter_ddl("public", "users", "users", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(
        ddl[0].contains("ALTER COLUMN \"age\" TYPE bigint"),
        "Got: {}",
        ddl[0]
    );
}

// ── Change nullable ────────────────────────────────────────────────

#[test]
fn test_set_not_null() {
    let originals = vec![col("email", "text", true, "", false)];
    let current = vec![col_state(Some("email"), "email", "text", false, "", false, false)];
    let ddl = generate_alter_ddl("public", "users", "users", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("SET NOT NULL"), "Got: {}", ddl[0]);
}

#[test]
fn test_drop_not_null() {
    let originals = vec![col("email", "text", false, "", false)];
    let current = vec![col_state(Some("email"), "email", "text", true, "", false, false)];
    let ddl = generate_alter_ddl("public", "users", "users", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("DROP NOT NULL"), "Got: {}", ddl[0]);
}

// ── Change default ─────────────────────────────────────────────────

#[test]
fn test_set_default() {
    let originals = vec![col("status", "int", true, "", false)];
    let current = vec![col_state(Some("status"), "status", "int", true, "0", false, false)];
    let ddl = generate_alter_ddl("public", "t", "t", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("SET DEFAULT 0"), "Got: {}", ddl[0]);
}

#[test]
fn test_drop_default() {
    let originals = vec![col("status", "int", true, "0", false)];
    let current = vec![col_state(Some("status"), "status", "int", true, "", false, false)];
    let ddl = generate_alter_ddl("public", "t", "t", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("DROP DEFAULT"), "Got: {}", ddl[0]);
}

#[test]
fn test_change_default() {
    let originals = vec![col("status", "int", true, "0", false)];
    let current = vec![col_state(Some("status"), "status", "int", true, "1", false, false)];
    let ddl = generate_alter_ddl("public", "t", "t", &originals, &current, &[], &[]);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("SET DEFAULT 1"), "Got: {}", ddl[0]);
}

// ── Indexes ────────────────────────────────────────────────────────

#[test]
fn test_add_index() {
    let current_idx = vec![idx_state(None, "idx_email", "email", false, false)];
    let ddl = generate_alter_ddl("public", "users", "users", &[], &[], &[], &current_idx);
    assert_eq!(ddl.len(), 1);
    assert!(
        ddl[0].contains("CREATE INDEX \"idx_email\" ON \"public\".\"users\" (email)"),
        "Got: {}",
        ddl[0]
    );
}

#[test]
fn test_add_unique_index() {
    let current_idx = vec![idx_state(None, "idx_email", "email", true, false)];
    let ddl = generate_alter_ddl("public", "users", "users", &[], &[], &[], &current_idx);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("CREATE UNIQUE INDEX"), "Got: {}", ddl[0]);
}

#[test]
fn test_drop_index() {
    let orig_idx = vec![idx("idx_email", &["email"], false)];
    let current_idx = vec![idx_state(Some("idx_email"), "idx_email", "email", false, true)];
    let ddl = generate_alter_ddl("public", "users", "users", &[], &[], &orig_idx, &current_idx);
    assert_eq!(ddl.len(), 1);
    assert!(ddl[0].contains("DROP INDEX \"idx_email\""), "Got: {}", ddl[0]);
}

#[test]
fn test_modify_index() {
    let orig_idx = vec![idx("idx_name", &["name"], false)];
    let current_idx = vec![idx_state(Some("idx_name"), "idx_name", "name", true, false)];
    let ddl = generate_alter_ddl("public", "users", "users", &[], &[], &orig_idx, &current_idx);
    assert_eq!(ddl.len(), 2);
    assert!(ddl[0].contains("DROP INDEX \"idx_name\""), "Got: {}", ddl[0]);
    assert!(ddl[1].contains("CREATE UNIQUE INDEX"), "Got: {}", ddl[1]);
}

#[test]
fn test_index_no_change() {
    let orig_idx = vec![idx("idx_name", &["name"], false)];
    let current_idx = vec![idx_state(Some("idx_name"), "idx_name", "name", false, false)];
    let ddl = generate_alter_ddl("public", "users", "users", &[], &[], &orig_idx, &current_idx);
    assert!(ddl.is_empty(), "Expected no DDL for unchanged index, got: {ddl:?}");
}

// ── Multiple changes ───────────────────────────────────────────────

#[test]
fn test_multiple_changes() {
    let originals = vec![
        col("id", "integer", false, "", true),
        col("name", "varchar", true, "", false),
        col("old_col", "text", true, "", false),
    ];
    let current = vec![
        col_state(Some("id"), "id", "integer", false, "", true, false),
        col_state(Some("name"), "full_name", "text", false, "", false, false),
        col_state(Some("old_col"), "old_col", "text", true, "", false, true),
        col_state(None, "email", "varchar", true, "", false, false),
    ];
    let ddl = generate_alter_ddl("public", "users", "users", &originals, &current, &[], &[]);
    // DROP old_col, RENAME name, TYPE change name, SET NOT NULL name, ADD email
    assert!(ddl.len() >= 4, "Expected >= 4 DDL statements, got {}: {ddl:?}", ddl.len());
    assert!(ddl.iter().any(|s| s.contains("DROP COLUMN \"old_col\"")));
    assert!(ddl.iter().any(|s| s.contains("RENAME COLUMN \"name\" TO \"full_name\"")));
    assert!(ddl.iter().any(|s| s.contains("ADD COLUMN \"email\"")));
}

// ── Schema qualification ───────────────────────────────────────────

#[test]
fn test_uses_schema_qualified_name() {
    let originals = vec![col("id", "int", false, "", true)];
    let current = vec![col_state(None, "col", "text", true, "", false, false)];
    let ddl = generate_alter_ddl("audit", "logs", "logs", &originals, &current, &[], &[]);
    assert!(ddl[0].contains("\"audit\".\"logs\""), "Got: {}", ddl[0]);
}

#[test]
fn test_rename_table_uses_new_name_for_subsequent() {
    let originals = vec![];
    let current = vec![col_state(None, "col", "text", true, "", false, false)];
    let ddl = generate_alter_ddl("public", "old_name", "new_name", &originals, &current, &[], &[]);
    assert!(ddl.len() >= 2, "Expected rename + add column, got: {ddl:?}");
    assert!(ddl[0].contains("RENAME TO \"new_name\""));
    assert!(ddl[1].contains("\"public\".\"new_name\""), "Subsequent DDL should use new name, got: {}", ddl[1]);
}

// ── Empty column name skipped ──────────────────────────────────────

#[test]
fn test_empty_column_name_skipped() {
    let originals = vec![];
    let current = vec![col_state(None, "", "text", true, "", false, false)];
    let ddl = generate_alter_ddl("public", "t", "t", &originals, &current, &[], &[]);
    assert!(ddl.is_empty());
}

#[test]
fn test_empty_index_name_skipped() {
    let current_idx = vec![idx_state(None, "", "col", false, false)];
    let ddl = generate_alter_ddl("public", "t", "t", &[], &[], &[], &current_idx);
    assert!(ddl.is_empty());
}
