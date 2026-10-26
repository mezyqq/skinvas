use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    // General
    pub lang: String,
    pub theme: String,
    pub accent: String,
    pub radius: f64,
    pub animations: bool,
    pub font_scale: f64,
    pub last_page: String,
    pub recent: Vec<String>,
    pub onboarded: bool,
    pub tablet: bool,
    pub autosave_secs: u32,
    // Editor
    pub show_grid: bool,
    pub grid_color: String,
    pub checker: bool,
    pub undo_limit: u32,
    pub confirm_unsaved: bool,
    pub right_click_picks: bool,
    pub palette_size: f64,
    pub picker_size: f64,
    // Browser
    pub cache: bool,
    pub safe: bool,
    pub discord: bool,
    pub discord_id: String,
    // 3D viewer
    pub slim_default: bool,
    pub auto_rotate: bool,
    pub rotate_speed: f64,
    pub walk: bool,
    pub walk_speed: f64,
    pub layers3d: bool,
    pub thickness: f64,
    pub shading: bool,
    pub fov: f64,
    pub bg: String,
    pub quality: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: "ru".into(),
            theme: "dark".into(),
            accent: "#4caf50".into(),
            radius: 12.,
            animations: true,
            font_scale: 1.,
            last_page: "editor".into(),
            recent: vec![],
            onboarded: false,
            tablet: false,
            autosave_secs: 60,
            show_grid: true,
            grid_color: "#00000040".into(),
            checker: true,
            undo_limit: 200,
            confirm_unsaved: true,
            right_click_picks: true,
            palette_size: 20.,
            picker_size: 300.,
            cache: false,
            safe: true,
            discord: false,
            discord_id: String::new(),
            slim_default: false,
            auto_rotate: false,
            rotate_speed: 0.5,
            walk: false,
            walk_speed: 1.,
            layers3d: true,
            thickness: 1.,
            shading: true,
            fov: 35.,
            bg: "#1e1f24".into(),
            quality: 1.,
        }
    }
}

fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("skinvas").join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = path();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        if let Err(e) = std::fs::write(&p, serde_json::to_vec_pretty(self).unwrap()) {
            eprintln!("skinvas: cannot save settings: {e}");
        }
    }
}

/// "#rrggbb" or "#rrggbbaa" -> RGBA.
pub fn parse_hex(s: &str) -> [u8; 4] {
    let h = s.trim_start_matches('#');
    let b = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
    [b(0).unwrap_or(0), b(2).unwrap_or(0), b(4).unwrap_or(0), b(6).unwrap_or(255)]
}

pub fn to_hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
}
