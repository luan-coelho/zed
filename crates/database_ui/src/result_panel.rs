use database_core::QueryResult;
use gpui::{
    actions, div, Action, App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled, WeakEntity,
    Window,
};
use theme::ActiveTheme;
use ui::{prelude::*, Tooltip};
use workspace::dock::{DockPosition, Panel, PanelEvent};
use workspace::Workspace;

fn escape_csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
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
    Table(usize),
}

pub struct DatabaseResultPanel {
    focus_handle: FocusHandle,
    results: Vec<QueryResultEntry>,
    active_view: ResultView,
    selected_row: Option<usize>,
    output_log: Vec<String>,
}

struct QueryResultEntry {
    query: String,
    result: QueryResult,
    page: usize,
    timestamp: String,
}

impl DatabaseResultPanel {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        Self {
            focus_handle,
            results: Vec::new(),
            active_view: ResultView::Output,
            selected_row: None,
            output_log: Vec::new(),
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

        self.results.push(QueryResultEntry {
            query,
            result,
            page: 0,
            timestamp: now,
        });
        let idx = self.results.len() - 1;
        self.active_view = ResultView::Table(idx);
        self.selected_row = None;
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
            ResultView::Output => None,
        }
    }

    fn copy_active_result_as_tsv(&self, cx: &mut Context<Self>) {
        let Some(result) = self.active_result_data() else {
            return;
        };
        let mut output = result.columns.join("\t");
        output.push('\n');
        for row in &result.rows {
            output.push_str(&row.join("\t"));
            output.push('\n');
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(output));
    }

    fn copy_active_result_as_csv(&self, cx: &mut Context<Self>) {
        let Some(result) = self.active_result_data() else {
            return;
        };
        let mut output = String::new();
        // Header
        output.push_str(
            &result
                .columns
                .iter()
                .map(|c| escape_csv(c))
                .collect::<Vec<_>>()
                .join(","),
        );
        output.push('\n');
        // Rows
        for row in &result.rows {
            output.push_str(
                &row.iter()
                    .map(|v| escape_csv(v))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            output.push('\n');
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(output));
    }

    fn copy_active_result_as_json(&self, cx: &mut Context<Self>) {
        let Some(result) = self.active_result_data() else {
            return;
        };
        let rows: Vec<serde_json::Value> = result
            .rows
            .iter()
            .map(|row| {
                let mut obj = serde_json::Map::new();
                for (i, col) in result.columns.iter().enumerate() {
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

        if let Ok(json) = serde_json::to_string_pretty(&rows) {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(json));
        }
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_output_active = self.active_view == ResultView::Output;

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
                        cx.notify();
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

    fn render_active_result(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.active_view == ResultView::Output {
            return self.render_output_log(cx).into_any_element();
        }

        let idx = match self.active_view {
            ResultView::Table(i) => i,
            ResultView::Output => unreachable!(),
        };
        let Some(entry) = self.results.get(idx) else {
            return self.render_empty().into_any_element();
        };

        let result = &entry.result;

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

        // Pagination
        let total_rows = result.rows.len();
        let total_pages = (total_rows + PAGE_SIZE - 1) / PAGE_SIZE;
        let current_page = entry.page.min(total_pages.saturating_sub(1));
        let page_start = current_page * PAGE_SIZE;
        let page_end = (page_start + PAGE_SIZE).min(total_rows);
        let page_rows: Vec<Vec<String>> = result.rows[page_start..page_end].to_vec();
        let selected_row = self.selected_row;

        div()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            // Column header
            .child({
                let mut header = div()
                    .flex()
                    .flex_none()
                    .w_full()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .bg(cx.theme().colors().title_bar_background);

                // Row number column
                header = header.child(
                    div()
                        .w(px(40.))
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .child(Label::new("#").size(LabelSize::XSmall).color(Color::Muted)),
                );

                for col in &result.columns {
                    header = header.child(
                        div()
                            .w(col_width)
                            .flex_shrink_0()
                            .px_2()
                            .py_1()
                            .child(
                                Label::new(col.clone())
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
                                    let is_null = val == "NULL";
                                    let is_error = columns
                                        .first()
                                        .map(|c| c == "Error")
                                        .unwrap_or(false)
                                        && col_idx == 0;

                                    let val_clone = val.clone();
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
                                                let v = val_clone.clone();
                                                move |_, _w, cx| {
                                                    cx.write_to_clipboard(
                                                        gpui::ClipboardItem::new_string(v.clone()),
                                                    );
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
                                    .on_click(cx.listener(|this, _, _w, cx| {
                                        this.copy_active_result_as_tsv(cx);
                                    })),
                            )
                            .child(
                                Button::new("export-csv", "CSV")
                                    .style(ButtonStyle::Subtle)
                                    .label_size(LabelSize::XSmall)
                                    .on_click(cx.listener(|this, _, _w, cx| {
                                        this.copy_active_result_as_csv(cx);
                                    })),
                            )
                            .child(
                                Button::new("export-json", "JSON")
                                    .style(ButtonStyle::Subtle)
                                    .label_size(LabelSize::XSmall)
                                    .on_click(cx.listener(|this, _, _w, cx| {
                                        this.copy_active_result_as_json(cx);
                                    })),
                            ),
                    ),
            )
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
            // Tab bar
            .child(
                div()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(self.render_tab_bar(cx)),
            )
            // Active result
            .child(self.render_active_result(cx))
    }
}
