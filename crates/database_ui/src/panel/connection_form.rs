use crate::database_panel::*;
use database_core::{DatabaseConfig, ProviderRegistry};
use editor::Editor;
use gpui::{AppContext, Context, Focusable, Window};
use gpui_tokio::Tokio;

impl DatabasePanel {
    pub(crate) fn show_connection_form(
        &mut self,
        editing: Option<String>,
        scope: ConnectionScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default_provider = database_core::ProviderRegistry::new()
            .available_providers()
            .first()
            .map(|p| p.id)
            .unwrap_or("");
        self.show_connection_form_for_provider(editing, scope, default_provider, window, cx);
    }

    pub(crate) fn show_connection_form_for_provider(
        &mut self,
        editing: Option<String>,
        scope: ConnectionScope,
        default_provider: &str,
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
                (String::new(), String::new(), String::new(), String::new(), String::new(), default_provider.to_string())
            };

        let registry = ProviderRegistry::new();
        let provider_info = registry.available_providers();
        let info = provider_info.iter().find(|p| p.id == provider_val);
        let placeholders = info.map(|i| i.form_placeholders);

        let default_port = placeholders
            .as_ref()
            .map(|p| p.port)
            .unwrap_or("");
        let default_user = placeholders
            .as_ref()
            .map(|p| p.user)
            .unwrap_or("");
        let db_placeholder = placeholders
            .as_ref()
            .map(|p| p.database)
            .unwrap_or("");

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
            name_editor: make_editor(cx, window, "my_connection", &name_val),
            host_editor: make_editor(cx, window, "localhost", &host_val),
            port_editor: make_editor(cx, window, default_port, &port_val),
            database_editor: make_editor(cx, window, db_placeholder, &db_val),
            user_editor: make_editor(cx, window, default_user, &user_val),
            password_editor: cx.new(|cx| {
                let mut e = Editor::single_line(window, cx);
                e.set_placeholder_text("stored in OS keychain", window, cx);
                e.set_masked(true, cx);
                e
            }),
            provider: provider_val,
            scope,
            editing,
            error_message: None,
            test_status: None,
        });
        cx.notify();
    }

    pub(crate) fn test_form_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ref form) = self.conn_form else {
            return;
        };

        let provider = form.provider.clone();
        let host_raw = form.host_editor.read(cx).text(cx).trim().to_string();
        let port_raw = form.port_editor.read(cx).text(cx).trim().to_string();
        let database = form.database_editor.read(cx).text(cx).trim().to_string();
        let user_raw = form.user_editor.read(cx).text(cx).trim().to_string();
        let password = form.password_editor.read(cx).text(cx).trim().to_string();

        let registry = ProviderRegistry::new();
        let provider_info = registry.available_providers();
        let info = provider_info.iter().find(|p| p.id == provider);
        let default_port_str = info
            .map(|i| i.form_placeholders.port)
            .unwrap_or("");
        let default_user_str = info
            .map(|i| i.form_placeholders.user)
            .unwrap_or("");
        let is_file_based = info.map(|i| i.is_file_based).unwrap_or(false);

        let host = if host_raw.is_empty() { "localhost".to_string() } else { host_raw };
        let port = if port_raw.is_empty() { default_port_str.to_string() } else { port_raw };
        let user = if user_raw.is_empty() { default_user_str.to_string() } else { user_raw };

        if database.is_empty() && !is_file_based {
            // For non-file-based providers, database can be optional (e.g., MySQL lists all)
            // Only fail if provider is not multi-database capable
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

        let provider_box = database_core::providers::get_provider(&provider);
        let Some(provider_impl) = provider_box else {
            if let Some(ref mut f) = self.conn_form {
                f.test_status = Some(TestStatus::Failed("Unknown provider".into()));
            }
            cx.notify();
            return;
        };

        let url = match config.connection_url_with_metadata(
            if password.is_empty() { None } else { Some(&password) },
            provider_impl.metadata(),
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

    pub(crate) fn focus_next_form_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(crate) fn focus_prev_form_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(crate) fn cancel_form(&mut self, cx: &mut Context<Self>) {
        self.conn_form = None;
        cx.notify();
    }

    pub(crate) fn save_form(&mut self, cx: &mut Context<Self>) {
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
        let registry = ProviderRegistry::new();
        let provider_info = registry.available_providers();
        let info = provider_info.iter().find(|p| p.id == provider);
        let default_port_str = info
            .map(|i| i.form_placeholders.port)
            .unwrap_or("");
        let default_user_str = info
            .map(|i| i.form_placeholders.user)
            .unwrap_or("");

        let host = if host_raw.is_empty() { "localhost".to_string() } else { host_raw };
        let port = if port_raw.is_empty() { default_port_str.to_string() } else { port_raw };
        let user = if user_raw.is_empty() { default_user_str.to_string() } else { user_raw };

        if name.is_empty() {
            if let Some(ref mut f) = self.conn_form {
                f.error_message = Some("Connection name is required".into());
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
            let cred_user = if user.is_empty() { default_user_str.to_string() } else { user.clone() };
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
}
