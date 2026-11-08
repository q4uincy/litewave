use super::{Entry, SpEvent, REDIRECT_URI};
use crate::library::Track;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use librespot::{
    core::{
        authentication::Credentials, config::SessionConfig, session::Session,
        spotify_id::SpotifyId, SpotifyUri,
    },
    playback::{
        audio_backend,
        config::{AudioFormat, PlayerConfig},
        mixer::NoOpVolume,
        player::{Player as LsPlayer, PlayerEvent},
    },
};
use rand::{distributions::Alphanumeric, Rng};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        mpsc::{channel, Receiver},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const SCOPES: &str = "streaming user-read-private playlist-read-private playlist-read-collaborative user-library-read user-follow-read";
const API: &str = "https://api.spotify.com/v1";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const MAX_PAGES: usize = 40;

static NULL: Value = Value::Null;

struct Tokens {
    access: String,
    refresh: String,
    expires: Instant,
}

fn token_request(form: &[(&str, &str)]) -> Result<Value, String> {
    ureq::post(TOKEN_URL)
        .send_form(form)
        .map_err(|e| format!("Spotify login failed: {e}"))?
        .into_json::<Value>()
        .map_err(|e| e.to_string())
}

fn parse_tokens(v: &Value, old_refresh: &str) -> Result<Tokens, String> {
    let access = v["access_token"]
        .as_str()
        .ok_or("Spotify returned no access token")?
        .to_string();
    let refresh = v["refresh_token"].as_str().unwrap_or(old_refresh).to_string();
    let secs = v["expires_in"].as_u64().unwrap_or(3600);
    Ok(Tokens {
        access,
        refresh,
        expires: Instant::now() + Duration::from_secs(secs.saturating_sub(60)),
    })
}

fn artists_of(v: &Value) -> String {
    v["artists"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// Picks a cover: the smallest image at least 300 px wide, else the largest available.
fn pick_image(images: &Value) -> Option<String> {
    let arr = images.as_array()?;
    let mut v: Vec<(&str, u64)> = arr
        .iter()
        .filter_map(|i| Some((i["url"].as_str()?, i["width"].as_u64().unwrap_or(0))))
        .collect();
    v.sort_by_key(|x| x.1);
    v.iter()
        .find(|x| x.1 >= 300)
        .or(v.last())
        .map(|x| x.0.to_string())
}

fn track_from(v: &Value, album_fallback: &str, art_fallback: Option<&str>) -> Option<Track> {
    if v["type"].as_str().map_or(false, |t| t != "track") {
        return None;
    }
    let id = v["id"].as_str()?;
    let name = v["name"].as_str()?;
    let album = v["album"]["name"].as_str().unwrap_or(album_fallback);
    let art = pick_image(&v["album"]["images"]).or_else(|| art_fallback.map(|a| a.to_string()));
    Some(Track::spotify(
        id,
        name,
        &artists_of(v),
        album,
        v["duration_ms"].as_u64().unwrap_or(0),
        art,
    ))
}

#[derive(Clone)]
pub struct Client {
    client_id: String,
    tokens: Arc<Mutex<Tokens>>,
}

impl Client {
    /// Interactive login: opens the browser and waits for the redirect on 127.0.0.1:8898.
    pub fn login(client_id: &str) -> Result<Client, String> {
        let mut rng = rand::thread_rng();
        let verifier: String = (0..64).map(|_| rng.sample(Alphanumeric) as char).collect();
        let state: String = (0..16).map(|_| rng.sample(Alphanumeric) as char).collect();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

        let url = format!(
            "https://accounts.spotify.com/authorize?response_type=code&client_id={}&scope={}&redirect_uri={}&code_challenge_method=S256&code_challenge={}&state={}",
            urlencoding::encode(client_id),
            urlencoding::encode(SCOPES),
            urlencoding::encode(REDIRECT_URI),
            challenge,
            state
        );

        let listener = TcpListener::bind("127.0.0.1:8898")
            .map_err(|e| format!("Port 8898 is busy (finish or restart the pending login): {e}"))?;
        webbrowser::open(&url).map_err(|e| format!("Could not open the browser: {e}"))?;
        let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;

        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
        let req = String::from_utf8_lossy(&buf[..n]).into_owned();
        let target = req
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("");
        let query = target.split_once('?').map(|x| x.1).unwrap_or("");

        let mut code: Option<String> = None;
        let mut got_state: Option<String> = None;
        let mut denied: Option<String> = None;
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                let v = urlencoding::decode(v)
                    .map(|c| c.into_owned())
                    .unwrap_or_default();
                match k {
                    "code" => code = Some(v),
                    "state" => got_state = Some(v),
                    "error" => denied = Some(v),
                    _ => {}
                }
            }
        }

        let body = "<html><body style='font-family:sans-serif;text-align:center;margin-top:20vh'><h2>litewave is connected.</h2><p>You can close this tab.</p></body></html>";
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        drop(stream);
        drop(listener);

        if let Some(e) = denied {
            return Err(format!("Spotify denied access: {e}"));
        }
        if got_state.as_deref() != Some(state.as_str()) {
            return Err("Login failed: state mismatch".to_string());
        }
        let code = code.ok_or("Login failed: no authorization code received")?;

        let v = token_request(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", client_id),
            ("code_verifier", verifier.as_str()),
        ])?;
        let tokens = parse_tokens(&v, "")?;
        Ok(Client {
            client_id: client_id.to_string(),
            tokens: Arc::new(Mutex::new(tokens)),
        })
    }

    pub fn from_refresh(client_id: &str, refresh: &str) -> Result<Client, String> {
        let v = token_request(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", client_id),
        ])?;
        let tokens = parse_tokens(&v, refresh)?;
        Ok(Client {
            client_id: client_id.to_string(),
            tokens: Arc::new(Mutex::new(tokens)),
        })
    }

    pub fn refresh_token(&self) -> String {
        self.tokens
            .lock()
            .map(|t| t.refresh.clone())
            .unwrap_or_default()
    }

    pub fn access_token(&self) -> Result<String, String> {
        let mut t = self
            .tokens
            .lock()
            .map_err(|_| "token lock poisoned".to_string())?;
        if Instant::now() >= t.expires {
            let refresh = t.refresh.clone();
            let v = token_request(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", self.client_id.as_str()),
            ])?;
            *t = parse_tokens(&v, &refresh)?;
        }
        Ok(t.access.clone())
    }

    fn get(&self, url: &str) -> Result<Value, String> {
        let token = self.access_token()?;
        for attempt in 0..2 {
            match ureq::get(url)
                .set("Authorization", &format!("Bearer {token}"))
                .call()
            {
                Ok(r) => return r.into_json::<Value>().map_err(|e| e.to_string()),
                Err(ureq::Error::Status(429, r)) if attempt == 0 => {
                    let wait = r
                        .header("Retry-After")
                        .and_then(|h| h.parse::<u64>().ok())
                        .unwrap_or(2)
                        .min(10);
                    std::thread::sleep(Duration::from_secs(wait));
                }
                Err(ureq::Error::Status(403, _)) => {
                    return Err("Spotify refused this request (403). In Development Mode apps can only read playlists you own or collaborate on.".to_string())
                }
                Err(ureq::Error::Status(code, _)) => {
                    return Err(format!("Spotify API error {code}"))
                }
                Err(e) => return Err(format!("Network error: {e}")),
            }
        }
        Err("Spotify is rate limiting requests, try again shortly".to_string())
    }

    /// Follows `next` links, collecting every item. `root` names a wrapper object
    /// (e.g. "artists" for /me/following).
    fn collect(&self, first: String, root: Option<&str>) -> Result<Vec<Value>, String> {
        let mut url = first;
        let mut out = Vec::new();
        for _ in 0..MAX_PAGES {
            let v = self.get(&url)?;
            let node = match root {
                Some(k) => &v[k],
                None => &v,
            };
            if let Some(items) = node["items"].as_array() {
                out.extend(items.iter().cloned());
            }
            match node["next"].as_str() {
                Some(next) => url = next.to_string(),
                None => break,
            }
        }
        Ok(out)
    }

    pub fn playlists(&self) -> Result<Vec<Entry>, String> {
        let items = self.collect(format!("{API}/me/playlists?limit=50"), None)?;
        Ok(items
            .iter()
            .filter_map(|p| {
                Some(Entry {
                    id: p["id"].as_str()?.to_string(),
                    name: p["name"].as_str().unwrap_or("Untitled").to_string(),
                    sub: p["owner"]["display_name"].as_str().unwrap_or("").to_string(),
                    art: pick_image(&p["images"]),
                })
            })
            .collect())
    }

    pub fn playlist_tracks(&self, id: &str) -> Result<Vec<Track>, String> {
        let items = self.collect(format!("{API}/playlists/{id}/items?limit=50"), None)?;
        Ok(items
            .iter()
            .filter_map(|it| {
                let t = it
                    .get("item")
                    .filter(|x| !x.is_null())
                    .or_else(|| it.get("track"))
                    .unwrap_or(&NULL);
                track_from(t, "", None)
            })
            .collect())
    }

    pub fn liked(&self) -> Result<Vec<Track>, String> {
        let items = self.collect(format!("{API}/me/tracks?limit=50"), None)?;
        Ok(items
            .iter()
            .filter_map(|it| track_from(&it["track"], "", None))
            .collect())
    }

    pub fn saved_albums(&self) -> Result<Vec<Entry>, String> {
        let items = self.collect(format!("{API}/me/albums?limit=50"), None)?;
        Ok(items
            .iter()
            .filter_map(|it| {
                let a = &it["album"];
                Some(Entry {
                    id: a["id"].as_str()?.to_string(),
                    name: a["name"].as_str().unwrap_or("Untitled").to_string(),
                    sub: artists_of(a),
                    art: pick_image(&a["images"]),
                })
            })
            .collect())
    }

    pub fn album_tracks(&self, id: &str, name: &str) -> Result<Vec<Track>, String> {
        let art = self
            .get(&format!("{API}/albums/{id}"))
            .ok()
            .and_then(|a| pick_image(&a["images"]));
        let items = self.collect(format!("{API}/albums/{id}/tracks?limit=50"), None)?;
        Ok(items
            .iter()
            .filter_map(|t| track_from(t, name, art.as_deref()))
            .collect())
    }

    pub fn followed_artists(&self) -> Result<Vec<Entry>, String> {
        let items = self.collect(
            format!("{API}/me/following?type=artist&limit=50"),
            Some("artists"),
        )?;
        Ok(items
            .iter()
            .filter_map(|a| {
                Some(Entry {
                    id: a["id"].as_str()?.to_string(),
                    name: a["name"].as_str().unwrap_or("Unknown").to_string(),
                    sub: String::new(),
                    art: pick_image(&a["images"]),
                })
            })
            .collect())
    }

    pub fn artist_albums(&self, id: &str) -> Result<Vec<Entry>, String> {
        let items = self.collect(
            format!("{API}/artists/{id}/albums?include_groups=album,single&limit=50"),
            None,
        )?;
        Ok(items
            .iter()
            .filter_map(|a| {
                Some(Entry {
                    id: a["id"].as_str()?.to_string(),
                    name: a["name"].as_str().unwrap_or("Untitled").to_string(),
                    sub: a["release_date"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(4)
                        .collect(),
                    art: pick_image(&a["images"]),
                })
            })
            .collect())
    }
}

// ---- diagnostics ---------------------------------------------------------

static LAST_LOG: Mutex<String> = Mutex::new(String::new());
static LOGGER: SpLogger = SpLogger;

struct SpLogger;

impl log::Log for SpLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn && m.target().starts_with("librespot")
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("[{}] {}: {}", r.level(), r.target(), r.args());
        if let Ok(mut last) = LAST_LOG.lock() {
            *last = line.clone();
        }
        if let Some(path) = log_path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    fn flush(&self) {}
}

/// Where librespot warnings and errors are written.
pub fn log_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("librespot.log"))
}

/// The most recent librespot warning or error.
pub fn last_log() -> String {
    LAST_LOG.lock().map(|l| l.clone()).unwrap_or_default()
}

fn init_logging() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Warn);
        }
    });
}

fn short(s: String) -> String {
    s.chars().take(90).collect()
}

// ---- playback ------------------------------------------------------------

/// Built-in playback through librespot. Owns its own tokio runtime.
pub struct Engine {
    rt: tokio::runtime::Runtime,
    player: Arc<LsPlayer>,
    /// The track we are currently expecting events for.
    want: Arc<Mutex<Option<SpotifyUri>>>,
    ev_rx: Receiver<SpEvent>,
}

impl Engine {
    pub fn connect(access_token: &str) -> Result<Engine, String> {
        init_logging();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let token = access_token.to_string();
        let player = rt.block_on(async move {
            // Ask for the rodio backend explicitly so audio can never be routed to "pipe".
            let backend = audio_backend::find(Some("rodio".to_string()))
                .or_else(|| audio_backend::find(None))
                .ok_or_else(|| "No audio backend available".to_string())?;
            let session = Session::new(SessionConfig::default(), None);
            session
                .connect(Credentials::with_access_token(&token), false)
                .await
                .map_err(|e| format!("Spotify playback login failed (Premium is required): {e}"))?;
            let format = AudioFormat::default();
            Ok::<_, String>(LsPlayer::new(
                PlayerConfig::default(),
                session,
                Box::new(NoOpVolume),
                move || backend(None, format),
            ))
        })?;

        let want: Arc<Mutex<Option<SpotifyUri>>> = Arc::new(Mutex::new(None));
        let (ev_tx, ev_rx) = channel();
        let mut events = player.get_player_event_channel();
        let w = want.clone();
        rt.spawn(async move {
            while let Some(ev) = events.recv().await {
                let mine = |id: &SpotifyUri| {
                    w.lock()
                        .map(|g| g.as_ref() == Some(id))
                        .unwrap_or(false)
                };
                let out = match &ev {
                    PlayerEvent::Playing { track_id, position_ms, .. } if mine(track_id) => {
                        SpEvent::Playing(*position_ms)
                    }
                    PlayerEvent::Paused { track_id, .. } if mine(track_id) => SpEvent::Paused,
                    PlayerEvent::EndOfTrack { track_id, .. } if mine(track_id) => SpEvent::Ended,
                    PlayerEvent::Unavailable { track_id, .. } if mine(track_id) => {
                        SpEvent::Unavailable
                    }
                    PlayerEvent::TrackChanged { .. } | PlayerEvent::PositionCorrection { .. } => {
                        continue
                    }
                    other => SpEvent::Info(short(format!("{other:?}"))),
                };
                if ev_tx.send(out).is_err() {
                    break;
                }
            }
        });

        Ok(Engine {
            rt,
            player,
            want,
            ev_rx,
        })
    }

    pub fn load(&self, id: &str, start: Duration, autoplay: bool) -> Result<(), String> {
        let sid = SpotifyId::from_base62(id).map_err(|e| format!("Bad Spotify id: {e:?}"))?;
        let uri = SpotifyUri::Track { id: sid };
        if let Ok(mut w) = self.want.lock() {
            *w = Some(uri.clone());
        }
        self.player
            .load(uri, autoplay, start.as_millis() as u32);
        Ok(())
    }

    pub fn pause(&self) {
        self.player.pause();
    }

    pub fn resume(&self) {
        self.player.play();
    }

    pub fn stop(&self) {
        if let Ok(mut w) = self.want.lock() {
            *w = None;
        }
        self.player.stop();
    }

    /// Drains pending events.
    pub fn poll(&self) -> Vec<SpEvent> {
        let _ = &self.rt;
        let mut v = Vec::new();
        while let Ok(e) = self.ev_rx.try_recv() {
            v.push(e);
        }
        v
    }
}
