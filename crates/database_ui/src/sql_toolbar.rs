use crate::database_panel::DatabasePanel;
use crate::sql_completion::SqlCompletionProvider;
use editor::Editor;
use gpui::{
    anchored, deferred, div, Context, Corner, DismissEvent, Entity, EventEmitter, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window,
};
use multi_buffer;
use std::rc::Rc;
use ui::{prelude::*, ContextMenu, Tooltip};
use workspace::item::ItemHandle;
use workspace::{ToolbarItemEvent, ToolbarItemLocation, ToolbarItemView, Workspace};

pub struct SqlToolbarItem {
    workspace: gpui::WeakEntity<Workspace>,
    active: bool,
    schema_menu: Option<(Entity<ContextMenu>, Subscription)>,
    tx_menu: Option<(Entity<ContextMenu>, Subscription)>,
}

impl SqlToolbarItem {
    pub fn new(workspace: gpui::WeakEntity<Workspace>) -> Self {
        Self {
            workspace,
            active: true,
            schema_menu: None,
            tx_menu: None,
        }
    }

    fn get_sql(workspace: &Workspace, cx: &gpui::App) -> Option<String> {
        let editor_entity = workspace.active_item(cx)?.act_as::<Editor>(cx)?;
        let editor = editor_entity.read(cx);

        let newest = editor.selections.newest_anchor();
        if newest.start != newest.end {
            let snapshot = editor.buffer().read(cx).snapshot(cx);
            let start: multi_buffer::MultiBufferOffset = snapshot.summary_for_anchor(&newest.start);
            let end: multi_buffer::MultiBufferOffset = snapshot.summary_for_anchor(&newest.end);
            if start != end {
                let text: String = snapshot.text_for_range(start..end).collect();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
        }

        let text = editor.text(cx).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }

    fn is_sql_editor(editor: &Entity<Editor>, cx: &gpui::App) -> bool {
        let editor_ref = editor.read(cx);
        let buffer = editor_ref.buffer().read(cx);

        if buffer
            .explicit_title()
            .is_some_and(|t| t.contains("console"))
        {
            return true;
        }

        if let Some(singleton) = buffer.as_singleton() {
            let buf = singleton.read(cx);
            if let Some(file) = buf.file() {
                let name = file.file_name(cx);
                if name.ends_with(".sql") {
                    return true;
                }
            }
            if let Some(lang) = buf.language() {
                let name = lang.name();
                if name.as_ref().eq_ignore_ascii_case("sql") {
                    return true;
                }
            }
        }

        false
    }

    fn selector_label(&self, cx: &gpui::App) -> Option<SharedString> {
        let ws = self.workspace.upgrade()?;
        let panel = ws.read(cx).panel::<DatabasePanel>(cx)?;
        let panel_ref = panel.read(cx);
        let (db_name, active_schema) = panel_ref.active_connection_info()?;

        if panel_ref.is_multi_database() {
            let active_db = panel_ref
                .active_database_name()
                .unwrap_or(&db_name);
            Some(format!("{active_db}.{active_schema}").into())
        } else {
            Some(format!("{db_name}.{active_schema}").into())
        }
    }

    fn tx_label(&self, cx: &gpui::App) -> Option<(String, bool)> {
        let ws = self.workspace.upgrade()?;
        let panel = ws.read(cx).panel::<DatabasePanel>(cx)?;
        let panel_ref = panel.read(cx);
        let mode_label = match panel_ref.transaction_mode {
            crate::database_panel::TransactionMode::Auto => "Auto",
            crate::database_panel::TransactionMode::Manual => "Manual",
        };
        Some((format!("Tx: {mode_label}"), panel_ref.in_transaction))
    }

    fn deploy_tx_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace.upgrade() else { return };
        let Some(panel) = ws.read(cx).panel::<DatabasePanel>(cx) else { return };
        let panel_ref = panel.read(cx);

        let current_mode = panel_ref.transaction_mode;
        let current_isolation = panel_ref.isolation_level;
        let panel_entity = panel.clone();

        use crate::database_panel::{IsolationLevel, TransactionMode};

        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            menu = menu.header("Transaction Mode");

            let modes = [
                (TransactionMode::Auto, "Auto"),
                (TransactionMode::Manual, "Manual"),
            ];
            for (mode, label) in modes {
                let is_active = mode == current_mode;
                let panel = panel_entity.clone();
                let display = if is_active {
                    format!("✓ {label}")
                } else {
                    format!("  {label}")
                };
                menu = menu.entry(display, None, move |_window, cx| {
                    panel.update(cx, |panel, cx| {
                        panel.set_transaction_mode(mode, cx);
                    });
                });
            }

            menu = menu.separator();
            menu = menu.header("Transaction Isolation");

            let levels = [
                IsolationLevel::DatabaseDefault,
                IsolationLevel::ReadCommitted,
                IsolationLevel::RepeatableRead,
                IsolationLevel::Serializable,
            ];
            for level in levels {
                let is_active = level == current_isolation;
                let panel = panel_entity.clone();
                let label = level.label();
                let display = if is_active {
                    format!("✓ {label}")
                } else {
                    format!("  {label}")
                };
                menu = menu.entry(display, None, move |_window, cx| {
                    panel.update(cx, |panel, cx| {
                        panel.set_isolation_level(level, cx);
                    });
                });
            }

            menu
        });

        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, _cx| {
            this.tx_menu.take();
        });
        self.tx_menu = Some((menu, subscription));
        cx.notify();
    }

    fn deploy_selector_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.workspace.upgrade() else { return };
        let Some(panel) = ws.read(cx).panel::<DatabasePanel>(cx) else { return };
        let panel_ref = panel.read(cx);

        let is_multi_db = panel_ref.is_multi_database();
        let databases = panel_ref.available_databases();
        let active_schema = panel_ref
            .active_schema_name()
            .unwrap_or("public")
            .to_string();
        let active_db = panel_ref
            .active_database_name()
            .unwrap_or("")
            .to_string();

        let schemas = panel_ref.available_schemas();

        // Collect schemas per database for multi-db providers
        let schemas_per_db: std::collections::HashMap<String, Vec<String>> = databases
            .iter()
            .map(|db| (db.clone(), panel_ref.schemas_for_database(db)))
            .collect();

        let panel_entity = panel.clone();

        let menu = if is_multi_db && !databases.is_empty() {
            ContextMenu::build(window, cx, move |mut menu, _, _| {
                for db in &databases {
                    let db_name = db.clone();
                    let is_active_db = db_name == active_db;
                    let panel_for_db = panel_entity.clone();
                    let db_schemas = schemas_per_db
                        .get(&db_name)
                        .cloned()
                        .unwrap_or_default();
                    let active_schema_clone = active_schema.clone();

                    let label = if is_active_db {
                        format!("{db_name} (active)")
                    } else {
                        db_name.clone()
                    };

                    menu = menu.submenu(label, move |mut submenu, _window, _cx| {
                        if db_schemas.is_empty() {
                            // Schemas not loaded yet — offer to select this database
                            // (expanding the database in the tree will load schemas)
                            let db = db_name.clone();
                            let panel = panel_for_db.clone();
                            submenu = submenu.entry(
                                format!("Use {db}"),
                                None,
                                move |_window, cx| {
                                    panel.update(cx, |panel, cx| {
                                        panel.set_active_database(db.clone(), cx);
                                    });
                                },
                            );
                        } else {
                            for schema_name in &db_schemas {
                                let is_active =
                                    is_active_db && *schema_name == active_schema_clone;
                                let s = schema_name.clone();
                                let db = db_name.clone();
                                let panel = panel_for_db.clone();

                                let schema_label = if is_active {
                                    format!("{s} (active)")
                                } else {
                                    s.clone()
                                };

                                submenu =
                                    submenu.entry(schema_label, None, move |_window, cx| {
                                        panel.update(cx, |panel, cx| {
                                            panel.set_active_database(db.clone(), cx);
                                            panel.set_active_schema(s.clone(), cx);
                                        });
                                    });
                            }
                        }
                        submenu
                    });
                }
                menu
            })
        } else {
            // Single-database: flat list of schemas
            ContextMenu::build(window, cx, move |mut menu, _, _| {
                for schema_name in &schemas {
                    let is_active = *schema_name == active_schema;
                    let s = schema_name.clone();
                    let panel = panel_entity.clone();

                    let label = if is_active {
                        format!("{s} (active)")
                    } else {
                        s.clone()
                    };

                    menu = menu.entry(label, None, move |_window, cx| {
                        panel.update(cx, |panel, cx| {
                            panel.set_active_schema(s.clone(), cx);
                        });
                    });
                }
                menu
            })
        };

        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, _cx| {
            this.schema_menu.take();
        });
        self.schema_menu = Some((menu, subscription));
        cx.notify();
    }
}

impl ToolbarItemView for SqlToolbarItem {
    fn set_active_pane_item(
        &mut self,
        active_pane_item: Option<&dyn ItemHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ToolbarItemLocation {
        let is_sql = active_pane_item
            .and_then(|item| item.act_as::<Editor>(cx))
            .map(|editor| Self::is_sql_editor(&editor, cx))
            .unwrap_or(false);

        if !is_sql {
            return ToolbarItemLocation::Hidden;
        }

        if let Some(item) = active_pane_item {
            if let Some(editor) = item.act_as::<Editor>(cx) {
                let ws_weak = self.workspace.clone();
                let editor_weak = editor.downgrade();
                cx.spawn_in(window, async move |_this, cx| {
                    let Some(ws) = ws_weak.upgrade() else { return };
                    let Some(editor) = editor_weak.upgrade() else { return };
                    let _ = ws.update_in(cx, |workspace, _window, cx| {
                        let panel = workspace.panel::<DatabasePanel>(cx);
                        let schema_arc = panel.as_ref().map(|p| p.read(cx).active_schema_arc());
                        let schema_name_arc =
                            panel.as_ref().map(|p| p.read(cx).active_schema_name_arc());
                        if let (Some(schema), Some(schema_name)) = (schema_arc, schema_name_arc) {
                            let has_schema = schema.blocking_read().is_some();
                            if has_schema {
                                editor.update(cx, |editor, _cx| {
                                    let provider =
                                        SqlCompletionProvider::new(schema, schema_name);
                                    editor.set_completion_provider(Some(Rc::new(provider)));
                                });
                            }
                        }
                    });
                })
                .detach();
            }
        }

        ToolbarItemLocation::PrimaryRight
    }
}

impl EventEmitter<ToolbarItemEvent> for SqlToolbarItem {}

impl Render for SqlToolbarItem {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.active {
            return div().into_any_element();
        }

        let label = self.selector_label(cx);
        let tx_info = self.tx_label(cx);

        div()
            .flex()
            .items_center()
            .gap_1()
            .when_some(label, |d, label| {
                d.child(
                    Button::new("db-schema-selector", label)
                        .label_size(LabelSize::Small)
                        .style(ButtonStyle::Subtle)
                        .end_icon(
                            Icon::new(IconName::ChevronDown)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.deploy_selector_menu(window, cx);
                        })),
                )
            })
            // Transaction mode selector
            .when_some(tx_info, |d, (tx_label, in_tx)| {
                d.child(
                    Button::new("tx-selector", tx_label)
                        .label_size(LabelSize::Small)
                        .style(ButtonStyle::Subtle)
                        .color(if in_tx { Color::Warning } else { Color::Default })
                        .end_icon(
                            Icon::new(IconName::ChevronDown)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.deploy_tx_menu(window, cx);
                        })),
                )
                .when(in_tx, |d| {
                    d.child(
                        Label::new("In Tx")
                            .size(LabelSize::XSmall)
                            .color(Color::Warning),
                    )
                    .child(
                        IconButton::new("tx-commit", IconName::Check)
                            .icon_size(IconSize::Small)
                            .icon_color(Color::Success)
                            .tooltip(|_w, cx| Tooltip::simple("Commit Transaction", cx))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(ws) = this.workspace.upgrade() {
                                    ws.update(cx, |workspace, cx| {
                                        if let Some(panel) = workspace.panel::<DatabasePanel>(cx) {
                                            panel.update(cx, |panel, cx| {
                                                panel.run_sql("COMMIT;".into(), window, cx);
                                            });
                                        }
                                    });
                                }
                            })),
                    )
                    .child(
                        IconButton::new("tx-rollback", IconName::Close)
                            .icon_size(IconSize::Small)
                            .icon_color(Color::Error)
                            .tooltip(|_w, cx| Tooltip::simple("Rollback Transaction", cx))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(ws) = this.workspace.upgrade() {
                                    ws.update(cx, |workspace, cx| {
                                        if let Some(panel) = workspace.panel::<DatabasePanel>(cx) {
                                            panel.update(cx, |panel, cx| {
                                                panel.run_sql("ROLLBACK;".into(), window, cx);
                                            });
                                        }
                                    });
                                }
                            })),
                    )
                })
            })
            .child(
                IconButton::new("sql-run", IconName::PlayFilled)
                    .icon_size(IconSize::Small)
                    .icon_color(Color::Success)
                    .tooltip(|_w, cx| Tooltip::simple("Run SQL (Ctrl+Alt+R)", cx))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(ws) = this.workspace.upgrade() {
                            let sql = Self::get_sql(ws.read(cx), cx);
                            if let Some(sql) = sql {
                                ws.update(cx, |workspace, cx| {
                                    if let Some(panel) = workspace.panel::<DatabasePanel>(cx) {
                                        panel.update(cx, |panel, cx| {
                                            panel.run_sql(sql, window, cx);
                                        });
                                    }
                                });
                            }
                        }
                    })),
            )
            .child(
                IconButton::new("sql-explain", IconName::ListTree)
                    .icon_size(IconSize::Small)
                    .icon_color(Color::Muted)
                    .tooltip(|_w, cx| Tooltip::simple("Explain SQL (Ctrl+Alt+E)", cx))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(ws) = this.workspace.upgrade() {
                            let sql = Self::get_sql(ws.read(cx), cx);
                            if let Some(sql) = sql {
                                ws.update(cx, |workspace, cx| {
                                    if let Some(panel) = workspace.panel::<DatabasePanel>(cx) {
                                        panel.update(cx, |panel, cx| {
                                            panel.explain_sql(sql, window, cx);
                                        });
                                    }
                                });
                            }
                        }
                    })),
            )
            .when_some(self.schema_menu.as_ref(), |d, (menu, _)| {
                d.child(
                    deferred(
                        anchored()
                            .anchor(Corner::TopRight)
                            .child(menu.clone()),
                    )
                    .with_priority(1),
                )
            })
            .when_some(self.tx_menu.as_ref(), |d, (menu, _)| {
                d.child(
                    deferred(
                        anchored()
                            .anchor(Corner::TopRight)
                            .child(menu.clone()),
                    )
                    .with_priority(1),
                )
            })
            .into_any_element()
    }
}
