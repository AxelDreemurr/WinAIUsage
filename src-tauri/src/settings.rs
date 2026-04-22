use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri_plugin_autostart::ManagerExt;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppSettings {
    pub enable_claude: bool,
    pub enable_codex: bool,
    pub enable_antigravity: bool,
    pub enable_notifications: bool,
    pub open_on_startup: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            enable_claude: true,
            enable_codex: true,
            enable_antigravity: true,
            enable_notifications: true,
            open_on_startup: false,
        }
    }
}

pub fn get_settings_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(appdata).join("WinAIUsage");
    if !dir.exists() {
        let _ = fs::create_dir_all(&dir);
    }
    dir.join("settings.json")
}

pub fn read_settings() -> AppSettings {
    let path = get_settings_path();
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(settings) = serde_json::from_str(&content) {
            return settings;
        }
    }
    AppSettings::default()
}

pub fn write_settings(settings: &AppSettings) {
    let path = get_settings_path();
    if let Ok(content) = serde_json::to_string_pretty(settings) {
        let _ = fs::write(&path, content);
    }
}

#[tauri::command]
pub fn get_settings() -> AppSettings {
    read_settings()
}

#[tauri::command]
pub fn save_settings(app_handle: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    write_settings(&settings);
    
    // Manage autostart based on settings.open_on_startup
    let autostart_manager = app_handle.autolaunch();
    if settings.open_on_startup {
        let _ = autostart_manager.enable();
    } else {
        let _ = autostart_manager.disable();
    }

    Ok(())
}
