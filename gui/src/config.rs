use serde::{Deserialize, Serialize};

use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub bios7_path: Option<PathBuf>,
    pub bios9_path: Option<PathBuf>,
    pub firmware_path: Option<PathBuf>,
    pub last_rom_path: Option<PathBuf>,

    pub window_width: i32,
    pub window_height: i32,

    pub audio_volume: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bios7_path: None,
            bios9_path: None,
            firmware_path: None,
            last_rom_path: None,

            window_width: 512,
            window_height: 768,

            audio_volume: 100.0,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        "./config.json".into()
    }

    pub fn load() -> Self {
        let path = Self::path();

        match fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),

            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let path = Self::path();

        let json = serde_json::to_string_pretty(self).unwrap();

        fs::write(path, json).unwrap();
    }
}
