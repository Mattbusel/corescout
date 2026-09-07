//! The desktop shell.
//!
//! # What this process is responsible for
//!
//! Three things, and deliberately nothing else: it keeps the service running,
//! it holds the loopback token so the web view never has to, and it puts an
//! icon in the tray. Every question about what CoreScout knows is forwarded.
//!
//! # Why the token stays here
//!
//! A web view that holds a credential is a web view that can leak one. The
//! window calls one command, this process adds the token, and the page has no
//! way to learn it even if something were injected into it.
//!
//! # Closing the window does not stop CoreScout
//!
//! The whole product is that the computer keeps learning while you work.
//! Closing the window hides it; the tray icon is how it comes back, and Quit
//! is how it stops.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, State, WindowEvent};

mod service;
mod startup;

/// Everything the shell holds on to.
struct Shell {
    service: Mutex<service::Service>,
}

/// Forward a call to the running service.
///
/// The one bridge between the window and CoreScout. The token is added here.
#[tauri::command]
async fn corescout_call(
    state: State<'_, Shell>,
    method: String,
    params: Value,
) -> Result<Value, String> {
    let client = {
        let mut service = state
            .service
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        service.client()?
    };
    client.call(&method, params).map_err(|error| error.to_string())
}

/// Whether CoreScout starts when this user signs in, and whether the
/// application is the thing that decides.
#[tauri::command]
fn launch_at_login() -> startup::Startup {
    startup::state()
}

/// Turn starting at sign-in on or off.
#[tauri::command]
fn set_launch_at_login(enabled: bool) -> Result<startup::Startup, String> {
    startup::set_enabled(enabled).map_err(|error| error.to_string())?;
    Ok(startup::state())
}

/// Open the folder CoreScout keeps its data in.
///
/// The Privacy page names the path; this is the button next to it. Showing
/// someone a path they then have to paste somewhere is a small unkindness.
#[tauri::command]
fn reveal_data_folder() -> Result<(), String> {
    let path = corescout_storage::paths::data_dir();
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(windows))]
    {
        let _ = &path;
    }
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(Shell {
            service: Mutex::new(service::Service::new()),
        })
        .invoke_handler(tauri::generate_handler![
            corescout_call,
            launch_at_login,
            set_launch_at_login,
            reveal_data_folder
        ])
        .setup(|app| {
            {
                let shell: State<'_, Shell> = app.state();
                let mut service = shell
                    .service
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                // A failure here is not fatal. The window opens, says the
                // service is not reachable, and offers to try again, which is
                // considerably more useful than a splash screen that vanishes.
                if let Err(error) = service.ensure_running() {
                    eprintln!("corescout: {error}");
                }
            }
            build_tray(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Hidden, not stopped. The product is a computer that keeps
                // learning while you work.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("CoreScout could not start");
}

/// The tray icon and its menu.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open CoreScout", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause observation", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume", "Resume", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit CoreScout", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &resume,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id("corescout")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("the application icon is missing".into())
        })?)
        .tooltip("CoreScout is learning")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show(app),
            "pause" => call_quietly(app, "pause"),
            "resume" => call_quietly(app, "resume"),
            "quit" => {
                // Stop the service as well. Leaving it behind is how a user
                // ends up with a process they cannot find and did not ask for.
                let shell: State<'_, Shell> = app.state();
                let mut service = shell
                    .service
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                service.stop();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                show(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Call a method from the tray, where there is nowhere to show a failure.
///
/// The window is told either way, so if it is open the user sees the result
/// on the Settings page rather than nothing happening.
fn call_quietly(app: &tauri::AppHandle, method: &str) {
    let shell: State<'_, Shell> = app.state();
    let client = {
        let mut service = shell
            .service
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        service.client()
    };
    if let Ok(client) = client {
        let _ = client.call(method, serde_json::json!({}));
    }
    let _ = app.emit("corescout://changed", method);
}
