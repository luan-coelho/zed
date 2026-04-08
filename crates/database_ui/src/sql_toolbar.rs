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
}

impl SqlToolbarItem {
    pub fn new(workspace: gpui::WeakEntity<Workspace>) -> Self {
        Self {
            workspace,
            active: true,
            schema_menu: None,
        }
    }

    /// Extract SQL from the active editor (selection or full text).
    fn get_sql(workspace: &Workspace, cx: &gpui::App) -> Option<String> {
        let editor_entity = workspace.active_item(cx)?.act_as::<Editor>(cx)?;
        let editor = editor_entity.read(cx);

        // Check selection
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

    /// Read schema info from the DatabasePanel (connection name, database, schemas).
    fn read_schema_info(
        &self,
        cx: &gpui::App,
    ) -> Option<(String, String, Vec<String>, Entity<DatabasePanel>)> {
        let ws = self.workspace.upgrade()?;
        let panel = ws.read(cx).panel::<DatabasePanel>(cx)?;
        let panel_ref = panel.read(cx);
        let (db_name, active_schema) = panel_ref.active_connection_info()?;
        let schemas = panel_ref.available_schemas();
        Some((db_name, active_schema, schemas, panel.clone()))
    }

    /// Check if the editor is a SQL file or a database console.
    fn is_sql_editor(editor: &Entity<Editor>, cx: &gpui::App) -> bool {
        let editor_ref = editor.read(cx);
        let buffer = editor_ref.buffer().read(cx);

        // Check if title is "console" (our database console editors)
        if buffer
            .explicit_title()
            .is_some_and(|t| t.starts_with("console"))
        {
            return true;
        }

        // Check if file extension is .sql
        if let Some(singleton) = buffer.as_singleton() {
            let buf = singleton.read(cx);
            if let Some(file) = buf.file() {
                let name = file.file_name(cx);
                if name.ends_with(".sql") {
                    return true;
                }
            }
            // Check language name
            if let Some(lang) = buf.language() {
                let name = lang.name();
                if name.as_ref().eq_ignore_ascii_case("sql") {
                    return true;
                }
            }
        }

        false
    }

    fn deploy_schema_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((db_name, active_schema, schemas, panel_entity)) =
            self.read_schema_info(cx)
        else {
            return;
        };

        if schemas.is_empty() {
            return;
        }

        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            // Database header (non-clickable)
            menu = menu.header(db_name.clone());

            for schema_name in &schemas {
                let is_active = *schema_name == active_schema;
                let s = schema_name.clone();
                let panel = panel_entity.clone();

                let label = if is_active {
                    format!("  {} (active)", s)
                } else {
                    format!("  {}", s)
                };

                menu = menu.entry(label, None, move |_window, cx| {
                    panel.update(cx, |panel, cx| {
                        panel.set_active_schema(s.clone(), cx);
                    });
                });
            }

            menu
        });

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

        // Defer SQL completion injection to avoid "cannot read Workspace while being updated".
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

        // Read schema info for the dropdown label
        let schema_label: Option<SharedString> = self.read_schema_info(cx).map(
            |(db_name, active_schema, _, _)| format!("{}.{}", db_name, active_schema).into(),
        );

        div()
            .flex()
            .items_center()
            .gap_1()
            // Schema selector dropdown
            .when_some(schema_label, |d, label| {
                d.child(
                    Button::new("schema-selector", label)
                        .label_size(LabelSize::Small)
                        .style(ButtonStyle::Subtle)
                        .end_icon(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.deploy_schema_menu(window, cx);
                        })),
                )
            })
            // Run SQL button
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
            // Explain SQL button
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
            // Schema context menu overlay
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
            .into_any_element()
    }
}
