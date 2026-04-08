use crate::database_panel::*;
use database_core::Table;
use editor::Editor;
use gpui::{div, AnyElement, Context, Entity, FontWeight, IntoElement, ParentElement, PromptLevel, SharedString, Styled, Window};
use theme::ActiveTheme;
use ui::prelude::*;
use ui::Checkbox;

// ── Pure DDL generation (testable without GPUI) ────────────────────

pub(crate) struct ColumnState {
    pub(crate) original_name: Option<String>,
    pub(crate) name: String,
    pub(crate) data_type: String,
    pub(crate) nullable: bool,
    pub(crate) default_value: String,
    #[allow(dead_code)]
    pub(crate) is_primary_key: bool,
    pub(crate) marked_for_deletion: bool,
}

pub(crate) struct IndexState {
    pub(crate) original_name: Option<String>,
    pub(crate) name: String,
    pub(crate) columns: String,
    pub(crate) is_unique: bool,
    pub(crate) marked_for_deletion: bool,
}

pub(crate) fn generate_alter_ddl(
    schema: &str,
    original_name: &str,
    current_name: &str,
    original_columns: &[OriginalColumn],
    columns: &[ColumnState],
    original_indexes: &[OriginalIndex],
    indexes: &[IndexState],
) -> Vec<String> {
    let qualified_original = format!("\"{schema}\".\"{original_name}\"");
    let qualified_current = format!("\"{schema}\".\"{current_name}\"");

    let mut statements = Vec::new();

    // Table rename
    if !current_name.is_empty() && current_name != original_name {
        statements.push(format!(
            "ALTER TABLE {qualified_original} RENAME TO \"{current_name}\";",
        ));
    }

    let table_ref = if !current_name.is_empty() && current_name != original_name {
        &qualified_current
    } else {
        &qualified_original
    };

    // Drop columns
    for col in columns {
        if col.marked_for_deletion {
            if let Some(ref orig_name) = col.original_name {
                statements.push(format!(
                    "ALTER TABLE {table_ref} DROP COLUMN \"{orig_name}\";",
                ));
            }
        }
    }

    // Process columns
    for (i, col) in columns.iter().enumerate() {
        if col.marked_for_deletion || col.name.is_empty() {
            continue;
        }

        match col.original_name {
            None => {
                let mut stmt = format!(
                    "ALTER TABLE {table_ref} ADD COLUMN \"{}\" {}",
                    col.name, col.data_type,
                );
                if !col.nullable {
                    stmt.push_str(" NOT NULL");
                }
                if !col.default_value.is_empty() {
                    stmt.push_str(&format!(" DEFAULT {}", col.default_value));
                }
                stmt.push(';');
                statements.push(stmt);
            }
            Some(ref orig_name) => {
                let original = original_columns.get(i);

                if col.name != *orig_name {
                    statements.push(format!(
                        "ALTER TABLE {table_ref} RENAME COLUMN \"{orig_name}\" TO \"{}\";",
                        col.name,
                    ));
                }

                if let Some(orig) = original {
                    if col.data_type != orig.data_type {
                        statements.push(format!(
                            "ALTER TABLE {table_ref} ALTER COLUMN \"{}\" TYPE {};",
                            col.name, col.data_type,
                        ));
                    }

                    if col.nullable != orig.is_nullable {
                        if col.nullable {
                            statements.push(format!(
                                "ALTER TABLE {table_ref} ALTER COLUMN \"{}\" DROP NOT NULL;",
                                col.name,
                            ));
                        } else {
                            statements.push(format!(
                                "ALTER TABLE {table_ref} ALTER COLUMN \"{}\" SET NOT NULL;",
                                col.name,
                            ));
                        }
                    }

                    let orig_default = orig.default_value.as_deref().unwrap_or("").trim();
                    if col.default_value != orig_default {
                        if col.default_value.is_empty() {
                            statements.push(format!(
                                "ALTER TABLE {table_ref} ALTER COLUMN \"{}\" DROP DEFAULT;",
                                col.name,
                            ));
                        } else {
                            statements.push(format!(
                                "ALTER TABLE {table_ref} ALTER COLUMN \"{}\" SET DEFAULT {};",
                                col.name, col.default_value,
                            ));
                        }
                    }
                }
            }
        }
    }

    // Drop indexes
    for idx in indexes {
        if idx.marked_for_deletion {
            if let Some(ref orig_name) = idx.original_name {
                statements.push(format!("DROP INDEX \"{orig_name}\";"));
            }
        }
    }

    // Process indexes
    for (i, idx) in indexes.iter().enumerate() {
        if idx.marked_for_deletion || idx.name.is_empty() || idx.columns.is_empty() {
            continue;
        }

        match idx.original_name {
            None => {
                let unique = if idx.is_unique { "UNIQUE " } else { "" };
                statements.push(format!(
                    "CREATE {unique}INDEX \"{}\" ON {table_ref} ({});",
                    idx.name, idx.columns,
                ));
            }
            Some(ref orig_name) => {
                let original = original_indexes.get(i);
                let changed = original.map_or(true, |orig| {
                    idx.name != orig.name
                        || idx.columns != orig.columns.join(", ")
                        || idx.is_unique != orig.is_unique
                });

                if changed {
                    statements.push(format!("DROP INDEX \"{orig_name}\";"));
                    let unique = if idx.is_unique { "UNIQUE " } else { "" };
                    statements.push(format!(
                        "CREATE {unique}INDEX \"{}\" ON {table_ref} ({});",
                        idx.name, idx.columns,
                    ));
                }
            }
        }
    }

    statements
}

impl DatabasePanel {
    pub(crate) fn open_table_editor(
        &mut self,
        conn_name: &str,
        schema_name: &str,
        table_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (table, database) = self.find_table(conn_name, schema_name, table_name);
        let Some(table) = table else {
            return;
        };

        let name_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(table.name.clone(), window, cx);
            editor
        });

        let comment_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Table comment...", window, cx);
            editor
        });

        let mut original_columns = Vec::new();
        let mut columns = Vec::new();
        for col in &table.columns {
            original_columns.push(OriginalColumn {
                name: col.name.clone(),
                data_type: col.data_type.clone(),
                is_nullable: col.is_nullable,
                default_value: col.default_value.clone(),
                is_primary_key: col.is_primary_key,
            });

            let name_editor = cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_text(col.name.clone(), window, cx);
                editor
            });
            let type_editor = cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_text(col.data_type.clone(), window, cx);
                editor
            });
            let default_editor = cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_placeholder_text("DEFAULT", window, cx);
                if let Some(ref default_value) = col.default_value {
                    editor.set_text(default_value.clone(), window, cx);
                }
                editor
            });

            columns.push(ColumnEditor {
                original_name: Some(col.name.clone()),
                name_editor,
                type_editor,
                nullable: col.is_nullable,
                default_editor,
                is_primary_key: col.is_primary_key,
                marked_for_deletion: false,
            });
        }

        let mut original_indexes = Vec::new();
        let mut indexes = Vec::new();
        for idx in &table.indexes {
            if idx.is_primary {
                continue;
            }
            original_indexes.push(OriginalIndex {
                name: idx.name.clone(),
                columns: idx.columns.clone(),
                is_unique: idx.is_unique,
            });

            let name_editor = cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_text(idx.name.clone(), window, cx);
                editor
            });
            let columns_editor = cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_text(idx.columns.join(", "), window, cx);
                editor
            });

            indexes.push(IndexEditor {
                original_name: Some(idx.name.clone()),
                name_editor,
                columns_editor,
                is_unique: idx.is_unique,
                marked_for_deletion: false,
            });
        }

        self.table_editor = Some(TableEditor {
            conn_name: conn_name.to_string(),
            database,
            original_schema: schema_name.to_string(),
            original_name: table.name.clone(),
            original_columns,
            original_indexes,
            name_editor,
            comment_editor,
            columns,
            indexes,
            error_message: None,
        });
        cx.notify();
    }

    fn find_table(
        &self,
        conn_name: &str,
        schema_name: &str,
        table_name: &str,
    ) -> (Option<Table>, Option<String>) {
        if let Some(db_schema) = self.schemas.get(conn_name) {
            let table = db_schema
                .tables
                .iter()
                .find(|t| t.schema == schema_name && t.name == table_name)
                .cloned();
            return (table, None);
        }

        let prefix = format!("{conn_name}.");
        for (key, db_schema) in &self.database_schemas {
            if key.starts_with(&prefix) {
                let db_name = key.strip_prefix(&prefix).unwrap_or(&db_schema.name);
                let table = db_schema
                    .tables
                    .iter()
                    .find(|t| t.schema == schema_name && t.name == table_name)
                    .cloned();
                if table.is_some() {
                    return (table, Some(db_name.to_string()));
                }
            }
        }

        (None, None)
    }

    pub(crate) fn table_editor_add_column(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ref mut editor) = self.table_editor else {
            return;
        };

        let name_editor = cx.new(|cx| {
            let mut e = Editor::single_line(window, cx);
            e.set_placeholder_text("column_name", window, cx);
            e
        });
        let type_editor = cx.new(|cx| {
            let mut e = Editor::single_line(window, cx);
            e.set_placeholder_text("text", window, cx);
            e
        });
        let default_editor = cx.new(|cx| {
            let mut e = Editor::single_line(window, cx);
            e.set_placeholder_text("DEFAULT", window, cx);
            e
        });

        editor.columns.push(ColumnEditor {
            original_name: None,
            name_editor,
            type_editor,
            nullable: true,
            default_editor,
            is_primary_key: false,
            marked_for_deletion: false,
        });
        cx.notify();
    }

    pub(crate) fn table_editor_remove_column(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(ref mut editor) = self.table_editor else {
            return;
        };
        if let Some(column) = editor.columns.get_mut(index) {
            column.marked_for_deletion = !column.marked_for_deletion;
        }
        cx.notify();
    }

    pub(crate) fn table_editor_add_index(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ref mut editor) = self.table_editor else {
            return;
        };

        let name_editor = cx.new(|cx| {
            let mut e = Editor::single_line(window, cx);
            e.set_placeholder_text("index_name", window, cx);
            e
        });
        let columns_editor = cx.new(|cx| {
            let mut e = Editor::single_line(window, cx);
            e.set_placeholder_text("col1, col2", window, cx);
            e
        });

        editor.indexes.push(IndexEditor {
            original_name: None,
            name_editor,
            columns_editor,
            is_unique: false,
            marked_for_deletion: false,
        });
        cx.notify();
    }

    pub(crate) fn table_editor_toggle_nullable(
        &mut self,
        col_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(ref mut editor) = self.table_editor else {
            return;
        };
        if let Some(column) = editor.columns.get_mut(col_index) {
            column.nullable = !column.nullable;
        }
        cx.notify();
    }

    pub(crate) fn table_editor_toggle_pk(
        &mut self,
        col_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(ref mut editor) = self.table_editor else {
            return;
        };
        if let Some(column) = editor.columns.get_mut(col_index) {
            column.is_primary_key = !column.is_primary_key;
        }
        cx.notify();
    }

    pub(crate) fn table_editor_toggle_index_unique(
        &mut self,
        idx_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(ref mut editor) = self.table_editor else {
            return;
        };
        if let Some(index) = editor.indexes.get_mut(idx_index) {
            index.is_unique = !index.is_unique;
        }
        cx.notify();
    }

    pub(crate) fn table_editor_remove_index(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(ref mut editor) = self.table_editor else {
            return;
        };
        if let Some(idx) = editor.indexes.get_mut(index) {
            idx.marked_for_deletion = !idx.marked_for_deletion;
        }
        cx.notify();
    }

    pub(crate) fn table_editor_generate_ddl(&self, cx: &Context<Self>) -> Vec<String> {
        let Some(ref editor) = self.table_editor else {
            return Vec::new();
        };

        let current_name = editor.name_editor.read(cx).text(cx).trim().to_string();

        let columns: Vec<ColumnState> = editor
            .columns
            .iter()
            .map(|col| ColumnState {
                original_name: col.original_name.clone(),
                name: col.name_editor.read(cx).text(cx).trim().to_string(),
                data_type: col.type_editor.read(cx).text(cx).trim().to_string(),
                nullable: col.nullable,
                default_value: col.default_editor.read(cx).text(cx).trim().to_string(),
                is_primary_key: col.is_primary_key,
                marked_for_deletion: col.marked_for_deletion,
            })
            .collect();

        let indexes: Vec<IndexState> = editor
            .indexes
            .iter()
            .map(|idx| IndexState {
                original_name: idx.original_name.clone(),
                name: idx.name_editor.read(cx).text(cx).trim().to_string(),
                columns: idx.columns_editor.read(cx).text(cx).trim().to_string(),
                is_unique: idx.is_unique,
                marked_for_deletion: idx.marked_for_deletion,
            })
            .collect();

        generate_alter_ddl(
            &editor.original_schema,
            &editor.original_name,
            &current_name,
            &editor.original_columns,
            &columns,
            &editor.original_indexes,
            &indexes,
        )
    }

    pub(crate) fn table_editor_apply(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ddl = self.table_editor_generate_ddl(cx);
        if ddl.is_empty() {
            self.table_editor = None;
            cx.notify();
            return;
        }

        let detail = ddl.join("\n");
        let answer = window.prompt(
            PromptLevel::Warning,
            "Apply table changes?",
            Some(&detail),
            &["Apply", "Cancel"],
            cx,
        );

        let sql = ddl.join("\n");

        let task = cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }

            let _ = this.update_in(cx, |this, window, cx| {
                this.run_sql(sql, window, cx);
                this.table_editor = None;

                // Refresh the tree after applying changes
                if let Some(conn_name) = this.active_query_connection.clone() {
                    this.connect(&conn_name, window, cx);
                }
                cx.notify();
            });
        });
        self._pending_task = Some(task);
    }

    pub(crate) fn table_editor_cancel(&mut self, cx: &mut Context<Self>) {
        self.table_editor = None;
        cx.notify();
    }

    pub(crate) fn render_table_editor(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(ref editor) = self.table_editor else {
            return div().into_any_element();
        };

        let schema = &editor.original_schema;
        let original_name = &editor.original_name;
        let header_text: SharedString =
            format!("Modify Table: {schema}.{original_name}").into();

        let ddl_preview = self.table_editor_generate_ddl(cx);

        div()
            .id("table-editor-scroll")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    // Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::new(IconName::Settings)
                                    .size(IconSize::Small)
                                    .color(Color::Accent),
                            )
                            .child(
                                Label::new(header_text)
                                    .size(LabelSize::Small)
                                    .weight(FontWeight::SEMIBOLD),
                            ),
                    )
                    // Name field
                    .child(self.render_table_editor_field("Table Name", &editor.name_editor, cx))
                    // Comment field
                    .child(self.render_table_editor_field("Comment", &editor.comment_editor, cx))
                    // Columns section
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                Label::new("Columns")
                                    .size(LabelSize::Small)
                                    .weight(FontWeight::SEMIBOLD),
                            )
                            .child(self.render_columns_table(cx))
                            .child(
                                Button::new("add_column", "+ Add Column")
                                    .style(ButtonStyle::Subtle)
                                    .label_size(LabelSize::XSmall)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.table_editor_add_column(window, cx);
                                    })),
                            ),
                    )
                    // Indexes section
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                Label::new("Indexes")
                                    .size(LabelSize::Small)
                                    .weight(FontWeight::SEMIBOLD),
                            )
                            .child(self.render_indexes_table(cx))
                            .child(
                                Button::new("add_index", "+ Add Index")
                                    .style(ButtonStyle::Subtle)
                                    .label_size(LabelSize::XSmall)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.table_editor_add_index(window, cx);
                                    })),
                            ),
                    )
                    // DDL Preview (always visible)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Icon::new(IconName::ListTree)
                                            .size(IconSize::XSmall)
                                            .color(Color::Accent),
                                    )
                                    .child(
                                        Label::new("Preview")
                                            .size(LabelSize::Small)
                                            .weight(FontWeight::SEMIBOLD),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(cx.theme().colors().border)
                                    .bg(cx.theme().colors().editor_background)
                                    .min_h(px(60.))
                                    .when(ddl_preview.is_empty(), |d| {
                                        d.child(
                                            Label::new("No changes detected")
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                    })
                                    .when(!ddl_preview.is_empty(), |d| {
                                        let mut container = div().flex().flex_col().gap_px();
                                        for stmt in &ddl_preview {
                                            container = container.child(
                                                Label::new(stmt.clone())
                                                    .size(LabelSize::XSmall)
                                                    .color(Color::Success),
                                            );
                                        }
                                        d.child(container)
                                    }),
                            ),
                    )
                    // Error message
                    .when_some(
                        editor.error_message.clone(),
                        |container, message| {
                            container.child(
                                Label::new(message)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Error),
                            )
                        },
                    )
                    // Buttons
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .pt_1()
                            .w_full()
                            .child(
                                div().flex_1().child(
                                    Button::new("te_cancel", "Cancel")
                                        .full_width()
                                        .style(ButtonStyle::Subtle)
                                        .on_click(cx.listener(|this, _, _window, cx| {
                                            this.table_editor_cancel(cx);
                                        })),
                                ),
                            )
                            .child(
                                div().flex_1().child(
                                    Button::new("te_apply", "Apply")
                                        .full_width()
                                        .style(ButtonStyle::Filled)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.table_editor_apply(window, cx);
                                        })),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_table_editor_field(
        &self,
        label: &'static str,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_px()
            .w_full()
            .child(
                Label::new(label)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                div()
                    .w_full()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .bg(cx.theme().colors().editor_background)
                    .child(editor.clone()),
            )
            .into_any_element()
    }

    fn render_columns_table(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(ref editor) = self.table_editor else {
            return div().into_any_element();
        };

        let mut table = div().flex().flex_col().w_full();

        // Header row
        table = table.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .px_1()
                .py_px()
                .border_b_1()
                .border_color(cx.theme().colors().border)
                .child(div().w(px(120.)).child(
                    Label::new("Name").size(LabelSize::XSmall).weight(FontWeight::SEMIBOLD).color(Color::Muted),
                ))
                .child(div().w(px(100.)).child(
                    Label::new("Type").size(LabelSize::XSmall).weight(FontWeight::SEMIBOLD).color(Color::Muted),
                ))
                .child(div().w(px(40.)).child(
                    Label::new("Null").size(LabelSize::XSmall).weight(FontWeight::SEMIBOLD).color(Color::Muted),
                ))
                .child(div().w(px(32.)).child(
                    Label::new("PK").size(LabelSize::XSmall).weight(FontWeight::SEMIBOLD).color(Color::Muted),
                ))
                .child(div().flex_1().child(
                    Label::new("Default").size(LabelSize::XSmall).weight(FontWeight::SEMIBOLD).color(Color::Muted),
                ))
                .child(div().w(px(50.)).child(
                    Label::new("Delete").size(LabelSize::XSmall).weight(FontWeight::SEMIBOLD).color(Color::Muted),
                )),
        );

        // Column rows
        for (i, col) in editor.columns.iter().enumerate() {
            let row_opacity = if col.marked_for_deletion { 0.4 } else { 1.0 };
            let row_bg = if col.marked_for_deletion {
                cx.theme().status().error_background
            } else {
                gpui::transparent_black()
            };

            let nullable_state = if col.nullable {
                ToggleState::Selected
            } else {
                ToggleState::Unselected
            };

            let pk_state = if col.is_primary_key {
                ToggleState::Selected
            } else {
                ToggleState::Unselected
            };

            let col_index = i;
            table = table.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_1()
                    .py_px()
                    .bg(row_bg)
                    .opacity(row_opacity)
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        div()
                            .w(px(120.))
                            .px_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .bg(cx.theme().colors().editor_background)
                            .child(col.name_editor.clone()),
                    )
                    .child(
                        div()
                            .w(px(100.))
                            .px_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .bg(cx.theme().colors().editor_background)
                            .child(col.type_editor.clone()),
                    )
                    .child(
                        div().w(px(40.)).flex().justify_center().child(
                            Checkbox::new(
                                ElementId::Name(format!("nullable-{col_index}").into()),
                                nullable_state,
                            )
                            .on_click(cx.listener(move |this, _, _window, _cx| {
                                this.table_editor_toggle_nullable(col_index, _cx);
                            })),
                        ),
                    )
                    .child(
                        div().w(px(32.)).flex().justify_center().child(
                            Checkbox::new(
                                ElementId::Name(format!("pk-{col_index}").into()),
                                pk_state,
                            )
                            .on_click(cx.listener(move |this, _, _window, _cx| {
                                this.table_editor_toggle_pk(col_index, _cx);
                            })),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .px_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .bg(cx.theme().colors().editor_background)
                            .child(col.default_editor.clone()),
                    )
                    .child(
                        div().w(px(50.)).flex().justify_center().child(
                            IconButton::new(
                                ElementId::Name(format!("del-col-{col_index}").into()),
                                if col.marked_for_deletion {
                                    IconName::Undo
                                } else {
                                    IconName::Trash
                                },
                            )
                            .icon_size(IconSize::XSmall)
                            .icon_color(if col.marked_for_deletion {
                                Color::Warning
                            } else {
                                Color::Error
                            })
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.table_editor_remove_column(col_index, cx);
                            })),
                        ),
                    ),
            );
        }

        table.into_any_element()
    }

    fn render_indexes_table(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(ref editor) = self.table_editor else {
            return div().into_any_element();
        };

        let mut table = div().flex().flex_col().w_full();

        // Header row
        table = table.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .px_1()
                .py_px()
                .border_b_1()
                .border_color(cx.theme().colors().border)
                .child(div().w(px(120.)).child(
                    Label::new("Name").size(LabelSize::XSmall).weight(FontWeight::SEMIBOLD).color(Color::Muted),
                ))
                .child(div().flex_1().child(
                    Label::new("Columns").size(LabelSize::XSmall).weight(FontWeight::SEMIBOLD).color(Color::Muted),
                ))
                .child(div().w(px(50.)).child(
                    Label::new("Unique").size(LabelSize::XSmall).weight(FontWeight::SEMIBOLD).color(Color::Muted),
                ))
                .child(div().w(px(50.)).child(
                    Label::new("Delete").size(LabelSize::XSmall).weight(FontWeight::SEMIBOLD).color(Color::Muted),
                )),
        );

        // Index rows
        for (i, idx) in editor.indexes.iter().enumerate() {
            let row_opacity = if idx.marked_for_deletion { 0.4 } else { 1.0 };
            let row_bg = if idx.marked_for_deletion {
                cx.theme().status().error_background
            } else {
                gpui::transparent_black()
            };

            let unique_state = if idx.is_unique {
                ToggleState::Selected
            } else {
                ToggleState::Unselected
            };

            let idx_index = i;
            table = table.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_1()
                    .py_px()
                    .bg(row_bg)
                    .opacity(row_opacity)
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        div()
                            .w(px(120.))
                            .px_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .bg(cx.theme().colors().editor_background)
                            .child(idx.name_editor.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .px_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .bg(cx.theme().colors().editor_background)
                            .child(idx.columns_editor.clone()),
                    )
                    .child(
                        div().w(px(50.)).flex().justify_center().child(
                            Checkbox::new(
                                ElementId::Name(format!("unique-{idx_index}").into()),
                                unique_state,
                            )
                            .on_click(cx.listener(move |this, _, _window, _cx| {
                                this.table_editor_toggle_index_unique(idx_index, _cx);
                            })),
                        ),
                    )
                    .child(
                        div().w(px(50.)).flex().justify_center().child(
                            IconButton::new(
                                ElementId::Name(format!("del-idx-{idx_index}").into()),
                                if idx.marked_for_deletion {
                                    IconName::Undo
                                } else {
                                    IconName::Trash
                                },
                            )
                            .icon_size(IconSize::XSmall)
                            .icon_color(if idx.marked_for_deletion {
                                Color::Warning
                            } else {
                                Color::Error
                            })
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.table_editor_remove_index(idx_index, cx);
                            })),
                        ),
                    ),
            );
        }

        table.into_any_element()
    }
}
