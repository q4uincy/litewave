//! The theme gallery: a list of themes (index.json) hosted in a public GitHub repository.
//! litewave downloads the list, shows previews, and installs a theme into the themes folder.

use crate::theme::{self, Theme};
use serde::Deserialize;
use std::io::Read;

/// Theme index lives in the `litewave-themes` repository
/// (see theme-gallery/README.md). The file is `index.json` at the top of that repository.
pub const INDEX_URL: &str =
    "https://raw.githubusercontent.com/q4uincy/litewave-themes/main/index.json";

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct StoreItem {
    pub name: String,
    pub author: String,
    pub description: String,
    /// Link to the theme .json file.
    pub json: String,
    /// Link to a screenshot (png / jpg), ideally 4:3.
    pub preview: String,
    /// Optional link to the wallpaper image the theme uses.
    pub image: String,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(20))
        .build()
}

fn get_bytes(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err("Only https links are allowed".to_string());
    }
    let resp = agent().get(url).call().map_err(|e| format!("Download failed: {e}"))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(limit)
        .read_to_end(&mut buf)
        .map_err(|e| format!("Download failed: {e}"))?;
    Ok(buf)
}

/// Downloads the list of themes. Blocking: call it from a thread.
pub fn fetch_index() -> Result<Vec<StoreItem>, String> {
    if INDEX_URL.contains("YOUR-NAME") {
        return Err("The theme gallery is not set up yet (see theme-gallery/README.md).".to_string());
    }
    let bytes = get_bytes(INDEX_URL, 1_000_000)?;
    let items: Vec<StoreItem> =
        serde_json::from_slice(&bytes).map_err(|e| format!("The theme list is not valid: {e}"))?;
    Ok(items
        .into_iter()
        .filter(|i| !i.name.trim().is_empty() && i.json.starts_with("https://"))
        .collect())
}

/// Downloads one theme (and its wallpaper) into the themes folder. Returns its name. Blocking.
pub fn install(item: &StoreItem) -> Result<String, String> {
    let text = get_bytes(&item.json, 200_000)?;
    let mut t: Theme = serde_json::from_slice(&text).map_err(|e| format!("Not a litewave theme: {e}"))?;
    if t.name.trim().is_empty() {
        t.name = item.name.clone();
    }
    if t.author.trim().is_empty() {
        t.author = item.author.clone();
    }
    let dir = theme::themes_dir().ok_or("No config directory available")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut slug: String = t
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    if slug.trim_matches('-').is_empty() {
        slug = "theme".to_string();
    }
    t.background = None;
    if item.image.starts_with("https://") {
        let ext = if item.image.to_lowercase().contains(".png") {
            "png"
        } else if item.image.to_lowercase().contains(".webp") {
            "webp"
        } else {
            "jpg"
        };
        let bytes = get_bytes(&item.image, 12_000_000)?;
        let file = format!("{slug}.{ext}");
        std::fs::write(dir.join(&file), bytes).map_err(|e| e.to_string())?;
        t.background = Some(file); // relative: resolved against the theme file when loaded
    }
    let json = serde_json::to_string_pretty(&t).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{slug}.json")), json).map_err(|e| e.to_string())?;
    Ok(t.name)
}
