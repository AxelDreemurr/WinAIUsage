mod providers;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

// ── Shared types ──────────────────────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
pub struct AllUsageData {
    pub claude_code: providers::claude_code::ClaudeCodeData,
    pub antigravity: providers::antigravity::AntigravityData,
}

impl AllUsageData {
    fn loading() -> Self {
        Self {
            claude_code: providers::claude_code::ClaudeCodeData::loading(),
            antigravity: providers::antigravity::AntigravityData::loading(),
        }
    }
}

pub struct UsageState(pub Arc<Mutex<Option<AllUsageData>>>);

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
fn hide_window(window: tauri::WebviewWindow) {
    let _ = window.hide();
}

#[tauri::command]
async fn get_all_usage_data(state: tauri::State<'_, UsageState>) -> Result<AllUsageData, ()> {
    Ok(state.0.lock().unwrap().clone().unwrap_or_else(AllUsageData::loading))
}

#[tauri::command]
async fn refresh_now(app_handle: tauri::AppHandle, state: tauri::State<'_, UsageState>) -> Result<(), ()> {
    let payload = fetch_all().await;
    {
        let mut g = state.0.lock().unwrap();
        *g = Some(payload.clone());
    }
    let _ = app_handle.emit("usage-updated", payload);
    Ok(())
}

// ── Fetch helper ──────────────────────────────────────────────────────────────

async fn fetch_all() -> AllUsageData {
    let (claude, antigravity) = tokio::join!(
        providers::claude_code::get_data(),
        providers::antigravity::get_data(),
    );
    AllUsageData { claude_code: claude, antigravity }
}

// ── Window positioning ────────────────────────────────────────────────────────

fn position_window_above_tray(window: &tauri::WebviewWindow) {
    if let Some(monitor) = window.primary_monitor().ok().flatten() {
        let wa = monitor.work_area();
        let win_size = window
            .outer_size()
            .unwrap_or(tauri::PhysicalSize::new(360, 400));
        let x = wa.position.x + wa.size.width as i32 - win_size.width as i32 - 12;
        let y = wa.position.y + wa.size.height as i32 - win_size.height as i32 - 8;
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

fn toggle_window(window: &tauri::WebviewWindow) {
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        position_window_above_tray(window);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// ── App entry point ───────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(UsageState(Arc::new(Mutex::new(None))))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![hide_window, get_all_usage_data, refresh_now])
        .setup(|app| {
            // ── Background polling ────────────────────────────────────────────
            let poll_app = app.handle().clone();
            let poll_state = app.state::<UsageState>().0.clone();

            tauri::async_runtime::spawn(async move {
                loop {
                    let payload = fetch_all().await;
                    {
                        let mut g = poll_state.lock().unwrap();
                        *g = Some(payload.clone());
                    }
                    let _ = poll_app.emit("usage-updated", payload);
                    tokio::time::sleep(Duration::from_secs(300)).await;
                }
            });

            // ── Tray setup ────────────────────────────────────────────────────
            let focus_lost_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
            let focus_lost_at_tray = focus_lost_at.clone();

            let show_item = MenuItem::with_id(app, "show", "Mostrar", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("WinAIUsage")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            toggle_window(&window);
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(move |tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let recently_hidden = focus_lost_at_tray
                            .lock()
                            .ok()
                            .and_then(|g| *g)
                            .map(|t| t.elapsed().as_millis() < 300)
                            .unwrap_or(false);

                        if !recently_hidden {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                toggle_window(&window);
                            }
                        }
                    }
                })
                .build(app)?;

            // ── Hide window on focus lost ──────────────────────────────────────
            let main_window = app.get_webview_window("main").unwrap();
            let win_clone = main_window.clone();
            main_window.on_window_event(move |event| {
                if let tauri::WindowEvent::Focused(false) = event {
                    *focus_lost_at.lock().unwrap() = Some(Instant::now());
                    let _ = win_clone.hide();
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
