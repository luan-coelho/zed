use crate::formatter::{format_query_result, format_schema};
use crate::schema::*;

fn sample_result() -> QueryResult {
    QueryResult {
        columns: vec!["id".into(), "name".into(), "email".into()],
        rows: vec![
            vec!["1".into(), "Alice".into(), "alice@example.com".into()],
            vec!["2".into(), "Bob".into(), "bob@example.com".into()],
        ],
        rows_affected: 2,
        execution_time_ms: 42,
    }
}

fn sample_schema() -> DatabaseSchema {
    DatabaseSchema {
        name: "testdb".into(),
        tables: vec![Table {
            schema: "public".into(),
            name: "users".into(),
            columns: vec![
                DbColumn {
                    name: "id".into(),
                    data_type: "uuid".into(),
                    is_nullable: false,
                    default_value: Some("gen_random_uuid()".into()),
                    is_primary_key: true,
                    foreign_key: None,
                    comment: None,
                },
                DbColumn {
                    name: "name".into(),
                    data_type: "text".into(),
                    is_nullable: false,
                    default_value: None,
                    is_primary_key: false,
                    foreign_key: None,
                    comment: None,
                },
                DbColumn {
                    name: "org_id".into(),
                    data_type: "uuid".into(),
                    is_nullable: true,
                    default_value: None,
                    is_primary_key: false,
                    foreign_key: Some(ForeignKey {
                        referenced_table: "organizations".into(),
                        referenced_column: "id".into(),
                    }),
                    comment: None,
                },
            ],
            indexes: vec![Index {
                name: "users_pkey".into(),
                columns: vec!["id".into()],
                is_unique: true,
                is_primary: true,
            }],
            constraints: vec![],
            row_count_estimate: Some(1500),
        }],
        views: vec![View {
            schema: "public".into(),
            name: "active_users".into(),
            columns: vec![DbColumn {
                name: "id".into(),
                data_type: "uuid".into(),
                is_nullable: false,
                default_value: None,
                is_primary_key: false,
                foreign_key: None,
                comment: None,
            }],
        }],
    }
}

#[test]
fn test_format_query_result_header() {
    let result = sample_result();
    let md = format_query_result(&result);

    assert!(md.contains("## Query Results"));
    assert!(md.contains("**Rows:** 2"));
    assert!(md.contains("**Time:** 42ms"));
}

#[test]
fn test_format_query_result_columns() {
    let result = sample_result();
    let md = format_query_result(&result);

    assert!(md.contains("| id"));
    assert!(md.contains("| name"));
    assert!(md.contains("| email"));
}

#[test]
fn test_format_query_result_rows() {
    let result = sample_result();
    let md = format_query_result(&result);

    assert!(md.contains("Alice"));
    assert!(md.contains("bob@example.com"));
}

#[test]
fn test_format_query_result_empty() {
    let result = QueryResult {
        columns: vec![],
        rows: vec![],
        rows_affected: 0,
        execution_time_ms: 5,
    };
    let md = format_query_result(&result);
    assert!(md.contains("No columns returned"));
}

#[test]
fn test_format_schema_database_name() {
    let schema = sample_schema();
    let md = format_schema(&schema);
    assert!(md.contains("Database: `testdb`"));
}

#[test]
fn test_format_schema_table() {
    let schema = sample_schema();
    let md = format_schema(&schema);

    assert!(md.contains("Tables (1)"));
    assert!(md.contains("`public.users`"));
    assert!(md.contains("~1500 rows"));
}

#[test]
fn test_format_schema_columns() {
    let schema = sample_schema();
    let md = format_schema(&schema);

    assert!(md.contains("`id`"));
    assert!(md.contains("uuid"));
    assert!(md.contains("PK"));
    assert!(md.contains("`name`"));
    assert!(md.contains("text"));
}

#[test]
fn test_format_schema_foreign_key() {
    let schema = sample_schema();
    let md = format_schema(&schema);
    assert!(md.contains("FK -> organizations.id"));
}

#[test]
fn test_format_schema_indexes() {
    let schema = sample_schema();
    let md = format_schema(&schema);
    assert!(md.contains("users_pkey"));
    assert!(md.contains("PRIMARY"));
    assert!(md.contains("UNIQUE"));
}

#[test]
fn test_format_schema_views() {
    let schema = sample_schema();
    let md = format_schema(&schema);
    assert!(md.contains("Views (1)"));
    assert!(md.contains("`public.active_users`"));
}
