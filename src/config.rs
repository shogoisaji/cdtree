use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl From<RgbColor> for Color {
    fn from(c: RgbColor) -> Self {
        Color::Rgb(c.r, c.g, c.b)
    }
}

impl From<(u8, u8, u8)> for RgbColor {
    fn from((r, g, b): (u8, u8, u8)) -> Self {
        Self { r, g, b }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub border_fg: RgbColor,
    pub border_style: RgbColor, // Used for LIME in ui.rs
    pub border_style_soft: RgbColor, // Used for LIME_SOFT in ui.rs
    pub key_highlight: RgbColor, // ORANGE
    pub branch_color: RgbColor, // BRANCH_BLUE
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            border_fg: (120, 230, 80).into(), // LIME
            border_style: (120, 230, 80).into(), // LIME (reusing for simplicity based on ui.rs usage)
            border_style_soft: (90, 190, 70).into(), // LIME_SOFT
            key_highlight: (255, 160, 0).into(), // ORANGE
            branch_color: (202, 93, 42).into(), // BRANCH_BLUE
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub theme: Theme,
    #[serde(default)]
    pub show_files: bool,
    #[serde(default)]
    pub show_hidden: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            show_files: false,
            show_hidden: false,
        }
    }
}

impl Config {
    pub fn load() -> io::Result<Self> {
        let config_path = Self::get_config_path();
        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(config_path)?;
        let config: Config = serde_json::from_str(&content).unwrap_or_default();
        Ok(config)
    }

    pub fn save(&self) -> io::Result<()> {
        let config_path = Self::get_config_path();
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(config_path, content)?;
        Ok(())
    }

    fn get_config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Path::new(&home).join(".config").join("cdtree").join("config.json")
    }
}


