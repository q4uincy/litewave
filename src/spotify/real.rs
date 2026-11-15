use super::{Entry, RecentPlay, SearchResults, Shelf, ShelfCard, ShelfKind, SpEvent};
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
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        mpsc::{channel, Receiver},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const SCOPES: &str = "streaming user-read-private playlist-read-private playlist-read-collaborative user-library-read user-follow-read user-library-modify user-follow-modify user-read-recently-played user-top-read playlist-modify-public playlist-modify-private";
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

// ---- browser sign-in -----------------------------------------------------

/// A random PKCE verifier, its S256 challenge, and a state value for one sign-in.
fn pkce() -> (String, String, String) {
    let mut rng = rand::thread_rng();
    let verifier: String = (0..64).map(|_| rng.sample(Alphanumeric) as char).collect();
    let state: String = (0..16).map(|_| rng.sample(Alphanumeric) as char).collect();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge, state)
}

fn authorize_url(client_id: &str, challenge: &str, state: &str) -> String {
    format!(
        "https://accounts.spotify.com/authorize?response_type=code&client_id={}&scope={}&redirect_uri={}&code_challenge_method=S256&code_challenge={}&state={}",
        urlencoding::encode(client_id),
        urlencoding::encode(SCOPES),
        urlencoding::encode(REDIRECT_URI),
        challenge,
        state
    )
}

/// Waits until the browser comes back to `listener` with `code=` or `error=` in the address.
/// Returns the open connection (so it can still be answered) and the query string.
fn wait_for_redirect(listener: &TcpListener) -> Result<(TcpStream, String), String> {
    let started = Instant::now();
    loop {
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
            return Ok((s, q));
        }
        // Not the redirect (favicon, preconnect): answer and keep waiting.
        let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
}

/// The authorization code in a redirect's query, after checking `state` and that access was not denied.
fn code_from_query(query: &str, state: &str) -> Result<String, String> {
    let mut code: Option<String> = None;
    let mut got_state: Option<String> = None;
    let mut denied: Option<String> = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let v = urlencoding::decode(v).map(|c| c.into_owned()).unwrap_or_default();
            match k {
                "code" => code = Some(v),
                "state" => got_state = Some(v),
                "error" => denied = Some(v),
                _ => {}
            }
        }
    }
    if let Some(e) = denied {
        return Err(format!("Spotify denied access: {e}"));
    }
    if got_state.as_deref() != Some(state) {
        return Err("Login failed: state mismatch".to_string());
    }
    code.ok_or_else(|| "Login failed: no authorization code received".to_string())
}

/// Answers the browser with a small page and closes the connection.
fn send_page(mut stream: TcpStream, heading: &str, text: &str) {
    let body = format!(
        "<html><head><meta charset='utf-8'><title>litewave</title></head><body style='font-family:sans-serif;text-align:center;margin-top:20vh'><h2>{heading}</h2><p>{text}</p></body></html>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
}

/// Sends the browser on to another address straight away.
fn send_redirect(mut stream: TcpStream, to: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
}

/// Trades an authorization code for tokens.
fn exchange_code(client_id: &str, code: &str, verifier: &str) -> Result<Client, String> {
    let v = token_request(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", REDIRECT_URI),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ])?;
    let tokens = parse_tokens(&v, "")?;
    Ok(Client {
        client_id: client_id.to_string(),
        own: client_id != SHARED_CLIENT_ID,
        tokens: Arc::new(Mutex::new(tokens)),
    })
}

/// One browser sign-in, or two in a row in the same tab (see `Client::login_pair`). One listener on
/// port 8898 serves both; each sign-in has its own state value.
fn sign_in_pair(first_id: &str, second_id: Option<&str>) -> Result<(Client, Option<Client>), String> {
    let (verifier1, challenge1, state1) = pkce();
    let listener = TcpListener::bind("127.0.0.1:8898")
        .map_err(|e| format!("Port 8898 is busy (finish or restart the pending login): {e}"))?;
    webbrowser::open(&authorize_url(first_id, &challenge1, &state1))
        .map_err(|e| format!("Could not open the browser: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let (stream, query) = wait_for_redirect(&listener)?;
    let code1 = match code_from_query(&query, &state1) {
        Ok(c) => c,
        Err(e) => {
            send_page(stream, "Spotify sign-in did not finish", &e);
            return Err(e);
        }
    };
    // Prepare the second sign-in before answering, so the browser can be sent to it at once.
    let next = second_id.map(|id| {
        let (verifier, challenge, state) = pkce();
        let url = authorize_url(id, &challenge, &state);
        (id, verifier, state, url)
    });
    match &next {
        Some((_, _, _, url)) => send_redirect(stream, url),
        None => send_page(stream, "litewave is connected.", "You can close this tab and go back to litewave."),
    }
    let first = exchange_code(first_id, &code1, &verifier1)?;

    let Some((id2, verifier2, state2, _)) = next else {
        return Ok((first, None));
    };
    let (stream, query) = wait_for_redirect(&listener)?;
    let code2 = match code_from_query(&query, &state2) {
        Ok(c) => c,
        Err(e) => {
            send_page(stream, "Spotify sign-in did not finish", &e);
            return Err(e);
        }
    };
    send_page(stream, "litewave is connected.", "You can close this tab and go back to litewave.");
    drop(listener);
    let second = exchange_code(id2, &code2, &verifier2)?;
    Ok((first, Some(second)))
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
        sign_in_pair(client_id, None).map(|(first, _)| first)
    }

    /// Two sign-ins (playback, then the user's own app) in one browser tab: when Spotify sends the
    /// browser back after the first, the same tab is sent straight on to the second sign-in, so there
    /// is no "finished" page and no pause in between. With `second_id` None it is a plain login.
    pub fn login_pair(first_id: &str, second_id: Option<&str>) -> Result<(Client, Option<Client>), String> {
        sign_in_pair(first_id, second_id)
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

    /// Spotify's own "made for you" playlists found by search (Daily Mix 1-6, Discover Weekly ...), for
    /// when the private Home feed does not answer. Only playlists owned by the user `spotify` count,
    /// as in Spotifast.
    pub fn made_for_you_search(&self) -> Result<Vec<ShelfCard>, String> {
        let mut out: Vec<ShelfCard> = Vec::new();
        let mut err: Option<String> = None;
        for term in MADE_FOR_YOU_TERMS {
            let url = format!("{API}/search?q={}&type=playlist&limit=10", urlencoding::encode(term));
            let v = match self.get(&url) {
                Ok(v) => v,
                Err(e) => {
                    err = Some(e);
                    break;
                }
            };
            let mut found: Vec<ShelfCard> = Vec::new();
            for p in v["playlists"]["items"].as_array().into_iter().flatten() {
                let Some(id) = p["id"].as_str() else { continue };
                let Some(name) = p["name"].as_str() else { continue };
                let name = name.trim();
                if !made_for_you_name(term, name)
                    || p["owner"]["id"].as_str() != Some("spotify")
                    || p["uri"].as_str() == Some(DJ_URI)
                {
                    continue;
                }
                let key = name.to_lowercase();
                if out.iter().chain(found.iter()).any(|c| c.entry.name.to_lowercase() == key) {
                    continue;
                }
                let desc = strip_html(p["description"].as_str().unwrap_or(""));
                found.push(ShelfCard {
                    kind: ShelfKind::Playlist,
                    entry: Entry {
                        id: id.to_string(),
                        name: name.to_string(),
                        sub: if desc.is_empty() { "By Spotify".to_string() } else { desc },
                        art: pick_image(&p["images"]),
                    },
                });
            }
            found.sort_by(|a, b| a.entry.name.cmp(&b.entry.name));
            out.extend(found.into_iter().take(6));
        }
        if out.is_empty() {
            Err(err.unwrap_or_else(|| "no Made for you playlists found".to_string()))
        } else {
            Ok(out)
        }
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
                    sub: p["owner"]["display_name"].as_str().unwrap_or("").to_string(),
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

    /// From Spotify's recently played list (newest first): the ids of the playlists / albums / artists
    /// played from (to sort "Recents"), and the albums of those songs as cards for Home.
    pub fn recent_contexts(&self) -> Result<(Vec<String>, Vec<Entry>, Vec<RecentPlay>), String> {
        let v = self.get(&format!("{API}/me/player/recently-played?limit=50"))?;
        let mut ids: Vec<String> = Vec::new();
        let mut albums: Vec<Entry> = Vec::new();
        let mut plays: Vec<RecentPlay> = Vec::new();
        for it in v["items"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            if let Some(track) = track_from(&it["track"], "", None) {
                plays.push(RecentPlay { context: it["context"]["uri"].as_str().map(|s| s.to_string()), track });
            }
            if let Some(uri) = it["context"]["uri"].as_str() {
                let id = uri.rsplit(':').next().unwrap_or("");
                if id.len() == 22 && !ids.iter().any(|x| x == id) {
                    ids.push(id.to_string());
                }
            }
            let t = &it["track"];
            let a = &t["album"];
            if let Some(id) = a["id"].as_str() {
                if !albums.iter().any(|e| e.id == id) {
                    albums.push(Entry {
                        id: id.to_string(),
                        name: a["name"].as_str().unwrap_or("Untitled").to_string(),
                        sub: artists_of(t),
                        art: pick_image(&a["images"]),
                    });
                }
            }
        }
        Ok((ids, albums, plays))
    }

    /// Your most played artists (needs the `user-top-read` permission).
    pub fn top_artists(&self) -> Result<Vec<Entry>, String> {
        let v = self.get(&format!("{API}/me/top/artists?limit=20&time_range=medium_term"))?;
        Ok(v["items"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter_map(|a| {
                Some(Entry {
                    id: a["id"].as_str()?.to_string(),
                    name: a["name"].as_str().unwrap_or("Unknown").to_string(),
                    sub: "Artist".to_string(),
                    art: pick_image(&a["images"]),
                })
            })
            .collect())
    }

    /// Your most played songs for `range` (`short_term` = last 4 weeks, `medium_term` = 6 months,
    /// `long_term` = past year). Needs the `user-top-read` permission.
    pub fn top_tracks(&self, range: &str) -> Result<Vec<Track>, String> {
        let v = self.get(&format!("{API}/me/top/tracks?limit=20&time_range={range}"))?;
        Ok(v["items"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter_map(|t| track_from(t, "", None))
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

/// Writes one progress line to the playback log (Settings > Spotify > Help > open log). Never pass tokens.
pub fn note(msg: &str) {
    init_logging();
    log::info!("litewave: {msg}");
    // Keep the newest Home-shelves failure on its own: the 5-line log above is soon full of playback chatter.
    if msg.starts_with("home shelves") && !msg.contains(" cards") && !msg.contains("found by search") {
        if let Ok(mut l) = LAST_SHELVES_NOTE.lock() {
            *l = msg.to_string();
        }
    }
}

static LAST_SHELVES_NOTE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Why Spotify's Home shelves last failed (empty if nothing was noted).
pub fn last_shelves_note() -> String {
    LAST_SHELVES_NOTE.lock().map(|l| l.clone()).unwrap_or_default()
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
    eq: crate::eq::Eq,
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
        let mut out: Vec<f32> = samples.iter().map(|&x| (x * gain) as f32).collect();
        self.eq.refresh(44100, 2);
        if self.eq.is_active() {
            for (i, v) in out.iter_mut().enumerate() {
                *v = self.eq.sample(*v, i & 1).clamp(-1.0, 1.0);
            }
        }
        crate::audio_tap::push_interleaved(&out, 2, 44100);
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
                                eq: crate::eq::Eq::new(),
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

// ---- Spotify's own Home shelves -------------------------------------------
//
// Spotify's apps get their Home page from a private "Pathfinder" service, not the public Web API.
// This follows the open-source Spotifast (MIT): the `homeSection` query for two shelves by their
// section ids (Made for you, Your top mixes), sent with the playback session's own tokens as headers
// and a Web API access token as `sp_t`. There is no second guess with another token. If the private
// request fails, the app asks `Client::made_for_you_search` for Spotify's own playlists instead.
// Spotify can change or refuse all of this at any time, so every failure just means "no extra
// shelves" (Home keeps showing the user's library) and is written to the log, never shown as a pop-up.

const PATHFINDER: &str = "https://api-partner.spotify.com/pathfinder/v2/query";
/// The web player's persisted `homeSection` query (one shelf by its section id), the one Spotifast sends.
const HOME_SECTION_HASH: &str = "eb3fba2d388cf4fc4d696b1757a58584e9538a3b515ea742e9cc9465807340be";
/// Cards asked for per shelf (Home shows at most ten, whatever the window width).
const HOME_PAGE: usize = 20;
/// Spotify's DJ is a special context that librespot cannot play as a playlist.
const DJ_URI: &str = "spotify:playlist:37i9dQZF1EYkqdzj48dyYq";
/// The two shelves Spotifast reads: section id and title. The title is fixed here (not taken from
/// Spotify's answer) so the rest of the app can recognise the shelf in any language.
const HOME_SECTIONS: [(&str, &str); 2] = [
    ("spotify:section:0JQ5DAUnp4wcj0bCb3wh3S", "Made for you"),
    ("spotify:section:0JQ5DAnM3wGh0gz1MXnu89", "Your top mixes"),
];
/// Names searched for when the private Home feed does not answer.
const MADE_FOR_YOU_TERMS: [&str; 5] = ["Daily Mix", "Discover Weekly", "Release Radar", "On Repeat", "Repeat Rewind"];

/// True when a playlist called `name` is the Spotify playlist searched for with `term`
/// ("Daily Mix" also takes "Daily Mix 1", "Daily Mix 2" ...).
fn made_for_you_name(term: &str, name: &str) -> bool {
    let name = name.trim();
    name.eq_ignore_ascii_case(term)
        || (term == "Daily Mix"
            && name
                .strip_prefix("Daily Mix ")
                .map_or(false, |n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
}

impl SessionHandle {
    /// Spotify's own Home shelves (Made for you, Your top mixes), in that order. Blocking: call it from
    /// a worker thread. `sp_t` is a Web API access token. `Err` holds the seconds Spotify asked us to
    /// wait (0 = none); the reason for any failure is written to the log.
    pub fn home_shelves(&self, sp_t: &str) -> Result<Vec<Shelf>, u64> {
        let session = self.session.clone();
        let auth = self.handle.block_on(async move {
            let token = session.login5().auth_token().await.map_err(|_| ())?;
            let client_token = session.spclient().client_token().await.map_err(|_| ())?;
            Ok::<_, ()>((token.access_token.clone(), client_token, session.country()))
        });
        let (bearer, client_token, country) = match auth {
            Ok(a) => a,
            Err(_) => {
                note("home shelves: could not get the playback connection's tokens");
                return Err(0);
            }
        };
        let zone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_string());

        let mut out: Vec<Shelf> = Vec::new();
        let mut wait = 0u64;
        for (uri, title) in HOME_SECTIONS {
            match fetch_section(&bearer, &client_token, &country, &zone, sp_t, uri, title) {
                Ok(Some(shelf)) if !shelf.cards.is_empty() => {
                    note(&format!("home shelves: {title}: {} cards", shelf.cards.len()));
                    out.push(shelf);
                }
                Ok(_) => {}
                Err(secs) => {
                    note(&format!("home shelves: Spotify asked us to wait {secs}s"));
                    wait = wait.max(secs);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        if out.is_empty() {
            Err(wait)
        } else {
            Ok(out)
        }
    }
}

/// Sends one Pathfinder request. `Ok(None)`: it failed or the answer was not JSON (the reason is
/// logged). `Err(secs)`: Spotify asked us to slow down. No token is ever logged or put in an error.
fn pathfinder(bearer: &str, client_token: &str, body: Value) -> Result<Option<Value>, u64> {
    let res = agent()
        .post(PATHFINDER)
        .set("Authorization", &format!("Bearer {bearer}"))
        .set("client-token", client_token)
        .send_json(body);
    match res {
        Ok(r) => match r.into_json::<Value>() {
            Ok(v) => Ok(Some(v)),
            Err(_) => {
                note("home shelves: Spotify's answer was not JSON");
                Ok(None)
            }
        },
        Err(ureq::Error::Status(429, r)) => Err(r
            .header("Retry-After")
            .and_then(|h| h.parse::<u64>().ok())
            .unwrap_or(30)
            .clamp(5, 600)),
        Err(ureq::Error::Status(code, _)) => {
            note(&format!("home shelves: Spotify answered HTTP {code}"));
            Ok(None)
        }
        Err(_) => {
            note("home shelves: no connection to Spotify");
            Ok(None)
        }
    }
}

/// One shelf by its section id, titled `title`.
fn fetch_section(
    bearer: &str,
    client_token: &str,
    country: &str,
    zone: &str,
    sp_t: &str,
    uri: &str,
    title: &str,
) -> Result<Option<Shelf>, u64> {
    let body = json!({
        "operationName": "homeSection",
        "variables": {
            "uri": uri,
            "country": country,
            "timeZone": zone,
            "sp_t": sp_t,
            "sectionItemsLimit": HOME_PAGE,
            "sectionItemsOffset": 0
        },
        "extensions": {"persistedQuery": {"version": 1, "sha256Hash": HOME_SECTION_HASH}}
    });
    let Some(v) = pathfinder(bearer, client_token, body)? else {
        return Ok(None);
    };
    match parse_section(&v, title) {
        Ok(mut shelf) => {
            shelf.title = title.to_string();
            Ok(Some(shelf))
        }
        Err(e) => {
            note(&format!("home shelves: {title}: {e}"));
            Ok(None)
        }
    }
}

/// Text of a description with its HTML tags and common entities removed.
fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}

/// `spotify:<kind>:<id>` with a 22-character id -> `<id>`.
fn uri_id(uri: &str, kind: &str) -> Option<String> {
    let id = uri.strip_prefix("spotify:")?.strip_prefix(kind)?.strip_prefix(':')?;
    (id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| id.to_string())
}

/// One card (`content.data` of a section item): playlists, albums, artists and podcasts.
/// Anything else is skipped.
fn shelf_card(data: &Value) -> Option<ShelfCard> {
    let uri = data["uri"].as_str()?;
    match data["__typename"].as_str()? {
        "Playlist" => {
            if uri == DJ_URI {
                return None;
            }
            let id = uri_id(uri, "playlist")?;
            let name = data["name"].as_str()?.to_string();
            let desc = strip_html(data["description"].as_str().unwrap_or(""));
            let sub = if !desc.is_empty() {
                desc
            } else {
                match data["ownerV2"]["data"]["name"].as_str() {
                    Some(o) if !o.is_empty() => format!("By {o}"),
                    _ => "Playlist".to_string(),
                }
            };
            let art = pick_image(&data["images"]["items"][0]["sources"]);
            Some(ShelfCard { kind: ShelfKind::Playlist, entry: Entry { id, name, sub, art } })
        }
        "Album" => {
            let id = uri_id(uri, "album")?;
            let name = data["name"].as_str()?.to_string();
            let artists = data["artists"]["items"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x["profile"]["name"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let sub = if artists.is_empty() { "Album".to_string() } else { artists };
            let art = pick_image(&data["coverArt"]["sources"]);
            Some(ShelfCard { kind: ShelfKind::Album, entry: Entry { id, name, sub, art } })
        }
        "Artist" => {
            let id = uri_id(uri, "artist")?;
            let name = data["profile"]["name"].as_str()?.to_string();
            let art = pick_image(&data["visuals"]["avatarImage"]["sources"]);
            Some(ShelfCard { kind: ShelfKind::Artist, entry: Entry { id, name, sub: "Artist".to_string(), art } })
        }
        "Podcast" => {
            let id = uri_id(uri, "show")?;
            let name = data["name"].as_str()?.to_string();
            let sub = match data["publisher"]["name"].as_str() {
                Some(p) if !p.is_empty() => p.to_string(),
                _ => "Podcast".to_string(),
            };
            let art = pick_image(&data["coverArt"]["sources"]);
            Some(ShelfCard { kind: ShelfKind::Show, entry: Entry { id, name, sub, art } })
        }
        _ => None,
    }
}

/// Adds the cards of a section's `sectionItems.items` list to `cards` (duplicates are left out).
fn push_cards(cards: &mut Vec<ShelfCard>, items: Option<&Value>) {
    let Some(items) = items.and_then(|i| i.as_array()) else { return };
    for item in items {
        let Some(data) = item.pointer("/content/data") else { continue };
        if let Some(card) = shelf_card(data) {
            if !cards.iter().any(|c| c.kind == card.kind && c.entry.id == card.entry.id) {
                cards.push(card);
            }
        }
    }
}

fn has_errors(body: &Value) -> bool {
    body.get("errors")
        .map_or(false, |e| !e.is_null() && e.as_array().map_or(true, |a| !a.is_empty()))
}

/// A `homeSection` answer (one shelf) as a shelf: Spotify's own title (or `fallback`) and its cards
/// in Spotify's order. An answer without the sections is an error rather than an empty shelf.
fn parse_section(body: &Value, fallback: &str) -> Result<Shelf, String> {
    if has_errors(body) {
        return Err("Spotify refused the Home request".to_string());
    }
    let sections = body
        .pointer("/data/homeSections/sections")
        .and_then(|s| s.as_array())
        .ok_or_else(|| "Spotify's Home answer was not in the expected shape".to_string())?;
    if sections.is_empty() {
        return Err("Spotify's Home answer had no sections".to_string());
    }
    let title = sections
        .iter()
        .filter_map(|s| s.pointer("/data/title/text").and_then(|t| t.as_str()))
        .map(|t| t.trim())
        .find(|t| !t.is_empty())
        .unwrap_or(fallback)
        .to_string();
    let mut cards: Vec<ShelfCard> = Vec::new();
    for section in sections {
        push_cards(&mut cards, section.pointer("/sectionItems/items"));
    }
    Ok(Shelf { title, subtitle: String::new(), cards })
}

#[cfg(test)]
mod shelf_tests {
    use super::*;

    const A: &str = "1234567890123456789012";
    const B: &str = "2234567890123456789012";

    #[test]
    fn made_for_you_names_match_like_spotifast() {
        assert!(made_for_you_name("Daily Mix", "Daily Mix 3"));
        assert!(made_for_you_name("Daily Mix", "daily mix"));
        assert!(made_for_you_name("Discover Weekly", "Discover Weekly"));
        assert!(!made_for_you_name("Daily Mix", "Daily Mix Remixed"));
        assert!(!made_for_you_name("Daily Mix", "Daily Mix "));
        assert!(!made_for_you_name("Discover Weekly", "Discover Weekly 2"));
    }

    #[test]
    fn reads_card_kinds_and_strips_description_html() {
        let v = json!({"data":{"homeSections":{"sections":[
            {"data":{"title":{"text":"Made for Sam"}},"sectionItems":{"items":[
                {"content":{"data":{"__typename":"Playlist","uri":format!("spotify:playlist:{A}"),"name":"Daily Mix 1",
                    "description":"<a href=spotify:playlist:x>Dua Lipa</a> &amp; more"}}},
                {"content":{"data":{"__typename":"Artist","uri":format!("spotify:artist:{B}"),"profile":{"name":"Sam"},
                    "visuals":{"avatarImage":{"sources":[{"url":"https://i.scdn.co/image/b","width":160}]}}}}},
                {"content":{"data":{"__typename":"NotFound"}}},
                {"content":null}
            ]}}
        ]}}});
        let shelf = parse_section(&v, "fallback").unwrap();
        assert_eq!(shelf.title, "Made for Sam");
        assert_eq!(shelf.cards.len(), 2);
        assert_eq!(shelf.cards[0].entry.sub, "Dua Lipa & more");
        assert_eq!(shelf.cards[1].kind, ShelfKind::Artist);
    }

    #[test]
    fn skips_bad_ids_the_dj_and_duplicates() {
        let v = json!({"data":{"homeSections":{"sections":[{"sectionItems":{"items":[
            {"content":{"data":{"__typename":"Playlist","uri":"spotify:playlist:bad","name":"bad"}}},
            {"content":{"data":{"__typename":"Playlist","uri":DJ_URI,"name":"DJ"}}},
            {"content":{"data":{"__typename":"Playlist","uri":format!("spotify:playlist:{A}"),"name":"One"}}},
            {"content":{"data":{"__typename":"Playlist","uri":format!("spotify:playlist:{A}"),"name":"One again"}}}
        ]}}]}}});
        let shelf = parse_section(&v, "Made for you").unwrap();
        assert_eq!(shelf.title, "Made for you");
        assert_eq!(shelf.cards.len(), 1);
        assert_eq!(shelf.cards[0].entry.name, "One");
        assert_eq!(shelf.cards[0].entry.sub, "By Spotify");
    }
}
