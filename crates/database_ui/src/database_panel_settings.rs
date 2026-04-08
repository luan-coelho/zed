use workspace::dock::DockPosition;

/// Temporary hardcoded settings for the database panel.
/// TODO: Integrate with settings_content and RegisterSetting macro.
pub struct DatabasePanelSettings;

impl DatabasePanelSettings {
    pub fn dock() -> DockPosition {
        DockPosition::Right
    }

    pub fn default_width() -> gpui::Pixels {
        gpui::px(360.)
    }
}

pub fn init(_cx: &mut gpui::App) {
    // TODO: Register settings once integrated with settings_content
}
