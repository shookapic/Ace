mod auth;
mod browser_cookies;
mod chat;
mod history;
mod screenshot;
mod window;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{command, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const MODS: Modifiers = Modifiers::CONTROL.union(Modifiers::SHIFT);

// Action names shared with the frontend. Each maps to a user-rebindable global
// shortcut; the defaults are Ctrl+Shift+{A,X,S}.
const ACTION_TOGGLE: &str = "toggle";
const ACTION_CLICK_THROUGH: &str = "click_through";
const ACTION_SCREENSHOT: &str = "screenshot";

fn default_shortcuts() -> HashMap<String, Shortcut> {
    let mut m = HashMap::new();
    m.insert(ACTION_TOGGLE.to_string(), Shortcut::new(Some(MODS), Code::KeyA));
    m.insert(ACTION_CLICK_THROUGH.to_string(), Shortcut::new(Some(MODS), Code::KeyX));
    m.insert(ACTION_SCREENSHOT.to_string(), Shortcut::new(Some(MODS), Code::KeyS));
    m
}

/// Maps each action to its currently-bound global shortcut.
struct ShortcutBindings(Mutex<HashMap<String, Shortcut>>);

impl Default for ShortcutBindings {
    fn default() -> Self {
        ShortcutBindings(Mutex::new(default_shortcuts()))
    }
}

/// Clear and re-register every bound shortcut. Duplicate/among-conflicting binds
/// just get skipped with a log rather than aborting the whole set.
fn reregister_shortcuts(app: &tauri::AppHandle, map: &HashMap<String, Shortcut>) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    for (action, sc) in map {
        // On Windows the plugin's registry can lag `unregister_all`, so a combo
        // still reads as registered and `register` errors with "already
        // registered". Clear this specific combo first to keep it idempotent.
        if gs.is_registered(*sc) {
            let _ = gs.unregister(*sc);
        }
        if let Err(e) = gs.register(*sc) {
            eprintln!("could not register shortcut for {action}: {e}");
        }
    }
}

fn dispatch_shortcut(app: &tauri::AppHandle, action: &str) {
    match action {
        ACTION_TOGGLE => toggle_main_window(app),
        ACTION_CLICK_THROUGH => toggle_click_through(app),
        ACTION_SCREENSHOT => {
            let _ = app.emit("shortcut://screenshot", ());
        }
        _ => {}
    }
}

#[derive(Default)]
struct ClickThroughState(Mutex<bool>);

fn set_click_through_internal(app: &tauri::AppHandle, enabled: bool) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_ignore_cursor_events(enabled);
    }
    if let Ok(mut guard) = app.state::<ClickThroughState>().0.lock() {
        *guard = enabled;
    }
    // Keep the frontend toggle in sync when tray/shortcut flips this.
    let _ = app.emit("window://click-through", enabled);
}

fn toggle_click_through(app: &tauri::AppHandle) {
    let current = app
        .state::<ClickThroughState>()
        .0
        .lock()
        .map(|g| *g)
        .unwrap_or(false);
    set_click_through_internal(app, !current);
}

#[command]
fn set_click_through(app: tauri::AppHandle, enabled: bool) {
    set_click_through_internal(&app, enabled);
}

fn show_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let _ = window.set_skip_taskbar(false);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

fn hide_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let _ = window.hide();
    // Park it in the tray only: drop the taskbar button while hidden.
    let _ = window.set_skip_taskbar(true);
}

fn toggle_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    match window.is_visible() {
        Ok(true) => hide_main_window(app),
        _ => show_main_window(app),
    }
}

/// Rebind one action's global shortcut. Fails only if the combo is invalid or
/// the OS refuses it (e.g. another app already owns it), so the UI can report it.
#[command]
fn set_action_shortcut(
    app: tauri::AppHandle,
    action: String,
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
    code: String,
) -> Result<(), String> {
    let key = Code::from_str(&code).map_err(|_| format!("unrecognized key code: {code}"))?;

    let mut mods = Modifiers::empty();
    if ctrl {
        mods |= Modifiers::CONTROL;
    }
    if shift {
        mods |= Modifiers::SHIFT;
    }
    if alt {
        mods |= Modifiers::ALT;
    }
    if meta {
        mods |= Modifiers::META;
    }

    let shortcut = Shortcut::new(if mods.is_empty() { None } else { Some(mods) }, key);

    let state = app.state::<ShortcutBindings>();
    let mut map = state.0.lock().map_err(|_| "shortcut lock poisoned".to_string())?;

    // Validate the new combo in isolation first so we can surface a clear error
    // (e.g. taken by another app) without disturbing the other bindings.
    let _ = app.global_shortcut().unregister(shortcut);
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| format!("that combo is unavailable (maybe another app uses it): {e}"))?;

    map.insert(action, shortcut);
    reregister_shortcuts(&app, &map);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    // Find which action this shortcut is bound to and dispatch it.
                    let action = {
                        let state = app.state::<ShortcutBindings>();
                        let Ok(map) = state.0.lock() else { return };
                        map.iter()
                            .find(|(_, sc)| *sc == shortcut)
                            .map(|(a, _)| a.clone())
                    };
                    if let Some(action) = action {
                        dispatch_shortcut(app, &action);
                    }
                })
                .build(),
        )
        .manage(ShortcutBindings::default())
        .manage(ClickThroughState::default())
        .manage(chat::ClaudeWebSession::default())
        .manage(chat::ChatGptWebSession::default())
        .manage(chat::OpenAiRelay::default())
        .manage(chat::PreviewFile::default())
        .manage(chat::ChatCancels::default())
        .invoke_handler(tauri::generate_handler![
            auth::start_oauth_login,
            auth::get_auth_status,
            auth::sign_out,
            chat::send_chat_message,
            chat::send_claude_web_message,
            chat::send_openai_web_message,
            chat::cancel_chat_message,
            chat::list_models,
            chat::pick_files,
            chat::save_file,
            chat::open_html_in_browser,
            chat::open_file_preview,
            chat::get_preview_file,
            chat::transcribe_audio,
            chat::list_conversations,
            chat::get_conversation,
            chat::open_claude_login,
            chat::open_chatgpt_login,
            chat::openai_webview_send,
            window::effects::set_window_opacity,
            window::effects::set_capture_hidden,
            set_action_shortcut,
            set_click_through,
            screenshot::capture_screen,
            history::load_conversations,
            history::save_conversations,
        ])
        .setup(|app| {
            let main_window = app.get_webview_window("main").expect("main window missing");
            // Default to capture-hidden on launch so Ace doesn't leak into
            // screenshots/screen-shares by accident.
            let _ = window::effects::set_capture_hidden(main_window.clone(), true);
            // Give the live window a high-res (256px) icon so Windows downscales
            // it crisply for the taskbar at fractional display scaling (e.g. 125%)
            // instead of upscaling a small 32px icon and blurring it.
            if let Ok(icon) = tauri::image::Image::from_bytes(include_bytes!("../icons/128x128@2x.png")) {
                let _ = main_window.set_icon(icon);
            }
            // Default to fully opaque on launch.
            let _ = window::effects::set_window_opacity(main_window, 1.0);

            // Register the default binds; the frontend re-applies any custom ones
            // it has saved once it mounts.
            if let Ok(map) = app.state::<ShortcutBindings>().0.lock() {
                reregister_shortcuts(app.handle(), &map);
            }

            // System-tray icon: left-click toggles the window; the menu offers an
            // explicit Show and Quit. Hiding parks Ace here instead of the taskbar.
            let show_item = MenuItem::with_id(app, "show", "Show Ace", true, None::<&str>)?;
            let click_through_item = MenuItem::with_id(
                app,
                "toggle_click_through",
                "Toggle click-through",
                true,
                None::<&str>,
            )?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &click_through_item, &quit_item])?;

            let mut tray = TrayIconBuilder::new()
                .tooltip("Ace")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "toggle_click_through" => toggle_click_through(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
