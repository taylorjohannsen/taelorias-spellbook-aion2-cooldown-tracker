#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod capture;
mod demo;
mod focus;
mod frames;
mod net;
mod npcap;
mod overlay;
mod packets;
mod settings;
mod shell;
mod skills;
mod tracker;
mod wire;

use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri_plugin_updater::UpdaterExt;
use tauri::{AppHandle, Emitter, Listener, LogicalSize, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use overlay::{Overlay, View};
use settings::Settings;
use skills::Skill;

const WINDOW: &str = "overlay";
const BUFFS_WINDOW: &str = "buffs";
const SETTINGS_WINDOW: &str = "settings";
const LAUNCHER_WINDOW: &str = "launcher";
const POSITION_FILE: &str = "position.json";
const BUFFS_POSITION_FILE: &str = "buffs-position.json";
/// Until they're first moved, the bar and buffs window sit centred on the main
/// screen, the bar's top this far down it and the buffs just above the bar
const BAR_TOP: f64 = 0.64;
const BUFFS_ABOVE_BAR: f64 = 4.0;
/// The windows that sit over the game, ignore the mouse, and move when unlocked
const MOVABLE: [(&str, &str); 2] = [(WINDOW, POSITION_FILE), (BUFFS_WINDOW, BUFFS_POSITION_FILE)];
/// The settings button's window, which sits right in the corner of the screen
const LAUNCHER_SIZE: f64 = 34.0;
const NPCAP_DOWNLOAD: &str = "https://npcap.com/#download";

/// The tray's Unlock/Lock item, so the settings window can flip it too
struct LockItem(MenuItem<tauri::Wry>);

#[tauri::command]
fn snapshot(overlay: State<Overlay>) -> View {
    overlay.view()
}

#[tauri::command]
fn class_skills(overlay: State<Overlay>) -> Vec<Skill> {
    overlay.class_skills()
}

/// With the tracked buffs of the class that's playing
#[tauri::command]
fn settings(overlay: State<Overlay>, current: State<Mutex<Settings>>) -> Settings {
    current.lock().unwrap().for_class(overlay.class())
}

#[tauri::command]
fn default_settings() -> Settings {
    Settings::default()
}

/// The settings window sends all of them on every change, so the bar follows
/// along. Its tracked buffs are the playing class's; other classes' stay as they were.
#[tauri::command]
fn save_settings(app: AppHandle, overlay: State<Overlay>, current: State<Mutex<Settings>>, settings: Settings) {
    let class = overlay.class();
    let mut settings = settings.clamped();
    let mut stored = current.lock().unwrap();
    settings.class_buffs = std::mem::take(&mut stored.class_buffs);
    settings.keep_tracked(class);
    settings.save(overlay.dir());
    *stored = settings;
    let _ = app.emit("settings", stored.for_class(class));
}

/// An overlay page asks for exactly the room its contents take
#[tauri::command]
fn fit_window(window: WebviewWindow, width: f64, height: f64) {
    let _ = window.set_size(LogicalSize::new(width.max(1.0), height.max(1.0)));
    // still where they started, so they stay centred as they grow and shrink
    place_unmoved(window.app_handle());
}

#[tauri::command]
fn set_unlocked(app: AppHandle, unlocked: bool) {
    set_lock(&app, unlocked);
}

#[tauri::command]
fn autostart() -> bool {
    autostart::enabled()
}

/// Says whether it's on afterwards, which it isn't if Windows refused
#[tauri::command]
fn set_autostart(on: bool) -> bool {
    if let Err(e) = autostart::set(on) {
        eprintln!("couldn't change starting with Windows: {e}");
    }
    autostart::enabled()
}

/// The settings button lets clicks through while it's out of sight
#[tauri::command]
fn set_clickable(window: WebviewWindow, clickable: bool) {
    let _ = window.set_ignore_cursor_events(!clickable);
}

#[tauri::command]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
fn open_npcap_download() {
    shell::open(NPCAP_DOWNLOAD);
}

/// Where tjcd.log and the settings live, for bug reports
#[tauri::command]
fn open_log_folder(overlay: State<Overlay>) {
    shell::open(&overlay.dir().to_string_lossy());
}

/// Npcap is only looked for at start-up, so installing it takes a restart
#[tauri::command]
fn restart(app: AppHandle) {
    app.restart();
}

#[tauri::command]
fn show_settings(app: AppHandle) {
    open_settings(&app);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let demo = args.iter().any(|a| a == "--demo");
    let trace = args.iter().any(|a| a == "--trace");
    let replay = args.iter().position(|a| a == "--replay").and_then(|i| args.get(i + 1)).cloned();

    // release builds have no console to show why they stopped
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::write(std::env::temp_dir().join("tjcd-crash.log"), info.to_string());
        eprintln!("{info}");
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![snapshot, class_skills, settings, default_settings, save_settings, fit_window, set_unlocked, set_clickable, autostart, set_autostart, version, open_npcap_download, open_log_folder, restart, show_settings])
        .setup(move |app| {
            let dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&dir)?;
            // everything the pages ask for first, since the overlay's page is already loading
            let overlay = Overlay::new(app.handle().clone(), dir.clone());
            app.manage(overlay.clone());
            app.manage(Mutex::new(Settings::load(&dir)));
            if let Some(loadout) = skills::load_cached(&dir) {
                // started mid-session there's no loading screen to ask NC for the classes
                let server = loadout.server;
                overlay.set_loadout(loadout);
                if server != 0 {
                    let overlay = overlay.clone();
                    std::thread::spawn(move || overlay.refresh_classes(server));
                }
            }
            // other classes' skills name buffs, tracked ones included
            let handle = app.handle().clone();
            app.listen_any("names", move |_| name_tracked(&handle));
            if let Some((classes, _)) = skills::load_classes(&dir) {
                overlay.set_classes(classes);
            }

            let window = app.get_webview_window(WINDOW).expect("overlay window");
            // its page sizes it, and a single box is smaller than Windows allows by default
            window.set_min_size(Some(LogicalSize::new(1.0, 1.0)))?;
            restore_position(&window, &dir, POSITION_FILE);
            window.set_ignore_cursor_events(true)?;
            buffs_window(app, &dir)?;
            place_unmoved(app.handle());
            tray(app)?;
            launcher(app)?;

            update(app.handle());
            if demo {
                std::thread::spawn(move || demo::run(overlay));
            } else {
                focus::watch(overlay.clone());
                capture::start(overlay, replay, trace);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("couldn't start the overlay");
}

/// An installed copy updates itself from the latest GitHub release when it
/// starts: the new installer closes tjcd, updates it, and opens it again.
/// Builds from source are left alone.
fn update(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let overlay = app.state::<Overlay>();
        match install_update(&app).await {
            Ok(None) => overlay.log("up to date"),
            Ok(Some(version)) => overlay.log(format!("updated to {version}")),
            Err(e) => overlay.log(format!("couldn't check for updates: {e}")),
        }
    });
}

/// On Windows this doesn't come back once an update is installing
async fn install_update(app: &AppHandle) -> tauri_plugin_updater::Result<Option<String>> {
    let Some(update) = app.updater()?.check().await? else { return Ok(None) };
    app.state::<Overlay>().log(format!("updating to {}", update.version));
    update.download_and_install(|_, _| {}, || {}).await?;
    Ok(Some(update.version))
}

/// Buffs tracked before they could be named get their names now
fn name_tracked(app: &AppHandle) {
    let overlay = app.state::<Overlay>();
    let current = app.state::<Mutex<Settings>>();
    let mut settings = current.lock().unwrap();
    let mut changed = false;
    let unnamed = settings.class_buffs.values_mut().flatten().filter(|b| b.name.is_empty() || b.icon.is_empty());
    for buff in unnamed {
        let Some(found) = overlay.buff_info(buff.id) else { continue };
        if buff.name.is_empty() && !found.name.is_empty() {
            buff.name = found.name;
            changed = true;
        }
        if buff.icon.is_empty() && !found.icon.is_empty() {
            buff.icon = found.icon;
            changed = true;
        }
    }
    if changed {
        settings.save(overlay.dir());
        let _ = app.emit("settings", settings.for_class(overlay.class()));
    }
}

fn tray(app: &tauri::App) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Unlock", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &toggle, &quit])?;
    let mut builder = TrayIconBuilder::new().tooltip("tjcd").menu(&menu);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "settings" => open_settings(app),
            "toggle" => set_lock(app, !app.state::<Overlay>().view().unlocked),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    app.manage(LockItem(toggle));
    Ok(())
}

/// The overlays ignore the mouse so they never get in the way of the game.
/// Unlocking them, from the tray or the settings, lets them be dragged around.
fn set_lock(app: &AppHandle, unlocked: bool) {
    let overlay = app.state::<Overlay>();
    for (label, file) in MOVABLE {
        let Some(window) = app.get_webview_window(label) else { continue };
        let _ = window.set_ignore_cursor_events(!unlocked);
        if !unlocked {
            save_position(&window, overlay.dir(), file);
        }
    }
    if let Some(item) = app.try_state::<LockItem>() {
        let _ = item.0.set_text(if unlocked { "Lock" } else { "Unlock" });
    }
    overlay.update(|v| v.unlocked = unlocked);
}

/// Tracked buffs, in a window of their own so they can sit apart from the
/// bar. It starts out under the bar, whose page hasn't sized it yet.
fn buffs_window(app: &tauri::App, dir: &Path) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, BUFFS_WINDOW, WebviewUrl::App("buffs.html".into()))
        .title("tjcd buffs")
        .inner_size(1.0, 1.0)
        // its page sizes it, smaller than Windows allows by default
        .min_inner_size(1.0, 1.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    restore_position(&window, dir, BUFFS_POSITION_FILE);
    window.set_ignore_cursor_events(true)?;
    window.show()
}

/// Puts the bar and buffs window where they start out, if they've never been
/// moved: centred on the main screen, the buffs just above the bar. Unlocked,
/// they may be being dragged, and locking saves where they are.
fn place_unmoved(app: &AppHandle) {
    let overlay = app.state::<Overlay>();
    if overlay.view().unlocked {
        return;
    }
    let dir = overlay.dir().to_path_buf();
    let Some(bar) = app.get_webview_window(WINDOW) else { return };
    let (Ok(at), Ok(size), Ok(scale)) = (bar.outer_position(), bar.outer_size(), bar.scale_factor()) else { return };
    let (mut x, mut top) = (at.x, at.y);
    if !dir.join(POSITION_FILE).exists() {
        let Ok(Some(screen)) = app.primary_monitor() else { return };
        x = screen.position().x + (screen.size().width as i32 - size.width as i32) / 2;
        top = screen.position().y + (screen.size().height as f64 * BAR_TOP).round() as i32;
        let _ = bar.set_position(PhysicalPosition::new(x, top));
    }
    let Some(buffs) = app.get_webview_window(BUFFS_WINDOW) else { return };
    let Ok(buffs_size) = buffs.outer_size() else { return };
    if !dir.join(BUFFS_POSITION_FILE).exists() {
        let centre = x + size.width as i32 / 2;
        let gap = (BUFFS_ABOVE_BAR * scale).round() as i32;
        let _ = buffs.set_position(PhysicalPosition::new(centre - buffs_size.width as i32 / 2, top - gap - buffs_size.height as i32));
    }
}

/// A small button in the bottom left corner of the screen for the settings,
/// since the overlay itself never takes clicks. The whole screen rather than
/// the space above the taskbar, which the game covers anyway.
fn launcher(app: &tauri::App) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LAUNCHER_WINDOW, WebviewUrl::App("launcher.html".into()))
        .title("tjcd")
        .inner_size(LAUNCHER_SIZE, LAUNCHER_SIZE)
        // Windows makes new windows at least title-bar sized, and only listens to
        // a smaller minimum once the window exists, hence the resize below
        .min_inner_size(LAUNCHER_SIZE, LAUNCHER_SIZE)
        .max_inner_size(LAUNCHER_SIZE, LAUNCHER_SIZE)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    window.set_size(LogicalSize::new(LAUNCHER_SIZE, LAUNCHER_SIZE))?;
    if let Some(monitor) = app.primary_monitor()? {
        let size = window.outer_size()?;
        let x = monitor.position().x;
        let y = monitor.position().y + monitor.size().height as i32 - size.height as i32;
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    window.show()
}

/// Building a window on the thread that handles the menu deadlocks WebView2,
/// so the settings window gets built from another one. While it's open the bar
/// shows even if it's set to hide, so changes can be seen.
fn open_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let built = WebviewWindowBuilder::new(&app, SETTINGS_WINDOW, WebviewUrl::App("settings.html".into()))
            .title("tjcd settings")
            .inner_size(620.0, 900.0)
            .min_inner_size(520.0, 600.0)
            .build();
        let Ok(window) = built else { return };
        let overlay = app.state::<Overlay>().inner().clone();
        overlay.update(|v| v.editing = true);
        window.on_window_event(move |event| {
            if let WindowEvent::Destroyed = event {
                overlay.update(|v| v.editing = false);
            }
        });
    });
}

#[derive(Serialize, Deserialize)]
struct Position {
    x: i32,
    y: i32,
}

/// False when there's nothing saved yet
fn restore_position(window: &WebviewWindow, dir: &Path, file: &str) {
    let saved = std::fs::read(dir.join(file)).ok().and_then(|b| serde_json::from_slice::<Position>(&b).ok());
    if let Some(p) = saved {
        let _ = window.set_position(PhysicalPosition::new(p.x, p.y));
    }
}

fn save_position(window: &WebviewWindow, dir: &Path, file: &str) {
    if let Ok(p) = window.outer_position() {
        if let Ok(json) = serde_json::to_vec(&Position { x: p.x, y: p.y }) {
            let _ = std::fs::write(dir.join(file), json);
        }
    }
}
