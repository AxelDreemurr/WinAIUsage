mod providers;
pub mod settings;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
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
    pub codex: providers::codex::CodexData,
}

impl AllUsageData {
    fn loading() -> Self {
        Self {
            claude_code: providers::claude_code::ClaudeCodeData::loading(),
            antigravity: providers::antigravity::AntigravityData::loading(),
            codex: providers::codex::CodexData::loading(),
        }
    }
}

pub struct UsageState(pub Arc<Mutex<Option<AllUsageData>>>);
pub struct AlertedSet(pub Arc<Mutex<HashSet<String>>>);

pub static LANG_ES: AtomicBool = AtomicBool::new(true);
pub static IS_PINNED: AtomicBool = AtomicBool::new(false);

pub fn is_es() -> bool {
    LANG_ES.load(Ordering::Relaxed)
}

pub fn t<'a>(es: &'a str, en: &'a str) -> &'a str {
    if is_es() { es } else { en }
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
async fn open_url(url: String) {
    let _ = open::that(url);
}

#[tauri::command]
fn get_lang() -> String {
    if is_es() { "es".to_string() } else { "en".to_string() }
}

#[tauri::command]
fn set_lang(lang: String) {
    LANG_ES.store(lang.starts_with("es"), Ordering::Relaxed);
}

#[tauri::command]
fn toggle_pin(window: tauri::WebviewWindow) -> bool {
    let new_val = !IS_PINNED.load(Ordering::Relaxed);
    IS_PINNED.store(new_val, Ordering::Relaxed);
    let _ = window.set_always_on_top(new_val);
    new_val
}

#[tauri::command]
fn hide_window(window: tauri::WebviewWindow) {
    let _ = window.hide();
}

#[tauri::command]
async fn get_all_usage_data(state: tauri::State<'_, UsageState>) -> Result<AllUsageData, ()> {
    Ok(state
        .0
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(AllUsageData::loading))
}

#[tauri::command]
async fn refresh_now(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, UsageState>,
) -> Result<(), ()> {
    let payload = fetch_all().await;
    {
        let mut g = state.0.lock().unwrap();
        *g = Some(payload.clone());
    }
    check_and_notify(&app_handle, &payload);
    let _ = app_handle.emit("usage-updated", payload);
    Ok(())
}

// ── Fetch helper ──────────────────────────────────────────────────────────────

async fn fetch_all() -> AllUsageData {
    let settings = settings::read_settings();

    let claude_fut = async {
        if settings.enable_claude {
            providers::claude_code::get_data().await
        } else {
            providers::claude_code::ClaudeCodeData::loading()
        }
    };

    let antigravity_fut = async {
        if settings.enable_antigravity {
            providers::antigravity::get_data().await
        } else {
            providers::antigravity::AntigravityData::loading()
        }
    };

    let codex_fut = async {
        if settings.enable_codex {
            providers::codex::get_data().await
        } else {
            providers::codex::CodexData::loading()
        }
    };

    let (claude, antigravity, codex) = tokio::join!(claude_fut, antigravity_fut, codex_fut);
    AllUsageData {
        claude_code: claude,
        antigravity,
        codex,
    }
}

// ── Notifications ─────────────────────────────────────────────────────────────

fn check_and_notify(app: &tauri::AppHandle, data: &AllUsageData) {
    let settings = settings::read_settings();
    if !settings.enable_notifications {
        return;
    }

    let alerted = app.state::<AlertedSet>().0.clone();
    let thresholds = [(80.0_f64, "80%"), (99.0_f64, "99%")];

    if let Some(p) = &data.claude_code.five_hour {
        fire_if_needed(app, &alerted, "claude:5h", p.utilization, &thresholds, t("Claude Code · Sesión (5h)", "Claude Code · Session (5h)"));
    }
    if let Some(p) = &data.claude_code.seven_day {
        fire_if_needed(app, &alerted, "claude:7d", p.utilization, &thresholds, t("Claude Code · Semana", "Claude Code · Weekly"));
    }
    for m in &data.antigravity.models {
        let key_prefix = format!("antigravity:{}", m.label);
        fire_if_needed(app, &alerted, &key_prefix, m.percent_used, &thresholds, &format!("Antigravity · {}", m.label));
    }
    for (i, p) in data.codex.periods.iter().enumerate() {
        let label = if p.label_key == "codexWeek" {
            t("Codex · Semanal", "Codex · Weekly")
        } else {
            t("Codex · Sesión", "Codex · Session")
        };
        fire_if_needed(app, &alerted, &format!("codex:{}", i), p.utilization, &thresholds, label);
    }
}

fn fire_if_needed(
    app: &tauri::AppHandle,
    alerted: &Arc<Mutex<HashSet<String>>>,
    key_prefix: &str,
    pct: f64,
    thresholds: &[(f64, &str)],
    label: &str,
) {
    use tauri_plugin_notification::NotificationExt;
    let mut set = alerted.lock().unwrap();
    for (threshold, threshold_label) in thresholds {
        if pct >= *threshold {
            let key = format!("{}:{}", key_prefix, threshold_label);
            if !set.contains(&key) {
                let body = if *threshold >= 99.0 {
                    t("Límite de uso alcanzado", "Usage limit reached").to_string()
                } else {
                    if is_es() {
                        format!("{:.0}% de cuota utilizada", pct)
                    } else {
                        format!("{:.0}% quota used", pct)
                    }
                };
                let _ = app.notification()
                    .builder()
                    .title(&format!("⚠️ {}", label))
                    .body(&body)
                    .sound("default")
                    .show();
                set.insert(key);
            }
        }
    }
}

// ── Window positioning ────────────────────────────────────────────────────────

fn position_window_above_tray(window: &tauri::WebviewWindow) {
    if let Some(monitor) = window.primary_monitor().ok().flatten() {
        let wa = monitor.work_area();
        let win_size = window
            .outer_size()
            .unwrap_or(tauri::PhysicalSize::new(380, 650));
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
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![])))
        .manage(UsageState(Arc::new(Mutex::new(None))))
        .manage(AlertedSet(Arc::new(Mutex::new(HashSet::new()))))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            hide_window,
            get_all_usage_data,
            refresh_now,
            open_url,
            get_lang,
            set_lang,
            toggle_pin,
            settings::get_settings,
            settings::save_settings
        ])
        .setup(|app| {
            let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_string());
            LANG_ES.store(locale.starts_with("es"), Ordering::Relaxed);

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
                    check_and_notify(&poll_app, &payload);
                    let _ = poll_app.emit("usage-updated", payload);
                    tokio::time::sleep(Duration::from_secs(300)).await;
                }
            });

            // ── Tray setup ────────────────────────────────────────────────────
            let focus_lost_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
            let focus_lost_at_tray = focus_lost_at.clone();

            let show_item = MenuItem::with_id(app, "show", t("Mostrar", "Show"), true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", t("Salir", "Quit"), true, None::<&str>)?;
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
                    if !IS_PINNED.load(Ordering::Relaxed) {
                        let _ = win_clone.hide();
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
