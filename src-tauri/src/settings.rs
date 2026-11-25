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

// ── MSIX startup task ─────────────────────────────────────────────────────────
// Packaged (MSIX) builds can't autostart through the HKCU Run key: the write is
// virtualized into the package. They use the `windows.startupTask` extension
// declared in AppxManifest.xml instead, controlled via the StartupTask API.

/// Must match `<desktop:StartupTask TaskId="..."/>` in AppxManifest.xml.
#[cfg(windows)]
const STARTUP_TASK_ID: &str = "WinAIUsageStartup";

#[cfg(windows)]
mod startup_task {
    use windows::core::HSTRING;
    use windows::ApplicationModel::{StartupTask, StartupTaskState};

    fn task() -> windows::core::Result<StartupTask> {
        StartupTask::GetAsync(&HSTRING::from(super::STARTUP_TASK_ID))?.get()
    }

    fn is_on(state: StartupTaskState) -> bool {
        state == StartupTaskState::Enabled || state == StartupTaskState::EnabledByPolicy
    }

    pub fn is_enabled() -> Option<bool> {
        task().and_then(|t| t.State()).ok().map(is_on)
    }

    pub enum SetResult {
        Applied(bool),
        /// The user turned it off in Settings/Task Manager; only they can re-enable it.
        DisabledByUser,
        Failed,
    }

    pub fn set_enabled(enable: bool) -> SetResult {
        let Ok(task) = task() else {
            return SetResult::Failed;
        };
        if !enable {
            return match task.Disable().and_then(|_| task.State()) {
                Ok(state) => SetResult::Applied(is_on(state)),
                Err(_) => SetResult::Failed,
            };
        }
        match task.RequestEnableAsync().and_then(|op| op.get()) {
            Ok(StartupTaskState::DisabledByUser) => SetResult::DisabledByUser,
            Ok(state) => SetResult::Applied(is_on(state)),
            Err(_) => SetResult::Failed,
        }
    }
}

/// Settings as the UI should see them: when packaged, `open_on_startup`
/// mirrors the real startup task state (the user may change it from Windows).
fn effective_settings(mut settings: AppSettings) -> AppSettings {
    #[cfg(windows)]
    if crate::is_packaged() {
        if let Some(enabled) = startup_task::is_enabled() {
            settings.open_on_startup = enabled;
        }
    }
    settings
}

#[tauri::command(async)]
pub fn get_settings() -> AppSettings {
    effective_settings(read_settings())
}

/// Saves the settings and returns them as actually applied.
#[tauri::command(async)]
pub fn save_settings(app_handle: tauri::AppHandle, mut settings: AppSettings) -> Result<AppSettings, String> {
    #[cfg(windows)]
    if crate::is_packaged() {
        use startup_task::SetResult;
        match startup_task::set_enabled(settings.open_on_startup) {
            SetResult::Applied(enabled) => settings.open_on_startup = enabled,
            SetResult::DisabledByUser => {
                settings.open_on_startup = false;
                use tauri_plugin_opener::OpenerExt;
                let _ = app_handle
                    .opener()
                    .open_url("ms-settings:startupapps", None::<&str>);
            }
            SetResult::Failed => {}
        }
        write_settings(&settings);
        return Ok(settings);
    }

    // Unpackaged builds: HKCU Run key via tauri-plugin-autostart.
    let autostart_manager = app_handle.autolaunch();
    if settings.open_on_startup {
        let _ = autostart_manager.enable();
    } else {
        let _ = autostart_manager.disable();
    }
    write_settings(&settings);

    Ok(settings)
}
