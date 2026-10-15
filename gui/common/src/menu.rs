//! The menu bar, laid out like melonDS's (`frontend/qt_sdl/Window.cpp`),
//! defined once for both GUIs.
//!
//! ```text
//!  MenuState (what is loaded / paused / chosen)
//!    |  menu_bar()
//!    v
//!  Vec<Menu>  -- egui: walked with menu_button / checkbox / radio
//!             -- Tauri: returned by `get_menu`, drawn by React
//!    |  a click
//!    v
//!  Action     -- egui: App::apply ; Tauri: `menu_action` command
//! ```
//!
//! Entries melonDS has but this emulator cannot back are kept, **disabled**,
//! with the reason as their `reason` (shown on hover), so the menu keeps
//! melonDS's shape and what is missing is visible.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::speed::{self, SPEEDS};
use crate::view::{MAX_RENDER_SCALE, ROTATIONS, SCREEN_GAPS, ScreenLayout, ScreenSizing, View};

/// Remembered ROMs in File > Open recent.
pub const RECENT_LIMIT: usize = 10;

/// An auxiliary window (dialog).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Window {
    RomInfo,
    Multiplayer,
    EmuSettings,
    Input,
    Audio,
    Wifi,
    Paths,
    About,
    /// System > Setup cheat codes.
    Cheats,
}

/// What a menu entry asks for.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum Action {
    /// Pick a ROM with a file dialog.
    OpenRom,
    /// Run this ROM (Open recent, ROM folder).
    OpenRomPath(PathBuf),
    ClearRecent,
    /// Stop the game (the DS slot is emptied).
    Eject,
    /// Replace the game's save with a `.sav` file.
    ImportSavefile,
    /// Show the instance folder (saves, settings, logs).
    OpenDirectory,
    Quit,
    TogglePause,
    Reset,
    Stop,
    FrameStep,
    /// Start another emulator process (it takes the next instance folder).
    LaunchInstance,
    /// Resize the window so the screens are this many times native size.
    ScreenSize(u8),
    SetRotation(u16),
    SetGap(u16),
    SetLayout(ScreenLayout),
    ToggleSwap,
    SetSizing(ScreenSizing),
    ToggleIntegerScaling,
    ToggleFiltering,
    ToggleShowFps,
    ToggleLimitFramerate,
    /// Emulation speed in percent (one of `speed::SPEEDS`).
    SetSpeed(u16),
    /// The next speed (gamepad left stick button).
    CycleSpeed,
    /// Internal resolution factor (Config > Video settings).
    SetRenderScale(u8),
    ToggleXbrz,
    /// System > Enable cheats.
    ToggleCheats,
    Open(Window),
}

impl Action {
    /// The Ctrl+key shortcut for an action. Plain keys are left to the DS
    /// buttons (arrows, X, Z, A, S, Q, W, Enter, Backspace).
    pub const fn shortcut(&self) -> Option<&'static str> {
        match self {
            Self::OpenRom => Some("Ctrl+O"),
            Self::TogglePause => Some("Ctrl+P"),
            Self::Reset => Some("Ctrl+R"),
            Self::FrameStep => Some("Ctrl+N"),
            Self::Quit => Some("Ctrl+Q"),
            _ => None,
        }
    }

    /// The action of Ctrl+`key` (an ASCII letter, any case).
    pub const fn from_shortcut(key: char) -> Option<Self> {
        match key.to_ascii_uppercase() {
            'O' => Some(Self::OpenRom),
            'P' => Some(Self::TogglePause),
            'R' => Some(Self::Reset),
            'N' => Some(Self::FrameStep),
            'Q' => Some(Self::Quit),
            _ => None,
        }
    }
}

/// One menu entry.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Item {
    /// A clickable entry; `checked` makes it a checkbox (`radio`: a radio
    /// button). Disabled entries carry the `reason`.
    #[serde(rename_all = "camelCase")]
    Entry {
        label: String,
        action: Option<Action>,
        enabled: bool,
        checked: Option<bool>,
        radio: bool,
        reason: Option<String>,
        shortcut: Option<String>,
        /// Extra text on hover (e.g. a recent ROM's full path).
        hint: Option<String>,
    },
    Submenu {
        label: String,
        enabled: bool,
        items: Vec<Self>,
    },
    Separator,
    /// Plain text (e.g. "DS slot: <game>").
    Label {
        text: String,
    },
}

/// A top-level menu.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Menu {
    pub label: String,
    pub items: Vec<Item>,
}

/// Why an entry is disabled.
pub mod reason {
    pub const NOT_LOADED: &str = "No game is running";
    pub const FIRMWARE_BOOT: &str =
        "This emulator boots games directly; booting the firmware menu is not supported";
    pub const GBA_SLOT: &str = "The GBA slot is not emulated";
    pub const SAVESTATE: &str = "Savestates are not supported by this emulator yet";
    pub const RAM_SEARCH: &str = "RAM search is not implemented yet";
    pub const DSI: &str = "DSi mode is not emulated";
    pub const CAMERA: &str = "The DSi cameras are not emulated";
    pub const POWER: &str = "The battery and the power button are not emulated yet";
    pub const DATE_TIME: &str = "The DS clock follows this PC's date and time";
    pub const XBRZ_SCALE: &str = "xBRZ needs an internal resolution of 2x or more";
    pub const FIRMWARE_SETTINGS: &str =
        "Firmware user settings (nickname, birthday, ...) are not editable yet";
    pub const INTERFACE: &str = "No interface options yet";
    pub const AUDIO_SYNC: &str = "Audio follows the frame rate; audio sync is not implemented";
    pub const NEW_WINDOW: &str = "A second window is not implemented";
    pub const HYBRID: &str = "The hybrid layout is not implemented";
    pub const AUTO_SIZING: &str = "Automatic screen sizing is not implemented";
    pub const ASPECT: &str = "Only the DS's own 4:3 aspect ratio is supported";
}

/// What the menus depend on.
#[derive(Debug, Clone)]
pub struct MenuState<'a> {
    /// The running (or paused) game, if any.
    pub loaded: Option<&'a str>,
    pub paused: bool,
    pub view: &'a View,
    pub limit_framerate: bool,
    pub recent: &'a [PathBuf],
    /// `.nds` files of the configured ROM folder.
    pub folder_roms: &'a [PathBuf],
    /// `Err(reason)` when this GUI has no multiplayer.
    pub multiplayer: Result<(), &'a str>,
}

/// Builder helpers.
fn entry(label: &str, action: Action, enabled: bool) -> Item {
    let shortcut = action.shortcut().map(str::to_owned);
    Item::Entry {
        label: label.to_owned(),
        action: Some(action),
        enabled,
        checked: None,
        radio: false,
        reason: (!enabled).then(|| reason::NOT_LOADED.to_owned()),
        shortcut,
        hint: None,
    }
}

fn check(label: &str, action: Action, checked: bool, enabled: bool) -> Item {
    Item::Entry {
        label: label.to_owned(),
        shortcut: action.shortcut().map(str::to_owned),
        action: Some(action),
        enabled,
        checked: Some(checked),
        radio: false,
        reason: (!enabled).then(|| reason::NOT_LOADED.to_owned()),
        hint: None,
    }
}

/// A radio entry that is always the selected one (the only choice).
fn only_choice(label: &str) -> Item {
    Item::Entry {
        label: label.to_owned(),
        action: None,
        enabled: true,
        checked: Some(true),
        radio: true,
        reason: None,
        shortcut: None,
        hint: None,
    }
}

fn radio(label: &str, action: Action, checked: bool) -> Item {
    Item::Entry {
        label: label.to_owned(),
        action: Some(action),
        enabled: true,
        checked: Some(checked),
        radio: true,
        reason: None,
        shortcut: None,
        hint: None,
    }
}

/// An entry that cannot be used here, with the reason.
fn unavailable(label: &str, why: &str) -> Item {
    Item::Entry {
        label: label.to_owned(),
        action: None,
        enabled: false,
        checked: None,
        radio: false,
        reason: Some(why.to_owned()),
        shortcut: None,
        hint: None,
    }
}

fn submenu(label: &str, items: Vec<Item>) -> Item {
    Item::Submenu {
        label: label.to_owned(),
        enabled: true,
        items,
    }
}

const fn label(text: String) -> Item {
    Item::Label { text }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// A list of ROMs, each running its path.
fn rom_list(paths: &[PathBuf], numbered: bool) -> Vec<Item> {
    paths
        .iter()
        .enumerate()
        .map(|(i, path)| Item::Entry {
            label: if numbered {
                format!("{}.  {}", i + 1, file_name(path))
            } else {
                file_name(path)
            },
            action: Some(Action::OpenRomPath(path.clone())),
            enabled: true,
            checked: None,
            radio: false,
            reason: None,
            shortcut: None,
            hint: Some(path.display().to_string()),
        })
        .collect()
}

fn file_menu(s: &MenuState<'_>) -> Menu {
    let loaded = s.loaded.is_some();
    let mut recent = rom_list(&s.recent[..s.recent.len().min(RECENT_LIMIT)], true);
    if recent.is_empty() {
        recent.push(unavailable("(none yet)", "No ROM has been opened yet"));
    } else {
        recent.push(Item::Separator);
        recent.push(entry("Clear", Action::ClearRecent, true));
    }
    let mut folder = rom_list(s.folder_roms, false);
    if folder.is_empty() {
        folder.push(unavailable(
            "(no .nds files)",
            "Set the ROM folder in Config > Path settings",
        ));
    }
    let slots = |load: bool| -> Vec<Item> {
        let mut items: Vec<Item> = (1..=8)
            .map(|n| unavailable(&n.to_string(), reason::SAVESTATE))
            .collect();
        items.push(Item::Separator);
        items.push(unavailable(
            if load {
                "Load from file..."
            } else {
                "Save to file..."
            },
            reason::SAVESTATE,
        ));
        items
    };

    Menu {
        label: "File".to_owned(),
        items: vec![
            entry("Open ROM...", Action::OpenRom, true),
            submenu("Open recent", recent),
            submenu("ROM folder", folder),
            unavailable("Boot firmware", reason::FIRMWARE_BOOT),
            Item::Separator,
            label(format!("DS slot: {}", s.loaded.unwrap_or("(none)"))),
            entry("Insert cart...", Action::OpenRom, true),
            entry("Eject cart", Action::Eject, loaded),
            Item::Separator,
            label("GBA slot: (none)".to_owned()),
            unavailable("Insert ROM cart...", reason::GBA_SLOT),
            unavailable("Insert add-on cart", reason::GBA_SLOT),
            unavailable("Eject cart", reason::GBA_SLOT),
            Item::Separator,
            entry("Import savefile...", Action::ImportSavefile, loaded),
            Item::Separator,
            submenu("Save state", slots(false)),
            submenu("Load state", slots(true)),
            unavailable("Undo state load", reason::SAVESTATE),
            Item::Separator,
            entry("Open data directory", Action::OpenDirectory, true),
            Item::Separator,
            entry("Quit", Action::Quit, true),
        ],
    }
}

fn system_menu(s: &MenuState<'_>) -> Menu {
    let loaded = s.loaded.is_some();
    let multiplayer = match s.multiplayer {
        Ok(()) => vec![
            entry("Launch new instance", Action::LaunchInstance, true),
            Item::Separator,
            entry(
                "Host / join LAN game...",
                Action::Open(Window::Multiplayer),
                true,
            ),
        ],
        Err(why) => vec![
            entry("Launch new instance", Action::LaunchInstance, true),
            Item::Separator,
            unavailable("Host LAN game", why),
            unavailable("Join LAN game", why),
        ],
    };
    let pause = check("Pause", Action::TogglePause, s.paused, loaded);
    Menu {
        label: "System".to_owned(),
        items: vec![
            pause,
            entry("Reset", Action::Reset, loaded),
            entry("Stop", Action::Stop, loaded),
            entry("Frame step", Action::FrameStep, loaded),
            Item::Separator,
            unavailable("Power management", reason::POWER),
            unavailable("Date and time", reason::DATE_TIME),
            Item::Separator,
            check(
                "Enable cheats",
                Action::ToggleCheats,
                s.view.cheats_enabled,
                true,
            ),
            entry("Setup cheat codes", Action::Open(Window::Cheats), loaded),
            Item::Separator,
            entry("ROM info", Action::Open(Window::RomInfo), loaded),
            unavailable("RAM search", reason::RAM_SEARCH),
            unavailable("Manage DSi titles", reason::DSI),
            Item::Separator,
            submenu("Multiplayer", multiplayer),
        ],
    }
}

fn view_menu(s: &MenuState<'_>) -> Menu {
    let v = s.view;
    let sizes = (1..=4)
        .map(|n| entry(&format!("{n}x"), Action::ScreenSize(n), true))
        .collect();
    let rotations = ROTATIONS
        .iter()
        .map(|&r| radio(&format!("{r}°"), Action::SetRotation(r), v.rotation() == r))
        .collect();
    let gaps = SCREEN_GAPS
        .iter()
        .map(|&g| radio(&format!("{g} px"), Action::SetGap(g), v.gap == g))
        .collect();
    let mut layouts: Vec<Item> = ScreenLayout::ALL
        .iter()
        .map(|&l| radio(l.label(), Action::SetLayout(l), v.layout == l))
        .collect();
    layouts.push(unavailable("Hybrid", reason::HYBRID));
    layouts.push(Item::Separator);
    layouts.push(check("Swap screens", Action::ToggleSwap, v.swap, true));
    let mut sizing: Vec<Item> = ScreenSizing::ALL
        .iter()
        .map(|&z| radio(z.label(), Action::SetSizing(z), v.sizing == z))
        .collect();
    sizing.insert(3, unavailable("Auto", reason::AUTO_SIZING));
    sizing.push(Item::Separator);
    sizing.push(check(
        "Integer scaling",
        Action::ToggleIntegerScaling,
        v.integer_scaling,
        true,
    ));
    Menu {
        label: "View".to_owned(),
        items: vec![
            submenu("Screen size", sizes),
            submenu("Screen rotation", rotations),
            submenu("Screen gap", gaps),
            submenu("Screen layout", layouts),
            submenu("Screen sizing", sizing),
            submenu(
                "Aspect ratio",
                vec![
                    only_choice("Top 4:3 (native)"),
                    unavailable("Top 16:9", reason::ASPECT),
                    Item::Separator,
                    only_choice("Bottom 4:3 (native)"),
                    unavailable("Bottom 16:9", reason::ASPECT),
                ],
            ),
            Item::Separator,
            unavailable("Open new window", reason::NEW_WINDOW),
            Item::Separator,
            check(
                "Screen filtering",
                Action::ToggleFiltering,
                v.filtering,
                true,
            ),
            check(
                "Show OSD (frame rate)",
                Action::ToggleShowFps,
                v.show_fps,
                true,
            ),
        ],
    }
}

fn config_menu(s: &MenuState<'_>) -> Menu {
    let multiplayer = match s.multiplayer {
        Ok(()) => entry(
            "Multiplayer settings",
            Action::Open(Window::Multiplayer),
            true,
        ),
        Err(why) => unavailable("Multiplayer settings", why),
    };
    Menu {
        label: "Config".to_owned(),
        items: vec![
            entry("Emu settings", Action::Open(Window::EmuSettings), true),
            Item::Separator,
            entry("Input and hotkeys", Action::Open(Window::Input), true),
            submenu("Video settings", video_items(s.view)),
            unavailable("Camera settings", reason::CAMERA),
            entry("Audio settings", Action::Open(Window::Audio), true),
            multiplayer,
            entry("Wifi settings", Action::Open(Window::Wifi), true),
            unavailable("Firmware settings", reason::FIRMWARE_SETTINGS),
            unavailable("Interface settings", reason::INTERFACE),
            entry("Path settings", Action::Open(Window::Paths), true),
            Item::Separator,
            check(
                "Limit framerate",
                Action::ToggleLimitFramerate,
                s.limit_framerate,
                true,
            ),
            unavailable("Audio sync", reason::AUDIO_SYNC),
            submenu("Emulation speed", speed_items(s.view)),
        ],
    }
}

/// Config > Emulation speed: one radio per speed.
fn speed_items(v: &View) -> Vec<Item> {
    let current = speed::valid(v.speed);
    let mut items: Vec<Item> = SPEEDS
        .iter()
        .map(|&p| radio(&speed::label(p), Action::SetSpeed(p), p == current))
        .collect();
    items.push(Item::Separator);
    items.push(Item::Entry {
        label: "Next speed".to_owned(),
        action: Some(Action::CycleSpeed),
        enabled: true,
        checked: None,
        radio: false,
        reason: None,
        shortcut: Some("Left stick press".to_owned()),
        hint: None,
    });
    items
}

/// Config > Video settings: internal resolution and the xBRZ filter.
fn video_items(v: &View) -> Vec<Item> {
    let scale = v.render_scale();
    let mut items: Vec<Item> = (1..=MAX_RENDER_SCALE)
        .map(|n| {
            radio(
                &format!(
                    "Internal resolution {n}x ({}x{})",
                    256 * u32::from(n),
                    192 * u32::from(n)
                ),
                Action::SetRenderScale(n),
                n == scale,
            )
        })
        .collect();
    items.push(Item::Separator);
    let mut xbrz = check(
        "xBRZ filter (2D)",
        Action::ToggleXbrz,
        v.xbrz && scale >= 2,
        scale >= 2,
    );
    if let Item::Entry { reason, .. } = &mut xbrz {
        *reason = (scale < 2).then(|| reason::XBRZ_SCALE.to_owned());
    }
    items.push(xbrz);
    items
}

fn help_menu() -> Menu {
    Menu {
        label: "Help".to_owned(),
        items: vec![entry("About...", Action::Open(Window::About), true)],
    }
}

/// The whole bar: File, System, View, Config, Help.
pub fn menu_bar(s: &MenuState<'_>) -> Vec<Menu> {
    vec![
        file_menu(s),
        system_menu(s),
        view_menu(s),
        config_menu(s),
        help_menu(),
    ]
}

/// Adds `path` to the front of a recent list (deduplicated, at most
/// [`RECENT_LIMIT`]).
pub fn remember(recent: &mut Vec<PathBuf>, path: &std::path::Path) {
    recent.retain(|p| p != path);
    recent.insert(0, path.to_owned());
    recent.truncate(RECENT_LIMIT);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state<'a>(loaded: Option<&'a str>, view: &'a View) -> MenuState<'a> {
        MenuState {
            loaded,
            paused: false,
            view,
            limit_framerate: true,
            recent: &[],
            folder_roms: &[],
            multiplayer: Err("not here"),
        }
    }

    fn walk<'a>(items: &'a [Item], out: &mut Vec<&'a Item>) {
        for item in items {
            out.push(item);
            if let Item::Submenu { items, .. } = item {
                walk(items, out);
            }
        }
    }

    fn find<'a>(menus: &'a [Menu], wanted: &str) -> Option<&'a Item> {
        let mut all = Vec::new();
        for m in menus {
            walk(&m.items, &mut all);
        }
        all.into_iter().find(|i| match i {
            Item::Entry { label, .. } | Item::Submenu { label, .. } => label == wanted,
            _ => false,
        })
    }

    #[test]
    fn has_melonds_shape() {
        let view = View::default();
        let menus = menu_bar(&state(None, &view));
        let labels: Vec<&str> = menus.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["File", "System", "View", "Config", "Help"]);
        for wanted in [
            "Open ROM...",
            "Open recent",
            "Boot firmware",
            "Import savefile...",
            "Save state",
            "Load state",
            "Undo state load",
            "Open data directory",
            "Quit",
            "Pause",
            "Reset",
            "Stop",
            "Frame step",
            "Power management",
            "Date and time",
            "Enable cheats",
            "Setup cheat codes",
            "ROM info",
            "RAM search",
            "Manage DSi titles",
            "Multiplayer",
            "Screen size",
            "Screen rotation",
            "Screen gap",
            "Screen layout",
            "Screen sizing",
            "Aspect ratio",
            "Open new window",
            "Screen filtering",
            "Emu settings",
            "Input and hotkeys",
            "Video settings",
            "Camera settings",
            "Audio settings",
            "Multiplayer settings",
            "Wifi settings",
            "Firmware settings",
            "Interface settings",
            "Path settings",
            "Limit framerate",
            "Audio sync",
            "Emulation speed",
            "About...",
        ] {
            assert!(find(&menus, wanted).is_some(), "missing {wanted}");
        }
    }

    #[test]
    fn disabled_entries_say_why() {
        let view = View::default();
        for loaded in [None, Some("game.nds")] {
            let menus = menu_bar(&state(loaded, &view));
            let mut all = Vec::new();
            for m in &menus {
                walk(&m.items, &mut all);
            }
            for item in all {
                if let Item::Entry {
                    label,
                    enabled: false,
                    reason,
                    ..
                } = item
                {
                    assert!(reason.is_some(), "{label} is disabled without a reason");
                }
            }
        }
    }

    #[test]
    fn game_entries_follow_loaded() {
        let view = View::default();
        let enabled = |loaded, label| {
            let menus = menu_bar(&state(loaded, &view));
            matches!(find(&menus, label), Some(Item::Entry { enabled: true, .. }))
        };
        for label in [
            "Reset",
            "Stop",
            "Frame step",
            "Pause",
            "ROM info",
            "Setup cheat codes",
            "Eject cart",
        ] {
            assert!(!enabled(None, label), "{label} without a game");
            assert!(enabled(Some("game.nds"), label), "{label} with a game");
        }
        assert!(enabled(None, "Open ROM..."));
    }

    #[test]
    fn actions_round_trip_through_json() {
        for action in [
            Action::OpenRomPath(PathBuf::from("a/b.nds")),
            Action::SetLayout(ScreenLayout::Natural),
            Action::Open(Window::RomInfo),
            Action::TogglePause,
            Action::SetSpeed(150),
            Action::Open(Window::Cheats),
        ] {
            let json = serde_json::to_string(&action).unwrap_or_default();
            assert_eq!(serde_json::from_str::<Action>(&json).ok(), Some(action));
        }
        assert_eq!(
            serde_json::to_string(&Action::ScreenSize(2)).unwrap_or_default(),
            r#"{"type":"screenSize","value":2}"#
        );
    }

    #[test]
    fn recent_list_is_mru() {
        let mut recent = Vec::new();
        for name in ["a", "b", "a"] {
            remember(&mut recent, std::path::Path::new(name));
        }
        assert_eq!(recent, [PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(Action::from_shortcut('p'), Some(Action::TogglePause));
    }
}
