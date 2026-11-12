//! Themes: colors, roundness and an optional wallpaper. Built-in presets plus JSON files
//! in the user's `litewave/themes` folder (a theme is a .json file, optionally with an image
//! next to it, so themes are easy to share).

use eframe::egui::{self, Color32, Rounding, Stroke, Visuals};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub name: String,
    pub author: String,
    /// Window background, "#rrggbb".
    pub bg: String,
    /// Panels and cards.
    pub panel: String,
    pub text: String,
    pub subtext: String,
    pub accent: String,
    /// 0-255. How opaque panels are when a wallpaper is set (lower shows more wallpaper).
    pub panel_alpha: u8,
    /// 0-255. How much the wallpaper is darkened.
    pub dim: u8,
    pub rounding: f32,
    /// Wallpaper image path. Relative paths are resolved against the theme file.
    pub background: Option<String>,
    /// Second accent ("#rrggbb"): links, the bottom glow. Empty = same as `accent`.
    pub accent2: String,
    /// Border / divider color. Empty = derived from panel and text.
    pub border: String,
    /// Color laid over the wallpaper (with `dim` as its strength). Empty = black.
    pub tint: String,
    /// 0-12. Gaussian blur on the wallpaper (a soft, frosted look).
    pub blur: u8,
    /// 0-255. Strength of a soft `accent2` glow rising from the bottom edge.
    pub glow: u8,
}

impl Default for Theme {
    fn default() -> Self {
        presets().remove(0)
    }
}

fn preset(name: &str, bg: &str, panel: &str, text: &str, sub: &str, accent: &str) -> Theme {
    Theme {
        name: name.to_string(),
        author: "litewave".to_string(),
        bg: bg.to_string(),
        panel: panel.to_string(),
        text: text.to_string(),
        subtext: sub.to_string(),
        accent: accent.to_string(),
        panel_alpha: 200,
        dim: 120,
        rounding: 8.0,
        background: None,
        accent2: String::new(),
        border: String::new(),
        tint: String::new(),
        blur: 0,
        glow: 0,
    }
}

pub fn presets() -> Vec<Theme> {
    vec![
        preset("Midnight", "#0e1014", "#171a20", "#e8eaed", "#8b929c", "#2ec4b6"),
        preset("Greenroom", "#0b0b0b", "#161616", "#f1f1f1", "#a0a0a0", "#1ed760"),
        preset("Sakura", "#150f13", "#201519", "#fbe9f0", "#b99aa8", "#ff7eb6"),
        preset("Ocean", "#08121c", "#0f1d2b", "#e6f1fa", "#8aa4b8", "#4cc9f0"),
        preset("Ember", "#140f0c", "#1f1713", "#f6ebe3", "#b3a093", "#ff9248"),
        preset("Violet", "#100d17", "#191425", "#ece8f8", "#9a90b8", "#a78bfa"),
        preset("Paper", "#f3f0ea", "#ffffff", "#22252b", "#6b7280", "#d9480f"),
    ]
}

pub fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn to_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

impl Theme {
    pub fn bg(&self) -> Color32 {
        parse_hex(&self.bg).unwrap_or(Color32::from_rgb(14, 16, 20))
    }
    pub fn panel(&self) -> Color32 {
        parse_hex(&self.panel).unwrap_or(Color32::from_rgb(23, 26, 32))
    }
    pub fn text(&self) -> Color32 {
        parse_hex(&self.text).unwrap_or(Color32::from_rgb(232, 234, 237))
    }
    pub fn subtext(&self) -> Color32 {
        parse_hex(&self.subtext).unwrap_or(Color32::from_rgb(139, 146, 156))
    }
    pub fn accent(&self) -> Color32 {
        parse_hex(&self.accent).unwrap_or(Color32::from_rgb(46, 196, 182))
    }

    pub fn accent2(&self) -> Color32 {
        parse_hex(&self.accent2).unwrap_or_else(|| self.accent())
    }
    pub fn border(&self) -> Color32 {
        parse_hex(&self.border).unwrap_or_else(|| mix(self.panel(), self.text(), 0.12))
    }
    pub fn tint(&self) -> Color32 {
        parse_hex(&self.tint).unwrap_or(Color32::BLACK)
    }

    pub fn is_light(&self) -> bool {
        let b = self.bg();
        (0.299 * b.r() as f32 + 0.587 * b.g() as f32 + 0.114 * b.b() as f32) > 140.0
    }

    /// Pushes this theme into egui's visuals. `wallpaper` makes panels translucent.
    pub fn apply(&self, ctx: &egui::Context, wallpaper: bool) {
        let light = self.is_light();
        let mut v = if light { Visuals::light() } else { Visuals::dark() };
        let (panel, text, accent) = (self.panel(), self.text(), self.accent());
        let alpha = if wallpaper { self.panel_alpha } else { 255 };

        v.panel_fill = Color32::from_rgba_unmultiplied(panel.r(), panel.g(), panel.b(), alpha);
        v.window_fill = panel;
        v.extreme_bg_color = mix(panel, if light { Color32::WHITE } else { Color32::BLACK }, 0.35);
        v.faint_bg_color = mix(panel, text, 0.05);
        v.override_text_color = Some(text);
        v.selection.bg_fill = mix(panel, accent, 0.45);
        v.selection.stroke = Stroke::new(1.0, accent);
        v.hyperlink_color = self.accent2();

        let r = Rounding::same(self.rounding.clamp(0.0, 20.0));
        let base = mix(panel, text, 0.08);
        let hover = mix(panel, text, 0.16);
        let active = mix(panel, accent, 0.5);
        v.widgets.inactive.weak_bg_fill = base;
        v.widgets.inactive.bg_fill = base;
        v.widgets.hovered.weak_bg_fill = hover;
        v.widgets.hovered.bg_fill = hover;
        v.widgets.active.weak_bg_fill = active;
        v.widgets.active.bg_fill = active;
        v.widgets.noninteractive.rounding = r;
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.rounding = r;
            w.bg_stroke = Stroke::NONE;
        }
        v.window_rounding = Rounding::same((self.rounding * 1.5).clamp(0.0, 28.0));
        v.window_stroke = Stroke::new(1.0, self.border());
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border());
        ctx.set_visuals(v);
        // Roomier, calmer spacing: more padding in buttons, thin floating scrollbars.
        ctx.style_mut(|st| {
            st.spacing.item_spacing = egui::vec2(8.0, 7.0);
            st.spacing.button_padding = egui::vec2(12.0, 6.0);
            st.spacing.interact_size.y = 28.0;
            st.spacing.scroll = egui::style::ScrollStyle::floating();
        });
    }
}

// ---- user themes on disk ---------------------------------------------------

pub fn themes_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("themes"))
}

pub fn import_theme(path: &Path) -> Option<Theme> {
    let mut t: Theme = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    if t.name.trim().is_empty() {
        t.name = path.file_stem()?.to_string_lossy().into_owned();
    }
    if let (Some(b), Some(dir)) = (t.background.as_mut(), path.parent()) {
        if Path::new(b.as_str()).is_relative() {
            *b = dir.join(b.as_str()).to_string_lossy().into_owned();
        }
    }
    Some(t)
}

pub fn load_user_themes() -> Vec<Theme> {
    let Some(dir) = themes_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<Theme> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x == "json"))
        .filter_map(|p| import_theme(&p))
        .collect();
    out.sort_by_key(|t| t.name.to_lowercase());
    out
}

pub fn all_themes() -> Vec<Theme> {
    let mut v = presets();
    v.extend(load_user_themes());
    v
}

/// Saves `t` as `<name>.json` in the themes folder. A wallpaper is copied next to it so the
/// pair can be shared. Returns the theme with its wallpaper path resolved.
pub fn save_user_theme(t: &Theme) -> Result<Theme, String> {
    let dir = themes_dir().ok_or("No config directory available")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut slug: String = t
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    if slug.trim_matches('-').is_empty() {
        slug = "theme".to_string();
    }

    let mut saved = t.clone();
    if let Some(bg) = &t.background {
        let src = Path::new(bg);
        if src.is_file() {
            if let Some(ext) = src.extension().and_then(|e| e.to_str()) {
                let file = format!("{slug}.{ext}");
                let dest = dir.join(&file);
                if src != dest.as_path() {
                    std::fs::copy(src, &dest).map_err(|e| e.to_string())?;
                }
                saved.background = Some(file);
            }
        }
    }
    let json = serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{slug}.json")), json).map_err(|e| e.to_string())?;

    if let Some(b) = saved.background.as_mut() {
        if Path::new(b.as_str()).is_relative() {
            *b = dir.join(b.as_str()).to_string_lossy().into_owned();
        }
    }
    Ok(saved)
}

pub fn open_folder(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    #[cfg(target_os = "windows")]
    let prog = "explorer";
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let prog = "xdg-open";
    let _ = std::process::Command::new(prog).arg(dir).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let c = parse_hex("#2ec4b6").unwrap();
        assert_eq!(to_hex(c), "#2ec4b6");
        assert!(parse_hex("nope").is_none());
    }

    #[test]
    fn presets_parse() {
        for t in presets() {
            assert!(parse_hex(&t.bg).is_some(), "{}", t.name);
            assert!(parse_hex(&t.accent).is_some(), "{}", t.name);
        }
    }

    #[test]
    fn new_fields_are_optional() {
        let t: Theme = serde_json::from_str(r##"{"name":"x","glow":90,"blur":6,"tint":"#102030"}"##).unwrap();
        assert_eq!(t.glow, 90);
        assert_eq!(t.tint(), Color32::from_rgb(0x10, 0x20, 0x30));
        assert_eq!(t.accent2(), t.accent());
    }

    #[test]
    fn partial_json_uses_defaults() {
        let t: Theme = serde_json::from_str(r##"{"name":"x","accent":"#ff0000"}"##).unwrap();
        assert_eq!(t.accent, "#ff0000");
        assert_eq!(t.rounding, Theme::default().rounding);
    }
}
