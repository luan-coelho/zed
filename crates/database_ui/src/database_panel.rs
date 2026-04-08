use crate::database_panel_settings::DatabasePanelSettings;
use crate::result_panel::DatabaseResultPanel;
use crate::sql_completion::SqlCompletionProvider;
use anyhow::Result;
use database_core::{ConnectionManager, DatabaseConfig, DatabaseSchema, Table, View};
use editor::Editor;
use gpui::{
    actions, anchored, deferred, div, Action, AnyElement, App, AsyncWindowContext, Context, Corner,
    DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Point, Render, SharedString, Styled, Subscription,
    Task, WeakEntity, Window,
};
use gpui_tokio::Tokio;
use std::collections::HashSet;
use std::sync::Arc;
use theme::ActiveTheme;
use tokio::sync::RwLock;
use ui::{prelude::*, ContextMenu, Tooltip};
use workspace::dock::{DockPosition, Panel, PanelEvent};
use workspace::Workspace;

actions!(
    database_panel,
    [
        Toggle,
        ToggleFocus,
        Close,
        RefreshConnections,
        RunQuery,
        ExplainQuery,
        NewConnection,
        FocusNextField,
        FocusPrevField,
    ]
);

// ── Data types ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
enum ConnectionScope {
    Global,
    Project,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct ConnectionEntry {
    name: String,
    provider: String,
    host: String,
    port: String,
    database: String,
    status: ConnectionStatus,
    scope: ConnectionScope,
}

#[derive(Clone, Debug, PartialEq)]
enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

struct ConnectionForm {
    name_editor: Entity<Editor>,
    host_editor: Entity<Editor>,
    port_editor: Entity<Editor>,
    database_editor: Entity<Editor>,
    user_editor: Entity<Editor>,
    password_editor: Entity<Editor>,
    provider: String,
    scope: ConnectionScope,
    editing: Option<String>,
    error_message: Option<String>,
    test_status: Option<TestStatus>,
}

impl ConnectionForm {
    fn editors(&self) -> Vec<&Entity<Editor>> {
        vec![
            &self.name_editor,
            &self.host_editor,
            &self.port_editor,
            &self.database_editor,
            &self.user_editor,
            &self.password_editor,
        ]
    }
}

#[derive(Clone, Debug)]
enum TestStatus {
    Testing,
    Success,
    Failed(String),
}

const PROVIDERS: &[(&str, &str, &str)] = &[
    ("postgres", "PostgreSQL", "5432"),
    ("mysql", "MySQL", "3306"),
    ("sqlite", "SQLite", ""),
];

// ── Main struct ─────────────────────────────────────────────────────

pub struct DatabasePanel {
    focus_handle: FocusHandle,
    _workspace: WeakEntity<Workspace>,
    connection_manager: Arc<RwLock<ConnectionManager>>,
    connections: Vec<ConnectionEntry>,
    schemas: std::collections::HashMap<String, DatabaseSchema>,
    active_schema_for_completions: Arc<RwLock<Option<DatabaseSchema>>>,
    active_schema_name_for_completions: Arc<RwLock<Option<String>>>,
    expanded_nodes: HashSet<String>,
    selected_node: Option<String>,
    project_path: Option<String>,
    global_path: String,
    active_query_connection: Option<String>,
    active_schema: Option<String>,
    conn_form: Option<ConnectionForm>,
    context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    search_editor: Entity<Editor>,
    search_filter: String,
    _pending_task: Option<Task<()>>,
}

fn make_entry(
    name: &str,
    conn: &database_core::ConnectionConfig,
    scope: ConnectionScope,
    old: &[ConnectionEntry],
) -> ConnectionEntry {
    ConnectionEntry {
        name: name.to_string(),
        provider: conn.provider.clone(),
        host: conn.host.clone().unwrap_or_else(|| "localhost".into()),
        port: conn.port.map(|p| p.to_string()).unwrap_or_else(|| "5432".into()),
        database: conn.database.clone().unwrap_or_default(),
        status: old
            .iter()
            .find(|c| c.name == name && c.scope == scope)
            .map(|c| c.status.clone())
            .unwrap_or(ConnectionStatus::Disconnected),
        scope,
    }
}

// ── Construction ────────────────────────────────────────────────────

impl DatabasePanel {
    fn new(
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        let weak_workspace = cx.entity().downgrade();
        let project_path = workspace
            .project()
            .read(cx)
            .worktrees(cx)
            .next()
            .map(|wt| wt.read(cx).abs_path().to_string_lossy().to_string());

        let global_path = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());

        cx.new(|cx| {
            let focus_handle = cx.focus_handle();
            cx.on_focus(&focus_handle, window, |this: &mut Self, _window, cx| {
                this.re_detect_project_path(cx);
                this.load_connections(cx);
            })
            .detach();

            let search_editor = cx.new(|cx| {
                let mut e = Editor::single_line(window, cx);
                e.set_placeholder_text("Filter tables...", window, cx);
                e
            });

            // Subscribe to search editor changes
            cx.subscribe(&search_editor, |this, editor, event, cx| {
                if let editor::EditorEvent::BufferEdited = event {
                    this.search_filter = editor.read(cx).text(cx).to_lowercase();
                    cx.notify();
                }
            })
            .detach();

            let mut panel = Self {
                focus_handle,
                _workspace: weak_workspace,
                connection_manager: Arc::new(RwLock::new(ConnectionManager::new())),
                connections: Vec::new(),
                schemas: std::collections::HashMap::new(),
                active_schema_for_completions: Arc::new(RwLock::new(None)),
                active_schema_name_for_completions: Arc::new(RwLock::new(None)),
                expanded_nodes: HashSet::new(),
                selected_node: None,
                project_path,
                global_path,
                active_query_connection: None,
                active_schema: None,
                conn_form: None,
                context_menu: None,
                search_editor,
                search_filter: String::new(),
                _pending_task: None,
            };
            panel.load_connections(cx);
            panel
        })
    }

    pub fn active_schema_arc(&self) -> Arc<RwLock<Option<DatabaseSchema>>> {
        self.active_schema_for_completions.clone()
    }

    pub fn active_schema_name_arc(&self) -> Arc<RwLock<Option<String>>> {
        self.active_schema_name_for_completions.clone()
    }

    pub fn has_active_connection(&self) -> bool {
        self.connections
            .iter()
            .any(|c| c.status == ConnectionStatus::Connected)
    }

    /// Returns (database_name, active_schema) for the active connection.
    pub fn active_connection_info(&self) -> Option<(String, String)> {
        let conn_name = self.active_query_connection.as_ref()?;
        let db_schema = self.schemas.get(conn_name)?;
        let schema = self
            .active_schema
            .clone()
            .unwrap_or_else(|| "public".to_string());
        Some((db_schema.name.clone(), schema))
    }

    /// Returns available schema names for the active connection.
    pub fn available_schemas(&self) -> Vec<String> {
        let Some(conn_name) = &self.active_query_connection else {
            return vec![];
        };
        let Some(db_schema) = self.schemas.get(conn_name) else {
            return vec![];
        };

        let mut schemas: Vec<String> = db_schema
            .tables
            .iter()
            .map(|t| t.schema.clone())
            .chain(db_schema.views.iter().map(|v| v.schema.clone()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        schemas.sort();
        schemas
    }

    /// Returns the current active schema name.
    pub fn active_schema_name(&self) -> Option<&str> {
        self.active_schema.as_deref()
    }

    /// Set the active schema for query execution.
    pub fn set_active_schema(&mut self, schema: String, cx: &mut Context<Self>) {
        self.active_schema = Some(schema.clone());
        let arc = self.active_schema_name_for_completions.clone();
        cx.background_executor()
            .spawn(async move {
                let mut s = arc.write().await;
                *s = Some(schema);
            })
            .detach();
        cx.notify();
    }

    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            DatabasePanel::new(workspace, window, cx)
        })
    }

    // ── Config & connections ────────────────────────────────────────

    fn re_detect_project_path(&mut self, cx: &Context<Self>) {
        if self.project_path.is_none() {
            if let Some(ws) = self._workspace.upgrade() {
                self.project_path = ws
                    .read(cx)
                    .project()
                    .read(cx)
                    .worktrees(cx)
                    .next()
                    .map(|wt| wt.read(cx).abs_path().to_string_lossy().to_string());
            }
        }
    }

    fn load_connections(&mut self, cx: &mut Context<Self>) {
        let old_connections = std::mem::take(&mut self.connections);
        let mut all_configs = DatabaseConfig { connections: Default::default() };

        // Load global connections
        let mut global_entries = Vec::new();
        if let Ok(config) = DatabaseConfig::load_from_workspace(&self.global_path) {
            for (name, conn) in &config.connections {
                global_entries.push(make_entry(name, conn, ConnectionScope::Global, &old_connections));
            }
            for (k, v) in config.connections {
                all_configs.connections.insert(k, v);
            }
        }

        // Load project connections
        let mut project_entries = Vec::new();
        if let Some(ref project_path) = self.project_path {
            if let Ok(config) = DatabaseConfig::load_from_workspace(project_path) {
                for (name, conn) in &config.connections {
                    project_entries.push(make_entry(name, conn, ConnectionScope::Project, &old_connections));
                }
                for (k, v) in config.connections {
                    all_configs.connections.insert(k, v);
                }
            }
        }

        global_entries.sort_by(|a, b| a.name.cmp(&b.name));
        project_entries.sort_by(|a, b| a.name.cmp(&b.name));

        self.connections = global_entries;
        self.connections.extend(project_entries);

        // Load all configs into connection manager
        let conn_manager = self.connection_manager.clone();
        cx.background_executor()
            .spawn(async move {
                let mut mgr = conn_manager.write().await;
                mgr.load_config(all_configs);
            })
            .detach();

        cx.notify();
    }

    fn connect(&mut self, conn_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.connections.iter_mut().find(|c| c.name == conn_name) else {
            return;
        };
        entry.status = ConnectionStatus::Connecting;
        cx.notify();

        let name = conn_name.to_string();
        let credential_key = format!("database-panel://{name}");
        let read_creds_task = cx.read_credentials(&credential_key);
        let conn_manager = self.connection_manager.clone();

        let task = cx.spawn_in(window, async move |this, cx| {
            let keychain_password = read_creds_task
                .await
                .ok()
                .flatten()
                .and_then(|(_, pw_bytes)| String::from_utf8(pw_bytes).ok());

            {
                let mut mgr = conn_manager.write().await;
                mgr.set_active_connection(&name);
                if let Some(ref pw) = keychain_password {
                    mgr.set_password(&name, pw.clone());
                }
            }

            let conn_mgr_clone = conn_manager.clone();
            let db_result = Tokio::spawn_result(cx, async move {
                let mut mgr = conn_mgr_clone.write().await;
                mgr.test_connection().await?;
                let schema = mgr.get_schema().await.ok();
                Ok(schema)
            })
            .await;

            match db_result {
                Ok(schema) => {
                    let _ = this.update_in(cx, |this, _window, cx| {
                        if let Some(e) = this.connections.iter_mut().find(|c| c.name == name) {
                            e.status = ConnectionStatus::Connected;
                        }
                        if let Some(s) = schema {
                            this.schemas.insert(name.clone(), s);
                        }
                        this.expanded_nodes.insert(format!("conn:{name}"));
                        this.active_query_connection = Some(name.clone());
                        // Set default active schema (prefer "public", fall back to first)
                        if let Some(schema) = this.schemas.get(&name) {
                            let schemas: Vec<String> = schema
                                .tables
                                .iter()
                                .map(|t| t.schema.clone())
                                .chain(schema.views.iter().map(|v| v.schema.clone()))
                                .collect::<HashSet<_>>()
                                .into_iter()
                                .collect();
                            if schemas.contains(&"public".to_string()) {
                                this.active_schema = Some("public".to_string());
                            } else if let Some(first) = schemas.first() {
                                this.active_schema = Some(first.clone());
                            }
                        }
                        // Update schema + schema name for SQL completions
                        if let Some(schema) = this.schemas.get(&name) {
                            let schema_arc = this.active_schema_for_completions.clone();
                            let schema_clone = schema.clone();
                            let name_arc = this.active_schema_name_for_completions.clone();
                            let active = this.active_schema.clone();
                            cx.background_executor()
                                .spawn(async move {
                                    let mut s = schema_arc.write().await;
                                    *s = Some(schema_clone);
                                    let mut n = name_arc.write().await;
                                    *n = active;
                                })
                                .detach();
                        }
                        cx.notify();
                    });
                }
                Err(e) => {
                    let err_msg = format!("{e:#}");
                    eprintln!("[database_panel] Connection error: {err_msg}");
                    let _ = this.update_in(cx, |this, _window, cx| {
                        if let Some(entry) = this.connections.iter_mut().find(|c| c.name == name) {
                            entry.status = ConnectionStatus::Error(err_msg);
                        }
                        cx.notify();
                    });
                }
            }
        });
        self._pending_task = Some(task);
    }

    fn disconnect(&mut self, conn_name: &str, cx: &mut Context<Self>) {
        if let Some(e) = self.connections.iter_mut().find(|c| c.name == conn_name) {
            e.status = ConnectionStatus::Disconnected;
        }
        self.schemas.remove(conn_name);
        let name = conn_name.to_string();
        let conn_manager = self.connection_manager.clone();
        cx.background_executor()
            .spawn(async move {
                let mut mgr = conn_manager.write().await;
                mgr.clear_provider(&name);
            })
            .detach();
        cx.notify();
    }

    fn set_active_for_queries(&mut self, conn_name: &str, cx: &mut Context<Self>) {
        let is_connected = self
            .connections
            .iter()
            .any(|c| c.name == conn_name && c.status == ConnectionStatus::Connected);
        if is_connected {
            self.active_query_connection = Some(conn_name.to_string());
            let name = conn_name.to_string();
            let conn_manager = self.connection_manager.clone();
            cx.background_executor()
                .spawn(async move {
                    let mut mgr = conn_manager.write().await;
                    mgr.set_active_connection(&name);
                })
                .detach();

            // Update schema for completions
            if let Some(schema) = self.schemas.get(conn_name) {
                let schema_arc = self.active_schema_for_completions.clone();
                let schema_clone = schema.clone();
                cx.background_executor()
                    .spawn(async move {
                        let mut s = schema_arc.write().await;
                        *s = Some(schema_clone);
                    })
                    .detach();
            }
        }
    }

    fn toggle_node(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.expanded_nodes.contains(key) {
            self.expanded_nodes.remove(key);
        } else {
            self.expanded_nodes.insert(key.to_string());
        }
        self.selected_node = Some(key.to_string());
        cx.notify();
    }

    #[allow(dead_code)]
    fn select_node(&mut self, key: &str, cx: &mut Context<Self>) {
        self.selected_node = Some(key.to_string());
        cx.notify();
    }

    fn open_query_console(
        &mut self,
        _conn_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sql_editor(None, window, cx);
    }

    fn query_table(
        &mut self,
        schema: &str,
        table: &str,
        _conn_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = if schema == "public" {
            format!("SELECT * FROM {table} LIMIT 100;")
        } else {
            format!("SELECT * FROM {schema}.{table} LIMIT 100;")
        };
        self.open_sql_editor(Some(query), window, cx);
    }

    fn show_ddl(
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

    fn count_table_rows(
        &mut self,
        table_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sql = format!("SELECT COUNT(*) as count FROM {table_name}");
        let conn_manager = self.connection_manager.clone();
        let weak_workspace = self._workspace.clone();
        let label = format!("COUNT(*) FROM {table_name}");
        let schema = self.active_schema.clone();

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

    fn open_sql_editor(
        &mut self,
        initial_text: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self._workspace.upgrade() else {
            return;
        };

        let schema = self.active_schema_for_completions.clone();
        let schema_name = self.active_schema_name_for_completions.clone();

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
                    mb.set_title("console".to_string(), cx);
                });
                let provider = SqlCompletionProvider::new(schema, schema_name);
                editor.set_completion_provider(Some(std::rc::Rc::new(provider)));
                editor
            });

            // Subscribe to buffer edits for SQL diagnostics
            let diag_buffer = buffer.clone();
            cx.subscribe(&buffer, move |_workspace, _buf, event, cx| {
                if matches!(event, language::BufferEvent::Edited { .. }) {
                    crate::sql_diagnostics::update_sql_diagnostics(&diag_buffer, cx);
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
                    let _ = buffer.update_in(cx, |buf, _window, cx| {
                        buf.set_language(Some(sql_lang), cx);
                    });
                }
            })
            .detach();
        });
    }

    // ── Connection form ─────────────────────────────────────────────

    fn show_connection_form(
        &mut self,
        editing: Option<String>,
        scope: ConnectionScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let base_path = match scope {
            ConnectionScope::Global => Some(self.global_path.clone()),
            ConnectionScope::Project => self.project_path.clone(),
        };

        let (name_val, host_val, port_val, db_val, user_val, provider_val) =
            if let Some(ref edit_name) = editing {
                let config = base_path
                    .as_ref()
                    .and_then(|p| DatabaseConfig::load_from_workspace(p).ok());
                let conn = config.as_ref().and_then(|c| c.connections.get(edit_name));
                match conn {
                    Some(c) => (
                        edit_name.clone(),
                        c.host.clone().unwrap_or_default(),
                        c.port.map(|p| p.to_string()).unwrap_or_default(),
                        c.database.clone().unwrap_or_default(),
                        c.user.clone().unwrap_or_default(),
                        c.provider.clone(),
                    ),
                    None => return,
                }
            } else {
                (String::new(), String::new(), String::new(), String::new(), String::new(), "postgres".into())
            };

        let default_port = PROVIDERS
            .iter()
            .find(|(id, _, _)| *id == provider_val)
            .map(|(_, _, p)| *p)
            .unwrap_or("5432");

        let make_editor =
            |cx: &mut Context<Self>, window: &mut Window, placeholder: &str, value: &str| {
                cx.new(|cx| {
                    let mut e = Editor::single_line(window, cx);
                    e.set_placeholder_text(placeholder, window, cx);
                    if !value.is_empty() {
                        e.set_text(value, window, cx);
                    }
                    e
                })
            };

        self.conn_form = Some(ConnectionForm {
            name_editor: make_editor(cx, window, "my_database", &name_val),
            host_editor: make_editor(cx, window, "localhost", &host_val),
            port_editor: make_editor(cx, window, default_port, &port_val),
            database_editor: make_editor(cx, window, "mydb", &db_val),
            user_editor: make_editor(cx, window, "postgres", &user_val),
            password_editor: make_editor(cx, window, "stored in OS keychain", ""),
            provider: provider_val,
            scope,
            editing,
            error_message: None,
            test_status: None,
        });
        cx.notify();
    }

    fn test_form_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ref form) = self.conn_form else {
            return;
        };

        let provider = form.provider.clone();
        let host_raw = form.host_editor.read(cx).text(cx).trim().to_string();
        let port_raw = form.port_editor.read(cx).text(cx).trim().to_string();
        let database = form.database_editor.read(cx).text(cx).trim().to_string();
        let user_raw = form.user_editor.read(cx).text(cx).trim().to_string();
        let password = form.password_editor.read(cx).text(cx).trim().to_string();

        let default_port = PROVIDERS
            .iter()
            .find(|(id, _, _)| *id == provider)
            .map(|(_, _, p)| *p)
            .unwrap_or("5432");
        let default_user = if provider == "mysql" || provider == "mariadb" { "root" } else { "postgres" };

        let host = if host_raw.is_empty() { "localhost".to_string() } else { host_raw };
        let port = if port_raw.is_empty() { default_port.to_string() } else { port_raw };
        let user = if user_raw.is_empty() { default_user.to_string() } else { user_raw };

        if database.is_empty() {
            if let Some(ref mut f) = self.conn_form {
                f.test_status = Some(TestStatus::Failed("Database is required".into()));
            }
            cx.notify();
            return;
        }

        if let Some(ref mut f) = self.conn_form {
            f.test_status = Some(TestStatus::Testing);
        }
        cx.notify();

        // Build a temporary ConnectionConfig to test
        let config = database_core::ConnectionConfig {
            provider: provider.clone(),
            host: if host.is_empty() { None } else { Some(host) },
            port: port.parse().ok(),
            database: Some(database),
            user: if user.is_empty() { None } else { Some(user) },
            password_env: None,
            connection_string_env: None,
            ssl: Some(false),
            default: None,
        };

        let url = match config.connection_url_with_password(
            if password.is_empty() { None } else { Some(&password) },
        ) {
            Ok(u) => u,
            Err(e) => {
                if let Some(ref mut f) = self.conn_form {
                    f.test_status = Some(TestStatus::Failed(format!("{e:#}")));
                }
                cx.notify();
                return;
            }
        };

        let provider_box = database_core::providers::get_provider(&provider);
        let Some(provider_impl) = provider_box else {
            if let Some(ref mut f) = self.conn_form {
                f.test_status = Some(TestStatus::Failed("Unknown provider".into()));
            }
            cx.notify();
            return;
        };

        let task = cx.spawn_in(window, async move |this, cx| {
            let result = Tokio::spawn_result(cx, async move {
                provider_impl.test_connection(&url).await
            })
            .await;

            let _ = this.update_in(cx, |this, _w, cx| {
                if let Some(ref mut f) = this.conn_form {
                    f.test_status = Some(match result {
                        Ok(()) => TestStatus::Success,
                        Err(e) => TestStatus::Failed(format!("{e:#}")),
                    });
                }
                cx.notify();
            });
        });

        self._pending_task = Some(task);
    }

    fn focus_next_form_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ref form) = self.conn_form else { return };
        let editors = form.editors();
        let current = editors
            .iter()
            .position(|e| e.focus_handle(cx).contains_focused(window, cx));
        let next = match current {
            Some(i) => (i + 1) % editors.len(),
            None => 0,
        };
        editors[next].focus_handle(cx).focus(window, cx);
    }

    fn focus_prev_form_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ref form) = self.conn_form else { return };
        let editors = form.editors();
        let current = editors
            .iter()
            .position(|e| e.focus_handle(cx).contains_focused(window, cx));
        let prev = match current {
            Some(0) | None => editors.len() - 1,
            Some(i) => i - 1,
        };
        editors[prev].focus_handle(cx).focus(window, cx);
    }

    fn cancel_form(&mut self, cx: &mut Context<Self>) {
        self.conn_form = None;
        cx.notify();
    }

    fn save_form(&mut self, cx: &mut Context<Self>) {
        let Some(ref form) = self.conn_form else { return };

        let name = form.name_editor.read(cx).text(cx).trim().to_string();
        let host_raw = form.host_editor.read(cx).text(cx).trim().to_string();
        let port_raw = form.port_editor.read(cx).text(cx).trim().to_string();
        let database = form.database_editor.read(cx).text(cx).trim().to_string();
        let user_raw = form.user_editor.read(cx).text(cx).trim().to_string();
        let password = form.password_editor.read(cx).text(cx).trim().to_string();
        let provider = form.provider.clone();
        let editing = form.editing.clone();

        // Apply defaults for empty fields
        let default_port = PROVIDERS
            .iter()
            .find(|(id, _, _)| *id == provider)
            .map(|(_, _, p)| *p)
            .unwrap_or("5432");
        let default_user = if provider == "mysql" || provider == "mariadb" {
            "root"
        } else {
            "postgres"
        };

        let host = if host_raw.is_empty() { "localhost".to_string() } else { host_raw };
        let port = if port_raw.is_empty() { default_port.to_string() } else { port_raw };
        let user = if user_raw.is_empty() { default_user.to_string() } else { user_raw };

        if name.is_empty() {
            if let Some(ref mut f) = self.conn_form {
                f.error_message = Some("Connection name is required".into());
            }
            cx.notify();
            return;
        }
        if database.is_empty() {
            if let Some(ref mut f) = self.conn_form {
                f.error_message = Some("Database name is required".into());
            }
            cx.notify();
            return;
        }

        // Save password
        if !password.is_empty() {
            let conn_manager = self.connection_manager.clone();
            let pw = password.clone();
            let n = name.clone();
            cx.background_executor()
                .spawn(async move {
                    let mut mgr = conn_manager.write().await;
                    mgr.set_password(&n, pw);
                })
                .detach();

            let cred_key = format!("database-panel://{name}");
            let cred_user = if user.is_empty() { "postgres".to_string() } else { user.clone() };
            let write_task = cx.write_credentials(&cred_key, &cred_user, password.as_bytes());
            cx.background_executor()
                .spawn(async move { let _ = write_task.await; })
                .detach();
        }

        let scope = form.scope;
        let ws_path = match scope {
            ConnectionScope::Global => self.global_path.clone(),
            ConnectionScope::Project => match self.project_path.clone() {
                Some(p) => p,
                None => {
                    if let Some(ref mut f) = self.conn_form {
                        f.error_message = Some("No project open. Use Global scope or open a folder.".into());
                    }
                    cx.notify();
                    return;
                }
            },
        };

        eprintln!("[database_panel] save_form: saving to {ws_path}/.database/connections.toml (scope: {:?})", scope);

        let conn_config = database_core::ConnectionConfig {
            provider: provider.clone(),
            host: if host.is_empty() { None } else { Some(host) },
            port: port.parse().ok(),
            database: Some(database),
            user: if user.is_empty() { None } else { Some(user) },
            password_env: None,
            connection_string_env: None,
            ssl: Some(false),
            default: Some(false),
        };

        let config_path = std::path::Path::new(&ws_path).join(".database/connections.toml");
        let mut config = if config_path.exists() {
            DatabaseConfig::load_from_workspace(&ws_path)
                .unwrap_or(DatabaseConfig { connections: Default::default() })
        } else {
            DatabaseConfig { connections: Default::default() }
        };

        if let Some(ref old_name) = editing {
            if old_name != &name {
                config.connections.remove(old_name);
                let _ = cx.delete_credentials(&format!("database-panel://{old_name}"));
            }
        }

        if config.connections.is_empty() {
            let mut c = conn_config;
            c.default = Some(true);
            config.connections.insert(name, c);
        } else {
            config.connections.insert(name, conn_config);
        }

        let dir = std::path::Path::new(&ws_path).join(".database");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("[database_panel] save_form: failed to create dir: {e}");
            if let Some(ref mut f) = self.conn_form {
                f.error_message = Some(format!("Failed to create .database dir: {e}"));
            }
            cx.notify();
            return;
        }
        match toml::to_string_pretty(&config) {
            Ok(content) => {
                match std::fs::write(&config_path, &content) {
                    Ok(()) => eprintln!("[database_panel] save_form: saved successfully"),
                    Err(e) => {
                        eprintln!("[database_panel] save_form: write failed: {e}");
                        if let Some(ref mut f) = self.conn_form {
                            f.error_message = Some(format!("Failed to write: {e}"));
                        }
                        cx.notify();
                        return;
                    }
                }
            }
            Err(e) => {
                eprintln!("[database_panel] save_form: serialize failed: {e}");
                if let Some(ref mut f) = self.conn_form {
                    f.error_message = Some(format!("Failed to serialize: {e}"));
                }
                cx.notify();
                return;
            }
        }

        self.conn_form = None;
        self.load_connections(cx);
    }

    #[allow(dead_code)]
    fn delete_connection(&mut self, conn_name: &str, cx: &mut Context<Self>) {
        let scope = self.connections.iter().find(|c| c.name == conn_name).map(|c| c.scope).unwrap_or(ConnectionScope::Global);
        let ws_path = match scope {
            ConnectionScope::Global => self.global_path.clone(),
            ConnectionScope::Project => match self.project_path.clone() {
                Some(p) => p,
                None => return,
            },
        };
        let config_path = std::path::Path::new(&ws_path).join(".database/connections.toml");
        if let Ok(mut config) = DatabaseConfig::load_from_workspace(&ws_path) {
            config.connections.remove(conn_name);
            if let Ok(content) = toml::to_string_pretty(&config) {
                let _ = std::fs::write(&config_path, content);
            }
        }
        let _ = cx.delete_credentials(&format!("database-panel://{conn_name}"));
        self.schemas.remove(conn_name);
        self.load_connections(cx);
    }

    // ── Action handlers ─────────────────────────────────────────────

    fn set_context_menu(
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

    fn deploy_new_connection_menu_at(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();

        let has_project = self.project_path.is_some();
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            let mut m = menu;

            for &(id, label, _) in PROVIDERS {
                let w = weak.clone();
                let id = id.to_string();
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

    fn show_connection_form_with_provider(
        &mut self,
        provider: &str,
        scope: ConnectionScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_connection_form(None, scope, window, cx);
        if let Some(ref mut form) = self.conn_form {
            form.provider = provider.to_string();
        }
        cx.notify();
    }

    fn deploy_connection_context_menu(
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

        let weak = cx.entity().downgrade();

        let menu = ContextMenu::build(window, cx, {
            let name = conn_name.clone();
            move |menu, _, _| {
                let mut m = menu;
                if is_connected {
                    m = m.entry("Open Query Console", None, {
                        let w = weak.clone();
                        let n = name.clone();
                        move |window, cx| {
                            if let Some(panel) = w.upgrade() {
                                panel.update(cx, |this, cx| {
                                    this.open_query_console(&n, window, cx);
                                });
                            }
                        }
                    });
                    m = m.separator();
                    m = m.entry("Refresh Schema", None, {
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
                                // Detect scope from existing connection
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

    fn deploy_table_context_menu(
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
                                this.query_table(&sn, &tn, &cn, window, cx);
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
                .entry("Count Rows", None, {
                    let w = weak.clone();
                    let fn_ = fn_.clone();
                    move |window, cx| {
                        if let Some(panel) = w.upgrade() {
                            panel.update(cx, |this, cx| {
                                this.count_table_rows(&fn_, window, cx);
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
            }
        });

        self.set_context_menu(menu, position, window, cx);
    }

    /// Get SQL from the active editor: selected text if any, otherwise the full text.
    fn get_sql_from_active_editor(&self, cx: &Context<Self>) -> Option<String> {
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

    // ── Auto-reconnect helpers ────────────────────────────────────

    /// Check if we need to auto-reconnect and prepare credential reading.
    /// Returns (connection_name, credential_read_task) if reconnection is needed.
    fn prepare_reconnect_if_needed(
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

    // ── Query execution ─────────────────────────────────────────

    pub fn run_sql(&mut self, sql: String, window: &mut Window, cx: &mut Context<Self>) {
        if sql.is_empty() { return; }

        let reconnect = self.prepare_reconnect_if_needed(cx);
        let conn_manager = self.connection_manager.clone();
        let weak_workspace = self._workspace.clone();
        let query_label = sql.clone();
        let schema = self.active_schema.clone();

        let task = cx.spawn_in(window, async move |this, cx| {
            // Phase 1: Auto-reconnect if needed
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

            // Phase 2: Execute the query
            let conn_mgr = conn_manager.clone();
            let result = Tokio::spawn_result(cx, async move {
                let mut mgr = conn_mgr.write().await;
                mgr.execute_query_in_schema(&sql, effective_schema.as_deref())
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

            Self::push_query_result(&weak_workspace, query_label, query_result, cx);
        });
        self._pending_task = Some(task);
    }

    pub fn explain_sql(&mut self, sql: String, window: &mut Window, cx: &mut Context<Self>) {
        if sql.is_empty() { return; }

        let reconnect = self.prepare_reconnect_if_needed(cx);
        let conn_manager = self.connection_manager.clone();
        let weak_workspace = self._workspace.clone();
        let query_label = format!("EXPLAIN {}", &sql[..sql.len().min(40)]);
        let schema = self.active_schema.clone();

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
                mgr.explain_query_in_schema(&sql, effective_schema.as_deref())
                    .await
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

    pub fn run_query_from_editor(
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

    pub fn explain_query_from_editor(
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
    async fn do_reconnect(
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

    /// Push a query result to the result panel.
    fn push_query_result(
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
    fn push_error_result(
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

    fn close(&mut self, _: &Close, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PanelEvent::Close);
    }

    fn refresh(&mut self, _: &RefreshConnections, _window: &mut Window, cx: &mut Context<Self>) {
        self.load_connections(cx);
    }

    fn new_connection_action(&mut self, _: &NewConnection, window: &mut Window, cx: &mut Context<Self>) {
        self.show_connection_form(None, ConnectionScope::Global, window, cx);
    }

    // ── Render: Toolbar ─────────────────────────────────────────────

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_conn = self.active_query_connection.as_ref().and_then(|name| {
            self.connections.iter().find(|c| c.name == *name && c.status == ConnectionStatus::Connected)
        }).or_else(|| {
            self.connections.iter().find(|c| c.status == ConnectionStatus::Connected)
        });

        let title_label: SharedString = match active_conn {
            Some(c) => format!("{}@{}", c.name, c.host).into(),
            None => "Database".into(),
        };
        let title_color = if active_conn.is_some() {
            Color::Success
        } else {
            Color::Accent
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_hidden()
                    .child(
                        Icon::new(IconName::DatabaseZap)
                            .size(IconSize::Small)
                            .color(title_color),
                    )
                    .child(
                        Label::new(title_label)
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD)
                            .color(title_color),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_0()
                    .child(
                        IconButton::new("add_conn", IconName::Plus)
                            .icon_size(IconSize::XSmall)
                            .tooltip(|_w, cx| Tooltip::simple("New Connection", cx))
                            .on_click(cx.listener(|this, event, window, cx| {
                                let position = match event {
                                    gpui::ClickEvent::Mouse(e) => e.down.position,
                                    gpui::ClickEvent::Keyboard(_) => gpui::point(px(0.), px(30.)),
                                };
                                this.deploy_new_connection_menu_at(position, window, cx);
                            })),
                    )
                    .child(
                        IconButton::new("refresh", IconName::RotateCw)
                            .icon_size(IconSize::XSmall)
                            .tooltip(|_w, cx| Tooltip::simple("Refresh", cx))
                            .on_click(cx.listener(|this, _, _w, cx| {
                                this.load_connections(cx);
                            })),
                    ),
            )
    }

    // ── Render: Search bar ────────────────────────────────────────────

    fn render_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_connections = self
            .connections
            .iter()
            .any(|c| c.status == ConnectionStatus::Connected);

        if !has_connections || self.conn_form.is_some() {
            return div().into_any_element();
        }

        div()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(Icon::new(IconName::MagnifyingGlass).size(IconSize::XSmall).color(Color::Muted))
            .child(div().flex_1().child(self.search_editor.clone()))
            .when(!self.search_filter.is_empty(), |d| {
                d.child(
                    IconButton::new("clear-search", IconName::Close)
                        .icon_size(IconSize::XSmall)
                        .icon_color(Color::Muted)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search_editor.update(cx, |editor, cx| {
                                editor.set_text("", window, cx);
                            });
                            this.search_filter.clear();
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }

    // ── Render: Tree view ───────────────────────────────────────────

    fn render_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.conn_form.is_some() {
            return self.render_form(cx);
        }

        if self.connections.is_empty() {
            return self.render_empty_state(cx);
        }

        let global_conns: Vec<_> = self.connections.iter().filter(|c| c.scope == ConnectionScope::Global).collect();
        let project_conns: Vec<_> = self.connections.iter().filter(|c| c.scope == ConnectionScope::Project).collect();

        let mut tree = div().flex().flex_col().w_full();

        // Global section
        if !global_conns.is_empty() {
            tree = tree.child(
                div().px_3().py_1().child(
                    Label::new("Global Data Sources")
                        .size(LabelSize::XSmall)
                        .weight(FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                ),
            );
            for conn in &global_conns {
                tree = tree.child(self.render_connection_node(conn, cx));
            }
        }

        // Project section
        if !project_conns.is_empty() {
            tree = tree.child(
                div().px_3().py_1().mt_1().child(
                    Label::new("Project Data Sources")
                        .size(LabelSize::XSmall)
                        .weight(FontWeight::SEMIBOLD)
                        .color(Color::Muted),
                ),
            );
            for conn in &project_conns {
                tree = tree.child(self.render_connection_node(conn, cx));
            }
        }

        div()
            .id("db-tree-scroll")
            .flex_1()
            .overflow_y_scroll()
            .child(tree)
            .into_any_element()
    }

    fn render_connection_node(&self, conn: &ConnectionEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let conn_key = format!("conn:{}", conn.name);
        let is_expanded = self.expanded_nodes.contains(&conn_key);
        let is_selected = self.selected_node.as_ref() == Some(&conn_key);
        let is_connected = conn.status == ConnectionStatus::Connected;
        let is_connecting = conn.status == ConnectionStatus::Connecting;
        let is_active_query = self
            .active_query_connection
            .as_ref()
            .map(|n| n == &conn.name)
            .unwrap_or(false)
            && is_connected;

        let (status_icon, status_color) = match &conn.status {
            ConnectionStatus::Disconnected => (IconName::Circle, Color::Muted),
            ConnectionStatus::Connecting => (IconName::ArrowCircle, Color::Warning),
            ConnectionStatus::Connected => (IconName::DatabaseZap, Color::Success),
            ConnectionStatus::Error(_) => (IconName::Close, Color::Error),
        };

        let name = conn.name.clone();
        let display: SharedString = format!("{}@{}", conn.name, conn.host).into();

        let mut node = div().flex().flex_col().w_full();

        // Connection header row
        node = node.child(
            div()
                .id(ElementId::Name(conn_key.clone().into()))
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .cursor_pointer()
                .when(is_selected, |d| d.bg(cx.theme().colors().ghost_element_selected).rounded_md())
                .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_md())
                .on_click(cx.listener({
                    let key = conn_key.clone();
                    let name = name.clone();
                    move |this, _, window, cx| {
                        if this.expanded_nodes.contains(&key) {
                            this.expanded_nodes.remove(&key);
                        } else {
                            this.expanded_nodes.insert(key.clone());
                            let status = this.connections.iter().find(|c| c.name == name).map(|c| &c.status);
                            if matches!(status, Some(ConnectionStatus::Disconnected) | Some(ConnectionStatus::Error(_))) {
                                this.connect(&name, window, cx);
                            }
                        }
                        // Set as active connection for queries
                        this.set_active_for_queries(&name, cx);
                        this.selected_node = Some(key.clone());
                        cx.notify();
                    }
                }))
                .on_mouse_down(MouseButton::Right, cx.listener({
                    let name = name.clone();
                    move |this, event: &MouseDownEvent, window, cx| {
                        this.deploy_connection_context_menu(
                            name.clone(),
                            event.position,
                            window,
                            cx,
                        );
                    }
                }))
                .child(
                    Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                        .size(IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(Icon::new(status_icon).size(IconSize::XSmall).color(status_color))
                .child(
                    Label::new(display)
                        .size(LabelSize::Small)
                        .weight(if is_active_query { FontWeight::BOLD } else { FontWeight::NORMAL }),
                )
                // Action buttons on hover/selection
                .when(is_connected, |d| {
                    let name = name.clone();
                    d.child(div().flex_1()).child(
                        div().flex().gap_0()
                            .child(
                                IconButton::new(
                                    ElementId::Name(format!("console-{name}").into()),
                                    IconName::Terminal,
                                )
                                .icon_size(IconSize::XSmall)
                                .icon_color(Color::Muted)
                                .tooltip(|_w, cx| Tooltip::simple("Open Query Console", cx))
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |this, _, window, cx| {
                                        this.open_query_console(&name, window, cx);
                                    }
                                })),
                            )
                            .child(
                                IconButton::new(
                                    ElementId::Name(format!("disconnect-{name}").into()),
                                    IconName::Close,
                                )
                                .icon_size(IconSize::XSmall)
                                .icon_color(Color::Muted)
                                .tooltip(|_w, cx| Tooltip::simple("Disconnect", cx))
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |this, _, _w, cx| {
                                        this.disconnect(&name, cx);
                                    }
                                })),
                            ),
                    )
                }),
        );

        // Error message
        if let ConnectionStatus::Error(ref msg) = conn.status {
            node = node.child(
                div().pl(px(28.)).pr_2().child(
                    Label::new(msg.clone())
                        .size(LabelSize::XSmall)
                        .color(Color::Error),
                ),
            );
        }

        // Expanded: show schema
        if is_expanded && is_connected {
            if let Some(schema) = self.schemas.get(&conn.name) {
                node = node.child(self.render_schema_tree(schema, &conn.name, cx));
            }
        }

        // Connecting indicator
        if is_connecting {
            node = node.child(
                div().pl(px(28.)).child(
                    Label::new("Connecting...")
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                ),
            );
        }

        node
    }

    fn render_schema_tree(
        &self,
        schema: &DatabaseSchema,
        conn_name: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut tree = div().flex().flex_col().w_full();

        // Tables
        if !schema.tables.is_empty() {
            let tables_key = format!("tables:{conn_name}");
            let tables_expanded = self.expanded_nodes.contains(&tables_key);

            tree = tree.child(self.render_tree_section(
                &tables_key,
                &format!("Tables ({})", schema.tables.len()),
                IconName::ListTree,
                tables_expanded,
                1,
                cx,
            ));

            if tables_expanded {
                let filter = &self.search_filter;
                for table in &schema.tables {
                    if !filter.is_empty()
                        && !table.name.to_lowercase().contains(filter)
                    {
                        continue;
                    }
                    let tbl_key = format!("tbl:{conn_name}.{}.{}", table.schema, table.name);
                    let tbl_expanded = self.expanded_nodes.contains(&tbl_key);
                    tree = tree.child(self.render_table_node(table, &tbl_key, tbl_expanded, conn_name, cx));
                }
            }
        }

        // Views
        if !schema.views.is_empty() {
            let views_key = format!("views:{conn_name}");
            let views_expanded = self.expanded_nodes.contains(&views_key);

            tree = tree.child(self.render_tree_section(
                &views_key,
                &format!("Views ({})", schema.views.len()),
                IconName::Eye,
                views_expanded,
                1,
                cx,
            ));

            if views_expanded {
                let filter = &self.search_filter;
                for view in &schema.views {
                    if !filter.is_empty()
                        && !view.name.to_lowercase().contains(filter)
                    {
                        continue;
                    }
                    let v_key = format!("view:{conn_name}.{}.{}", view.schema, view.name);
                    let v_expanded = self.expanded_nodes.contains(&v_key);
                    tree = tree.child(self.render_view_node(view, &v_key, v_expanded, cx));
                }
            }
        }

        tree
    }

    fn render_tree_section(
        &self,
        key: &str,
        label: &str,
        icon: IconName,
        is_expanded: bool,
        indent: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_selected = self.selected_node.as_ref().map(|s| s.as_str()) == Some(key);
        let pl = px((indent * 16 + 8) as f32);

        div()
            .id(ElementId::Name(key.to_string().into()))
            .flex()
            .items_center()
            .gap_1()
            .pl(pl)
            .pr_2()
            .py_px()
            .cursor_pointer()
            .when(is_selected, |d| d.bg(cx.theme().colors().ghost_element_selected).rounded_md())
            .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_md())
            .on_click(cx.listener({
                let key = key.to_string();
                move |this, _, _w, cx| { this.toggle_node(&key, cx); }
            }))
            .child(
                Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
            )
            .child(Icon::new(icon).size(IconSize::XSmall).color(Color::Accent))
            .child(Label::new(label.to_string()).size(LabelSize::XSmall))
    }

    fn render_table_node(
        &self,
        table: &Table,
        key: &str,
        is_expanded: bool,
        conn_name: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_selected = self.selected_node.as_ref().map(|s| s.as_str()) == Some(key);

        let mut node = div().flex().flex_col().w_full();

        // Table row
        node = node.child(
            div()
                .id(ElementId::Name(key.to_string().into()))
                .flex()
                .items_center()
                .gap_1()
                .pl(px(40.))
                .pr_2()
                .py_px()
                .cursor_pointer()
                .when(is_selected, |d| d.bg(cx.theme().colors().ghost_element_selected).rounded_md())
                .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_md())
                .on_click(cx.listener({
                    let key = key.to_string();
                    move |this, _, _w, cx| { this.toggle_node(&key, cx); }
                }))
                .on_mouse_down(MouseButton::Right, cx.listener({
                    let s = table.schema.clone();
                    let t = table.name.clone();
                    let cn = conn_name.to_string();
                    move |this, event: &MouseDownEvent, window, cx| {
                        this.deploy_table_context_menu(
                            s.clone(), t.clone(), cn.clone(),
                            event.position, window, cx,
                        );
                    }
                }))
                .child(
                    Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                        .size(IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(Icon::new(IconName::FileTextOutlined).size(IconSize::XSmall).color(Color::Accent))
                .child(Label::new(table.name.clone()).size(LabelSize::XSmall))
                .child(div().flex_1())
                .child({
                    let schema = table.schema.clone();
                    let table_name = table.name.clone();
                    let cn = conn_name.to_string();
                    IconButton::new(
                        ElementId::Name(format!("q-{key}").into()),
                        IconName::PlayFilled,
                    )
                    .icon_size(IconSize::XSmall)
                    .icon_color(Color::Muted)
                    .tooltip(|_w, cx| Tooltip::simple("SELECT * FROM ...", cx))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.query_table(&schema, &table_name, &cn, window, cx);
                    }))
                }),
        );

        // Columns
        if is_expanded {
            for col in &table.columns {
                node = node.child(self.render_column_row(col, 56));
            }
        }

        node
    }

    fn render_view_node(
        &self,
        view: &View,
        key: &str,
        is_expanded: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_selected = self.selected_node.as_ref().map(|s| s.as_str()) == Some(key);

        let mut node = div().flex().flex_col().w_full();

        node = node.child(
            div()
                .id(ElementId::Name(key.to_string().into()))
                .flex()
                .items_center()
                .gap_1()
                .pl(px(40.))
                .pr_2()
                .py_px()
                .cursor_pointer()
                .when(is_selected, |d| d.bg(cx.theme().colors().ghost_element_selected).rounded_md())
                .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_md())
                .on_click(cx.listener({
                    let key = key.to_string();
                    move |this, _, _w, cx| { this.toggle_node(&key, cx); }
                }))
                .child(
                    Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                        .size(IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(Icon::new(IconName::Eye).size(IconSize::XSmall).color(Color::Warning))
                .child(Label::new(view.name.clone()).size(LabelSize::XSmall)),
        );

        if is_expanded {
            for col in &view.columns {
                node = node.child(self.render_column_row(col, 56));
            }
        }

        node
    }

    fn render_column_row(&self, col: &database_core::DbColumn, indent_px: i32) -> impl IntoElement {
        let mut badges: Vec<&str> = Vec::new();
        if col.is_primary_key { badges.push("PK"); }
        if col.foreign_key.is_some() { badges.push("FK"); }
        if !col.is_nullable { badges.push("NOT NULL"); }

        let mut row = div()
            .flex()
            .items_center()
            .gap_1()
            .pl(px(indent_px as f32))
            .pr_2()
            .py_px()
            .child(Icon::new(IconName::Dash).size(IconSize::XSmall).color(Color::Muted))
            .child(Label::new(col.name.clone()).size(LabelSize::XSmall))
            .child(
                Label::new(col.data_type.clone())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );

        for badge in badges {
            row = row.child(
                div().px_1().rounded_sm().bg(gpui::rgb(0x2a2a3a)).child(
                    Label::new(badge).size(LabelSize::XSmall).color(Color::Accent),
                ),
            );
        }

        row
    }

    // ── Render: Empty state ─────────────────────────────────────────

    fn render_empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .flex_1()
            .gap_3()
            .p_4()
            .child(Icon::new(IconName::DatabaseZap).size(IconSize::Medium).color(Color::Muted))
            .child(Label::new("No connections").size(LabelSize::Small).color(Color::Muted))
            .child(Label::new("Add a connection to get started").size(LabelSize::XSmall).color(Color::Muted))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w(px(200.))
                    .child(
                        Button::new("add_pg", "PostgreSQL")
                            .full_width()
                            .style(ButtonStyle::Filled)
                            .start_icon(Icon::new(IconName::DatabaseZap).size(IconSize::Small))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_connection_form_with_provider("postgres", ConnectionScope::Global, window, cx);
                            })),
                    )
                    .child(
                        Button::new("add_mysql", "MySQL")
                            .full_width()
                            .style(ButtonStyle::Subtle)
                            .start_icon(Icon::new(IconName::DatabaseZap).size(IconSize::Small))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_connection_form_with_provider("mysql", ConnectionScope::Global, window, cx);
                            })),
                    )
                    .child(
                        Button::new("add_sqlite", "SQLite")
                            .full_width()
                            .style(ButtonStyle::Subtle)
                            .start_icon(Icon::new(IconName::DatabaseZap).size(IconSize::Small))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_connection_form_with_provider("sqlite", ConnectionScope::Global, window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    // ── Render: Connection form ─────────────────────────────────────

    fn render_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(ref form) = self.conn_form else {
            return div().into_any_element();
        };

        let is_sqlite = form.provider == "sqlite" || form.provider == "sqlite3";
        let title = if form.editing.is_some() { "Edit Connection" } else { "New Connection" };
        let title_icon = if form.editing.is_some() { IconName::Settings } else { IconName::Plus };

        div()
            .flex()
            .flex_col()
            .size_full()
            .key_context("DatabaseConnectionForm")
            .on_action(cx.listener(|this, _: &FocusNextField, window, cx| {
                this.focus_next_form_field(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusPrevField, window, cx| {
                this.focus_prev_form_field(window, cx);
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .child(
                        div().flex().items_center().gap_2()
                            .child(Icon::new(title_icon).size(IconSize::Small).color(Color::Accent))
                            .child(Label::new(title).size(LabelSize::Small).weight(FontWeight::SEMIBOLD)),
                    )
                    // Provider badge
                    .child({
                        let provider_label = PROVIDERS
                            .iter()
                            .find(|(id, _, _)| *id == form.provider)
                            .map(|(_, label, _)| *label)
                            .unwrap_or("Unknown");
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(cx.theme().colors().ghost_element_hover)
                            .child(
                                Icon::new(IconName::DatabaseZap)
                                    .size(IconSize::XSmall)
                                    .color(Color::Accent),
                            )
                            .child(
                                Label::new(provider_label)
                                    .size(LabelSize::XSmall)
                                    .weight(FontWeight::SEMIBOLD),
                            )
                    })
                    .child(self.render_field("Name", &form.name_editor, cx))
                    .when(is_sqlite, |d| {
                        d.child(self.render_sqlite_path_field(cx))
                    })
                    .when(!is_sqlite, |d| {
                        d.child(
                            div().flex().gap_2().w_full()
                                .child(div().flex_1().child(self.render_field("Host", &form.host_editor, cx)))
                                .child(div().w(px(70.)).child(self.render_field("Port", &form.port_editor, cx))),
                        )
                        .child(self.render_field("Database", &form.database_editor, cx))
                        .child(self.render_field("User", &form.user_editor, cx))
                        .child(self.render_field("Password", &form.password_editor, cx))
                    })
                    // Error / test status messages
                    .when_some(form.error_message.clone(), |d, msg| {
                        d.child(Label::new(msg).size(LabelSize::XSmall).color(Color::Error))
                    })
                    .when_some(form.test_status.clone(), |d, status| {
                        d.child(match status {
                            TestStatus::Testing => div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(Icon::new(IconName::ArrowCircle).size(IconSize::XSmall).color(Color::Warning))
                                .child(Label::new("Testing connection...").size(LabelSize::XSmall).color(Color::Warning))
                                .into_any_element(),
                            TestStatus::Success => div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(Icon::new(IconName::Check).size(IconSize::XSmall).color(Color::Success))
                                .child(Label::new("Connection successful!").size(LabelSize::XSmall).color(Color::Success))
                                .into_any_element(),
                            TestStatus::Failed(msg) => div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(Icon::new(IconName::Close).size(IconSize::XSmall).color(Color::Error))
                                .child(Label::new(msg).size(LabelSize::XSmall).color(Color::Error))
                                .into_any_element(),
                        })
                    })
                    // Test connection button
                    .child(
                        Button::new("test_conn", "Test Connection")
                            .full_width()
                            .style(ButtonStyle::Subtle)
                            .disabled(matches!(form.test_status, Some(TestStatus::Testing)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.test_form_connection(window, cx);
                            })),
                    )
                    // Save / Cancel buttons
                    .child(
                        div().flex().gap_2().pt_1().w_full()
                            .child(div().flex_1().child(
                                Button::new("cancel", "Cancel")
                                    .full_width()
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|this, _, _w, cx| this.cancel_form(cx))),
                            ))
                            .child(div().flex_1().child(
                                Button::new("save", "Save")
                                    .full_width()
                                    .style(ButtonStyle::Filled)
                                    .on_click(cx.listener(|this, _, _w, cx| this.save_form(cx))),
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_sqlite_path_field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ref form) = self.conn_form else {
            return div().into_any_element();
        };

        div()
            .flex()
            .flex_col()
            .gap_px()
            .w_full()
            .child(Label::new("Database File").size(LabelSize::XSmall).color(Color::Muted))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .w_full()
                    .child(
                        div()
                            .flex_1()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .bg(cx.theme().colors().editor_background)
                            .child(form.database_editor.clone()),
                    )
                    .child(
                        Button::new("browse_db", "Browse")
                            .style(ButtonStyle::Subtle)
                            .label_size(LabelSize::XSmall)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.browse_sqlite_file(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn browse_sqlite_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                if let Some(path) = paths.first() {
                    let path_str = path.to_string_lossy().to_string();
                    let _ = this.update_in(cx, |this, window, cx| {
                        if let Some(ref form) = this.conn_form {
                            form.database_editor.update(cx, |editor, cx| {
                                editor.set_text(path_str, window, cx);
                            });
                        }
                    });
                }
            }
        })
        .detach();
    }

    fn render_field(
        &self,
        label: &'static str,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_px()
            .w_full()
            .child(Label::new(label).size(LabelSize::XSmall).color(Color::Muted))
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
    }
}


// ── Trait implementations ───────────────────────────────────────────

impl EventEmitter<PanelEvent> for DatabasePanel {}

impl Focusable for DatabasePanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for DatabasePanel {
    fn persistent_name() -> &'static str { "DatabasePanel" }
    fn panel_key() -> &'static str { "database_panel" }

    fn position(&self, _w: &Window, _cx: &App) -> DockPosition {
        DatabasePanelSettings::dock()
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right | DockPosition::Bottom)
    }

    fn set_position(&mut self, _p: DockPosition, _w: &mut Window, _cx: &mut Context<Self>) {}

    fn default_size(&self, _w: &Window, _cx: &App) -> Pixels {
        DatabasePanelSettings::default_width()
    }

    fn icon(&self, _w: &Window, _cx: &App) -> Option<ui::IconName> {
        Some(ui::IconName::DatabaseZap)
    }

    fn icon_tooltip(&self, _w: &Window, _cx: &App) -> Option<&'static str> {
        Some("Database Panel")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 { 3 }
}

impl Render for DatabasePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("DatabasePanel")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::new_connection_action))
            .on_action(cx.listener(Self::run_query_from_editor))
            .on_action(cx.listener(Self::explain_query_from_editor))
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().colors().panel_background)
            .child(self.render_toolbar(cx))
            .child(self.render_search_bar(cx))
            .child(self.render_tree(cx))
            .children(self.context_menu.as_ref().map(|(menu, position, _)| {
                deferred(
                    anchored()
                        .position(*position)
                        .anchor(Corner::TopLeft)
                        .child(menu.clone()),
                )
                .with_priority(1)
            }))
    }
}
