use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub rom_dir: PathBuf,
    pub selected_rom: Option<PathBuf>,
    pub scale: f32,
    pub show_fps: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rom_dir: PathBuf::from(""),
            selected_rom: None,
            scale: 2.0,
            show_fps: true,
        }
    }
}

const CONFIG_PATH: &str = "config.json";

pub fn load_config() -> Config {
    if let Ok(text) = fs::read_to_string(CONFIG_PATH) {
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        let cfg = Config::default();
        save_config(&cfg);
        cfg
    }
}

pub fn save_config(cfg: &Config) {
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(CONFIG_PATH, text);
    }
}
