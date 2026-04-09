use crate::database_panel::*;
use gpui::{Context, DismissEvent, Entity, Focusable, Point, PromptLevel, Window};
use ui::{ContextMenu, Pixels};

impl DatabasePanel {
    fn run_sql_with_confirmation(
        &mut self,
        sql: String,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(&sql),
            &["Execute", "Cancel"],
            cx,
        );

        let conn_manager = self.connection_manager.clone();
        let weak_workspace = self._workspace.clone();
        let schema = self.active_schema.clone();
        let database = self.active_query_database.clone();

        let task = cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }

            let conn_mgr = conn_manager.clone();
            let sql_clone = sql.clone();
            let result = gpui_tokio::Tokio::spawn_result(cx, async move {
                let mut mgr = conn_mgr.write().await;
                match database {
                    Some(ref db) => {
                        mgr.execute_query_in_database(&sql_clone, db, schema.as_deref())
                            .await
                    }
                    None => {
                        mgr.execute_query_in_schema(&sql_clone, schema.as_deref())
                            .await
                    }
                }
            })
            .await;

            let query_result = match result {
                Ok(qr) => qr,
                Err(e) => database_core::QueryResult {
                    columns: vec!["Error".to_string()],
                    rows: vec![vec![format!("{e:#}")]],
                    rows_affected: 0,
                    execution_time_ms: 0,
                },
            };

            Self::push_query_result(&weak_workspace, sql, query_result, cx);

            let _ = this.update_in(cx, |this, window, cx| {
                // Refresh the tree after destructive operation
                if let Some(conn_name) = this.active_query_connection.clone() {
                    this.connect(&conn_name, window, cx);
                }
            });
        });
        self._pending_task = Some(task);
    }

    pub(crate) fn set_context_menu(
        &mut self,
        menu: Entity<ContextMenu>,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let subscription = cx.subscribe_in(
            &menu,
            window,
            |this, _, _: &DismissEvent, window, cx| {
                if this
                    .context_menu
                    .as_ref()
                    .is_some_and(|m| m.0.focus_handle(cx).contains_focused(window, cx))
                {
                    cx.focus_self(window);
                }
                this.context_menu.take();
                cx.notify();
            },
        );
        self.context_menu = Some((menu, position, subscription));
        cx.notify();
    }

    pub(crate) fn deploy_new_connection_menu_at(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();

        let has_project = self.project_path.is_some();
        let registry = database_core::ProviderRegistry::new();
        let providers = registry.available_providers();
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            let mut m = menu;

            for info in &providers {
                let w = weak.clone();
                let id = info.id.to_string();
                let label = info.display_name;
                m = m.entry(format!("{label} (Global)"), None, {
                    let w = w.clone();
                    let id = id.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                this.show_connection_form_with_provider(&id, ConnectionScope::Global, window, cx);
                            });
                        }
                    }
                });
                if has_project {
                    m = m.entry(format!("{label} (Project)"), None, {
                        let w = w.clone();
                        let id = id.clone();
                        move |window, cx| {
                            if let Some(panel) = w.upgrade() {
                                panel.update(cx, |this, cx| {
                                    this.show_connection_form_with_provider(&id, ConnectionScope::Project, window, cx);
                                });
                            }
                        }
                    });
                }
            }

            m
        });

        self.set_context_menu(menu, position, window, cx);
    }

    pub(crate) fn show_connection_form_with_provider(
        &mut self,
        provider: &str,
        scope: ConnectionScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Set the provider before creating the form so placeholders match
        self.show_connection_form_for_provider(None, scope, provider, window, cx);
    }

    pub(crate) fn deploy_connection_context_menu(
        &mut self,
        conn_name: String,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_connected = self
            .connections
            .iter()
            .find(|c| c.name == conn_name)
            .map(|c| c.status == ConnectionStatus::Connected)
            .unwrap_or(false);

        let caps = self
            .connection_capabilities
            .get(&conn_name)
            .cloned()
            .unwrap_or_default();

        let weak = cx.entity().downgrade();

        let menu = ContextMenu::build(window, cx, {
            let name = conn_name.clone();
            move |menu, _, _| {
                let mut m = menu;
                if is_connected {
                    // "New >" submenu with provider-specific options
                    m = m.submenu("New", {
                        let w = weak.clone();
                        let n = name.clone();
                        let caps = caps.clone();
                        move |submenu, _window, _cx| {
                            let mut s = submenu;
                            // Query Console — always available
                            s = s.entry("Query Console", None, {
                                let w = w.clone();
                                let n = n.clone();
                                move |window, cx| {
                                    if let Some(panel) = w.upgrade() {
                                        panel.update(cx, |this, cx| {
                                            this.open_query_console(&n, None, window, cx);
                                        });
                                    }
                                }
                            });
                            if caps.create_database {
                                s = s.entry("Database", None, {
                                    let w = w.clone();
                                    move |window, cx| {
                                        if let Some(panel) = w.upgrade() {
                                            panel.update(cx, |this, cx| {
                                                this.run_admin_operation(
                                                    "CREATE DATABASE new_database_name".into(),
                                                    |mgr| Box::pin(mgr.create_database("new_database_name")),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }
                                });
                            }
                            if caps.create_schema {
                                s = s.entry("Schema", None, {
                                    let w = w.clone();
                                    move |window, cx| {
                                        if let Some(panel) = w.upgrade() {
                                            panel.update(cx, |this, cx| {
                                                this.run_admin_operation(
                                                    "CREATE SCHEMA new_schema_name".into(),
                                                    |mgr| Box::pin(mgr.create_schema("new_schema_name")),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }
                                });
                            }
                            if caps.create_role {
                                s = s.entry("Role", None, {
                                    let w = w.clone();
                                    move |window, cx| {
                                        if let Some(panel) = w.upgrade() {
                                            panel.update(cx, |this, cx| {
                                                this.run_admin_operation(
                                                    "CREATE ROLE new_role_name".into(),
                                                    |mgr| Box::pin(mgr.create_role("new_role_name")),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }
                                });
                            }
                            if caps.users {
                                s = s.entry("User", None, {
                                    let w = w.clone();
                                    move |window, cx| {
                                        if let Some(panel) = w.upgrade() {
                                            panel.update(cx, |this, cx| {
                                                this.run_admin_operation(
                                                    "CREATE USER new_user".into(),
                                                    |mgr| Box::pin(mgr.create_user("new_user")),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }
                                    }
                                });
                            }
                            s
                        }
                    });
                    m = m.separator();
                    m = m.entry("Refresh", None, {
                        let w = weak.clone();
                        let n = name.clone();
                        move |window, cx| {
                            if let Some(panel) = w.upgrade() {
                                panel.update(cx, |this, cx| {
                                    this.connect(&n, window, cx);
                                });
                            }
                        }
                    });
                    m = m.entry("Disconnect", None, {
                        let w = weak.clone();
                        let n = name.clone();
                        move |_window, cx| {
                            if let Some(panel) = w.upgrade() {
                                panel.update(cx, |this, cx| {
                                    this.disconnect(&n, cx);
                                });
                            }
                        }
                    });
                } else {
                    m = m.entry("Connect", None, {
                        let w = weak.clone();
                        let n = name.clone();
                        move |window, cx| {
                            if let Some(panel) = w.upgrade() {
                                panel.update(cx, |this, cx| {
                                    this.connect(&n, window, cx);
                                });
                            }
                        }
                    });
                }
                m = m.separator();
                m = m.entry("Edit Connection", None, {
                    let w = weak.clone();
                    let n = name.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                let scope = this.connections.iter().find(|c| c.name == n).map(|c| c.scope).unwrap_or(ConnectionScope::Global);
                                this.show_connection_form(Some(n.clone()), scope, window, cx);
                            });
                        }
                    }
                });
                m = m.entry("Delete Connection", None, {
                    let w = weak.clone();
                    let n = name.clone();
                    move |_window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                this.delete_connection(&n, cx);
                            });
                        }
                    }
                });
                m
            }
        });

        self.set_context_menu(menu, position, window, cx);
    }

    pub(crate) fn deploy_table_context_menu(
        &mut self,
        schema_name: String,
        table_name: String,
        conn_name: String,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let full_name = if schema_name == "public" {
            table_name.clone()
        } else {
            format!("{schema_name}.{table_name}")
        };

        let menu = ContextMenu::build(window, cx, {
            let sn = schema_name.clone();
            let tn = table_name.clone();
            let cn = conn_name.clone();
            let fn_ = full_name.clone();
            move |menu, _, _| {
                menu.entry("SELECT * FROM ... LIMIT 100", None, {
                    let w = weak.clone();
                    let sn = sn.clone();
                    let tn = tn.clone();
                    let cn = cn.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                this.query_table(&sn, &tn, &cn, None, window, cx);
                            });
                        }
                    }
                })
                .entry("Show DDL (CREATE TABLE)", None, {
                    let w = weak.clone();
                    let sn = sn.clone();
                    let tn = tn.clone();
                    let cn = cn.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                this.show_ddl(&sn, &tn, &cn, window, cx);
                            });
                        }
                    }
                })
                .entry("Modify Table", None, {
                    let w = weak.clone();
                    let sn = sn.clone();
                    let tn = tn.clone();
                    let cn = cn.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                this.open_table_editor(&cn, &sn, &tn, window, cx);
                            });
                        }
                    }
                })
                .entry("Count Rows", None, {
                    let w = weak.clone();
                    let sn = sn.clone();
                    let tn = tn.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                this.count_table_rows(&sn, &tn, window, cx);
                            });
                        }
                    }
                })
                .separator()
                .entry("Copy Table Name", None, {
                    let fn_ = fn_.clone();
                    move |_window, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(fn_.clone()));
                    }
                })
                .separator()
                .entry("Truncate Table", None, {
                    let w = weak.clone();
                    let fn_ = fn_.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                let sql = format!("TRUNCATE TABLE {fn_};");
                                this.run_sql_with_confirmation(
                                    sql,
                                    format!("Truncate table {fn_}?"),
                                    window,
                                    cx,
                                );
                            });
                        }
                    }
                })
                .entry("Drop Table", None, {
                    let w = weak.clone();
                    let fn_ = fn_.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                let sql = format!("DROP TABLE {fn_};");
                                this.run_sql_with_confirmation(
                                    sql,
                                    format!("Drop table {fn_}? This cannot be undone."),
                                    window,
                                    cx,
                                );
                            });
                        }
                    }
                })
            }
        });

        self.set_context_menu(menu, position, window, cx);
    }

    pub(crate) fn deploy_schema_context_menu(
        &mut self,
        conn_name: String,
        schema_name: String,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();

        let menu = ContextMenu::build(window, cx, {
            let cn = conn_name.clone();
            let sn = schema_name.clone();
            move |menu, _, _| {
                menu.entry("Open Query Console", None, {
                    let w = weak.clone();
                    let cn = cn.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                this.open_query_console(&cn, None, window, cx);
                            });
                        }
                    }
                })
                .entry("Set as Active Schema", None, {
                    let w = weak.clone();
                    let sn = sn.clone();
                    move |_window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                this.set_active_schema(sn.clone(), cx);
                            });
                        }
                    }
                })
                .separator()
                .entry("Copy Schema Name", None, {
                    let sn = sn.clone();
                    move |_window, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(sn.clone()));
                    }
                })
                .separator()
                .entry("Drop Schema", None, {
                    let w = weak.clone();
                    let sn = sn.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                let sql = format!("DROP SCHEMA \"{sn}\" CASCADE;");
                                this.run_sql_with_confirmation(
                                    sql,
                                    format!("Drop schema {sn}? This will remove all objects in the schema."),
                                    window,
                                    cx,
                                );
                            });
                        }
                    }
                })
            }
        });

        self.set_context_menu(menu, position, window, cx);
    }

    pub(crate) fn deploy_database_context_menu(
        &mut self,
        conn_name: String,
        db_name: String,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let caps = self
            .connection_capabilities
            .get(&conn_name)
            .cloned()
            .unwrap_or_default();

        let menu = ContextMenu::build(window, cx, {
            let cn = conn_name.clone();
            let db = db_name.clone();
            move |menu, _, _| {
                let mut m = menu;
                m = m.entry("Open Query Console", None, {
                    let w = weak.clone();
                    let cn = cn.clone();
                    let db = db.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                this.open_query_console(&cn, Some(&db), window, cx);
                            });
                        }
                    }
                });
                if caps.create_schema {
                    m = m.entry("New Schema", None, {
                        let w = weak.clone();
                        move |window, cx| {
                            if let Some(panel) = w.upgrade() {
                                panel.update(cx, |this, cx| {
                                    this.run_admin_operation(
                                        "CREATE SCHEMA new_schema_name".into(),
                                        |mgr| Box::pin(mgr.create_schema("new_schema_name")),
                                        window,
                                        cx,
                                    );
                                });
                            }
                        }
                    });
                }
                m = m.separator();
                m = m.entry("Refresh", None, {
                    let w = weak.clone();
                    let cn = cn.clone();
                    let db = db.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                let detail_key = format!("{cn}.{db}");
                                this.database_schemas.remove(&detail_key);
                                this.load_database_schema(&cn, &db, window, cx);
                            });
                        }
                    }
                });
                m = m.separator();
                m = m.entry("Copy Database Name", None, {
                    let db = db.clone();
                    move |_window, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(db.clone()));
                    }
                });
                m = m.separator();
                m = m.entry("Drop Database", None, {
                    let w = weak.clone();
                    let db = db.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                let sql = format!("DROP DATABASE \"{db}\";");
                                this.run_sql_with_confirmation(
                                    sql,
                                    format!("Drop database {db}? This cannot be undone."),
                                    window,
                                    cx,
                                );
                            });
                        }
                    }
                });
                m
            }
        });

        self.set_context_menu(menu, position, window, cx);
    }
}
