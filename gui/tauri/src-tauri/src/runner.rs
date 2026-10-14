use crate::commands::{self, AppState};
use ds_common::{Instance, Settings};
use tauri::Manager as _;

/// Data folder: `instances/instance1` next to the executable (not the
/// working directory: under `tauri dev` that is `src-tauri`, and writing
/// there would retrigger the file watcher).
fn open_instance() -> Instance {
    let base = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(ToOwned::to_owned))
        .unwrap_or_else(|| ".".into());
    Instance::open_in(&base)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::expect_used, clippy::large_stack_frames)]
pub(crate) fn run() {
    let instance = open_instance();
    let mut settings = Settings::load(&instance.settings_path());
    if settings.ensure_mac() {
        settings.save(&instance.settings_path());
    }

    if let Err(err) = ds_common::log::setup_logging_in(&instance.logs_dir()) {
        log::error!("Failed to start logging: {err}");
    }
    log::info!("Started (instance folder: {})", instance.root().display());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new(instance, settings))
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::set_config,
            commands::list_roms,
            commands::start_emulator,
            commands::stop_emulator,
            commands::is_running,
            commands::set_keys,
            commands::set_touch,
            commands::get_frame,
            commands::get_status,
            commands::get_menu,
            commands::menu_action,
            commands::import_savefile,
            commands::rom_info,
            commands::get_settings,
            commands::set_settings,
            commands::layout_screens,
        ])
        .on_window_event(|window, event| {
            // Write the save before the process exits.
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                window.state::<AppState>().stop();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
