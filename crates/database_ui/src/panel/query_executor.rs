use crate::database_panel::*;
use crate::result_panel::DatabaseResultPanel;
use crate::sql_completion::SqlCompletionProvider;
use database_core::{ConnectionManager, DatabaseSchema};
use editor::Editor;
use gpui::{AppContext, AsyncWindowContext, Context, Task, WeakEntity, Window};
use gpui_tokio::Tokio;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use workspace::Workspace;

impl DatabasePanel {
    pub(crate) fn open_query_console(
        &mut self,
        conn_name: &str,
        database: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_query_connection = Some(conn_name.to_string());
        self.active_query_database = database.map(|d| d.to_string());
        self.open_sql_editor(None, window, cx);
    }


    pub(crate) fn query_table(
        &mut self,
        schema: &str,
        table: &str,
        _conn_name: &str,
        database: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_schema = Some(schema.to_string());
        if let Some(db) = database {
            self.active_query_database = Some(db.to_string());
        }
        let query = format!("SELECT * FROM {schema}.{table} LIMIT 100;");
        self.run_sql(query, window, cx);
    }

    pub(crate) fn show_ddl(
        &mut self,
        schema: &str,
        table: &str,
        conn_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Build a CREATE TABLE representation from the cached schema
        let Some(db_schema) = self.schemas.get(conn_name) else {
            return;
        };

        let tbl = db_schema
            .tables
            .iter()
            .find(|t| t.schema == schema && t.name == table);
        let Some(tbl) = tbl else { return };

        let mut ddl = format!("CREATE TABLE {}.{} (\n", tbl.schema, tbl.name);
        for (i, col) in tbl.columns.iter().enumerate() {
            ddl.push_str(&format!("    {} {}", col.name, col.data_type));
            if !col.is_nullable {
                ddl.push_str(" NOT NULL");
            }
            if let Some(ref def) = col.default_value {
                ddl.push_str(&format!(" DEFAULT {def}"));
            }
            if i < tbl.columns.len() - 1 {
                ddl.push(',');
            }
            ddl.push('\n');
        }

        // Primary key
        let pk_cols: Vec<_> = tbl
            .columns
            .iter()
            .filter(|c| c.is_primary_key)
            .map(|c| c.name.clone())
            .collect();
        if !pk_cols.is_empty() {
            // Remove last newline and add comma
            if ddl.ends_with('\n') {
                ddl.pop();
                if !ddl.ends_with(',') {
                    ddl.push(',');
                }
                ddl.push('\n');
            }
            ddl.push_str(&format!(
                "    PRIMARY KEY ({})\n",
                pk_cols.join(", ")
            ));
        }

        ddl.push_str(");\n");

        // Indexes
        for idx in &tbl.indexes {
            if idx.is_primary {
                continue;
            }
            let unique = if idx.is_unique { "UNIQUE " } else { "" };
            ddl.push_str(&format!(
                "\nCREATE {unique}INDEX {} ON {}.{} ({});\n",
                idx.name,
                tbl.schema,
                tbl.name,
                idx.columns.join(", ")
            ));
        }

        self.open_sql_editor(Some(ddl), window, cx);
    }

    pub(crate) fn count_table_rows(
        &mut self,
        schema_name: &str,
        table_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let qualified_name = if schema_name == "public" || schema_name.is_empty() {
            table_name.to_string()
        } else {
            format!("{schema_name}.{table_name}")
        };
        let sql = format!("SELECT COUNT(*) as count FROM {qualified_name}");
        let conn_manager = self.connection_manager.clone();
        let weak_workspace = self._workspace.clone();
        let label = format!("COUNT(*) FROM {qualified_name}");
        let schema = Some(schema_name.to_string());

        let task = cx.spawn_in(window, async move |_this, cx| {
            let conn_mgr = conn_manager.clone();
            let result = Tokio::spawn_result(cx, async move {
                let mut mgr = conn_mgr.write().await;
                mgr.execute_query_in_schema(&sql, schema.as_deref())
                    .await
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

            if let Some(ws) = weak_workspace.upgrade() {
                let _ = ws.update_in(cx, |workspace, window, cx| {
                    if let Some(result_panel) = workspace.panel::<DatabaseResultPanel>(cx) {
                        result_panel.update(cx, |panel, cx| {
                            panel.push_result(label, query_result, cx);
                        });
                        workspace.open_panel::<DatabaseResultPanel>(window, cx);
                    }
                });
            }
        });

        self._pending_task = Some(task);
    }

    pub(crate) fn open_sql_editor(
        &mut self,
        initial_text: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self._workspace.upgrade() else {
            return;
        };

        // Build a descriptive console title with provider info
        let registry = database_core::ProviderRegistry::new();
        let provider_list = registry.available_providers();
        let console_title = self
            .active_query_connection
            .as_ref()
            .and_then(|conn_name| {
                self.connections
                    .iter()
                    .find(|c| c.name == *conn_name)
                    .map(|c| {
                        let display_name = provider_list
                            .iter()
                            .find(|p| p.id == c.provider)
                            .map(|p| p.display_name)
                            .unwrap_or(c.provider.as_str());
                        match &self.active_query_database {
                            Some(db) => format!("{display_name} · {db} · console"),
                            None => format!("{display_name} · console"),
                        }
                    })
            })
            .unwrap_or_else(|| "console".to_string());

        let schema = self.active_schema_for_completions.clone();
        let schema_name = self.active_schema_name_for_completions.clone();

        let provider_id = self
            .active_query_connection
            .as_ref()
            .and_then(|conn_name| {
                self.connections
                    .iter()
                    .find(|c| c.name == *conn_name)
                    .map(|c| c.provider.clone())
            })
            .unwrap_or_else(|| {
                let registry = database_core::ProviderRegistry::new();
                registry
                    .available_providers()
                    .first()
                    .map(|p| p.id.to_string())
                    .unwrap_or_default()
            });

        workspace.update(cx, |workspace, cx| {
            let project = workspace.project().clone();
            let languages = project.read(cx).languages().clone();

            let buffer = cx.new(|cx| {
                language::Buffer::local(
                    initial_text.as_deref().unwrap_or(""),
                    cx,
                )
            });

            let editor = cx.new(|cx| {
                let mut editor =
                    Editor::for_buffer(buffer.clone(), Some(project), window, cx);
                editor.buffer().update(cx, |mb, cx| {
                    mb.set_title(console_title.clone(), cx);
                });
                let completion_provider = SqlCompletionProvider::new(schema, schema_name);
                editor.set_completion_provider(Some(std::rc::Rc::new(completion_provider)));
                editor
            });

            // Subscribe to buffer edits for SQL diagnostics
            let diag_buffer = buffer.clone();
            cx.subscribe(&buffer, move |_workspace, _buf, event, cx| {
                if matches!(event, language::BufferEvent::Edited { .. }) {
                    crate::sql_diagnostics::update_sql_diagnostics(&diag_buffer, &provider_id, cx);
                }
            })
            .detach();

            workspace.add_item_to_active_pane(
                Box::new(editor),
                None,
                true,
                window,
                cx,
            );

            // Set SQL language asynchronously (syntax highlighting)
            let sql_lang_task = languages.language_for_name("SQL");
            cx.spawn_in(window, async move |_ws, cx| {
                if let Ok(sql_lang) = sql_lang_task.await {
                    let _ = buffer.update_in(cx, |buf: &mut language::Buffer, _window, cx| {
                        buf.set_language(Some(sql_lang), cx);
                    });
                }
            })
            .detach();
        });
    }

    /// Get SQL from the active editor: selected text if any, otherwise the full text.
    pub(crate) fn get_sql_from_active_editor(&self, cx: &Context<Self>) -> Option<String> {
        let workspace = self._workspace.upgrade()?;
        let editor_entity = workspace
            .read(cx)
            .active_item(cx)?
            .act_as::<Editor>(cx)?;
        let editor = editor_entity.read(cx);

        // Check if there's a non-empty selection via anchors
        let newest = editor.selections.newest_anchor();
        if newest.start != newest.end {
            let snapshot = editor.buffer().read(cx).snapshot(cx);
            let start: multi_buffer::MultiBufferOffset =
                snapshot.summary_for_anchor(&newest.start);
            let end: multi_buffer::MultiBufferOffset =
                snapshot.summary_for_anchor(&newest.end);
            if start != end {
                let text: String = snapshot.text_for_range(start..end).collect();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
        }

        // No selection — use full text
        let text = editor.text(cx).trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Check if we need to auto-reconnect and prepare credential reading.
    /// Returns (connection_name, credential_read_task) if reconnection is needed.
    pub(crate) fn prepare_reconnect_if_needed(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(String, Task<anyhow::Result<Option<(String, Vec<u8>)>>>)> {
        let has_active = self
            .active_query_connection
            .as_ref()
            .and_then(|name| self.connections.iter().find(|c| &c.name == name))
            .is_some_and(|c| c.status == ConnectionStatus::Connected);

        if has_active {
            return None;
        }

        // Pick a connection to reconnect to: prefer active_query_connection, else first available
        let name = self
            .active_query_connection
            .clone()
            .or_else(|| self.connections.first().map(|c| c.name.clone()))?;

        // Mark as connecting in the UI
        if let Some(entry) = self.connections.iter_mut().find(|c| c.name == name) {
            entry.status = ConnectionStatus::Connecting;
        }
        cx.notify();

        let credential_key = format!("database-panel://{name}");
        let cred_task = cx.read_credentials(&credential_key);

        Some((name, cred_task))
    }

    pub(crate) fn run_sql(&mut self, sql: String, window: &mut Window, cx: &mut Context<Self>) {
        if sql.is_empty() { return; }

        let reconnect = self.prepare_reconnect_if_needed(cx);
        let is_read_only = self.read_only;
        let conn_manager = self.connection_manager.clone();
        let weak_workspace = self._workspace.clone();
        let schema = self.active_schema.clone();
        let database = self.active_query_database.clone();
        let tx_mode = self.transaction_mode;
        let isolation = self.isolation_level;

        let statements: Vec<String> = split_sql_statements(&sql);
        if statements.is_empty() { return; }

        let task = cx.spawn_in(window, async move |this, cx| {
            let mut effective_schema = schema;
            if let Some((name, cred_task)) = reconnect {
                match Self::do_reconnect(&name, cred_task, &conn_manager, &this, cx).await {
                    Ok(new_schema) => {
                        effective_schema = new_schema;
                    }
                    Err(e) => {
                        Self::push_error_result(
                            &weak_workspace,
                            statements.first().cloned().unwrap_or_default(),
                            format!("Auto-reconnect failed: {e:#}"),
                            cx,
                        );
                        return;
                    }
                }
            }

            for stmt in &statements {
                // Read-only check
                if is_read_only && !database_core::provider::is_read_query(stmt) {
                    let err = database_core::QueryResult {
                        columns: vec!["Error".to_string()],
                        rows: vec![vec!["Connection is read-only. Only SELECT queries are allowed.".to_string()]],
                        rows_affected: 0,
                        execution_time_ms: 0,
                    };
                    Self::push_query_result(&weak_workspace, stmt.clone(), err, cx);
                    break;
                }

                let tx_cmd = detect_transaction_command(stmt);

                let result = Self::execute_statement(
                    &conn_manager, stmt, &database, &effective_schema, isolation, tx_mode, tx_cmd, cx,
                ).await;

                // If this is a connection error, try reconnecting once and retrying
                let result = match &result {
                    Err(e) if is_connection_error(&format!("{e:#}")) => {
                        let conn_mgr_retry = conn_manager.clone();
                        let reconnect_ok = Tokio::spawn_result(cx, async move {
                            let mut mgr = conn_mgr_retry.write().await;
                            mgr.test_connection().await
                        })
                        .await
                        .is_ok();

                        if reconnect_ok {
                            let retry_tx_cmd = detect_transaction_command(stmt);
                            Self::execute_statement(
                                &conn_manager, stmt, &database, &effective_schema, isolation, tx_mode, retry_tx_cmd, cx,
                            ).await
                        } else {
                            result
                        }
                    }
                    _ => result,
                };

                let query_result = match result {
                    Ok(qr) => qr,
                    Err(e) => database_core::QueryResult {
                        columns: vec!["Error".to_string()],
                        rows: vec![vec![format!("{e:#}")]],
                        rows_affected: 0,
                        execution_time_ms: 0,
                    },
                };

                let is_error = query_result
                    .columns
                    .first()
                    .is_some_and(|c| c == "Error");

                Self::push_query_result(&weak_workspace, stmt.clone(), query_result, cx);

                // Update in_transaction state
                let conn_mgr_check = conn_manager.clone();
                let in_tx = Tokio::spawn_result(cx, async move {
                    let mut mgr = conn_mgr_check.write().await;
                    Ok::<bool, anyhow::Error>(mgr.has_active_transaction().await)
                })
                .await
                .unwrap_or(false);

                let _ = this.update_in(cx, |panel, _window, cx| {
                    panel.in_transaction = in_tx;
                    cx.notify();
                });

                if is_error {
                    break;
                }
            }
        });
        self._pending_task = Some(task);
    }

    pub(crate) fn explain_sql(&mut self, sql: String, window: &mut Window, cx: &mut Context<Self>) {
        if sql.is_empty() { return; }

        let reconnect = self.prepare_reconnect_if_needed(cx);
        let conn_manager = self.connection_manager.clone();
        let weak_workspace = self._workspace.clone();
        let query_label = format!("EXPLAIN {}", &sql[..sql.len().min(40)]);
        let schema = self.active_schema.clone();
        let database = self.active_query_database.clone();

        let task = cx.spawn_in(window, async move |this, cx| {
            let mut effective_schema = schema;
            if let Some((name, cred_task)) = reconnect {
                match Self::do_reconnect(&name, cred_task, &conn_manager, &this, cx).await {
                    Ok(new_schema) => {
                        effective_schema = new_schema;
                    }
                    Err(e) => {
                        Self::push_error_result(
                            &weak_workspace,
                            query_label,
                            format!("Auto-reconnect failed: {e:#}"),
                            cx,
                        );
                        return;
                    }
                }
            }

            let conn_mgr = conn_manager.clone();
            let result = Tokio::spawn_result(cx, async move {
                let mut mgr = conn_mgr.write().await;
                match database {
                    Some(ref db) => {
                        mgr.explain_query_in_database(&sql, db, effective_schema.as_deref())
                            .await
                    }
                    None => {
                        mgr.explain_query_in_schema(&sql, effective_schema.as_deref())
                            .await
                    }
                }
            })
            .await;

            let query_result = match result {
                Ok(plan) => database_core::QueryResult {
                    columns: vec!["QUERY PLAN".to_string()],
                    rows: plan.lines().filter(|l| !l.is_empty()).map(|l| vec![l.to_string()]).collect(),
                    rows_affected: 0,
                    execution_time_ms: 0,
                },
                Err(e) => database_core::QueryResult {
                    columns: vec!["Error".to_string()],
                    rows: vec![vec![format!("{e:#}")]],
                    rows_affected: 0,
                    execution_time_ms: 0,
                },
            };

            Self::push_query_result(&weak_workspace, query_label, query_result, cx);
        });
        self._pending_task = Some(task);
    }

    pub(crate) fn run_query_from_editor(
        &mut self,
        _: &RunQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(sql) = self.get_sql_from_active_editor(cx) {
            let sql = sql.trim().to_string();
            if !sql.is_empty() {
                self.run_sql(sql, window, cx);
            }
        }
    }

    pub(crate) fn explain_query_from_editor(
        &mut self,
        _: &ExplainQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(sql) = self.get_sql_from_active_editor(cx) {
            let sql = sql.trim().to_string();
            if !sql.is_empty() {
                self.explain_sql(sql, window, cx);
            }
        }
    }

    /// Perform auto-reconnect: read keychain password, connect, fetch schema.
    /// Returns the active schema on success.
    pub(crate) async fn do_reconnect(
        name: &str,
        cred_task: Task<anyhow::Result<Option<(String, Vec<u8>)>>>,
        conn_manager: &Arc<RwLock<ConnectionManager>>,
        this: &gpui::WeakEntity<Self>,
        cx: &mut AsyncWindowContext,
    ) -> anyhow::Result<Option<String>> {
        let keychain_password = cred_task
            .await
            .ok()
            .flatten()
            .and_then(|(_, pw_bytes)| String::from_utf8(pw_bytes).ok());

        {
            let mut mgr = conn_manager.write().await;
            mgr.set_active_connection(name);
            if let Some(ref pw) = keychain_password {
                mgr.set_password(name, pw.clone());
            }
        }

        let conn_mgr_clone = conn_manager.clone();
        let db_result: anyhow::Result<Option<DatabaseSchema>> =
            Tokio::spawn_result(cx, async move {
                let mut mgr = conn_mgr_clone.write().await;
                mgr.test_connection().await?;
                let schema = mgr.get_schema().await.ok();
                Ok(schema)
            })
            .await;

        let db_schema = db_result?;
        let name_owned = name.to_string();

        let active_schema = this
            .update_in(cx, |panel, _window, cx| {
                if let Some(e) = panel.connections.iter_mut().find(|c| c.name == name_owned) {
                    e.status = ConnectionStatus::Connected;
                }
                if let Some(s) = db_schema {
                    panel.schemas.insert(name_owned.clone(), s);
                }
                panel.expanded_nodes.insert(format!("conn:{name_owned}"));
                panel.active_query_connection = Some(name_owned.clone());

                // Set default active schema
                if let Some(schema) = panel.schemas.get(&name_owned) {
                    let schema_names: Vec<String> = schema
                        .tables
                        .iter()
                        .map(|t| t.schema.clone())
                        .chain(schema.views.iter().map(|v| v.schema.clone()))
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect();
                    if schema_names.contains(&"public".to_string()) {
                        panel.active_schema = Some("public".to_string());
                    } else if let Some(first) = schema_names.first() {
                        panel.active_schema = Some(first.clone());
                    }
                }

                // Update completions
                if let Some(schema) = panel.schemas.get(&name_owned) {
                    let schema_arc = panel.active_schema_for_completions.clone();
                    let schema_clone = schema.clone();
                    let name_arc = panel.active_schema_name_for_completions.clone();
                    let active = panel.active_schema.clone();
                    cx.background_executor()
                        .spawn(async move {
                            let mut s = schema_arc.write().await;
                            *s = Some(schema_clone);
                            let mut n = name_arc.write().await;
                            *n = active;
                        })
                        .detach();
                }

                let result = panel.active_schema.clone();
                cx.notify();
                result
            })
            .ok()
            .flatten();

        Ok(active_schema)
    }

    async fn execute_statement(
        conn_manager: &Arc<RwLock<ConnectionManager>>,
        stmt: &str,
        database: &Option<String>,
        schema: &Option<String>,
        isolation: crate::database_panel::IsolationLevel,
        tx_mode: crate::database_panel::TransactionMode,
        tx_cmd: Option<TransactionCommand>,
        cx: &mut AsyncWindowContext,
    ) -> anyhow::Result<database_core::QueryResult> {
        let conn_mgr = conn_manager.clone();
        let stmt_owned = stmt.to_string();
        let db = database.clone();
        let sch = schema.clone();

        Tokio::spawn_result(cx, async move {
            let mut mgr = conn_mgr.write().await;

            match tx_cmd {
                Some(TransactionCommand::Begin) => {
                    mgr.begin_transaction(isolation.as_sql()).await?;
                    Ok(database_core::QueryResult {
                        columns: vec!["status".to_string()],
                        rows: vec![vec!["BEGIN".to_string()]],
                        rows_affected: 0,
                        execution_time_ms: 0,
                    })
                }
                Some(TransactionCommand::Commit) => {
                    mgr.commit_transaction().await?;
                    Ok(database_core::QueryResult {
                        columns: vec!["status".to_string()],
                        rows: vec![vec!["COMMIT".to_string()]],
                        rows_affected: 0,
                        execution_time_ms: 0,
                    })
                }
                Some(TransactionCommand::Rollback) => {
                    mgr.rollback_transaction().await?;
                    Ok(database_core::QueryResult {
                        columns: vec!["status".to_string()],
                        rows: vec![vec!["ROLLBACK".to_string()]],
                        rows_affected: 0,
                        execution_time_ms: 0,
                    })
                }
                None => {
                    let in_tx = mgr.has_active_transaction().await;

                    if in_tx {
                        mgr.execute_in_transaction(&stmt_owned).await
                    } else if matches!(tx_mode, crate::database_panel::TransactionMode::Manual) {
                        mgr.begin_transaction(isolation.as_sql()).await?;
                        mgr.execute_in_transaction(&stmt_owned).await
                    } else {
                        match db {
                            Some(ref db) => {
                                mgr.execute_query_in_database(&stmt_owned, db, sch.as_deref()).await
                            }
                            None => {
                                mgr.execute_query_in_schema(&stmt_owned, sch.as_deref()).await
                            }
                        }
                    }
                }
            }
        })
        .await
    }

    /// Push a query result to the result panel.
    pub(crate) fn push_query_result(
        weak_workspace: &WeakEntity<Workspace>,
        label: String,
        result: database_core::QueryResult,
        cx: &mut AsyncWindowContext,
    ) {
        if let Some(ws) = weak_workspace.upgrade() {
            let _ = ws.update_in(cx, |workspace, window, cx| {
                if let Some(result_panel) = workspace.panel::<DatabaseResultPanel>(cx) {
                    result_panel.update(cx, |panel, cx| {
                        panel.push_result(label, result, cx);
                    });
                    workspace.open_panel::<DatabaseResultPanel>(window, cx);
                }
            });
        }
    }

    /// Push an error message to the result panel.
    pub(crate) fn push_error_result(
        weak_workspace: &WeakEntity<Workspace>,
        label: String,
        error: String,
        cx: &mut AsyncWindowContext,
    ) {
        let err_result = database_core::QueryResult {
            columns: vec!["Error".to_string()],
            rows: vec![vec![error]],
            rows_affected: 0,
            execution_time_ms: 0,
        };
        Self::push_query_result(weak_workspace, label, err_result, cx);
    }
}

enum TransactionCommand {
    Begin,
    Commit,
    Rollback,
}

fn detect_transaction_command(sql: &str) -> Option<TransactionCommand> {
    let upper = sql.trim().to_uppercase();
    if upper.starts_with("BEGIN") || upper.starts_with("START TRANSACTION") {
        Some(TransactionCommand::Begin)
    } else if upper.starts_with("COMMIT") || upper == "END" {
        Some(TransactionCommand::Commit)
    } else if upper.starts_with("ROLLBACK") || upper.starts_with("ABORT") {
        Some(TransactionCommand::Rollback)
    } else {
        None
    }
}

pub(crate) fn is_connection_error(error_msg: &str) -> bool {
    let lower = error_msg.to_lowercase();
    lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("connection closed")
        || lower.contains("broken pipe")
        || lower.contains("no connection")
        || lower.contains("server closed")
        || lower.contains("connection timed out")
        || lower.contains("pool timed out")
        || lower.contains("connection terminated")
}

pub(crate) fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];
        let next = if i + 1 < len { Some(chars[i + 1]) } else { None };

        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
            }
            current.push(ch);
            i += 1;
            continue;
        }

        if in_block_comment {
            current.push(ch);
            if ch == '*' && next == Some('/') {
                current.push('/');
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if in_single_quote {
            current.push(ch);
            if ch == '\'' && next == Some('\'') {
                current.push('\'');
                i += 2;
            } else if ch == '\'' {
                in_single_quote = false;
                i += 1;
            } else {
                i += 1;
            }
            continue;
        }

        if in_double_quote {
            current.push(ch);
            if ch == '"' {
                in_double_quote = false;
            }
            i += 1;
            continue;
        }

        match ch {
            '\'' => {
                in_single_quote = true;
                current.push(ch);
            }
            '"' => {
                in_double_quote = true;
                current.push(ch);
            }
            '-' if next == Some('-') => {
                in_line_comment = true;
                current.push(ch);
            }
            '/' if next == Some('*') => {
                in_block_comment = true;
                current.push(ch);
                current.push('*');
                i += 2;
                continue;
            }
            ';' => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    statements.push(trimmed);
                }
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
        i += 1;
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        statements.push(trimmed);
    }

    statements
}
