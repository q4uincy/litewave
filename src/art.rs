//! Cover art: loaded on worker threads, decoded to small square textures, and kept in a small
//! cache so memory stays low (a 80x80 cover is about 25 KB, a 256x256 one about 256 KB).

use eframe::egui::{self, ColorImage, TextureHandle, TextureId, TextureOptions};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
};

/// Sizes (pixels) covers are decoded at.
pub const THUMB: u32 = 80;
pub const MID: u32 = 128;
pub const BIG: u32 = 256;

const MAX_ENTRIES: usize = 220;
const MAX_ACTIVE: usize = 4;

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ArtKey {
    /// Embedded picture in a local audio file.
    File(PathBuf),
    /// Image URL (Spotify).
    Url(String),
    /// A wallpaper image file, cropped to 4:3.
    Image(PathBuf),
}

type Slot = (ArtKey, u32);
type Loaded = (Slot, Option<ColorImage>);

pub struct ArtCache {
    textures: HashMap<Slot, TextureHandle>,
    order: VecDeque<Slot>,
    pending: HashSet<Slot>,
    failed: HashSet<Slot>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    active: Arc<AtomicUsize>,
    counter: usize,
}

impl ArtCache {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            textures: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            tx,
            rx,
            active: Arc::new(AtomicUsize::new(0)),
            counter: 0,
        }
    }

    /// Uploads finished covers. Call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((slot, img)) = self.rx.try_recv() {
            self.pending.remove(&slot);
            match img {
                Some(img) => {
                    self.counter += 1;
                    let handle = ctx.load_texture(
                        format!("art{}", self.counter),
                        img,
                        TextureOptions::LINEAR,
                    );
                    self.order.push_back(slot.clone());
                    self.textures.insert(slot, handle);
                    while self.order.len() > MAX_ENTRIES {
                        if let Some(old) = self.order.pop_front() {
                            self.textures.remove(&old);
                        }
                    }
                }
                None => {
                    self.failed.insert(slot);
                }
            }
        }
    }

    /// Returns the texture if it is ready; otherwise starts loading it and returns None.
    pub fn get(&mut self, ctx: &egui::Context, key: &ArtKey, size: u32) -> Option<TextureId> {
        let slot: Slot = (key.clone(), size);
        if let Some(t) = self.textures.get(&slot) {
            return Some(t.id());
        }
        if self.pending.contains(&slot) || self.failed.contains(&slot) {
            return None;
        }
        if self.active.load(Ordering::Relaxed) >= MAX_ACTIVE {
            return None;
        }
        self.pending.insert(slot.clone());
        self.active.fetch_add(1, Ordering::Relaxed);
        let tx = self.tx.clone();
        let active = self.active.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let img = load_art(&slot.0, slot.1);
            let _ = tx.send((slot, img));
            active.fetch_sub(1, Ordering::Relaxed);
            ctx.request_repaint();
        });
        None
    }
}

fn load_art(key: &ArtKey, size: u32) -> Option<ColorImage> {
    if let ArtKey::Image(path) = key {
        let (w, h) = (size, size * 3 / 4);
        let img = image::open(path).ok()?;
        let img = img.resize_to_fill(w, h, image::imageops::FilterType::Triangle);
        let rgba = img.to_rgba8();
        return Some(ColorImage::from_rgba_unmultiplied(
            [w as usize, h as usize],
            rgba.as_raw(),
        ));
    }
    let bytes: Vec<u8> = match key {
        ArtKey::File(path) => {
            use lofty::prelude::*;
            let tagged = lofty::read_from_path(path).ok()?;
            let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
            tag.pictures().first()?.data().to_vec()
        }
        ArtKey::Url(url) => fetch_url(url)?,
        ArtKey::Image(_) => return None,
    };
    let img = image::load_from_memory(&bytes).ok()?;
    let img = img.resize_to_fill(size, size, image::imageops::FilterType::Triangle);
    let rgba = img.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied(
        [size as usize, size as usize],
        rgba.as_raw(),
    ))
}

#[cfg(feature = "spotify")]
fn fetch_url(url: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let resp = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .get(url)
        .call()
        .ok()?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(6_000_000)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

#[cfg(not(feature = "spotify"))]
fn fetch_url(_url: &str) -> Option<Vec<u8>> {
    None
}
