//! Headless snapshots of the GUI: freya-testing renders the app to PNG
//! files in `<temp>/lunaris_freya_snapshots`, to look at the layout without
//! opening a window. The ones running a game need the ROMs of
//! `D:/GAME/Emulator/DS/Roms` (`cargo test -p lunaris_ds_freya -- --ignored`).
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ds_common::frontend::{Frontend, GuiConfig};
use ds_common::menu::{Action, Window};
use ds_common::{Instance, Settings};
use freya::prelude::Size2D;
use freya_testing::TestingRunner;

use crate::App;

const ROMS: &str = "D:/GAME/Emulator/DS/Roms";
const HG: &str = "ポケモンハートゴールド.nds";

fn out_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("lunaris_freya_snapshots");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// A frontend on a fresh instance folder with `recent` ROMs.
fn frontend(name: &str, recent: &[&str]) -> Arc<Frontend> {
    let base = std::env::temp_dir().join(format!("lunaris_freya_test_{name}"));
    let _ = std::fs::remove_dir_all(&base);
    let instance = Instance::open_in(&base);
    let config = GuiConfig {
        rom_dir: PathBuf::from(ROMS),
        recent_roms: recent.iter().map(|r| Path::new(ROMS).join(r)).collect(),
        ..GuiConfig::default()
    };
    let _ = std::fs::write(
        instance.config_path(),
        serde_json::to_string(&config).unwrap_or_default(),
    );
    let mut settings = Settings::default();
    settings.ensure_mac();
    Arc::new(Frontend::new(instance, settings))
}

fn snapshot(
    front: &Arc<Frontend>,
    dialog: Option<Window>,
    size: (f32, f32),
    file: &str,
    wait: Duration,
) -> TestingRunner {
    let app = App {
        front: Arc::clone(front),
        dialog,
    };
    let (mut runner, ()) =
        TestingRunner::new(move || app.clone(), Size2D::new(size.0, size.1), |_| {}, 1.);
    runner.poll(Duration::from_millis(16), wait);
    runner.render_to_file(out_dir().join(file));
    runner
}

#[test]
#[ignore = "writes PNG snapshots; reads ROM banners from disk"]
fn welcome() {
    let front = frontend("welcome", &[HG, "伝説のスタフィー4.nds"]);
    let mut runner = snapshot(
        &front,
        None,
        (900., 640.),
        "welcome.png",
        Duration::from_millis(400),
    );
    // The View menu open.
    runner.click_cursor((227., 22.));
    runner.poll(Duration::from_millis(16), Duration::from_millis(300));
    runner.render_to_file(out_dir().join("menu_view.png"));
    runner.move_cursor((260., 90.));
    runner.poll(Duration::from_millis(16), Duration::from_millis(300));
    runner.render_to_file(out_dir().join("menu_view_sub.png"));
}

#[test]
#[ignore = "writes PNG snapshots"]
fn dialogs() {
    let front = frontend("dialogs", &[]);
    for (window, name) in [
        (Window::EmuSettings, "emu_settings"),
        (Window::Input, "input"),
        (Window::Audio, "audio"),
        (Window::Wifi, "wifi"),
        (Window::Paths, "paths"),
        (Window::About, "about"),
    ] {
        snapshot(
            &front,
            Some(window),
            (900., 700.),
            &format!("dialog_{name}.png"),
            Duration::from_millis(400),
        );
    }
}

#[test]
#[ignore = "runs a ROM from disk and writes PNG snapshots"]
fn game_and_cheats() {
    let front = frontend("game", &[HG]);
    let cheats = ds_common::cheats::cheats_path(&front.instance, &Path::new(ROMS).join(HG));
    let _ = ds_common::cheats::save(
        &cheats,
        &[
            ds_common::cheats::Cheat {
                name: "所持金 999999".to_owned(),
                memo: "[Money]\nLoads the save first.".to_owned(),
                code: "94000130 FCFF0000\nB2111880 00000000\n00000090 000F423F\nD2000000 00000000"
                    .to_owned(),
                enabled: true,
                ..Default::default()
            },
            ds_common::cheats::Cheat {
                name: "Walk through walls (hold R)".to_owned(),
                code: "94000130 FEFF0000\n1202D5AC 00001C20\nD2000000 00000000".to_owned(),
                ..Default::default()
            },
        ],
    );
    assert!(
        front
            .apply(Action::OpenRomPath(Path::new(ROMS).join(HG)))
            .is_ok()
    );
    // HG's title screen (with 3D) is up after ~1300 frames.
    std::thread::sleep(Duration::from_secs(30));
    let mut runner = snapshot(
        &front,
        None,
        (760., 900.),
        "game.png",
        Duration::from_millis(600),
    );
    // Stylus: the centre of the bottom screen is DS pixel (128, 96).
    runner.press_cursor((380., 661.));
    runner.poll(Duration::from_millis(16), Duration::from_millis(100));
    let touch = front.shared.touch_pos();
    runner.release_cursor((380., 661.));
    runner.poll(Duration::from_millis(16), Duration::from_millis(100));
    let (x, y) = touch.unwrap_or_default();
    assert!(
        (120..=136).contains(&x) && (88..=104).contains(&y),
        "touch {touch:?}"
    );
    drop(runner);

    // Stop, start again: the screens come back with live frames.
    assert!(front.apply(Action::Stop).is_ok());
    let mut runner = snapshot(
        &front,
        None,
        (760., 900.),
        "stopped.png",
        Duration::from_millis(300),
    );
    assert!(
        front
            .apply(Action::OpenRomPath(Path::new(ROMS).join(HG)))
            .is_ok()
    );
    runner.poll(Duration::from_millis(16), Duration::from_secs(3));
    runner.render_to_file(out_dir().join("restarted.png"));
    drop(runner);
    snapshot(
        &front,
        Some(Window::Cheats),
        (1180., 820.),
        "dialog_cheats.png",
        Duration::from_millis(600),
    );
    snapshot(
        &front,
        Some(Window::RomInfo),
        (900., 820.),
        "dialog_rom_info.png",
        Duration::from_millis(600),
    );

    // Rotated side by side, 2x internal resolution with xBRZ.
    for action in [
        Action::SetRotation(90),
        Action::SetLayout(ds_common::view::ScreenLayout::Natural),
        Action::SetRenderScale(2),
        Action::ToggleXbrz,
    ] {
        assert!(front.apply(action).is_ok());
    }
    std::thread::sleep(Duration::from_millis(500));
    let mut runner = snapshot(
        &front,
        None,
        (1000., 640.),
        "game_rotated_2x.png",
        Duration::from_millis(600),
    );
    // The Appearance popover (titlebar button left of minimize).
    runner.click_cursor((812., 22.));
    runner.poll(Duration::from_millis(16), Duration::from_millis(300));
    runner.render_to_file(out_dir().join("appearance.png"));
    front.stop();
}
