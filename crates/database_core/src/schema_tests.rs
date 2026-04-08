use crate::schema::{
    Constraint, ConstraintType, DatabaseEntry, DatabaseSchema, DbColumn, ForeignKey, Index,
    QueryResult, RoleEntry, SchemaEntry, Table, View,
};

fn sample_foreign_key() -> ForeignKey {
    ForeignKey {
        referenced_table: "departments".to_string(),
        referenced_column: "id".to_string(),
    }
}

fn sample_column_with_fk() -> DbColumn {
    DbColumn {
        name: "department_id".to_string(),
        data_type: "integer".to_string(),
        is_nullable: false,
        default_value: None,
        is_primary_key: false,
        foreign_key: Some(sample_foreign_key()),
        comment: Some("References the department".to_string()),
    }
}

fn sample_column_without_fk() -> DbColumn {
    DbColumn {
        name: "email".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        default_value: Some("''".to_string()),
        is_primary_key: false,
        foreign_key: None,
        comment: None,
    }
}

fn sample_index() -> Index {
    Index {
        name: "idx_users_email".to_string(),
        columns: vec!["email".to_string()],
        is_unique: true,
        is_primary: false,
    }
}

fn sample_primary_index() -> Index {
    Index {
        name: "pk_users".to_string(),
        columns: vec!["id".to_string()],
        is_unique: true,
        is_primary: true,
    }
}

fn sample_table() -> Table {
    Table {
        schema: "public".to_string(),
        name: "users".to_string(),
        columns: vec![
            DbColumn {
                name: "id".to_string(),
                data_type: "serial".to_string(),
                is_nullable: false,
                default_value: Some("nextval('users_id_seq')".to_string()),
                is_primary_key: true,
                foreign_key: None,
                comment: None,
            },
            sample_column_without_fk(),
            sample_column_with_fk(),
        ],
        indexes: vec![sample_primary_index(), sample_index()],
        constraints: vec![
            Constraint {
                name: "pk_users".to_string(),
                constraint_type: ConstraintType::PrimaryKey,
            },
            Constraint {
                name: "uq_users_email".to_string(),
                constraint_type: ConstraintType::Unique,
            },
        ],
        row_count_estimate: Some(1500),
    }
}

fn sample_view() -> View {
    View {
        schema: "public".to_string(),
        name: "active_users".to_string(),
        columns: vec![sample_column_without_fk()],
    }
}

#[test]
fn test_database_schema_serialization_roundtrip() {
    let schema = DatabaseSchema {
        name: "testdb".to_string(),
        tables: vec![sample_table()],
        views: vec![sample_view()],
    };

    let json = serde_json::to_string(&schema).expect("failed to serialize DatabaseSchema");
    let deserialized: DatabaseSchema =
        serde_json::from_str(&json).expect("failed to deserialize DatabaseSchema");

    assert_eq!(deserialized.name, "testdb");
    assert_eq!(deserialized.tables.len(), 1);
    assert_eq!(deserialized.tables[0].name, "users");
    assert_eq!(deserialized.tables[0].columns.len(), 3);
    assert_eq!(deserialized.views.len(), 1);
    assert_eq!(deserialized.views[0].name, "active_users");
}

#[test]
fn test_table_serialization_all_fields() {
    let table = sample_table();

    let json = serde_json::to_string(&table).expect("failed to serialize Table");
    let deserialized: Table = serde_json::from_str(&json).expect("failed to deserialize Table");

    assert_eq!(deserialized.schema, "public");
    assert_eq!(deserialized.name, "users");
    assert_eq!(deserialized.columns.len(), 3);
    assert_eq!(deserialized.indexes.len(), 2);
    assert_eq!(deserialized.constraints.len(), 2);
    assert_eq!(deserialized.row_count_estimate, Some(1500));
}

#[test]
fn test_db_column_with_foreign_key() {
    let column = sample_column_with_fk();

    let json = serde_json::to_string(&column).expect("failed to serialize DbColumn with FK");
    let deserialized: DbColumn =
        serde_json::from_str(&json).expect("failed to deserialize DbColumn with FK");

    assert_eq!(deserialized.name, "department_id");
    assert_eq!(deserialized.data_type, "integer");
    assert!(!deserialized.is_nullable);
    assert!(deserialized.default_value.is_none());
    assert!(!deserialized.is_primary_key);

    let foreign_key = deserialized
        .foreign_key
        .expect("expected foreign_key to be present");
    assert_eq!(foreign_key.referenced_table, "departments");
    assert_eq!(foreign_key.referenced_column, "id");

    assert_eq!(
        deserialized.comment.as_deref(),
        Some("References the department")
    );
}

#[test]
fn test_db_column_without_foreign_key() {
    let column = sample_column_without_fk();

    let json = serde_json::to_string(&column).expect("failed to serialize DbColumn without FK");
    let deserialized: DbColumn =
        serde_json::from_str(&json).expect("failed to deserialize DbColumn without FK");

    assert_eq!(deserialized.name, "email");
    assert_eq!(deserialized.data_type, "varchar(255)");
    assert!(deserialized.is_nullable);
    assert_eq!(deserialized.default_value.as_deref(), Some("''"));
    assert!(!deserialized.is_primary_key);
    assert!(deserialized.foreign_key.is_none());
    assert!(deserialized.comment.is_none());
}

#[test]
fn test_index_serialization() {
    let unique_index = sample_index();
    let primary_index = sample_primary_index();

    let json_unique =
        serde_json::to_string(&unique_index).expect("failed to serialize unique Index");
    let deserialized_unique: Index =
        serde_json::from_str(&json_unique).expect("failed to deserialize unique Index");
    assert_eq!(deserialized_unique.name, "idx_users_email");
    assert_eq!(deserialized_unique.columns, vec!["email"]);
    assert!(deserialized_unique.is_unique);
    assert!(!deserialized_unique.is_primary);

    let json_primary =
        serde_json::to_string(&primary_index).expect("failed to serialize primary Index");
    let deserialized_primary: Index =
        serde_json::from_str(&json_primary).expect("failed to deserialize primary Index");
    assert_eq!(deserialized_primary.name, "pk_users");
    assert_eq!(deserialized_primary.columns, vec!["id"]);
    assert!(deserialized_primary.is_unique);
    assert!(deserialized_primary.is_primary);
}

#[test]
fn test_constraint_with_each_type() {
    let variants = vec![
        (
            Constraint {
                name: "pk_test".to_string(),
                constraint_type: ConstraintType::PrimaryKey,
            },
            "PrimaryKey",
        ),
        (
            Constraint {
                name: "fk_test".to_string(),
                constraint_type: ConstraintType::ForeignKey,
            },
            "ForeignKey",
        ),
        (
            Constraint {
                name: "uq_test".to_string(),
                constraint_type: ConstraintType::Unique,
            },
            "Unique",
        ),
        (
            Constraint {
                name: "ck_test".to_string(),
                constraint_type: ConstraintType::Check,
            },
            "Check",
        ),
    ];

    for (constraint, expected_type_str) in variants {
        let json = serde_json::to_string(&constraint)
            .unwrap_or_else(|_| panic!("failed to serialize {expected_type_str} Constraint"));
        let deserialized: Constraint = serde_json::from_str(&json)
            .unwrap_or_else(|_| panic!("failed to deserialize {expected_type_str} Constraint"));

        assert_eq!(deserialized.name, constraint.name);
        assert!(json.contains(expected_type_str));
    }
}

#[test]
fn test_query_result_serialization() {
    let result = QueryResult {
        columns: vec!["id".to_string(), "name".to_string()],
        rows: vec![
            vec!["1".to_string(), "Alice".to_string()],
            vec!["2".to_string(), "Bob".to_string()],
        ],
        rows_affected: 2,
        execution_time_ms: 42,
    };

    let json = serde_json::to_string(&result).expect("failed to serialize QueryResult");
    let deserialized: QueryResult =
        serde_json::from_str(&json).expect("failed to deserialize QueryResult");

    assert_eq!(deserialized.columns, vec!["id", "name"]);
    assert_eq!(deserialized.rows.len(), 2);
    assert_eq!(deserialized.rows[0], vec!["1", "Alice"]);
    assert_eq!(deserialized.rows[1], vec!["2", "Bob"]);
    assert_eq!(deserialized.rows_affected, 2);
    assert_eq!(deserialized.execution_time_ms, 42);
}

#[test]
fn test_database_entry_serialization() {
    let entry = DatabaseEntry {
        name: "production_db".to_string(),
        is_current: true,
    };

    let json = serde_json::to_string(&entry).expect("failed to serialize DatabaseEntry");
    let deserialized: DatabaseEntry =
        serde_json::from_str(&json).expect("failed to deserialize DatabaseEntry");

    assert_eq!(deserialized.name, "production_db");
    assert!(deserialized.is_current);
}

#[test]
fn test_schema_entry_serialization() {
    let entry = SchemaEntry {
        name: "pg_catalog".to_string(),
        is_system: true,
    };

    let json = serde_json::to_string(&entry).expect("failed to serialize SchemaEntry");
    let deserialized: SchemaEntry =
        serde_json::from_str(&json).expect("failed to deserialize SchemaEntry");

    assert_eq!(deserialized.name, "pg_catalog");
    assert!(deserialized.is_system);

    let user_entry = SchemaEntry {
        name: "public".to_string(),
        is_system: false,
    };
    let json2 = serde_json::to_string(&user_entry).expect("failed to serialize user SchemaEntry");
    let deserialized2: SchemaEntry =
        serde_json::from_str(&json2).expect("failed to deserialize user SchemaEntry");
    assert_eq!(deserialized2.name, "public");
    assert!(!deserialized2.is_system);
}

#[test]
fn test_role_entry_serialization() {
    let role = RoleEntry {
        name: "admin".to_string(),
        is_superuser: true,
        can_login: true,
        can_create_db: true,
        can_create_role: true,
    };

    let json = serde_json::to_string(&role).expect("failed to serialize RoleEntry");
    let deserialized: RoleEntry =
        serde_json::from_str(&json).expect("failed to deserialize RoleEntry");

    assert_eq!(deserialized.name, "admin");
    assert!(deserialized.is_superuser);
    assert!(deserialized.can_login);
    assert!(deserialized.can_create_db);
    assert!(deserialized.can_create_role);

    let restricted_role = RoleEntry {
        name: "readonly".to_string(),
        is_superuser: false,
        can_login: true,
        can_create_db: false,
        can_create_role: false,
    };
    let json2 =
        serde_json::to_string(&restricted_role).expect("failed to serialize restricted RoleEntry");
    let deserialized2: RoleEntry =
        serde_json::from_str(&json2).expect("failed to deserialize restricted RoleEntry");
    assert_eq!(deserialized2.name, "readonly");
    assert!(!deserialized2.is_superuser);
    assert!(deserialized2.can_login);
    assert!(!deserialized2.can_create_db);
    assert!(!deserialized2.can_create_role);
}
