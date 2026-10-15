//! Per-game cheat lists (System > Setup cheat codes), shared by both GUIs.
//!
//! ```text
//!  cheats/<rom name>.json  <--load / save-->  Vec<Cheat>  (the dialog edits it)
//!                                                 |  enabled_codes()
//!                                                 v
//!                               Emulator::set_cheats (Action Replay engine)
//! ```
//!
//! Each cheat keeps its code as the text the user typed (two 8-digit hex
//! words per line) plus a free-text memo, so nothing is lost on a round
//! trip; the code is parsed only when it is handed to the emulator.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Instance;

/// The cheat format the emulator runs.
pub const ACTION_REPLAY: &str = "Action Replay";

/// One cheat.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Cheat {
    pub name: String,
    /// Notes (usage, templates, ...); not run.
    pub memo: String,
    /// Code text: `XXXXXXXX YYYYYYYY` per line.
    pub code: String,
    pub enabled: bool,
    /// Code format (always [`ACTION_REPLAY`] for now).
    pub kind: String,
}

impl Default for Cheat {
    fn default() -> Self {
        Self {
            name: String::new(),
            memo: String::new(),
            code: String::new(),
            enabled: false,
            kind: ACTION_REPLAY.to_owned(),
        }
    }
}

impl Cheat {
    /// The parsed code words, or why the code is invalid.
    ///
    /// # Errors
    /// See [`ds_emu::parse_ar_code`].
    pub fn words(&self) -> Result<Vec<u32>, String> {
        ds_emu::parse_ar_code(&self.code)
    }
}

/// `cheats/<rom name>.json` in the instance folder.
pub fn cheats_path(instance: &Instance, rom: &Path) -> PathBuf {
    instance
        .root()
        .join("cheats")
        .join(rom.with_extension("json").file_name().unwrap_or_default())
}

/// The cheats saved at `path` (none if the file is missing or unreadable).
pub fn load(path: &Path) -> Vec<Cheat> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        log::error!("Cannot read cheats {}: {e}", path.display());
        Vec::new()
    })
}

/// Writes `cheats` to `path` (creating the folder).
///
/// # Errors
/// If the folder or the file cannot be written.
pub fn save(path: &Path, cheats: &[Cheat]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(cheats).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// The codes to run: enabled cheats with a valid code (invalid ones are
/// logged and skipped). Empty when cheats are turned off.
pub fn enabled_codes(cheats: &[Cheat], on: bool) -> Vec<Vec<u32>> {
    if !on {
        return Vec::new();
    }
    cheats
        .iter()
        .filter(|c| c.enabled)
        .filter_map(|c| {
            c.words()
                .inspect_err(|e| log::warn!("Cheat \"{}\" skipped: {e}", c.name))
                .ok()
        })
        .filter(|words| !words.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cheat(name: &str, code: &str, enabled: bool) -> Cheat {
        Cheat {
            name: name.to_owned(),
            code: code.to_owned(),
            enabled,
            ..Cheat::default()
        }
    }

    #[test]
    fn only_enabled_valid_codes_run() {
        let list = [
            cheat("on", "02000000 00000001", true),
            cheat("off", "02000000 00000002", false),
            cheat("bad", "0200000 1", true),
        ];
        assert_eq!(enabled_codes(&list, true), vec![vec![0x0200_0000, 1]]);
        assert!(enabled_codes(&list, false).is_empty());
    }

    #[test]
    fn round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("cogids-cheats-{}", std::process::id()));
        let path = dir.join("game.json");
        let list = vec![Cheat {
            memo: "メモ".to_owned(),
            ..cheat("無敵", "02000000 00000001", true)
        }];
        assert!(save(&path, &list).is_ok());
        assert_eq!(load(&path), list);
        let _ = std::fs::remove_dir_all(dir);
        assert!(load(&path).is_empty());
    }
}
