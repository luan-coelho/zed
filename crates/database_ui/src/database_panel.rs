use crate::database_panel_settings::DatabasePanelSettings;
use anyhow::Result;
use database_core::{
    ConnectionManager, DatabaseConfig, DatabaseEntry, DatabaseSchema, ProviderCapabilities,
    RoleEntry, SchemaEntry,
};
use editor::Editor;
use gpui::{
    actions, anchored, deferred, div, Action, App, AsyncWindowContext, Context, Corner,
    Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Point, Render, Styled, Subscription,
    Task, WeakEntity, Window,
};
use gpui_tokio::Tokio;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use theme::ActiveTheme;
use tokio::sync::RwLock;
use ui::{prelude::*, ContextMenu};
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
pub(crate) enum ConnectionScope {
    Global,
    Project,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct ConnectionEntry {
    pub(crate) name: String,
    pub(crate) provider: String,
    pub(crate) host: String,
    pub(crate) port: String,
    pub(crate) database: String,
    pub(crate) status: ConnectionStatus,
    pub(crate) scope: ConnectionScope,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

pub(crate) struct ConnectionForm {
    pub(crate) name_editor: Entity<Editor>,
    pub(crate) host_editor: Entity<Editor>,
    pub(crate) port_editor: Entity<Editor>,
    pub(crate) database_editor: Entity<Editor>,
    pub(crate) user_editor: Entity<Editor>,
    pub(crate) password_editor: Entity<Editor>,
    pub(crate) provider: String,
    pub(crate) scope: ConnectionScope,
    pub(crate) editing: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) test_status: Option<TestStatus>,
}

impl ConnectionForm {
    pub(crate) fn editors(&self) -> Vec<&Entity<Editor>> {
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
pub(crate) enum TestStatus {
    Testing,
    Success,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub(crate) enum TransactionMode {
    #[default]
    Auto,
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub(crate) enum IsolationLevel {
    #[default]
    DatabaseDefault,
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl IsolationLevel {
    pub(crate) fn as_sql(&self) -> Option<&'static str> {
        match self {
            Self::DatabaseDefault => None,
            Self::ReadCommitted => Some("READ COMMITTED"),
            Self::RepeatableRead => Some("REPEATABLE READ"),
            Self::Serializable => Some("SERIALIZABLE"),
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::DatabaseDefault => "Database Default",
            Self::ReadCommitted => "Read Committed",
            Self::RepeatableRead => "Repeatable Read",
            Self::Serializable => "Serializable",
        }
    }
}

// ── Table Editor ───────────────────────────────────────────────────

pub(crate) struct TableEditor {
    #[allow(dead_code)]
    pub(crate) conn_name: String,
    #[allow(dead_code)]
    pub(crate) database: Option<String>,
    pub(crate) original_schema: String,
    pub(crate) original_name: String,
    pub(crate) original_columns: Vec<OriginalColumn>,
    pub(crate) original_indexes: Vec<OriginalIndex>,
    pub(crate) name_editor: Entity<Editor>,
    pub(crate) comment_editor: Entity<Editor>,
    pub(crate) columns: Vec<ColumnEditor>,
    pub(crate) indexes: Vec<IndexEditor>,
    pub(crate) error_message: Option<String>,
}

pub(crate) struct OriginalColumn {
    #[allow(dead_code)]
    pub(crate) name: String,
    pub(crate) data_type: String,
    pub(crate) is_nullable: bool,
    pub(crate) default_value: Option<String>,
    #[allow(dead_code)]
    pub(crate) is_primary_key: bool,
}

pub(crate) struct OriginalIndex {
    pub(crate) name: String,
    pub(crate) columns: Vec<String>,
    pub(crate) is_unique: bool,
}

pub(crate) struct ColumnEditor {
    pub(crate) original_name: Option<String>,
    pub(crate) name_editor: Entity<Editor>,
    pub(crate) type_editor: Entity<Editor>,
    pub(crate) nullable: bool,
    pub(crate) default_editor: Entity<Editor>,
    pub(crate) is_primary_key: bool,
    pub(crate) marked_for_deletion: bool,
}

pub(crate) struct IndexEditor {
    pub(crate) original_name: Option<String>,
    pub(crate) name_editor: Entity<Editor>,
    pub(crate) columns_editor: Entity<Editor>,
    pub(crate) is_unique: bool,
    pub(crate) marked_for_deletion: bool,
}

// ── Main struct ─────────────────────────────────────────────────────

pub struct DatabasePanel {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) _workspace: WeakEntity<Workspace>,
    pub(crate) connection_manager: Arc<RwLock<ConnectionManager>>,
    pub(crate) connections: Vec<ConnectionEntry>,
    pub(crate) schemas: HashMap<String, DatabaseSchema>,
    pub(crate) active_schema_for_completions: Arc<RwLock<Option<DatabaseSchema>>>,
    pub(crate) active_schema_name_for_completions: Arc<RwLock<Option<String>>>,
    pub(crate) expanded_nodes: HashSet<String>,
    pub(crate) selected_node: Option<String>,
    pub(crate) project_path: Option<String>,
    pub(crate) global_path: String,
    pub(crate) active_query_connection: Option<String>,
    pub(crate) active_query_database: Option<String>,
    pub(crate) active_schema: Option<String>,
    pub(crate) conn_form: Option<ConnectionForm>,
    pub(crate) context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    pub(crate) search_editor: Entity<Editor>,
    pub(crate) search_filter: String,
    pub(crate) _pending_task: Option<Task<()>>,
    pub(crate) database_lists: HashMap<String, Vec<DatabaseEntry>>,
    pub(crate) schema_lists: HashMap<String, Vec<SchemaEntry>>,
    pub(crate) database_schemas: HashMap<String, DatabaseSchema>,
    pub(crate) role_lists: HashMap<String, Vec<RoleEntry>>,
    pub(crate) connection_capabilities: HashMap<String, ProviderCapabilities>,
    pub(crate) loading_nodes: HashSet<String>,
    pub(crate) transaction_mode: TransactionMode,
    pub(crate) isolation_level: IsolationLevel,
    pub(crate) read_only: bool,
    pub(crate) in_transaction: bool,
    pub(crate) table_editor: Option<TableEditor>,
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
                schemas: HashMap::new(),
                active_schema_for_completions: Arc::new(RwLock::new(None)),
                active_schema_name_for_completions: Arc::new(RwLock::new(None)),
                expanded_nodes: HashSet::new(),
                selected_node: None,
                project_path,
                global_path,
                active_query_connection: None,
                active_query_database: None,
                active_schema: None,
                conn_form: None,
                context_menu: None,
                search_editor,
                search_filter: String::new(),
                _pending_task: None,
                database_lists: HashMap::new(),
                schema_lists: HashMap::new(),
                database_schemas: HashMap::new(),
                role_lists: HashMap::new(),
                connection_capabilities: HashMap::new(),
                loading_nodes: HashSet::new(),
                transaction_mode: TransactionMode::default(),
                isolation_level: IsolationLevel::default(),
                read_only: false,
                in_transaction: false,
                table_editor: None,
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
        let schema = self
            .active_schema
            .clone()
            .unwrap_or_else(|| "public".to_string());

        // Try flat schemas first (SQLite), then database_schemas (Postgres/MySQL)
        if let Some(db_schema) = self.schemas.get(conn_name) {
            return Some((db_schema.name.clone(), schema));
        }

        // For multi-database providers, find the first loaded database schema
        let prefix = format!("{conn_name}.");
        if let Some((key, db_schema)) = self
            .database_schemas
            .iter()
            .find(|(k, _)| k.starts_with(&prefix))
        {
            let db_name = key.strip_prefix(&prefix).unwrap_or(&db_schema.name);
            return Some((db_name.to_string(), schema));
        }

        // Fallback: use connection name
        Some((conn_name.clone(), schema))
    }

    /// Returns available schema names for the active connection.
    pub fn available_schemas(&self) -> Vec<String> {
        let Some(conn_name) = &self.active_query_connection else {
            return vec![];
        };

        // Try flat schemas first (SQLite)
        if let Some(db_schema) = self.schemas.get(conn_name) {
            let mut schemas: Vec<String> = db_schema
                .tables
                .iter()
                .map(|t| t.schema.clone())
                .chain(db_schema.views.iter().map(|v| v.schema.clone()))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            schemas.sort();
            return schemas;
        }

        // For multi-database providers, collect schemas from all loaded database_schemas
        let prefix = format!("{conn_name}.");
        let mut schemas: HashSet<String> = HashSet::new();
        for (key, db_schema) in &self.database_schemas {
            if key.starts_with(&prefix) {
                for table in &db_schema.tables {
                    schemas.insert(table.schema.clone());
                }
                for view in &db_schema.views {
                    schemas.insert(view.schema.clone());
                }
            }
        }
        let mut result: Vec<String> = schemas.into_iter().collect();
        result.sort();
        result
    }

    pub fn active_database_name(&self) -> Option<&str> {
        self.active_query_database.as_deref()
    }

    pub fn schemas_for_database(&self, database: &str) -> Vec<String> {
        let Some(conn_name) = &self.active_query_connection else {
            return vec![];
        };
        let key = format!("{conn_name}.{database}");
        let Some(db_schema) = self.database_schemas.get(&key) else {
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

    pub fn available_databases(&self) -> Vec<String> {
        let Some(conn_name) = &self.active_query_connection else {
            return vec![];
        };
        self.database_lists
            .get(conn_name)
            .map(|dbs| dbs.iter().map(|d| d.name.clone()).collect())
            .unwrap_or_default()
    }

    pub fn is_multi_database(&self) -> bool {
        let Some(conn_name) = &self.active_query_connection else {
            return false;
        };
        self.connection_capabilities
            .get(conn_name)
            .map(|c| c.multi_database)
            .unwrap_or(false)
    }

    pub fn set_active_database(&mut self, database: String, cx: &mut Context<Self>) {
        self.active_query_database = Some(database);
        self.active_schema = None;
        cx.notify();
    }

    pub fn active_schema_name(&self) -> Option<&str> {
        self.active_schema.as_deref()
    }

    pub(crate) fn set_transaction_mode(&mut self, mode: TransactionMode, cx: &mut Context<Self>) {
        self.transaction_mode = mode;
        cx.notify();
    }

    pub(crate) fn set_isolation_level(&mut self, level: IsolationLevel, cx: &mut Context<Self>) {
        self.isolation_level = level;
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        self.read_only = read_only;
        cx.notify();
    }

    pub fn is_in_transaction(&self) -> bool {
        self.in_transaction
    }

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

    pub(crate) fn load_connections(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn connect(&mut self, conn_name: &str, window: &mut Window, cx: &mut Context<Self>) {
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
            let name_for_spawn = name.clone();
            let db_result = Tokio::spawn_result(cx, async move {
                let mut mgr = conn_mgr_clone.write().await;
                mgr.test_connection().await?;

                let caps = mgr.capabilities_for(&name_for_spawn);

                if caps.multi_database {
                    let databases = mgr.list_databases().await.ok();
                    let roles = if caps.roles || caps.users {
                        mgr.list_roles().await.ok()
                    } else {
                        None
                    };
                    Ok((caps, None, databases, roles))
                } else {
                    let schema = mgr.get_schema().await.ok();
                    Ok((caps, schema, None, None))
                }
            })
            .await;

            match db_result {
                Ok((caps, schema, databases, roles)) => {
                    let _ = this.update_in(cx, |this, _window, cx| {
                        if let Some(e) = this.connections.iter_mut().find(|c| c.name == name) {
                            e.status = ConnectionStatus::Connected;
                        }
                        this.connection_capabilities.insert(name.clone(), caps.clone());
                        this.expanded_nodes.insert(format!("conn:{name}"));
                        this.active_query_connection = Some(name.clone());

                        if caps.multi_database {
                            if let Some(dbs) = databases {
                                this.database_lists.insert(name.clone(), dbs);
                            }
                            if let Some(roles) = roles {
                                this.role_lists.insert(name.clone(), roles);
                            }
                        } else {
                            if let Some(s) = schema {
                                this.schemas.insert(name.clone(), s);
                            }
                            // Set default active schema for non-multi-database providers
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
                            // Update schema for SQL completions
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

    pub(crate) fn disconnect(&mut self, conn_name: &str, cx: &mut Context<Self>) {
        if let Some(e) = self.connections.iter_mut().find(|c| c.name == conn_name) {
            e.status = ConnectionStatus::Disconnected;
        }
        self.schemas.remove(conn_name);
        self.database_lists.remove(conn_name);
        self.role_lists.remove(conn_name);
        self.connection_capabilities.remove(conn_name);
        // Remove all database_schemas entries for this connection
        self.database_schemas.retain(|key, _| !key.starts_with(&format!("{conn_name}.")));
        self.schema_lists.retain(|key, _| !key.starts_with(&format!("{conn_name}.")));
        self.loading_nodes.retain(|key| !key.contains(conn_name));
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

    pub(crate) fn set_active_for_queries(&mut self, conn_name: &str, cx: &mut Context<Self>) {
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

    pub(crate) fn toggle_node(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.expanded_nodes.contains(key) {
            self.expanded_nodes.remove(key);
        } else {
            self.expanded_nodes.insert(key.to_string());
        }
        self.selected_node = Some(key.to_string());
        cx.notify();
    }

    pub(crate) fn load_database_schema(
        &mut self,
        conn_name: &str,
        db_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detail_key = format!("{conn_name}.{db_name}");
        if self.database_schemas.contains_key(&detail_key) || self.loading_nodes.contains(&detail_key) {
            return;
        }

        self.loading_nodes.insert(detail_key.clone());
        cx.notify();

        let conn_manager = self.connection_manager.clone();
        let name = conn_name.to_string();
        let db = db_name.to_string();

        let task = cx.spawn_in(window, {
            let detail_key = detail_key.clone();
            async move |this, cx| {
                let conn_mgr = conn_manager.clone();
                let db_clone = db.clone();
                let name_clone = name.clone();
                let result = Tokio::spawn_result(cx, async move {
                    let mut mgr = conn_mgr.write().await;
                    mgr.set_active_connection(&name_clone);
                    mgr.get_schema_for_database(&db_clone).await
                })
                .await;

                let _ = this.update_in(cx, |this, _window, cx| {
                    this.loading_nodes.remove(&detail_key);
                    if let Ok(schema) = result {
                        // Update completions if this is the first loaded schema for this connection
                        let schema_arc = this.active_schema_for_completions.clone();
                        let name_arc = this.active_schema_name_for_completions.clone();
                        let schema_clone = schema.clone();
                        let active = this.active_schema.clone();
                        cx.background_executor()
                            .spawn(async move {
                                let mut s = schema_arc.write().await;
                                *s = Some(schema_clone);
                                let mut n = name_arc.write().await;
                                *n = active;
                            })
                            .detach();

                        // Set default active schema if not set
                        if this.active_schema.is_none() {
                            let schemas: Vec<String> = schema
                                .tables
                                .iter()
                                .map(|t| t.schema.clone())
                                .collect::<HashSet<_>>()
                                .into_iter()
                                .collect();
                            if schemas.contains(&"public".to_string()) {
                                this.active_schema = Some("public".to_string());
                            } else if let Some(first) = schemas.first() {
                                this.active_schema = Some(first.clone());
                            }
                        }

                        this.database_schemas.insert(detail_key, schema);
                    }
                    cx.notify();
                });
            }
        });
        self._pending_task = Some(task);
    }

    #[allow(dead_code)]
    fn select_node(&mut self, key: &str, cx: &mut Context<Self>) {
        self.selected_node = Some(key.to_string());
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn delete_connection(&mut self, conn_name: &str, cx: &mut Context<Self>) {
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

    fn close(&mut self, _: &Close, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PanelEvent::Close);
    }

    fn refresh(&mut self, _: &RefreshConnections, _window: &mut Window, cx: &mut Context<Self>) {
        self.load_connections(cx);
    }

    fn new_connection_action(&mut self, _: &NewConnection, window: &mut Window, cx: &mut Context<Self>) {
        self.show_connection_form(None, ConnectionScope::Global, window, cx);
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
        DatabasePanelSettings::global().dock
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right | DockPosition::Bottom)
    }

    fn set_position(&mut self, _p: DockPosition, _w: &mut Window, _cx: &mut Context<Self>) {}

    fn default_size(&self, _w: &Window, _cx: &App) -> Pixels {
        DatabasePanelSettings::global().default_width
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
