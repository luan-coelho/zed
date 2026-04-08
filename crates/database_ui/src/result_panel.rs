use database_core::QueryResult;
use editor::Editor;
use gpui::{
    actions, anchored, deferred, div, Action, App, AppContext, AsyncWindowContext, Context, Corner,
    DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Render, SharedString, Styled, Subscription,
    WeakEntity, Window,
};
use theme::ActiveTheme;
use ui::ContextMenu;
use ui::{prelude::*, TintColor, Tooltip};
use workspace::dock::{DockPosition, Panel, PanelEvent};
use workspace::Workspace;

fn escape_csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub(crate) fn format_as_tsv(columns: &[String], rows: &[Vec<String>]) -> String {
    let mut output = columns.join("\t");
    output.push('\n');
    for row in rows {
        output.push_str(&row.join("\t"));
        output.push('\n');
    }
    output
}

pub(crate) fn format_as_csv(columns: &[String], rows: &[Vec<String>]) -> String {
    let mut output = columns.iter().map(|c| escape_csv(c)).collect::<Vec<_>>().join(",");
    output.push('\n');
    for row in rows {
        output.push_str(&row.iter().map(|v| escape_csv(v)).collect::<Vec<_>>().join(","));
        output.push('\n');
    }
    output
}

pub(crate) fn format_as_json(columns: &[String], rows: &[Vec<String>]) -> Option<String> {
    let json_rows: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for (i, col) in columns.iter().enumerate() {
                let val = row.get(i).cloned().unwrap_or_default();
                obj.insert(
                    col.clone(),
                    if val == "NULL" {
                        serde_json::Value::Null
                    } else {
                        serde_json::Value::String(val)
                    },
                );
            }
            serde_json::Value::Object(obj)
        })
        .collect();
    serde_json::to_string_pretty(&json_rows).ok()
}

pub(crate) fn generate_update_sql(
    table_query: &str,
    columns: &[String],
    row: &[String],
    col_index: usize,
    new_value: &str,
) -> Option<String> {
    let table_name = extract_table_name(table_query)?;
    let col_name = columns.get(col_index)?;

    let pk_col_index = columns
        .iter()
        .position(|c| c.eq_ignore_ascii_case("id"))
        .unwrap_or(0);
    let pk_col = columns.get(pk_col_index)?;
    let pk_value = row.get(pk_col_index)?;

    let set_clause = if new_value.eq_ignore_ascii_case("NULL") {
        format!("\"{}\" = NULL", col_name)
    } else {
        format!(
            "\"{}\" = '{}'",
            col_name,
            new_value.replace('\'', "''")
        )
    };

    let where_clause = if pk_value.eq_ignore_ascii_case("NULL") {
        format!("\"{}\" IS NULL", pk_col)
    } else {
        format!(
            "\"{}\" = '{}'",
            pk_col,
            pk_value.replace('\'', "''")
        )
    };

    Some(format!(
        "UPDATE {} SET {} WHERE {};",
        table_name, set_clause, where_clause
    ))
}

fn extract_table_name(query: &str) -> Option<String> {
    let upper = query.to_uppercase();
    let from_pos = upper.find("FROM")?;
    let after_from = &query[from_pos + 4..];
    let trimmed = after_from.trim_start();
    let table_name: String = trimmed
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ';')
        .collect();
    if table_name.is_empty() {
        None
    } else {
        Some(table_name)
    }
}

actions!(database_result_panel, [ToggleFocus,]);


/// Global channel to send query results to the result panel.
pub struct QueryResultEvent {
    pub result: QueryResult,
    pub query: String,
}

const PAGE_SIZE: usize = 50;

/// Which view is active in the result panel.
#[derive(Clone, Copy, PartialEq)]
enum ResultView {
    Output,
    History,
    Table(usize),
}

pub struct DatabaseResultPanel {
    focus_handle: FocusHandle,
    results: Vec<QueryResultEntry>,
    active_view: ResultView,
    selected_row: Option<usize>,
    output_log: Vec<String>,
    history: Vec<HistoryEntry>,
    sort_column: Option<usize>,
    sort_ascending: bool,
    filter_text: String,
    filter_editor: Entity<Editor>,
    tab_context_menu: Option<(Entity<ContextMenu>, Subscription)>,
    editing_cell: Option<(usize, usize)>,
    cell_editor: Entity<Editor>,
    pending_updates: Vec<String>,
}

struct QueryResultEntry {
    query: String,
    result: QueryResult,
    page: usize,
    timestamp: String,
}

#[derive(Clone)]
struct HistoryEntry {
    query: String,
    timestamp: String,
    success: bool,
    rows_affected: u64,
    execution_time_ms: u64,
}

impl DatabaseResultPanel {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();

        let filter_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Filter rows...", window, cx);
            editor
        });

        cx.subscribe(&filter_editor, |this: &mut Self, editor, event, cx| {
            if matches!(event, editor::EditorEvent::BufferEdited { .. }) {
                this.filter_text = editor.read(cx).text(cx).trim().to_lowercase();
                cx.notify();
            }
        })
        .detach();

        let cell_editor = cx.new(|cx| Editor::single_line(window, cx));

        Self {
            focus_handle,
            results: Vec::new(),
            active_view: ResultView::Output,
            selected_row: None,
            output_log: Vec::new(),
            history: Vec::new(),
            sort_column: None,
            sort_ascending: true,
            filter_text: String::new(),
            filter_editor,
            tab_context_menu: None,
            editing_cell: None,
            cell_editor,
            pending_updates: Vec::new(),
        }
    }

    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> anyhow::Result<Entity<Self>> {
        workspace.update_in(&mut cx, |_workspace, window, cx| {
            cx.new(|cx| DatabaseResultPanel::new(window, cx))
        })
    }

    pub fn push_result(&mut self, query: String, result: QueryResult, cx: &mut Context<Self>) {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();

        // Append to output log
        let is_error = result.columns.first().is_some_and(|c| c == "Error");
        let query_preview = if query.len() > 80 {
            format!("{}...", &query[..77])
        } else {
            query.clone()
        };
        self.output_log
            .push(format!("[{}] > {}", now, query_preview));
        if is_error {
            let msg = result
                .rows
                .first()
                .and_then(|r| r.first())
                .cloned()
                .unwrap_or_default();
            self.output_log
                .push(format!("[{}] ERROR: {}", now, msg));
        } else {
            self.output_log.push(format!(
                "[{}] {} rows · {}ms",
                now, result.rows_affected, result.execution_time_ms
            ));
        }

        self.history.push(HistoryEntry {
            query: query.clone(),
            timestamp: now.clone(),
            success: !is_error,
            rows_affected: result.rows_affected,
            execution_time_ms: result.execution_time_ms,
        });

        self.results.push(QueryResultEntry {
            query,
            result,
            page: 0,
            timestamp: now,
        });
        let idx = self.results.len() - 1;
        self.active_view = ResultView::Table(idx);
        self.selected_row = None;
        self.sort_column = None;
        self.sort_ascending = true;
        cx.emit(PanelEvent::Activate);
        cx.notify();
    }

    /// Append a log message to the output (e.g., connection events).
    pub fn push_output_line(&mut self, line: String, cx: &mut Context<Self>) {
        self.output_log.push(line);
        cx.notify();
    }

    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        self.results.clear();
        self.active_view = ResultView::Output;
        self.output_log.clear();
        cx.notify();
    }

    fn active_result_data(&self) -> Option<&QueryResult> {
        match self.active_view {
            ResultView::Table(idx) => self.results.get(idx).map(|e| &e.result),
            ResultView::Output | ResultView::History => None,
        }
    }

    fn start_editing_cell(
        &mut self,
        row: usize,
        col: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_idx = match self.active_view {
            ResultView::Table(idx) => idx,
            _ => return,
        };
        let value = self
            .results
            .get(active_idx)
            .and_then(|entry| entry.result.rows.get(row))
            .and_then(|r| r.get(col))
            .cloned()
            .unwrap_or_default();

        self.editing_cell = Some((row, col));
        self.cell_editor.update(cx, |editor, cx| {
            editor.set_text(value, window, cx);
            editor.select_all(&editor::actions::SelectAll, window, cx);
        });
        self.cell_editor.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn commit_cell_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((row, col)) = self.editing_cell.take() else {
            return;
        };
        let active_idx = match self.active_view {
            ResultView::Table(idx) => idx,
            _ => return,
        };
        let new_value = self.cell_editor.read(cx).text(cx);

        let Some(entry) = self.results.get_mut(active_idx) else {
            return;
        };
        let old_value = entry
            .result
            .rows
            .get(row)
            .and_then(|r| r.get(col))
            .cloned()
            .unwrap_or_default();

        if new_value == old_value {
            cx.notify();
            return;
        }

        let columns = &entry.result.columns;
        let row_data = entry.result.rows.get(row);
        if let Some(sql) = row_data.and_then(|r| {
            generate_update_sql(&entry.query, columns, r, col, &new_value)
        }) {
            let now = chrono::Local::now().format("%H:%M:%S").to_string();
            self.output_log
                .push(format!("[{now}] PENDING: {sql}"));
            self.pending_updates.push(sql);
        }

        if let Some(row_data) = entry.result.rows.get_mut(row) {
            if let Some(cell) = row_data.get_mut(col) {
                *cell = new_value;
            }
        }

        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn cancel_cell_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_cell = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn discard_pending_updates(&mut self, cx: &mut Context<Self>) {
        let count = self.pending_updates.len();
        self.pending_updates.clear();
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        self.output_log
            .push(format!("[{now}] Discarded {count} pending update(s)"));
        cx.notify();
    }

    fn copy_active_result_as_tsv(&self, cx: &mut Context<Self>) {
        let Some(result) = self.active_result_data() else { return };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            format_as_tsv(&result.columns, &result.rows),
        ));
    }

    fn build_csv(&self) -> Option<String> {
        let result = self.active_result_data()?;
        Some(format_as_csv(&result.columns, &result.rows))
    }

    fn build_json(&self) -> Option<String> {
        let result = self.active_result_data()?;
        format_as_json(&result.columns, &result.rows)
    }

    fn copy_active_result_as_csv(&self, cx: &mut Context<Self>) {
        if let Some(csv) = self.build_csv() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(csv));
        }
    }

    fn copy_active_result_as_json(&self, cx: &mut Context<Self>) {
        if let Some(json) = self.build_json() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(json));
        }
    }

    fn export_as_csv(&self, cx: &mut Context<Self>) {
        let Some(csv) = self.build_csv() else { return };
        let receiver = cx.prompt_for_new_path(
            &std::path::PathBuf::from("."),
            Some("export.csv"),
        );
        cx.background_executor()
            .spawn(async move {
                if let Ok(Ok(Some(path))) = receiver.await {
                    let _ = std::fs::write(&path, csv);
                }
            })
            .detach();
    }

    fn export_as_json(&self, cx: &mut Context<Self>) {
        let Some(json) = self.build_json() else { return };
        let receiver = cx.prompt_for_new_path(
            &std::path::PathBuf::from("."),
            Some("export.json"),
        );
        cx.background_executor()
            .spawn(async move {
                if let Ok(Ok(Some(path))) = receiver.await {
                    let _ = std::fs::write(&path, json);
                }
            })
            .detach();
    }

    fn deploy_tab_context_menu(
        &mut self,
        tab_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();

        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            menu.entry("Close", None, {
                let w = weak.clone();
                move |_window, cx| {
                    if let Some(panel) = w.upgrade() {
                        panel.update(cx, |this, cx| this.close_tab(tab_index, cx));
                    }
                }
            })
            .entry("Close Others", None, {
                let w = weak.clone();
                move |_window, cx| {
                    if let Some(panel) = w.upgrade() {
                        panel.update(cx, |this, cx| this.close_other_tabs(tab_index, cx));
                    }
                }
            })
            .separator()
            .entry("Close Left", None, {
                let w = weak.clone();
                move |_window, cx| {
                    if let Some(panel) = w.upgrade() {
                        panel.update(cx, |this, cx| this.close_tabs_left(tab_index, cx));
                    }
                }
            })
            .entry("Close Right", None, {
                let w = weak.clone();
                move |_window, cx| {
                    if let Some(panel) = w.upgrade() {
                        panel.update(cx, |this, cx| this.close_tabs_right(tab_index, cx));
                    }
                }
            })
            .separator()
            .entry("Close All", None, {
                let w = weak.clone();
                move |_window, cx| {
                    if let Some(panel) = w.upgrade() {
                        panel.update(cx, |this, cx| this.close_all_tabs(cx));
                    }
                }
            })
        });

        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, _cx| {
            this.tab_context_menu.take();
        });
        self.tab_context_menu = Some((menu, subscription));
        cx.notify();
    }

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.results.len() {
            self.results.remove(index);
        }
        self.reset_tab_view(cx);
    }

    fn close_other_tabs(&mut self, keep: usize, cx: &mut Context<Self>) {
        if keep < self.results.len() {
            let kept = self.results.remove(keep);
            self.results.clear();
            self.results.push(kept);
        }
        self.reset_tab_view(cx);
    }

    fn close_tabs_left(&mut self, index: usize, cx: &mut Context<Self>) {
        if index > 0 && index <= self.results.len() {
            self.results.drain(..index);
        }
        self.reset_tab_view(cx);
    }

    fn close_tabs_right(&mut self, index: usize, cx: &mut Context<Self>) {
        if index + 1 < self.results.len() {
            self.results.truncate(index + 1);
        }
        self.reset_tab_view(cx);
    }

    fn close_all_tabs(&mut self, cx: &mut Context<Self>) {
        self.results.clear();
        self.reset_tab_view(cx);
    }

    fn reset_tab_view(&mut self, cx: &mut Context<Self>) {
        self.active_view = if self.results.is_empty() {
            ResultView::Output
        } else {
            ResultView::Table(self.results.len().saturating_sub(1))
        };
        self.sort_column = None;
        self.sort_ascending = true;
        cx.notify();
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_output_active = self.active_view == ResultView::Output;
        let is_history_active = self.active_view == ResultView::History;

        let mut tabs = div().flex().items_center().gap_0().px_1().overflow_hidden();

        // Fixed "Output" tab
        tabs = tabs.child(
            div()
                .id("output-tab")
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .cursor_pointer()
                .when(is_output_active, |d| {
                    d.border_b_2()
                        .border_color(cx.theme().colors().text_accent)
                })
                .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_t_md())
                .on_click(cx.listener(|this, _, _w, cx| {
                    this.active_view = ResultView::Output;
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::Terminal)
                        .size(IconSize::XSmall)
                        .color(if is_output_active {
                            Color::Default
                        } else {
                            Color::Muted
                        }),
                )
                .child(
                    Label::new("Output")
                        .size(LabelSize::XSmall)
                        .color(if is_output_active {
                            Color::Default
                        } else {
                            Color::Muted
                        }),
                ),
        );

        // Fixed "History" tab
        tabs = tabs.child(
            div()
                .id("history-tab")
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .cursor_pointer()
                .when(is_history_active, |d| {
                    d.border_b_2()
                        .border_color(cx.theme().colors().text_accent)
                })
                .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_t_md())
                .on_click(cx.listener(|this, _, _w, cx| {
                    this.active_view = ResultView::History;
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::CountdownTimer)
                        .size(IconSize::XSmall)
                        .color(if is_history_active {
                            Color::Default
                        } else {
                            Color::Muted
                        }),
                )
                .child(
                    Label::new(format!("History ({})", self.history.len()))
                        .size(LabelSize::XSmall)
                        .color(if is_history_active {
                            Color::Default
                        } else {
                            Color::Muted
                        }),
                ),
        );

        // Result tabs
        for (i, entry) in self.results.iter().enumerate() {
            let is_active = self.active_view == ResultView::Table(i);
            let is_error = entry
                .result
                .columns
                .first()
                .is_some_and(|c| c == "Error");

            let query_preview = if entry.query.len() > 25 {
                format!("{}...", &entry.query[..22])
            } else {
                entry.query.clone()
            };
            let label: SharedString = format!("[{}] {}", entry.timestamp, query_preview).into();

            tabs = tabs.child(
                div()
                    .id(ElementId::Name(format!("result-tab-{i}").into()))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .when(is_active, |d| {
                        d.border_b_2()
                            .border_color(cx.theme().colors().text_accent)
                    })
                    .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_t_md())
                    .on_click(cx.listener(move |this, _, _w, cx| {
                        this.active_view = ResultView::Table(i);
                        this.sort_column = None;
                        this.sort_ascending = true;
                        cx.notify();
                    }))
                    .on_mouse_down(MouseButton::Right, cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                        this.deploy_tab_context_menu(i, window, cx);
                    }))
                    .child(
                        Icon::new(IconName::ListTree)
                            .size(IconSize::XSmall)
                            .color(if is_error {
                                Color::Error
                            } else if is_active {
                                Color::Default
                            } else {
                                Color::Muted
                            }),
                    )
                    .child(
                        Label::new(label).size(LabelSize::XSmall).color(if is_error {
                            Color::Error
                        } else if is_active {
                            Color::Default
                        } else {
                            Color::Muted
                        }),
                    )
                    // Close tab
                    .child(
                        IconButton::new(
                            ElementId::Name(format!("close-tab-{i}").into()),
                            IconName::Close,
                        )
                        .icon_size(IconSize::XSmall)
                        .icon_color(Color::Muted)
                        .on_click(cx.listener(move |this, _, _w, cx| {
                            if i < this.results.len() {
                                this.results.remove(i);
                                this.active_view = if this.results.is_empty() {
                                    ResultView::Output
                                } else {
                                    ResultView::Table(this.results.len().saturating_sub(1))
                                };
                            }
                            cx.notify();
                        })),
                    ),
            );
        }

        tabs
    }

    fn render_output_log(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.output_log.is_empty() {
            return div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .flex_1()
                .gap_1()
                .child(
                    Icon::new(IconName::Terminal)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("No output yet. Run a query to see results here.")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .into_any_element();
        }

        let lines = self.output_log.clone();
        let error_color = Color::Error;

        div()
            .flex()
            .flex_col()
            .flex_1()
            .id("output-log-scroll")
            .overflow_y_scroll()
            .p_2()
            .child(
                gpui::uniform_list("output-lines", lines.len(), move |range, _w, _cx| {
                    range
                        .map(|ix| {
                            let line = &lines[ix];
                            let is_error = line.contains("ERROR:");
                            div()
                                .px_1()
                                .child(
                                    Label::new(line.clone())
                                        .size(LabelSize::XSmall)
                                        .color(if is_error { error_color } else { Color::Default }),
                                )
                                .into_any_element()
                        })
                        .collect()
                })
                .flex_1(),
            )
            .into_any_element()
    }

    fn render_history(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.history.is_empty() {
            return div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .flex_1()
                .gap_1()
                .child(
                    Icon::new(IconName::CountdownTimer)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("No query history yet.")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .into_any_element();
        }

        let entries: Vec<HistoryEntry> = self.history.iter().rev().cloned().collect();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .id("history-scroll")
            .overflow_y_scroll()
            .p_2()
            .child(
                gpui::uniform_list("history-entries", entries.len(), move |range, _w, cx| {
                    let alt_bg = cx.theme().colors().ghost_element_hover;
                    range
                        .map(|ix| {
                            let entry = &entries[ix];
                            let query_preview = if entry.query.len() > 100 {
                                format!("{}...", &entry.query[..97])
                            } else {
                                entry.query.clone()
                            };
                            let status_icon = if entry.success {
                                IconName::Check
                            } else {
                                IconName::Close
                            };
                            let status_color = if entry.success {
                                Color::Success
                            } else {
                                Color::Error
                            };
                            let stats = if entry.success {
                                format!(
                                    "{} rows · {}ms",
                                    entry.rows_affected, entry.execution_time_ms
                                )
                            } else {
                                "error".to_string()
                            };

                            let query_for_copy = entry.query.clone();

                            div()
                                .id(ElementId::Name(format!("history-{ix}").into()))
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .py_1()
                                .when(ix % 2 == 1, |d| d.bg(alt_bg))
                                .cursor_pointer()
                                .on_click(move |_, _w, cx| {
                                    cx.write_to_clipboard(
                                        gpui::ClipboardItem::new_string(query_for_copy.clone()),
                                    );
                                })
                                .child(
                                    Icon::new(status_icon)
                                        .size(IconSize::XSmall)
                                        .color(status_color),
                                )
                                .child(
                                    Label::new(entry.timestamp.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .overflow_hidden()
                                        .child(
                                            Label::new(query_preview)
                                                .size(LabelSize::XSmall),
                                        ),
                                )
                                .child(
                                    Label::new(stats)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .into_any_element()
                        })
                        .collect()
                })
                .flex_1(),
            )
            .into_any_element()
    }

    fn render_active_result(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.active_view == ResultView::Output {
            return self.render_output_log(cx).into_any_element();
        }
        if self.active_view == ResultView::History {
            return self.render_history(cx).into_any_element();
        }

        let idx = match self.active_view {
            ResultView::Table(i) => i,
            ResultView::Output | ResultView::History => unreachable!(),
        };
        let Some(entry) = self.results.get(idx) else {
            return self.render_empty().into_any_element();
        };

        let result = &entry.result;

        // Error results: display full-width error message instead of table
        let is_error = result
            .columns
            .first()
            .is_some_and(|c| c == "Error");
        if is_error {
            let error_msg = result
                .rows
                .first()
                .and_then(|r| r.first())
                .cloned()
                .unwrap_or_else(|| "Unknown error".to_string());
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .p_4()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::Close)
                                .size(IconSize::Small)
                                .color(Color::Error),
                        )
                        .child(
                            Label::new("Query Error")
                                .weight(FontWeight::SEMIBOLD)
                                .color(Color::Error),
                        ),
                )
                .child(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(cx.theme().colors().ghost_element_hover)
                        .w_full()
                        .child(
                            Label::new(error_msg)
                                .size(LabelSize::Small)
                                .color(Color::Error),
                        ),
                )
                .child(
                    Label::new(format!("Query: {}", entry.query))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .into_any_element();
        }

        if result.columns.is_empty() {
            return div()
                .flex()
                .items_center()
                .justify_center()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Icon::new(IconName::Check)
                                .size(IconSize::Small)
                                .color(Color::Success),
                        )
                        .child(
                            Label::new(format!(
                                "Query OK · {} rows affected · {}ms",
                                result.rows_affected, result.execution_time_ms
                            ))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        ),
                )
                .into_any_element();
        }

        let num_cols = result.columns.len();
        let col_width = if num_cols <= 3 {
            px(180.)
        } else if num_cols <= 6 {
            px(140.)
        } else {
            px(110.)
        };

        // Sort rows if a sort column is set
        let sorted_rows = if let Some(sort_col) = self.sort_column {
            let mut rows = result.rows.clone();
            let ascending = self.sort_ascending;
            rows.sort_by(|a, b| {
                let val_a = a.get(sort_col).map(|s| s.as_str()).unwrap_or("");
                let val_b = b.get(sort_col).map(|s| s.as_str()).unwrap_or("");
                let cmp = val_a.cmp(val_b);
                if ascending { cmp } else { cmp.reverse() }
            });
            rows
        } else {
            result.rows.clone()
        };

        // Filter rows by text
        let filtered_rows = if self.filter_text.is_empty() {
            sorted_rows
        } else {
            let filter_lower = self.filter_text.to_lowercase();
            sorted_rows
                .into_iter()
                .filter(|row| row.iter().any(|val| val.to_lowercase().contains(&filter_lower)))
                .collect()
        };

        // Pagination
        let total_rows = filtered_rows.len();
        let total_pages = (total_rows + PAGE_SIZE - 1) / PAGE_SIZE;
        let current_page = entry.page.min(total_pages.saturating_sub(1));
        let page_start = current_page * PAGE_SIZE;
        let page_end = (page_start + PAGE_SIZE).min(total_rows);
        let page_rows: Vec<Vec<String>> = filtered_rows[page_start..page_end].to_vec();
        let selected_row = self.selected_row;
        let sort_column = self.sort_column;
        let sort_ascending = self.sort_ascending;

        div()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            // Column header (clickable for sort)
            .child({
                let mut header = div()
                    .flex()
                    .flex_none()
                    .w_full()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .bg(cx.theme().colors().title_bar_background);

                header = header.child(
                    div()
                        .w(px(40.))
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .child(Label::new("#").size(LabelSize::XSmall).color(Color::Muted)),
                );

                for (col_idx, col) in result.columns.iter().enumerate() {
                    let is_sorted = sort_column == Some(col_idx);
                    let sort_indicator = if is_sorted {
                        if sort_ascending { " ▲" } else { " ▼" }
                    } else {
                        ""
                    };
                    let col_name = col.clone();
                    header = header.child(
                        div()
                            .id(ElementId::Name(format!("col-header-{col_idx}").into()))
                            .w(col_width)
                            .flex_shrink_0()
                            .px_2()
                            .py_1()
                            .cursor_pointer()
                            .hover(|d| d.bg(cx.theme().colors().ghost_element_hover))
                            .on_click(cx.listener(move |this, _, _w, cx| {
                                let direction = if this.sort_column == Some(col_idx) {
                                    this.sort_ascending = !this.sort_ascending;
                                    if this.sort_ascending { "ASC" } else { "DESC" }
                                } else {
                                    this.sort_column = Some(col_idx);
                                    this.sort_ascending = true;
                                    "ASC"
                                };
                                let now = chrono::Local::now().format("%H:%M:%S").to_string();
                                this.output_log.push(format!(
                                    "[{now}] ORDER BY {col_name} {direction}"
                                ));
                                cx.notify();
                            }))
                            .child(
                                Label::new(format!("{col}{sort_indicator}"))
                                    .size(LabelSize::XSmall)
                                    .weight(FontWeight::SEMIBOLD),
                            ),
                    );
                }
                header
            })
            // Data rows
            .child({
                let columns = result.columns.clone();
                let alt_bg = cx.theme().colors().ghost_element_hover;
                let selected_bg = cx.theme().colors().ghost_element_selected;
                let editing_cell = self.editing_cell;
                let cell_editor = self.cell_editor.clone();
                let weak_self = cx.entity().downgrade();

                gpui::uniform_list(
                    "result-rows",
                    page_rows.len(),
                    move |range, _w, _cx| {
                        range
                            .map(|ix| {
                                let global_ix = page_start + ix;
                                let is_selected = selected_row == Some(global_ix);

                                let mut row = div()
                                    .id(ElementId::Name(format!("row-{global_ix}").into()))
                                    .flex()
                                    .w_full()
                                    .cursor_pointer()
                                    .when(is_selected, |d| d.bg(selected_bg))
                                    .when(!is_selected && ix % 2 == 1, |d| d.bg(alt_bg));

                                // Row number
                                row = row.child(
                                    div()
                                        .w(px(40.))
                                        .flex_shrink_0()
                                        .px_2()
                                        .py_px()
                                        .child(
                                            Label::new(format!("{}", global_ix + 1))
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        ),
                                );

                                for (col_idx, val) in page_rows[ix].iter().enumerate() {
                                    let is_editing = editing_cell == Some((global_ix, col_idx));

                                    if is_editing {
                                        row = row.child(
                                            div()
                                                .w(col_width)
                                                .flex_shrink_0()
                                                .px_1()
                                                .py_px()
                                                .child(cell_editor.clone()),
                                        );
                                    } else {
                                        let is_null = val == "NULL";
                                        let is_error = columns
                                            .first()
                                            .map(|c| c == "Error")
                                            .unwrap_or(false)
                                            && col_idx == 0;

                                        let val_clone = val.clone();
                                        let weak = weak_self.clone();
                                        row = row.child(
                                            div()
                                                .id(ElementId::Name(
                                                    format!("cell-{global_ix}-{col_idx}").into(),
                                                ))
                                                .w(col_width)
                                                .flex_shrink_0()
                                                .px_2()
                                                .py_px()
                                                .overflow_hidden()
                                                .cursor_pointer()
                                                .hover(|d| d.bg(alt_bg))
                                                .tooltip({
                                                    let v = val_clone.clone();
                                                    move |_w, cx| Tooltip::simple(v.clone(), cx)
                                                })
                                                .on_click({
                                                    move |_, window, cx| {
                                                        if let Some(panel) = weak.upgrade() {
                                                            panel.update(cx, |this, cx| {
                                                                this.start_editing_cell(
                                                                    global_ix, col_idx, window, cx,
                                                                );
                                                            });
                                                        }
                                                    }
                                                })
                                                .child(
                                                    Label::new(val.clone())
                                                        .size(LabelSize::XSmall)
                                                        .color(if is_error {
                                                            Color::Error
                                                        } else if is_null {
                                                            Color::Muted
                                                        } else {
                                                            Color::Default
                                                        }),
                                                ),
                                        );
                                    }
                                }
                                row.into_any_element()
                            })
                            .collect()
                    },
                )
                .flex_1()
            })
            // Status bar: pagination + export
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().colors().border)
                    // Left: row info + pagination
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Label::new(format!(
                                    "{} rows · {}ms",
                                    result.rows_affected, result.execution_time_ms
                                ))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            )
                            .when(total_pages > 1, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            IconButton::new("prev-page", IconName::ArrowLeft)
                                                .icon_size(IconSize::XSmall)
                                                .disabled(current_page == 0)
                                                .on_click(cx.listener(move |this, _, _w, cx| {
                                                    if let Some(idx) = match this.active_view { ResultView::Table(idx) => Some(idx), _ => None } {
                                                        if let Some(entry) =
                                                            this.results.get_mut(idx)
                                                        {
                                                            entry.page =
                                                                entry.page.saturating_sub(1);
                                                        }
                                                    }
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            Label::new(format!(
                                                "{}/{}",
                                                current_page + 1,
                                                total_pages
                                            ))
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                        )
                                        .child(
                                            IconButton::new("next-page", IconName::ArrowRight)
                                                .icon_size(IconSize::XSmall)
                                                .disabled(current_page >= total_pages - 1)
                                                .on_click(cx.listener(move |this, _, _w, cx| {
                                                    if let Some(idx) = match this.active_view { ResultView::Table(idx) => Some(idx), _ => None } {
                                                        if let Some(entry) =
                                                            this.results.get_mut(idx)
                                                        {
                                                            if entry.page < total_pages - 1 {
                                                                entry.page += 1;
                                                            }
                                                        }
                                                    }
                                                    cx.notify();
                                                })),
                                        ),
                                )
                            }),
                    )
                    // Center: filter input
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Icon::new(IconName::MagnifyingGlass)
                                    .size(IconSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                div()
                                    .w(px(150.))
                                    .child(self.filter_editor.clone()),
                            )
                            .when(!self.filter_text.is_empty(), |d| {
                                d.child(
                                    Label::new(format!("{total_rows} matches"))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .child(
                                    IconButton::new("clear-filter", IconName::Close)
                                        .icon_size(IconSize::XSmall)
                                        .icon_color(Color::Muted)
                                        .tooltip(|_w, cx| Tooltip::simple("Clear filter", cx))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.filter_text.clear();
                                            this.filter_editor.update(cx, |editor, cx| {
                                                editor.set_text("", window, cx);
                                            });
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    // Right: export buttons
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("copy-all", "Copy")
                                    .style(ButtonStyle::Subtle)
                                    .label_size(LabelSize::XSmall)
                                    .tooltip(|_w, cx| Tooltip::simple("Copy as TSV to clipboard", cx))
                                    .on_click(cx.listener(|this, _, _w, cx| {
                                        this.copy_active_result_as_tsv(cx);
                                    })),
                            )
                            .child(
                                Button::new("copy-csv", "Copy CSV")
                                    .style(ButtonStyle::Subtle)
                                    .label_size(LabelSize::XSmall)
                                    .tooltip(|_w, cx| Tooltip::simple("Copy as CSV to clipboard", cx))
                                    .on_click(cx.listener(|this, _, _w, cx| {
                                        this.copy_active_result_as_csv(cx);
                                    })),
                            )
                            .child(
                                Button::new("copy-json", "Copy JSON")
                                    .style(ButtonStyle::Subtle)
                                    .label_size(LabelSize::XSmall)
                                    .tooltip(|_w, cx| Tooltip::simple("Copy as JSON to clipboard", cx))
                                    .on_click(cx.listener(|this, _, _w, cx| {
                                        this.copy_active_result_as_json(cx);
                                    })),
                            )
                            .child(
                                Button::new("export-csv", "Export CSV")
                                    .style(ButtonStyle::Subtle)
                                    .label_size(LabelSize::XSmall)
                                    .tooltip(|_w, cx| Tooltip::simple("Save as CSV file", cx))
                                    .on_click(cx.listener(|this, _, _w, cx| {
                                        this.export_as_csv(cx);
                                    })),
                            )
                            .child(
                                Button::new("export-json", "Export JSON")
                                    .style(ButtonStyle::Subtle)
                                    .label_size(LabelSize::XSmall)
                                    .tooltip(|_w, cx| Tooltip::simple("Save as JSON file", cx))
                                    .on_click(cx.listener(|this, _, _w, cx| {
                                        this.export_as_json(cx);
                                    })),
                            ),
                    ),
            )
            .when(!self.pending_updates.is_empty(), |d| {
                let count = self.pending_updates.len();
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .border_t_1()
                        .border_color(cx.theme().colors().border)
                        .bg(cx.theme().colors().title_bar_background)
                        .child(
                            Label::new(format!(
                                "{} pending change{}",
                                count,
                                if count == 1 { "" } else { "s" }
                            ))
                            .size(LabelSize::XSmall)
                            .color(Color::Warning),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    Button::new("apply-updates", "Apply")
                                        .style(ButtonStyle::Tinted(TintColor::Accent))
                                        .label_size(LabelSize::XSmall)
                                        .tooltip(|_w, cx| {
                                            Tooltip::simple(
                                                "Copy all pending UPDATE statements to clipboard",
                                                cx,
                                            )
                                        })
                                        .on_click(cx.listener(|this, _, _w, cx| {
                                            let sql =
                                                this.pending_updates.join("\n");
                                            cx.write_to_clipboard(
                                                gpui::ClipboardItem::new_string(sql),
                                            );
                                            let count = this.pending_updates.len();
                                            this.pending_updates.clear();
                                            let now = chrono::Local::now()
                                                .format("%H:%M:%S")
                                                .to_string();
                                            this.output_log.push(format!(
                                                "[{now}] Copied {count} UPDATE statement(s) to clipboard"
                                            ));
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("discard-updates", "Discard")
                                        .style(ButtonStyle::Subtle)
                                        .label_size(LabelSize::XSmall)
                                        .tooltip(|_w, cx| {
                                            Tooltip::simple(
                                                "Discard all pending changes",
                                                cx,
                                            )
                                        })
                                        .on_click(cx.listener(|this, _, _w, cx| {
                                            this.discard_pending_updates(cx);
                                        })),
                                ),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_empty(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .flex_1()
            .gap_1()
            .child(
                Icon::new(IconName::Terminal)
                    .size(IconSize::Small)
                    .color(Color::Muted),
            )
            .child(
                Label::new("No query results")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }
}

impl EventEmitter<PanelEvent> for DatabaseResultPanel {}

impl Focusable for DatabaseResultPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for DatabaseResultPanel {
    fn persistent_name() -> &'static str {
        "DatabaseResultPanel"
    }

    fn panel_key() -> &'static str {
        "database_result_panel"
    }

    fn starts_open(&self, _w: &Window, _cx: &App) -> bool {
        false
    }

    fn position(&self, _w: &Window, _cx: &App) -> DockPosition {
        DockPosition::Bottom
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Bottom | DockPosition::Right)
    }

    fn set_position(&mut self, _p: DockPosition, _w: &mut Window, _cx: &mut Context<Self>) {}

    fn default_size(&self, _w: &Window, _cx: &App) -> Pixels {
        px(250.)
    }

    fn icon(&self, _w: &Window, _cx: &App) -> Option<ui::IconName> {
        Some(ui::IconName::ListTree)
    }

    fn icon_tooltip(&self, _w: &Window, _cx: &App) -> Option<&'static str> {
        Some("Query Results")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        4
    }
}

impl Render for DatabaseResultPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("DatabaseResultPanel")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().colors().panel_background)
            .on_action(cx.listener(|this, _: &menu::Confirm, window, cx| {
                if this.editing_cell.is_some() {
                    this.commit_cell_edit(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &menu::Cancel, window, cx| {
                if this.editing_cell.is_some() {
                    this.cancel_cell_edit(window, cx);
                }
            }))
            // Tab bar
            .child(
                div()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(self.render_tab_bar(cx))
                    .child(div().flex_1())
                    .when(!self.results.is_empty(), |d| {
                        d.child(
                            IconButton::new("close-all-tabs", IconName::ListTree)
                                .icon_size(IconSize::XSmall)
                                .icon_color(Color::Muted)
                                .tooltip(|_w, cx| Tooltip::simple("Close all result tabs", cx))
                                .on_click(cx.listener(|this, _, _w, cx| {
                                    this.results.clear();
                                    this.active_view = ResultView::Output;
                                    this.sort_column = None;
                                    this.sort_ascending = true;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        IconButton::new("close-result-panel", IconName::Close)
                            .icon_size(IconSize::XSmall)
                            .icon_color(Color::Muted)
                            .tooltip(|_w, cx| Tooltip::simple("Close panel", cx))
                            .on_click(cx.listener(|_this, _, _w, cx| {
                                cx.emit(PanelEvent::Close);
                            })),
                    )
                    .px_1(),
            )
            // Active result
            .child(self.render_active_result(cx))
            .when_some(self.tab_context_menu.as_ref(), |d, (menu, _)| {
                d.child(
                    deferred(
                        anchored()
                            .anchor(Corner::TopLeft)
                            .child(menu.clone()),
                    )
                    .with_priority(1),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_update_sql_basic() {
        let columns = vec!["id".to_string(), "name".to_string(), "email".to_string()];
        let row = vec![
            "1".to_string(),
            "Alice".to_string(),
            "alice@example.com".to_string(),
        ];
        let result = generate_update_sql(
            "SELECT * FROM users LIMIT 100",
            &columns,
            &row,
            1,
            "Bob",
        );
        assert_eq!(
            result,
            Some("UPDATE users SET \"name\" = 'Bob' WHERE \"id\" = '1';".to_string())
        );
    }

    #[test]
    fn test_generate_update_sql_with_schema() {
        let columns = vec!["id".to_string(), "value".to_string()];
        let row = vec!["42".to_string(), "old".to_string()];
        let result = generate_update_sql(
            "SELECT * FROM public.settings LIMIT 50",
            &columns,
            &row,
            1,
            "new",
        );
        assert_eq!(
            result,
            Some("UPDATE public.settings SET \"value\" = 'new' WHERE \"id\" = '42';".to_string())
        );
    }

    #[test]
    fn test_generate_update_sql_null_value() {
        let columns = vec!["id".to_string(), "name".to_string()];
        let row = vec!["1".to_string(), "Alice".to_string()];
        let result = generate_update_sql(
            "SELECT * FROM users LIMIT 100",
            &columns,
            &row,
            1,
            "NULL",
        );
        assert_eq!(
            result,
            Some("UPDATE users SET \"name\" = NULL WHERE \"id\" = '1';".to_string())
        );
    }

    #[test]
    fn test_generate_update_sql_escapes_single_quotes() {
        let columns = vec!["id".to_string(), "name".to_string()];
        let row = vec!["1".to_string(), "Alice".to_string()];
        let result = generate_update_sql(
            "SELECT * FROM users LIMIT 100",
            &columns,
            &row,
            1,
            "O'Brien",
        );
        assert_eq!(
            result,
            Some("UPDATE users SET \"name\" = 'O''Brien' WHERE \"id\" = '1';".to_string())
        );
    }

    #[test]
    fn test_generate_update_sql_no_from_clause() {
        let columns = vec!["id".to_string(), "name".to_string()];
        let row = vec!["1".to_string(), "Alice".to_string()];
        let result = generate_update_sql("SHOW TABLES", &columns, &row, 1, "Bob");
        assert_eq!(result, None);
    }

    #[test]
    fn test_generate_update_sql_uses_first_column_as_pk_when_no_id() {
        let columns = vec!["user_id".to_string(), "name".to_string()];
        let row = vec!["99".to_string(), "Alice".to_string()];
        let result = generate_update_sql(
            "SELECT * FROM users LIMIT 100",
            &columns,
            &row,
            1,
            "Bob",
        );
        assert_eq!(
            result,
            Some("UPDATE users SET \"name\" = 'Bob' WHERE \"user_id\" = '99';".to_string())
        );
    }

    #[test]
    fn test_generate_update_sql_null_pk() {
        let columns = vec!["id".to_string(), "name".to_string()];
        let row = vec!["NULL".to_string(), "Alice".to_string()];
        let result = generate_update_sql(
            "SELECT * FROM users LIMIT 100",
            &columns,
            &row,
            1,
            "Bob",
        );
        assert_eq!(
            result,
            Some("UPDATE users SET \"name\" = 'Bob' WHERE \"id\" IS NULL;".to_string())
        );
    }

    #[test]
    fn test_extract_table_name_simple() {
        assert_eq!(
            extract_table_name("SELECT * FROM users LIMIT 100"),
            Some("users".to_string())
        );
    }

    #[test]
    fn test_extract_table_name_with_schema() {
        assert_eq!(
            extract_table_name("SELECT * FROM public.users LIMIT 100"),
            Some("public.users".to_string())
        );
    }

    #[test]
    fn test_extract_table_name_lowercase_from() {
        assert_eq!(
            extract_table_name("select * from my_table where id = 1"),
            Some("my_table".to_string())
        );
    }

    #[test]
    fn test_extract_table_name_no_from() {
        assert_eq!(extract_table_name("SHOW TABLES"), None);
    }
}
