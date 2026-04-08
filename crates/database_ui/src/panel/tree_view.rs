use crate::database_panel::{
    ConnectionEntry, ConnectionScope, ConnectionStatus, DatabasePanel,
    FocusNextField, FocusPrevField, TestStatus,
};
use database_core::{DatabaseEntry, DatabaseSchema, RoleEntry, Table, View};
use editor::Editor;
use gpui::{
    div, AnyElement, Context, Entity, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, Styled, Window,
};
use std::collections::HashSet;
use theme::ActiveTheme;
use ui::prelude::*;
use ui::Tooltip;

impl DatabasePanel {
    pub(crate) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .size(LabelSize::Default)
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

    pub(crate) fn render_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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

    pub(crate) fn render_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.conn_form.is_some() {
            return self.render_form(cx);
        }

        if self.table_editor.is_some() {
            return self.render_table_editor(cx);
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
                        .size(LabelSize::Default)
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
                        .size(LabelSize::Default)
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

    pub(crate) fn render_connection_node(&self, conn: &ConnectionEntry, cx: &mut Context<Self>) -> impl IntoElement {
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
                        .size(LabelSize::Default)
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
                                        this.open_query_console(&name, None, window, cx);
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
                        .size(LabelSize::Default)
                        .color(Color::Error),
                ),
            );
        }

        // Expanded: show tree content
        if is_expanded && is_connected {
            let caps = self.connection_capabilities.get(&conn.name);
            if caps.map(|c| c.multi_database).unwrap_or(false) {
                node = node.child(self.render_database_list(&conn.name, cx));
                if caps.map(|c| c.roles || c.users).unwrap_or(false) {
                    node = node.child(self.render_roles_section(&conn.name, caps.map(|c| c.users).unwrap_or(false), cx));
                }
            } else {
                // SQLite or providers without multi-database: flat tree
                if let Some(schema) = self.schemas.get(&conn.name) {
                    node = node.child(self.render_schema_tree(schema, &conn.name, cx));
                }
            }
        }

        // Connecting indicator
        if is_connecting {
            node = node.child(
                div().pl(px(28.)).child(
                    Label::new("Connecting...")
                        .size(LabelSize::Default)
                        .color(Color::Warning),
                ),
            );
        }

        node
    }

    pub(crate) fn render_schema_tree(
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

    pub(crate) fn render_database_list(&self, conn_name: &str, cx: &mut Context<Self>) -> AnyElement {
        let mut container = div().flex().flex_col().w_full();

        let Some(databases) = self.database_lists.get(conn_name) else {
            return container
                .child(
                    div().pl(px(28.)).child(
                        Label::new("Loading databases...")
                            .size(LabelSize::Default)
                            .color(Color::Muted),
                    ),
                )
                .into_any_element();
        };

        for db in databases {
            let db_key = format!("db:{conn_name}.{}", db.name);
            let is_expanded = self.expanded_nodes.contains(&db_key);
            container = container.child(self.render_database_node(conn_name, db, &db_key, is_expanded, cx));
        }

        container.into_any_element()
    }

    pub(crate) fn render_database_node(
        &self,
        conn_name: &str,
        db: &DatabaseEntry,
        key: &str,
        is_expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_selected = self.selected_node.as_ref().map(|s| s.as_str()) == Some(key);
        let icon_color = if db.is_current { Color::Success } else { Color::Accent };

        let mut node = div().flex().flex_col().w_full();

        // Database header row
        node = node.child(
            div()
                .id(ElementId::Name(key.to_string().into()))
                .flex()
                .items_center()
                .gap_1()
                .pl(px(24.))
                .pr_2()
                .py_px()
                .cursor_pointer()
                .when(is_selected, |d| {
                    d.bg(cx.theme().colors().ghost_element_selected)
                        .rounded_md()
                })
                .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_md())
                .on_click(cx.listener({
                    let key = key.to_string();
                    let conn = conn_name.to_string();
                    let db_name = db.name.clone();
                    move |this, _, window, cx| {
                        this.toggle_node(&key, cx);
                        if this.expanded_nodes.contains(&key) {
                            this.load_database_schema(&conn, &db_name, window, cx);
                        }
                    }
                }))
                .on_mouse_down(MouseButton::Right, cx.listener({
                    let conn = conn_name.to_string();
                    let db_name = db.name.clone();
                    move |this, event: &MouseDownEvent, window, cx| {
                        this.deploy_database_context_menu(
                            conn.clone(),
                            db_name.clone(),
                            event.position,
                            window,
                            cx,
                        );
                    }
                }))
                .child(
                    Icon::new(if is_expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
                )
                .child(Icon::new(IconName::DatabaseZap).size(IconSize::XSmall).color(icon_color))
                .child(
                    Label::new(db.name.clone())
                        .size(LabelSize::Default)
                        .weight(if db.is_current {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        }),
                ),
        );

        // Expanded content
        if is_expanded {
            let detail_key = format!("{conn_name}.{}", db.name);

            if self.loading_nodes.contains(&detail_key) {
                node = node.child(
                    div().pl(px(40.)).child(
                        Label::new("Loading schemas...")
                            .size(LabelSize::Default)
                            .color(Color::Muted),
                    ),
                );
            } else if let Some(schema) = self.database_schemas.get(&detail_key) {
                let caps = self.connection_capabilities.get(conn_name);
                if caps.map(|c| c.schemas).unwrap_or(false) {
                    // PostgreSQL: group tables by schema name
                    node = node.child(self.render_schema_groups(conn_name, &db.name, schema, cx));
                } else {
                    // MySQL: tables/views directly under database
                    node = node.child(self.render_schema_tree_at_indent(schema, conn_name, &db.name, 2, cx));
                }
            }
        }

        node.into_any_element()
    }

    pub(crate) fn render_schema_groups(
        &self,
        conn_name: &str,
        db_name: &str,
        schema: &DatabaseSchema,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut container = div().flex().flex_col().w_full();

        // Group tables and views by schema name
        let mut schema_names: Vec<String> = schema
            .tables
            .iter()
            .map(|t| t.schema.clone())
            .chain(schema.views.iter().map(|v| v.schema.clone()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        schema_names.sort();

        for schema_name in &schema_names {
            let schema_key = format!("schema:{conn_name}.{db_name}.{schema_name}");
            let is_expanded = self.expanded_nodes.contains(&schema_key);
            let is_selected = self.selected_node.as_ref().map(|s| s.as_str()) == Some(schema_key.as_str());

            // Schema header
            container = container.child(
                div()
                    .id(ElementId::Name(schema_key.clone().into()))
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl(px(40.))
                    .pr_2()
                    .py_px()
                    .cursor_pointer()
                    .when(is_selected, |d| {
                        d.bg(cx.theme().colors().ghost_element_selected)
                            .rounded_md()
                    })
                    .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_md())
                    .on_click(cx.listener({
                        let key = schema_key.clone();
                        move |this, _, _window, cx| {
                            this.toggle_node(&key, cx);
                        }
                    }))
                    .on_mouse_down(MouseButton::Right, cx.listener({
                        let conn = conn_name.to_string();
                        let schema = schema_name.clone();
                        move |this, event: &MouseDownEvent, window, cx| {
                            this.deploy_schema_context_menu(
                                conn.clone(),
                                schema.clone(),
                                event.position,
                                window,
                                cx,
                            );
                        }
                    }))
                    .child(
                        Icon::new(if is_expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(IconSize::XSmall)
                        .color(Color::Muted),
                    )
                    .child(Icon::new(IconName::Folder).size(IconSize::XSmall).color(Color::Muted))
                    .child(Label::new(schema_name.clone()).size(LabelSize::Default)),
            );

            if is_expanded {
                // Filter tables/views for this schema
                let filter = &self.search_filter;
                let schema_tables: Vec<&Table> = schema
                    .tables
                    .iter()
                    .filter(|t| t.schema == *schema_name)
                    .filter(|t| filter.is_empty() || t.name.to_lowercase().contains(filter))
                    .collect();
                let schema_views: Vec<&View> = schema
                    .views
                    .iter()
                    .filter(|v| v.schema == *schema_name)
                    .filter(|v| filter.is_empty() || v.name.to_lowercase().contains(filter))
                    .collect();

                // Tables section
                if !schema_tables.is_empty() {
                    let tables_key = format!("tables:{conn_name}.{db_name}.{schema_name}");
                    let tables_expanded = self.expanded_nodes.contains(&tables_key);
                    container = container.child(self.render_tree_section(
                        &tables_key,
                        &format!("Tables ({})", schema_tables.len()),
                        IconName::ListTree,
                        tables_expanded,
                        3,
                        cx,
                    ));
                    if tables_expanded {
                        for table in &schema_tables {
                            let tbl_key = format!(
                                "tbl:{conn_name}.{db_name}.{}.{}",
                                table.schema, table.name
                            );
                            let tbl_expanded = self.expanded_nodes.contains(&tbl_key);
                            container = container.child(self.render_table_node_at_indent(
                                table,
                                &tbl_key,
                                tbl_expanded,
                                conn_name,
                                4,
                                cx,
                            ));
                        }
                    }
                }

                // Views section
                if !schema_views.is_empty() {
                    let views_key = format!("views:{conn_name}.{db_name}.{schema_name}");
                    let views_expanded = self.expanded_nodes.contains(&views_key);
                    container = container.child(self.render_tree_section(
                        &views_key,
                        &format!("Views ({})", schema_views.len()),
                        IconName::Eye,
                        views_expanded,
                        3,
                        cx,
                    ));
                    if views_expanded {
                        for view in &schema_views {
                            let v_key = format!(
                                "view:{conn_name}.{db_name}.{}.{}",
                                view.schema, view.name
                            );
                            let v_expanded = self.expanded_nodes.contains(&v_key);
                            container = container.child(self.render_view_node_at_indent(
                                view, &v_key, v_expanded, 4, cx,
                            ));
                        }
                    }
                }
            }
        }

        container.into_any_element()
    }

    pub(crate) fn render_schema_tree_at_indent(
        &self,
        schema: &DatabaseSchema,
        conn_name: &str,
        db_name: &str,
        indent: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut tree = div().flex().flex_col().w_full();
        let filter = &self.search_filter;

        // Tables
        let filtered_tables: Vec<&Table> = schema
            .tables
            .iter()
            .filter(|t| filter.is_empty() || t.name.to_lowercase().contains(filter))
            .collect();

        if !filtered_tables.is_empty() {
            let tables_key = format!("tables:{conn_name}.{db_name}._");
            let tables_expanded = self.expanded_nodes.contains(&tables_key);
            tree = tree.child(self.render_tree_section(
                &tables_key,
                &format!("Tables ({})", filtered_tables.len()),
                IconName::ListTree,
                tables_expanded,
                indent,
                cx,
            ));
            if tables_expanded {
                for table in &filtered_tables {
                    let tbl_key = format!("tbl:{conn_name}.{db_name}.{}.{}", table.schema, table.name);
                    let tbl_expanded = self.expanded_nodes.contains(&tbl_key);
                    tree = tree.child(self.render_table_node_at_indent(
                        table,
                        &tbl_key,
                        tbl_expanded,
                        conn_name,
                        indent + 1,
                        cx,
                    ));
                }
            }
        }

        // Views
        let filtered_views: Vec<&View> = schema
            .views
            .iter()
            .filter(|v| filter.is_empty() || v.name.to_lowercase().contains(filter))
            .collect();

        if !filtered_views.is_empty() {
            let views_key = format!("views:{conn_name}.{db_name}._");
            let views_expanded = self.expanded_nodes.contains(&views_key);
            tree = tree.child(self.render_tree_section(
                &views_key,
                &format!("Views ({})", filtered_views.len()),
                IconName::Eye,
                views_expanded,
                indent,
                cx,
            ));
            if views_expanded {
                for view in &filtered_views {
                    let v_key = format!("view:{conn_name}.{db_name}.{}.{}", view.schema, view.name);
                    let v_expanded = self.expanded_nodes.contains(&v_key);
                    tree = tree.child(self.render_view_node_at_indent(
                        view, &v_key, v_expanded, indent + 1, cx,
                    ));
                }
            }
        }

        tree.into_any_element()
    }

    pub(crate) fn render_roles_section(&self, conn_name: &str, is_users: bool, cx: &mut Context<Self>) -> AnyElement {
        let roles_key = format!("roles:{conn_name}");
        let is_expanded = self.expanded_nodes.contains(&roles_key);
        let label_prefix = if is_users { "Users" } else { "Roles" };

        let mut container = div().flex().flex_col().w_full();

        let count = self.role_lists.get(conn_name).map(|r| r.len()).unwrap_or(0);
        container = container.child(self.render_tree_section(
            &roles_key,
            &format!("{label_prefix} ({count})"),
            IconName::Person,
            is_expanded,
            1,
            cx,
        ));

        if is_expanded {
            if let Some(roles) = self.role_lists.get(conn_name) {
                for role in roles {
                    container = container.child(self.render_role_node(conn_name, role, cx));
                }
            }
        }

        container.into_any_element()
    }

    pub(crate) fn render_role_node(
        &self,
        conn_name: &str,
        role: &RoleEntry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("role:{conn_name}.{}", role.name);
        let is_selected = self.selected_node.as_ref().map(|s| s.as_str()) == Some(key.as_str());

        let mut badges = Vec::new();
        if role.is_superuser {
            badges.push("SUPER");
        }
        if role.can_login {
            badges.push("LOGIN");
        }

        div()
            .id(ElementId::Name(key.clone().into()))
            .flex()
            .items_center()
            .gap_1()
            .pl(px(40.))
            .pr_2()
            .py_px()
            .cursor_pointer()
            .when(is_selected, |d| {
                d.bg(cx.theme().colors().ghost_element_selected)
                    .rounded_md()
            })
            .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_md())
            .on_click(cx.listener({
                let key = key.clone();
                move |this, _, _window, cx| {
                    this.selected_node = Some(key.clone());
                    cx.notify();
                }
            }))
            .child(Icon::new(IconName::Person).size(IconSize::XSmall).color(Color::Muted))
            .child(Label::new(role.name.clone()).size(LabelSize::Default))
            .when(!badges.is_empty(), |d| {
                d.child(
                    Label::new(badges.join(", "))
                        .size(LabelSize::Default)
                        .color(Color::Muted),
                )
            })
            .into_any_element()
    }

    pub(crate) fn render_tree_section(
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
            .child(Label::new(label.to_string()).size(LabelSize::Default))
    }

    pub(crate) fn render_table_node(
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
                .child(Label::new(table.name.clone()).size(LabelSize::Default))
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
                        this.query_table(&schema, &table_name, &cn, None, window, cx);
                    }))
                }),
        );

        // Table details (columns, keys, foreign keys, indexes)
        if is_expanded {
            node = node.child(self.render_table_details(table, key, 2, cx));
        }

        node
    }

    pub(crate) fn render_view_node(
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
                .child(Label::new(view.name.clone()).size(LabelSize::Default)),
        );

        if is_expanded {
            for col in &view.columns {
                node = node.child(self.render_column_row(col, 56));
            }
        }

        node
    }

    pub(crate) fn render_table_node_at_indent(
        &self,
        table: &Table,
        key: &str,
        is_expanded: bool,
        conn_name: &str,
        indent: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_selected = self.selected_node.as_ref().map(|s| s.as_str()) == Some(key);
        let pl = px((indent * 16 + 8) as f32);

        let mut node = div().flex().flex_col().w_full();

        node = node.child(
            div()
                .id(ElementId::Name(key.to_string().into()))
                .flex()
                .items_center()
                .gap_1()
                .pl(pl)
                .pr_2()
                .py_px()
                .cursor_pointer()
                .when(is_selected, |d| {
                    d.bg(cx.theme().colors().ghost_element_selected)
                        .rounded_md()
                })
                .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_md())
                .on_click(cx.listener({
                    let key = key.to_string();
                    move |this, _, _w, cx| {
                        this.toggle_node(&key, cx);
                    }
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener({
                        let s = table.schema.clone();
                        let t = table.name.clone();
                        let cn = conn_name.to_string();
                        move |this, event: &MouseDownEvent, window, cx| {
                            this.deploy_table_context_menu(
                                s.clone(),
                                t.clone(),
                                cn.clone(),
                                event.position,
                                window,
                                cx,
                            );
                        }
                    }),
                )
                .child(
                    Icon::new(if is_expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
                )
                .child(
                    Icon::new(IconName::FileTextOutlined)
                        .size(IconSize::XSmall)
                        .color(Color::Accent),
                )
                .child(Label::new(table.name.clone()).size(LabelSize::Default))
                .child(div().flex_1())
                .child({
                    let schema = table.schema.clone();
                    let table_name = table.name.clone();
                    let cn = conn_name.to_string();
                    // Extract database name from key: "tbl:{conn}.{db}.{schema}.{table}"
                    let db_from_key = key
                        .strip_prefix("tbl:")
                        .and_then(|rest| {
                            let parts: Vec<&str> = rest.splitn(4, '.').collect();
                            if parts.len() >= 2 { Some(parts[1].to_string()) } else { None }
                        });
                    IconButton::new(
                        ElementId::Name(format!("q-{key}").into()),
                        IconName::PlayFilled,
                    )
                    .icon_size(IconSize::XSmall)
                    .icon_color(Color::Muted)
                    .tooltip(|_w, cx| Tooltip::simple("SELECT * FROM ...", cx))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.query_table(&schema, &table_name, &cn, db_from_key.as_deref(), window, cx);
                    }))
                }),
        );

        if is_expanded {
            node = node.child(self.render_table_details(table, key, indent + 1, cx));
        }

        node.into_any_element()
    }

    pub(crate) fn render_view_node_at_indent(
        &self,
        view: &View,
        key: &str,
        is_expanded: bool,
        indent: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_selected = self.selected_node.as_ref().map(|s| s.as_str()) == Some(key);
        let pl = px((indent * 16 + 8) as f32);
        let col_indent = ((indent + 1) * 16 + 8) as i32;

        let mut node = div().flex().flex_col().w_full();

        node = node.child(
            div()
                .id(ElementId::Name(key.to_string().into()))
                .flex()
                .items_center()
                .gap_1()
                .pl(pl)
                .pr_2()
                .py_px()
                .cursor_pointer()
                .when(is_selected, |d| {
                    d.bg(cx.theme().colors().ghost_element_selected)
                        .rounded_md()
                })
                .hover(|d| d.bg(cx.theme().colors().ghost_element_hover).rounded_md())
                .on_click(cx.listener({
                    let key = key.to_string();
                    move |this, _, _w, cx| {
                        this.toggle_node(&key, cx);
                    }
                }))
                .child(
                    Icon::new(if is_expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
                )
                .child(Icon::new(IconName::Eye).size(IconSize::XSmall).color(Color::Warning))
                .child(Label::new(view.name.clone()).size(LabelSize::Default)),
        );

        if is_expanded {
            for col in &view.columns {
                node = node.child(self.render_column_row(col, col_indent));
            }
        }

        node.into_any_element()
    }

    pub(crate) fn render_table_details(
        &self,
        table: &Table,
        table_key: &str,
        indent: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut container = div().flex().flex_col().w_full();
        let child_indent = indent + 1;
        let leaf_indent_px = ((child_indent + 1) * 16 + 8) as i32;

        // Columns section
        let cols_key = format!("cols:{table_key}");
        let cols_expanded = self.expanded_nodes.contains(&cols_key);
        let col_count = table.columns.len();
        container = container.child(self.render_tree_section(
            &cols_key,
            &format!("columns {col_count}"),
            IconName::Folder,
            cols_expanded,
            indent,
            cx,
        ));
        if cols_expanded {
            for col in &table.columns {
                container = container.child(self.render_column_row(col, leaf_indent_px));
            }
        }

        // Keys section (primary keys)
        let pk_columns: Vec<&database_core::DbColumn> = table
            .columns
            .iter()
            .filter(|c| c.is_primary_key)
            .collect();
        let pk_count = pk_columns.len();
        let keys_key = format!("keys:{table_key}");
        let keys_expanded = self.expanded_nodes.contains(&keys_key);
        container = container.child(self.render_tree_section(
            &keys_key,
            &format!("keys {pk_count}"),
            IconName::Folder,
            keys_expanded,
            indent,
            cx,
        ));
        if keys_expanded {
            for col in &pk_columns {
                container = container.child(self.render_key_row(&col.name, "PK", leaf_indent_px));
            }
        }

        // Foreign keys section
        let fk_columns: Vec<&database_core::DbColumn> = table
            .columns
            .iter()
            .filter(|c| c.foreign_key.is_some())
            .collect();
        let fk_count = fk_columns.len();
        let fks_key = format!("fks:{table_key}");
        let fks_expanded = self.expanded_nodes.contains(&fks_key);
        container = container.child(self.render_tree_section(
            &fks_key,
            &format!("foreign keys {fk_count}"),
            IconName::Folder,
            fks_expanded,
            indent,
            cx,
        ));
        if fks_expanded {
            for col in &fk_columns {
                let fk_ref = col
                    .foreign_key
                    .as_ref()
                    .map(|fk| format!("{} → {}.{}", col.name, fk.referenced_table, fk.referenced_column))
                    .unwrap_or_else(|| col.name.clone());
                container = container.child(self.render_key_row(&fk_ref, "FK", leaf_indent_px));
            }
        }

        // Indexes section
        let idx_count = table.indexes.len();
        let idx_key = format!("idx:{table_key}");
        let idx_expanded = self.expanded_nodes.contains(&idx_key);
        container = container.child(self.render_tree_section(
            &idx_key,
            &format!("indexes {idx_count}"),
            IconName::Folder,
            idx_expanded,
            indent,
            cx,
        ));
        if idx_expanded {
            for index in &table.indexes {
                let label = format!(
                    "{}{}",
                    index.name,
                    if index.is_unique { " (UNIQUE)" } else { "" }
                );
                container = container.child(self.render_key_row(&label, "IDX", leaf_indent_px));
            }
        }

        container.into_any_element()
    }

    pub(crate) fn render_key_row(&self, label: &str, badge: &str, indent_px: i32) -> impl IntoElement {
        let badge_owned: SharedString = badge.to_string().into();
        div()
            .flex()
            .items_center()
            .gap_1()
            .pl(px(indent_px as f32))
            .pr_2()
            .py_px()
            .child(Icon::new(IconName::Dash).size(IconSize::XSmall).color(Color::Muted))
            .child(Label::new(label.to_string()).size(LabelSize::Default))
            .child(
                div().px_1().rounded_sm().bg(gpui::rgb(0x2a2a3a)).child(
                    Label::new(badge_owned).size(LabelSize::Default).color(Color::Accent),
                ),
            )
    }

    pub(crate) fn render_column_row(&self, col: &database_core::DbColumn, indent_px: i32) -> impl IntoElement {
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
            .child(Label::new(col.name.clone()).size(LabelSize::Default))
            .child(
                Label::new(col.data_type.clone())
                    .size(LabelSize::Default)
                    .color(Color::Muted),
            );

        for badge in badges {
            row = row.child(
                div().px_1().rounded_sm().bg(gpui::rgb(0x2a2a3a)).child(
                    Label::new(badge).size(LabelSize::Default).color(Color::Accent),
                ),
            );
        }

        row
    }

    pub(crate) fn render_empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        let registry = database_core::ProviderRegistry::new();
        let providers = registry.available_providers();

        let mut buttons = div().flex().flex_col().gap_1().w(px(200.));
        for (i, info) in providers.iter().enumerate() {
            let id = info.id;
            let style = if i == 0 {
                ButtonStyle::Filled
            } else {
                ButtonStyle::Subtle
            };
            buttons = buttons.child(
                Button::new(format!("add_{id}"), info.display_name)
                    .full_width()
                    .style(style)
                    .start_icon(Icon::new(IconName::DatabaseZap).size(IconSize::Small))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.show_connection_form_with_provider(
                            id,
                            ConnectionScope::Global,
                            window,
                            cx,
                        );
                    })),
            );
        }

        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .flex_1()
            .gap_3()
            .p_4()
            .child(Icon::new(IconName::DatabaseZap).size(IconSize::Medium).color(Color::Muted))
            .child(Label::new("No connections").size(LabelSize::Default).color(Color::Muted))
            .child(Label::new("Add a connection to get started").size(LabelSize::Default).color(Color::Muted))
            .child(buttons)
            .into_any_element()
    }

    pub(crate) fn render_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(ref form) = self.conn_form else {
            return div().into_any_element();
        };

        let registry = database_core::ProviderRegistry::new();
        let provider_info = registry.available_providers();
        let info = provider_info.iter().find(|p| p.id == form.provider);
        let is_file_based = info.map(|i| i.is_file_based).unwrap_or(false);
        let provider_display_name = info.map(|i| i.display_name).unwrap_or("Unknown");
        let db_label = info
            .map(|i| i.form_placeholders.database_label)
            .unwrap_or("Database");

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
                            .child(Label::new(title).size(LabelSize::Default).weight(FontWeight::SEMIBOLD)),
                    )
                    // Provider badge
                    .child(
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
                                Label::new(provider_display_name)
                                    .size(LabelSize::Default)
                                    .weight(FontWeight::SEMIBOLD),
                            )
                    )
                    .child(self.render_field("Name", &form.name_editor, cx))
                    .when(is_file_based, |d| {
                        d.child(self.render_sqlite_path_field(cx))
                    })
                    .when(!is_file_based, |d| {
                        d.child(
                            div().flex().gap_2().w_full()
                                .child(div().flex_1().child(self.render_field("Host", &form.host_editor, cx)))
                                .child(div().w(px(70.)).child(self.render_field("Port", &form.port_editor, cx))),
                        )
                        .child(self.render_field(db_label, &form.database_editor, cx))
                        .child(self.render_field("User", &form.user_editor, cx))
                        .child(self.render_field("Password", &form.password_editor, cx))
                    })
                    // Error / test status messages
                    .when_some(form.error_message.clone(), |d, msg| {
                        d.child(Label::new(msg).size(LabelSize::Default).color(Color::Error))
                    })
                    .when_some(form.test_status.clone(), |d, status| {
                        d.child(match status {
                            TestStatus::Testing => div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(Icon::new(IconName::ArrowCircle).size(IconSize::XSmall).color(Color::Warning))
                                .child(Label::new("Testing connection...").size(LabelSize::Default).color(Color::Warning))
                                .into_any_element(),
                            TestStatus::Success => div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(Icon::new(IconName::Check).size(IconSize::XSmall).color(Color::Success))
                                .child(Label::new("Connection successful!").size(LabelSize::Default).color(Color::Success))
                                .into_any_element(),
                            TestStatus::Failed(msg) => div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(Icon::new(IconName::Close).size(IconSize::XSmall).color(Color::Error))
                                .child(Label::new(msg).size(LabelSize::Default).color(Color::Error))
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

    pub(crate) fn render_sqlite_path_field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ref form) = self.conn_form else {
            return div().into_any_element();
        };

        div()
            .flex()
            .flex_col()
            .gap_px()
            .w_full()
            .child(Label::new("Database File").size(LabelSize::Default).color(Color::Muted))
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
                            .label_size(LabelSize::Small)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.browse_sqlite_file(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn browse_sqlite_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(crate) fn render_field(
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
            .child(Label::new(label).size(LabelSize::Default).color(Color::Muted))
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
