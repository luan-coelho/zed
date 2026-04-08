use workspace::dock::DockPosition;

pub struct DatabasePanelSettings {
    pub dock: DockPosition,
    pub default_width: gpui::Pixels,
}

impl Default for DatabasePanelSettings {
    fn default() -> Self {
        Self {
            dock: DockPosition::Right,
            default_width: gpui::px(360.),
        }
    }
}

impl DatabasePanelSettings {
    pub fn global() -> Self {
        Self::default()
    }
}

pub fn init(_cx: &mut gpui::App) {}
