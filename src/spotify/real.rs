use super::{Entry, SearchResults, SpEvent};
use crate::library::Track;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use librespot::{
    connect::{ConnectConfig, LoadRequest, LoadRequestOptions, Spirc},
    core::{
        authentication::Credentials,
        config::{DeviceType, SessionConfig},
        session::Session,
        spotify_id::SpotifyId,
        SpotifyUri,
    },
    metadata::{Metadata, Playlist as LsPlaylist, Track as LsTrack},
    playback::{
        audio_backend,
        config::{AudioFormat, Bitrate, PlayerConfig},
        mixer::{softmixer::SoftMixer, Mixer, MixerConfig},
        player::{Player as LsPlayer, PlayerEvent},
    },
};
use rand::{distributions::Alphanumeric, Rng};
use serde_json::{json, Value};
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

const SCOPES: &str = "streaming user-read-private playlist-read-private playlist-read-collaborative user-library-read user-follow-read user-library-modify user-follow-modify playlist-modify-public playlist-modify-private";
/// The client ID librespot itself uses. It is good for streaming (playback), but Spotify
/// rate-limits its Web API requests almost immediately, so browsing should use the
/// user's own Client ID instead (see `Client::login`).
pub const SHARED_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
/// Redirect URI. Users must add exactly this to their own Spotify app.
pub const REDIRECT_URI: &str = "http://127.0.0.1:8898/login";
const API: &str = "https://api.spotify.com/v1";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const MAX_PAGES: usize = 200;

static NULL: Value = Value::Null;
static OPT_QUALITY: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);
static OPT_NORMALISE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static OPT_GAPLESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Playback options for the next time the Spotify engine starts: quality (0 = 96, 1 = 160,
/// 2 = 320 kbps), volume normalisation and gapless playback.
pub fn set_playback_options(quality: u8, normalise: bool, gapless: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    OPT_QUALITY.store(quality, Relaxed);
    OPT_NORMALISE.store(normalise, Relaxed);
    OPT_GAPLESS.store(gapless, Relaxed);
}

fn player_config() -> PlayerConfig {
    use std::sync::atomic::Ordering::Relaxed;
    PlayerConfig {
        bitrate: match OPT_QUALITY.load(Relaxed) {
            0 => Bitrate::Bitrate96,
            2 => Bitrate::Bitrate320,
            _ => Bitrate::Bitrate160,
        },
        normalisation: OPT_NORMALISE.load(Relaxed),
        gapless: OPT_GAPLESS.load(Relaxed),
        ..PlayerConfig::default()
    }
}
static NOTE: Mutex<String> = Mutex::new(String::new());

fn set_note(s: &str) {
    if let Ok(mut n) = NOTE.lock() {
        *n = s.to_string();
    }
}

/// A short status for the UI while requests are being retried (e.g. rate limiting).
pub fn status_note() -> String {
    NOTE.try_lock().map(|n| n.clone()).unwrap_or_default()
}

struct Tokens {
    access: String,
    refresh: String,
    expires: Instant,
}

/// Spotify explains most errors in the response body, e.g. {"error":{"status":400,"message":"..."}}.
fn api_message(resp: ureq::Response) -> String {
    let body = resp.into_string().unwrap_or_default();
    serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| {
            v["error"]["message"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| v["error_description"].as_str().map(|s| s.to_string()))
                .or_else(|| v["error"].as_str().map(|s| s.to_string()))
        })
        .unwrap_or_default()
}

fn api_error(code: u16, resp: ureq::Response) -> String {
    let m = api_message(resp);
    if m.is_empty() {
        format!("Spotify API error {code}")
    } else {
        format!("Spotify API error {code}: {m}")
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()
}

fn token_request(form: &[(&str, &str)]) -> Result<Value, String> {
    agent()
        .post(TOKEN_URL)
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
    let mut t = Track::spotify(
        id,
        name,
        &artists_of(v),
        album,
        v["duration_ms"].as_u64().unwrap_or(0),
        art,
    );
    t.explicit = v["explicit"].as_bool().unwrap_or(false);
    t.album_id = v["album"]["id"].as_str().map(|s| s.to_string());
    t.artist_id = v["artists"][0]["id"].as_str().map(|s| s.to_string());
    Some(t)
}

#[derive(Clone)]
pub struct Client {
    client_id: String,
    /// True when signed in with the user's own Client ID (not the shared librespot one).
    own: bool,
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
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let started = Instant::now();
        let mut query = String::new();
        let mut stream = loop {
            if started.elapsed() > Duration::from_secs(240) {
                return Err("Spotify login timed out. Click Connect Spotify to try again.".to_string());
            }
            let mut s = match listener.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(e) => return Err(e.to_string()),
            };
            let _ = s.set_nonblocking(false);
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let target = req
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("")
                .to_string();
            let q = target.split_once('?').map(|x| x.1.to_string()).unwrap_or_default();
            if q.contains("code=") || q.contains("error=") {
                query = q;
                break s;
            }
            // Not the redirect (favicon, preconnect): answer and keep waiting.
            let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        };
        let query = query.as_str();

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
            own: client_id != SHARED_CLIENT_ID,
            tokens: Arc::new(Mutex::new(tokens)),
        })
    }

    pub fn from_refresh(refresh: &str, client_id: &str) -> Result<Client, String> {
        let v = token_request(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", client_id),
        ])?;
        let tokens = parse_tokens(&v, refresh)?;
        Ok(Client {
            client_id: client_id.to_string(),
            own: client_id != SHARED_CLIENT_ID,
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
        let refresh = {
            let t = self
                .tokens
                .lock()
                .map_err(|_| "token lock poisoned".to_string())?;
            if Instant::now() < t.expires {
                return Ok(t.access.clone());
            }
            t.refresh.clone()
        };
        // The lock is released here: the UI thread also reads it, and must never wait on the network.
        let v = token_request(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", self.client_id.as_str()),
        ])?;
        let fresh = parse_tokens(&v, &refresh)?;
        let access = fresh.access.clone();
        if let Ok(mut t) = self.tokens.lock() {
            *t = fresh;
        }
        Ok(access)
    }

    fn get(&self, url: &str) -> Result<Value, String> {
        let token = self.access_token()?;
        for attempt in 0..6 {
            match agent()
                .get(url)
                .set("Authorization", &format!("Bearer {token}"))
                .call()
            {
                Ok(r) => {
                    set_note("");
                    return r.into_json::<Value>().map_err(|e| e.to_string());
                }
                Err(ureq::Error::Status(429, r)) => {
                    let wait = r
                        .header("Retry-After")
                        .and_then(|h| h.parse::<u64>().ok())
                        .unwrap_or(3)
                        .clamp(1, 30);
                    if attempt == 5 {
                        set_note("");
                        return Err(if self.own {
                            format!("Spotify is rate-limiting your app (it asked us to wait {wait}s). Wait a minute, then press Refresh.")
                        } else {
                            "Spotify rejects browsing through the shared sign-in. Open the Spotify window and add your own Client ID.".to_string()
                        });
                    }
                    set_note(&format!("Spotify is busy. Retrying in {wait}s ({} of 5)...", attempt + 1));
                    std::thread::sleep(Duration::from_secs(wait));
                }
                Err(ureq::Error::Status(403, _)) => {
                    return Err("Spotify refused this request (403). Apps like this one can only open the songs of playlists you own or collaborate on, so other people's playlists (even ones you saved) and some made by Spotify cannot be read. Tip: add its songs to a playlist of your own in the Spotify app.".to_string())
                }
                Err(ureq::Error::Status(401, _)) => {
                    return Err("Spotify sign-in expired. Open the Spotify window and sign in again.".to_string())
                }
                Err(ureq::Error::Status(404, _)) => {
                    return Err("Spotify API error 404: that item was not found (it may have been removed).".to_string())
                }
                Err(ureq::Error::Status(code, r)) => {
                    return Err(api_error(code, r))
                }
                Err(e) => return Err(format!("Network error: {e}")),
            }
        }
        Err("Spotify is rate limiting requests, try again shortly".to_string())
    }

    /// Follows `next` links, collecting every item. `root` names a wrapper object
    /// (e.g. "artists" for /me/following).
    fn collect(&self, first: String, root: Option<&str>) -> Result<Vec<Value>, String> {
        self.collect_n(first, root, MAX_PAGES)
    }

    fn collect_n(&self, first: String, root: Option<&str>, pages: usize) -> Result<Vec<Value>, String> {
        let mut url = first;
        let mut out = Vec::new();
        for _ in 0..pages {
            let v = self.get(&url)?;
            let node = match root {
                Some(k) => &v[k],
                None => &v,
            };
            if let Some(items) = node["items"].as_array() {
                out.extend(items.iter().cloned());
            }
            match node["next"].as_str() {
                Some(next) => {
                    url = next.to_string();
                    // Spread long listings out so a big library does not trip the rate limit.
                    std::thread::sleep(Duration::from_millis(100));
                }
                None => break,
            }
        }
        Ok(out)
    }

    /// POST / PUT / DELETE with an optional JSON body. Spotify answers many of these with an empty body.
    fn send(&self, method: &str, url: &str, body: Option<Value>) -> Result<Value, String> {
        let token = self.access_token()?;
        let req = agent()
            .request(method, url)
            .set("Authorization", &format!("Bearer {token}"));
        let res = match body {
            Some(b) => req.send_json(b),
            None => req.send_string(""),
        };
        match res {
            Ok(r) => {
                let s = r.into_string().unwrap_or_default();
                Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
            }
            Err(ureq::Error::Status(403, _)) => Err("Spotify refused the change (403). You can only edit playlists you own. If this is your first edit, open the Spotify window and press Sign in again so litewave can ask for edit permission.".to_string()),
            Err(ureq::Error::Status(401, _)) => Err("Spotify sign-in expired. Open the Spotify window and sign in again.".to_string()),
            Err(ureq::Error::Status(429, _)) => Err("Spotify is busy (rate limit). Try again in a minute.".to_string()),
            Err(ureq::Error::Status(code, r)) => Err(api_error(code, r)),
            Err(e) => Err(format!("Network error: {e}")),
        }
    }

    pub fn search(&self, q: &str) -> Result<SearchResults, String> {
        // Spotify caps search at 10 results per type for apps in development mode.
        let url = format!(
            "{API}/search?q={}&type=track,album,artist,playlist,show&limit=10",
            urlencoding::encode(q)
        );
        let v = self.get(&url)?;
        let list = |k: &str| v[k]["items"].as_array().cloned().unwrap_or_default();
        let entries = |k: &str, sub: &dyn Fn(&Value) -> String| -> Vec<Entry> {
            list(k)
                .iter()
                .filter_map(|e| {
                    Some(Entry {
                        id: e["id"].as_str()?.to_string(),
                        name: e["name"].as_str().unwrap_or("Untitled").to_string(),
                        sub: sub(e),
                        art: pick_image(&e["images"]),
                    })
                })
                .collect()
        };
        Ok(SearchResults {
            tracks: list("tracks")
                .iter()
                .filter_map(|t| track_from(t, "", None))
                .collect(),
            artists: entries("artists", &|_| "Artist".to_string()),
            albums: entries("albums", &|a| artists_of(a)),
            playlists: entries("playlists", &|p| {
                p["owner"]["display_name"].as_str().unwrap_or("").to_string()
            }),
            shows: entries("shows", &|_| "Podcast".to_string()),
        })
    }

    pub fn saved_shows(&self) -> Result<Vec<Entry>, String> {
        let items = self.collect(format!("{API}/me/shows?limit=50"), None)?;
        Ok(items
            .iter()
            .filter_map(|it| {
                let s = &it["show"];
                Some(Entry {
                    id: s["id"].as_str()?.to_string(),
                    name: s["name"].as_str().unwrap_or("Untitled").to_string(),
                    sub: "Podcast".to_string(),
                    art: pick_image(&s["images"]),
                })
            })
            .collect())
    }

    /// Newest episodes first (up to 300). Episode ids are stored as `episode:<id>`.
    pub fn show_episodes(&self, id: &str, name: &str) -> Result<Vec<Track>, String> {
        let art = self
            .get(&format!("{API}/shows/{id}"))
            .ok()
            .and_then(|s| pick_image(&s["images"]));
        let items = self.collect_n(format!("{API}/shows/{id}/episodes?limit=50"), None, 6)?;
        Ok(items
            .iter()
            .filter_map(|e| {
                let eid = e["id"].as_str()?;
                let mut t = Track::spotify(
                    &format!("episode:{eid}"),
                    e["name"].as_str().unwrap_or("Episode"),
                    name,
                    e["release_date"].as_str().unwrap_or(""),
                    e["duration_ms"].as_u64().unwrap_or(0),
                    pick_image(&e["images"]).or_else(|| art.clone()),
                );
                t.explicit = e["explicit"].as_bool().unwrap_or(false);
                Some(t)
            })
            .collect())
    }

    /// Playlists the user owns or collaborates on (the only ones Spotify lets us edit).
    pub fn editable_playlists(&self) -> Result<Vec<Entry>, String> {
        let me = self.get(&format!("{API}/me"))?;
        let me_id = me["id"].as_str().unwrap_or("").to_string();
        let items = self.collect(format!("{API}/me/playlists?limit=50"), None)?;
        Ok(items
            .iter()
            .filter(|p| {
                p["collaborative"].as_bool() == Some(true)
                    || (!me_id.is_empty() && p["owner"]["id"].as_str() == Some(me_id.as_str()))
            })
            .filter_map(|p| {
                Some(Entry {
                    id: p["id"].as_str()?.to_string(),
                    name: p["name"].as_str().unwrap_or("Untitled").to_string(),
                    sub: String::new(),
                    art: pick_image(&p["images"]),
                })
            })
            .collect())
    }

    pub fn create_playlist(&self, name: &str) -> Result<Entry, String> {
        let v = self.send(
            "POST",
            &format!("{API}/me/playlists"),
            Some(json!({ "name": name, "public": false })),
        )?;
        Ok(Entry {
            id: v["id"]
                .as_str()
                .ok_or("Spotify did not return the new playlist")?
                .to_string(),
            name: name.to_string(),
            sub: String::new(),
            art: None,
        })
    }

    pub fn rename_playlist(&self, id: &str, name: &str) -> Result<(), String> {
        self.send("PUT", &format!("{API}/playlists/{id}"), Some(json!({ "name": name })))
            .map(|_| ())
    }

    pub fn add_to_playlist(&self, id: &str, uris: &[String]) -> Result<(), String> {
        self.send(
            "POST",
            &format!("{API}/playlists/{id}/items"),
            Some(json!({ "uris": uris })),
        )
        .map(|_| ())
    }

    pub fn remove_from_playlist(&self, id: &str, uris: &[String]) -> Result<(), String> {
        let items: Vec<Value> = uris.iter().map(|u| json!({ "uri": u })).collect();
        self.send(
            "DELETE",
            &format!("{API}/playlists/{id}/items"),
            Some(json!({ "items": items })),
        )
        .map(|_| ())
    }

    /// Saves songs, albums, shows or playlists to the library and follows artists
    /// (Spotify's `PUT /me/library` takes URIs of any kind).
    pub fn library_add(&self, uris: &[String]) -> Result<(), String> {
        let url = format!("{API}/me/library?uris={}", urlencoding::encode(&uris.join(",")));
        self.send("PUT", &url, None).map(|_| ())
    }

    /// Removes from the library / unfollows. Also how a playlist you own is deleted.
    pub fn library_remove(&self, uris: &[String]) -> Result<(), String> {
        let url = format!("{API}/me/library?uris={}", urlencoding::encode(&uris.join(",")));
        self.send("DELETE", &url, None).map(|_| ())
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
        let items = match self.collect(format!("{API}/playlists/{id}/items?limit=50"), None) {
            Ok(v) => v,
            Err(e) if e.contains("error 404") || e.contains("error 400") => {
                self.collect(format!("{API}/playlists/{id}/tracks?limit=50"), None)?
            }
            Err(e) => return Err(e),
        };
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

    pub fn user_name(&self) -> Result<String, String> {
        let me = self.get(&format!("{API}/me"))?;
        Ok(me["display_name"].as_str().unwrap_or("").to_string())
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
            format!("{API}/artists/{id}/albums?include_groups=album,single&limit=10"),
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

static LOG_LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LOGGER: SpLogger = SpLogger;

struct SpLogger;

impl log::Log for SpLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        (m.level() <= log::Level::Warn && m.target().starts_with("librespot"))
            || (m.level() <= log::Level::Info && m.target().starts_with("librespot_playback"))
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("[{}] {}: {}", r.level(), r.target(), r.args());
        if let Ok(mut lines) = LOG_LINES.lock() {
            lines.push(line.clone());
            if lines.len() > 14 {
                lines.remove(0);
            }
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

/// The most recent librespot messages (the cause is usually a line or two before the final error).
pub fn last_log() -> String {
    LOG_LINES
        .lock()
        .map(|l| {
            // Newest first, skipping exact repeats (the retry prints each error twice).
            let mut out: Vec<&str> = Vec::new();
            for line in l.iter().rev() {
                if !out.contains(&line.as_str()) {
                    out.push(line.as_str());
                }
                if out.len() == 5 {
                    break;
                }
            }
            out.reverse();
            out.join("  |  ")
        })
        .unwrap_or_default()
}

fn init_logging() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Some(p) = log_path() {
            if std::fs::metadata(&p).map_or(false, |m| m.len() > 500_000) {
                let _ = std::fs::remove_file(&p);
            }
        }
        if log::set_logger(&LOGGER).is_ok() {
            if let Some(p) = log_path() {
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                    let secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let _ = writeln!(f, "---- litewave playback session (unix time {secs}) ----");
                }
            }
            log::set_max_level(log::LevelFilter::Info);
        }
    });
}

fn short(s: String) -> String {
    s.chars().take(90).collect()
}

// ---- playback ------------------------------------------------------------

/// Built-in playback through librespot. Owns its own tokio runtime.
///
/// With Spotify Connect on, litewave also shows up as a device named "litewave" in every Spotify
/// app on the account. litewave stays in charge of its own queue; the Connect side lets the phone
/// see the current song, pause, seek and change the volume, or start something else here.
pub struct Engine {
    rt: tokio::runtime::Runtime,
    player: Arc<LsPlayer>,
    session: Session,
    mixer: Arc<dyn Mixer>,
    /// Present when litewave is registered as a Spotify Connect device.
    spirc: Option<Spirc>,
    /// The track we are currently expecting events for.
    want: Arc<Mutex<Option<SpotifyUri>>>,
    ev_rx: Receiver<SpEvent>,
    /// True once this device was made the active Spotify Connect device (done once, not per song).
    activated: std::sync::atomic::AtomicBool,
}

/// `spotify:track:<id>` / `spotify:episode:<id>` -> litewave's id (`<id>` / `episode:<id>`).
fn app_id(u: &SpotifyUri) -> Option<String> {
    let s = u.to_uri().ok()?;
    if let Some(x) = s.strip_prefix("spotify:track:") {
        Some(x.to_string())
    } else {
        s.strip_prefix("spotify:episode:").map(|x| format!("episode:{x}"))
    }
}

/// Audio output with a very small buffer, and the volume applied right before the samples are
/// queued. librespot's own rodio backend buffers about half a second *after* the volume, which
/// is why the slider felt late.
struct LowLatencySink {
    sink: rodio::Sink,
    _stream: rodio::OutputStream,
    volume: Box<dyn librespot::playback::mixer::VolumeGetter + Send>,
}

impl librespot::playback::audio_backend::Sink for LowLatencySink {
    fn start(&mut self) -> librespot::playback::audio_backend::SinkResult<()> {
        self.sink.play();
        Ok(())
    }

    fn stop(&mut self) -> librespot::playback::audio_backend::SinkResult<()> {
        self.sink.sleep_until_end();
        self.sink.pause();
        Ok(())
    }

    fn write(
        &mut self,
        packet: librespot::playback::decoder::AudioPacket,
        _converter: &mut librespot::playback::convert::Converter,
    ) -> librespot::playback::audio_backend::SinkResult<()> {
        let samples = packet
            .samples()
            .map_err(|e| librespot::playback::audio_backend::SinkError::OnWrite(format!("{e:?}")))?;
        let gain = self.volume.attenuation_factor();
        let out: Vec<f32> = samples.iter().map(|&x| (x * gain) as f32).collect();
        self.sink
            .append(rodio::buffer::SamplesBuffer::new(2, 44100, out));
        // About 6 chunks (roughly 0.1 s) may wait in the queue.
        while self.sink.len() > 6 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(())
    }
}

fn vol16(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0) as u16
}

impl Engine {
    /// `want_connect`: also register as a Spotify Connect device. If that fails for any reason the
    /// engine is started again without it, so plain playback keeps working.
    pub fn connect(
        access_token: &str,
        ctx: eframe::egui::Context,
        want_connect: bool,
        volume: f32,
    ) -> Result<Engine, String> {
        init_logging();
        if want_connect {
            match Self::start(access_token, ctx.clone(), true, volume) {
                Ok(e) => return Ok(e),
                Err(e) => log::warn!("Spotify Connect could not start, playing without it: {e}"),
            }
        }
        Self::start(access_token, ctx, false, volume)
    }

    fn start(
        access_token: &str,
        ctx: eframe::egui::Context,
        with_connect: bool,
        volume: f32,
    ) -> Result<Engine, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let token = access_token.to_string();
        let (player, session, mixer, spirc) = rt.block_on(async move {
            // Ask for the rodio backend explicitly so audio can never be routed to "pipe".
            let backend = audio_backend::find(Some("rodio".to_string()))
                .or_else(|| audio_backend::find(None))
                .ok_or_else(|| "No audio backend available".to_string())?;
            let session = Session::new(SessionConfig::default(), None);
            let creds = Credentials::with_access_token(&token);
            let mixer: Arc<dyn Mixer> = Arc::new(
                SoftMixer::open(MixerConfig::default()).map_err(|e| format!("Mixer: {e}"))?,
            );
            mixer.set_volume(vol16(volume));
            let format = AudioFormat::default();
            // The volume is applied in LowLatencySink, so the player itself gets a no-op volume.
            let sink_volume = mixer.get_soft_volume();
            let player = LsPlayer::new(
                player_config(),
                session.clone(),
                Box::new(librespot::playback::mixer::NoOpVolume),
                move || -> Box<dyn librespot::playback::audio_backend::Sink> {
                    match rodio::OutputStream::try_default() {
                        Ok((stream, handle)) => match rodio::Sink::try_new(&handle) {
                            Ok(sink) => Box::new(LowLatencySink {
                                sink,
                                _stream: stream,
                                volume: sink_volume,
                            }),
                            Err(_) => backend(None, format),
                        },
                        Err(_) => backend(None, format),
                    }
                },
            );
            let spirc = if with_connect {
                // Spirc signs the session in itself, so the session is not connected here.
                let cfg = ConnectConfig {
                    name: "litewave".to_string(),
                    device_type: DeviceType::Computer,
                    initial_volume: vol16(volume),
                    ..Default::default()
                };
                let (spirc, task) =
                    Spirc::new(cfg, session.clone(), creds, player.clone(), mixer.clone())
                        .await
                        .map_err(|e| format!("Spotify Connect login failed: {e}"))?;
                tokio::spawn(task);
                Some(spirc)
            } else {
                session
                    .connect(creds, false)
                    .await
                    .map_err(|e| format!("Spotify playback login failed (Premium is required): {e}"))?;
                None
            };
            Ok::<_, String>((player, session, mixer, spirc))
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
                    // Something litewave did not ask for started playing: the phone did it.
                    PlayerEvent::Playing { track_id, position_ms, .. } if with_connect => {
                        let Some(id) = app_id(track_id) else { continue };
                        if let Ok(mut g) = w.lock() {
                            *g = Some(track_id.clone());
                        }
                        SpEvent::Remote(id, *position_ms)
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
                ctx.request_repaint();
            }
        });

        // Make sure the soft volume is not left at 0 (Spirc can take the volume from the phone).
        mixer.set_volume(vol16(volume));
        if let Some(sp) = &spirc {
            let _ = sp.set_volume(vol16(volume));
        }

        Ok(Engine {
            rt,
            player,
            session,
            mixer,
            spirc,
            want,
            ev_rx,
            activated: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// True when litewave is registered as a Spotify Connect device.
    pub fn has_connect(&self) -> bool {
        self.spirc.is_some()
    }

    /// A cheap handle that can read playlists through the playback connection from any thread.
    pub fn handle(&self) -> SessionHandle {
        SessionHandle {
            handle: self.rt.handle().clone(),
            session: self.session.clone(),
        }
    }

    pub fn load(&self, id: &str, start: Duration, autoplay: bool) -> Result<(), String> {
        let uri = match id.strip_prefix("episode:") {
            Some(e) => SpotifyUri::Episode {
                id: SpotifyId::from_base62(e).map_err(|e| format!("Bad Spotify id: {e:?}"))?,
            },
            None => SpotifyUri::Track {
                id: SpotifyId::from_base62(id).map_err(|e| format!("Bad Spotify id: {e:?}"))?,
            },
        };
        if let Ok(mut w) = self.want.lock() {
            *w = Some(uri.clone());
        }
        match &self.spirc {
            // Going through Spirc is what makes the phone show this song.
            Some(sp) => {
                let opts = LoadRequestOptions {
                    start_playing: autoplay,
                    seek_to: start.as_millis() as u32,
                    context_options: None,
                    playing_track: None,
                };
                let uri_s = uri.to_uri().map_err(|e| format!("Bad Spotify id: {e}"))?;
                if !self.activated.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    let _ = sp.activate();
                }
                sp.load(LoadRequest::from_tracks(vec![uri_s], opts))
                    .map_err(|e| format!("Spotify Connect could not load it: {e}"))?;
            }
            None => self.player.load(uri, autoplay, start.as_millis() as u32),
        }
        Ok(())
    }

    /// Plays through the player directly, skipping Spotify Connect (used when Connect stalls).
    pub fn load_direct(&self, id: &str, start: Duration, autoplay: bool) -> Result<(), String> {
        let uri = match id.strip_prefix("episode:") {
            Some(e) => SpotifyUri::Episode {
                id: SpotifyId::from_base62(e).map_err(|e| format!("Bad Spotify id: {e:?}"))?,
            },
            None => SpotifyUri::Track {
                id: SpotifyId::from_base62(id).map_err(|e| format!("Bad Spotify id: {e:?}"))?,
            },
        };
        if let Ok(mut w) = self.want.lock() {
            *w = Some(uri.clone());
        }
        self.player.load(uri, autoplay, start.as_millis() as u32);
        Ok(())
    }

    /// Changes the loudness right away (no network). Used while the slider moves.
    pub fn set_volume_local(&self, v: f32) {
        self.mixer.set_volume(vol16(v));
    }

    /// Tells Spotify Connect (the phone) the new volume. Sent less often than the slider moves.
    pub fn set_volume_remote(&self, v: f32) {
        if let Some(sp) = &self.spirc {
            let _ = sp.set_volume(vol16(v));
        }
    }

    pub fn pause(&self) {
        match &self.spirc {
            Some(sp) => {
                let _ = sp.pause();
            }
            None => self.player.pause(),
        }
    }

    pub fn resume(&self) {
        match &self.spirc {
            Some(sp) => {
                let _ = sp.play();
            }
            None => self.player.play(),
        }
    }

    pub fn stop(&self) {
        if let Ok(mut w) = self.want.lock() {
            *w = None;
        }
        self.player.stop();
    }

    /// 0.0 to 1.0. With Connect on, the phone's volume slider moves the same control.
    pub fn set_volume(&self, v: f32) {
        match &self.spirc {
            Some(sp) => {
                let _ = sp.set_volume(vol16(v));
            }
            None => self.mixer.set_volume(vol16(v)),
        }
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

impl Drop for Engine {
    fn drop(&mut self) {
        // Take the "litewave" device off the phone's list straight away.
        if let Some(sp) = &self.spirc {
            let _ = sp.shutdown();
        }
    }
}

/// `spotify:<kind>:<id>` -> `<id>`.
fn id_of(u: &SpotifyUri, kind: &str) -> Option<String> {
    let s = u.to_uri().ok()?;
    s.strip_prefix(&format!("spotify:{kind}:")).map(|x| x.to_string())
}

/// Reads things the Web API refuses (playlists that belong to other people) through the
/// playback connection, which Spotify's own apps use. Blocking: call it from a worker thread.
#[derive(Clone)]
pub struct SessionHandle {
    handle: tokio::runtime::Handle,
    session: Session,
}

/// A librespot track as a litewave track.
fn track_from_ls(t: &LsTrack) -> Option<Track> {
    let tid = id_of(&t.id, "track")?;
    let artists = t
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    // Cover: the image closest to 300 px wide, served from Spotify's image host.
    let cover = t
        .album
        .covers
        .iter()
        .min_by_key(|i| (i.width - 300).abs())
        .map(|i| format!("https://i.scdn.co/image/{}", i.id));
    let mut tr = Track::spotify(&tid, &t.name, &artists, &t.album.name, t.duration.max(0) as u64, cover);
    tr.explicit = t.is_explicit;
    tr.album_id = id_of(&t.album.id, "album");
    tr.artist_id = t.artists.first().and_then(|a| id_of(&a.id, "artist"));
    Some(tr)
}

impl SessionHandle {
    /// One song by litewave id (`<id>` or `episode:<id>`). Used when the phone starts something
    /// litewave does not have in its list. Blocking: call it from a worker thread.
    pub fn track(&self, id: &str) -> Result<Track, String> {
        if id.starts_with("episode:") {
            return Ok(Track::spotify(id, "Podcast episode", "", "", 0, None));
        }
        let uri = SpotifyUri::from_uri(&format!("spotify:track:{id}"))
            .map_err(|e| format!("Bad track id: {e}"))?;
        let session = self.session.clone();
        self.handle.block_on(async move {
            let t = LsTrack::get(&session, &uri).await.map_err(|e| e.to_string())?;
            track_from_ls(&t).ok_or_else(|| "Unreadable track".to_string())
        })
    }

    /// All songs of a playlist (podcast episodes inside it are skipped).
    pub fn playlist(&self, id: &str) -> Result<Vec<Track>, String> {
        let uri = SpotifyUri::from_uri(&format!("spotify:playlist:{id}"))
            .map_err(|e| format!("Bad playlist id: {e}"))?;
        let session = self.session.clone();
        self.handle.block_on(async move {
            let pl = LsPlaylist::get(&session, &uri)
                .await
                .map_err(|e| format!("The playback connection could not open it either: {e}"))?;
            let uris: Vec<SpotifyUri> = pl.tracks().cloned().collect();
            let mut out: Vec<Track> = Vec::new();
            // Ask for 16 songs at a time so a long playlist does not take minutes.
            for chunk in uris.chunks(16) {
                let jobs: Vec<_> = chunk
                    .iter()
                    .cloned()
                    .map(|u| {
                        let s = session.clone();
                        tokio::spawn(async move { LsTrack::get(&s, &u).await })
                    })
                    .collect();
                for job in jobs {
                    let Ok(Ok(t)) = job.await else { continue };
                    let Some(tr) = track_from_ls(&t) else { continue };
                    out.push(tr);
                }
            }
            Ok(out)
        })
    }
}
