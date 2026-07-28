mod auth;
mod chat;
mod window;

use std::str::FromStr;
use std::sync::Mutex;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{command, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const DEFAULT_MODS: Modifiers = Modifiers::CONTROL.union(Modifiers::SHIFT);
const DEFAULT_CODE: Code = Code::KeyA;

#[derive(Default)]
struct ToggleShortcutState(Mutex<Option<Shortcut>>);

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

#[command]
fn set_toggle_shortcut(
    app: tauri::AppHandle,
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

    let state = app.state::<ToggleShortcutState>();
    let mut guard = state.0.lock().map_err(|_| "shortcut lock poisoned".to_string())?;

    app.global_shortcut()
        .unregister_all()
        .map_err(|e| format!("failed to clear previous shortcut: {e}"))?;
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| format!("failed to register shortcut: {e}"))?;

    *guard = Some(shortcut);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let state = app.state::<ToggleShortcutState>();
                    let Ok(guard) = state.0.lock() else { return };
                    if guard.as_ref() != Some(shortcut) {
                        return;
                    }
                    toggle_main_window(app);
                })
                .build(),
        )
        .manage(ToggleShortcutState::default())
        .manage(chat::ClaudeWebSession::default())
        .invoke_handler(tauri::generate_handler![
            auth::start_oauth_login,
            auth::get_auth_status,
            auth::sign_out,
            chat::send_chat_message,
            chat::list_models,
            chat::pick_files,
            chat::transcribe_audio,
            chat::list_conversations,
            chat::get_conversation,
            chat::open_claude_login,
            window::effects::set_window_opacity,
            window::effects::set_capture_hidden,
            set_toggle_shortcut,
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

            let default_shortcut = Shortcut::new(Some(DEFAULT_MODS), DEFAULT_CODE);
            app.global_shortcut().register(default_shortcut)?;
            *app.state::<ToggleShortcutState>().0.lock().unwrap() = Some(default_shortcut);

            // System-tray icon: left-click toggles the window; the menu offers an
            // explicit Show and Quit. Hiding parks Ace here instead of the taskbar.
            let show_item = MenuItem::with_id(app, "show", "Show Ace", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let mut tray = TrayIconBuilder::new()
                .tooltip("Ace")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
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
