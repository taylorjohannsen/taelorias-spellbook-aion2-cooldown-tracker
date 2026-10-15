#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod demo;
mod frames;
mod net;
mod npcap;
mod overlay;
mod packets;
mod skills;
mod tracker;
mod wire;

use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, PhysicalPosition, WebviewWindow};

use overlay::{Overlay, View};

const WINDOW: &str = "overlay";
const POSITION_FILE: &str = "position.json";

#[tauri::command]
fn snapshot(overlay: tauri::State<Overlay>) -> View {
    overlay.view()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let demo = args.iter().any(|a| a == "--demo");
    let trace = args.iter().any(|a| a == "--trace");
    let replay = args.iter().position(|a| a == "--replay").and_then(|i| args.get(i + 1)).cloned();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![snapshot])
        .setup(move |app| {
            let dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&dir)?;
            let overlay = Overlay::new(app.handle().clone(), dir.clone());
            app.manage(overlay.clone());

            let window = app.get_webview_window(WINDOW).expect("overlay window");
            restore_position(&window, &dir);
            window.set_ignore_cursor_events(true)?;
            tray(app, overlay.clone())?;

            if let Some(loadout) = skills::load_cached(&dir) {
                overlay.set_loadout(loadout);
            }
            if demo {
                std::thread::spawn(move || demo::run(overlay));
            } else {
                capture::start(overlay, replay, trace);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("couldn't start the overlay");
}

/// The overlay ignores the mouse so it never gets in the way of the game.
/// Unlocking it from the tray lets it be dragged somewhere else.
fn tray(app: &tauri::App, overlay: Overlay) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Unlock", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &quit])?;
    let mut builder = TrayIconBuilder::new().tooltip("AION2 Auras").menu(&menu);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "toggle" => {
                let Some(window) = app.get_webview_window(WINDOW) else { return };
                let unlocked = !overlay.view().unlocked;
                let _ = window.set_ignore_cursor_events(!unlocked);
                let _ = toggle.set_text(if unlocked { "Lock" } else { "Unlock" });
                if !unlocked {
                    save_position(&window, overlay.dir());
                }
                overlay.update(|v| v.unlocked = unlocked);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Position {
    x: i32,
    y: i32,
}

fn restore_position(window: &WebviewWindow, dir: &Path) {
    let saved = std::fs::read(dir.join(POSITION_FILE)).ok().and_then(|b| serde_json::from_slice::<Position>(&b).ok());
    if let Some(p) = saved {
        let _ = window.set_position(PhysicalPosition::new(p.x, p.y));
    }
}

fn save_position(window: &WebviewWindow, dir: &Path) {
    if let Ok(p) = window.outer_position() {
        if let Ok(json) = serde_json::to_vec(&Position { x: p.x, y: p.y }) {
            let _ = std::fs::write(dir.join(POSITION_FILE), json);
        }
    }
}
