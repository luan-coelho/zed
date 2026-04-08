mod database_panel;
mod database_panel_settings;
mod panel;
pub mod result_panel;
pub mod sql_completion;
mod sql_diagnostics;
pub mod sql_toolbar;

#[cfg(test)]
mod sql_completion_tests;
#[cfg(test)]
mod sql_diagnostics_tests;

pub use database_panel::DatabasePanel;
pub use result_panel::DatabaseResultPanel;

use database_panel::{ExplainQuery, RunQuery, Toggle, ToggleFocus};
use workspace::Workspace;

pub fn init(cx: &mut gpui::App) {
    database_panel_settings::init(cx);

    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<DatabasePanel>(window, cx);
        });
        workspace.register_action(|workspace, _: &Toggle, window, cx| {
            if !workspace.toggle_panel_focus::<DatabasePanel>(window, cx) {
                workspace.close_panel::<DatabasePanel>(window, cx);
            }
        });
        workspace.register_action(
            |workspace, _: &result_panel::ToggleFocus, window, cx| {
                workspace.toggle_panel_focus::<DatabaseResultPanel>(window, cx);
            },
        );
        // Run/Explain query from any editor — extract SQL here (workspace is
        // already locked by register_action) and pass it to the panel.
        workspace.register_action(|workspace, _: &RunQuery, window, cx| {
            let sql = extract_sql_from_active_editor(workspace, cx);
            if let (Some(sql), Some(panel)) = (sql, workspace.panel::<DatabasePanel>(cx)) {
                panel.update(cx, |panel, cx| {
                    panel.run_sql(sql, window, cx);
                });
            }
        });
        workspace.register_action(|workspace, _: &ExplainQuery, window, cx| {
            let sql = extract_sql_from_active_editor(workspace, cx);
            if let (Some(sql), Some(panel)) = (sql, workspace.panel::<DatabasePanel>(cx)) {
                panel.update(cx, |panel, cx| {
                    panel.explain_sql(sql, window, cx);
                });
            }
        });
    })
    .detach();
}

/// Extract SQL from the active editor (selection or full text).
/// Called from workspace context where workspace is already available.
fn extract_sql_from_active_editor(workspace: &Workspace, cx: &gpui::App) -> Option<String> {
    use editor::Editor;

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
