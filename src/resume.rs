//! Remembers the last local song and position, so litewave continues where you stopped.
use std::path::{Path, PathBuf};

fn file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("resume.json"))
}

pub fn load() -> Option<(PathBuf, f32)> {
    let text = std::fs::read_to_string(file()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let path = PathBuf::from(v["path"].as_str()?);
    let pos = v["position"].as_f64()? as f32;
    Some((path, pos))
}

pub fn save(path: &Path, position: f32) {
    let Some(f) = file() else { return };
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let v = serde_json::json!({ "path": path.to_string_lossy(), "position": position });
    let _ = std::fs::write(f, v.to_string());
}
