use super::{Entry, REDIRECT_URI};
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
        player::Player as LsPlayer,
    },
};
use rand::{distributions::Alphanumeric, Rng};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{channel, Receiver, Sender},
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

fn track_from(v: &Value, album_fallback: &str) -> Option<Track> {
    if v["type"].as_str().map_or(false, |t| t != "track") {
        return None;
    }
    let id = v["id"].as_str()?;
    let name = v["name"].as_str()?;
    let album = v["album"]["name"].as_str().unwrap_or(album_fallback);
    Some(Track::spotify(
        id,
        name,
        &artists_of(v),
        album,
        v["duration_ms"].as_u64().unwrap_or(0),
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
                track_from(t, "")
            })
            .collect())
    }

    pub fn liked(&self) -> Result<Vec<Track>, String> {
        let items = self.collect(format!("{API}/me/tracks?limit=50"), None)?;
        Ok(items
            .iter()
            .filter_map(|it| track_from(&it["track"], ""))
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
                })
            })
            .collect())
    }

    pub fn album_tracks(&self, id: &str, name: &str) -> Result<Vec<Track>, String> {
        let items = self.collect(format!("{API}/albums/{id}/tracks?limit=50"), None)?;
        Ok(items.iter().filter_map(|t| track_from(t, name)).collect())
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
                })
            })
            .collect())
    }
}

/// Built-in playback through librespot. Owns its own tokio runtime.
pub struct Engine {
    rt: tokio::runtime::Runtime,
    player: Arc<LsPlayer>,
    generation: Arc<AtomicU64>,
    end_tx: Sender<()>,
    end_rx: Receiver<()>,
}

impl Engine {
    pub fn connect(access_token: &str) -> Result<Engine, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let token = access_token.to_string();
        let player = rt.block_on(async move {
            let backend = audio_backend::find(None)
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
        let (end_tx, end_rx) = channel();
        Ok(Engine {
            rt,
            player,
            generation: Arc::new(AtomicU64::new(0)),
            end_tx,
            end_rx,
        })
    }

    pub fn load(&self, id: &str, start: Duration, autoplay: bool) -> Result<(), String> {
        let sid = SpotifyId::from_base62(id).map_err(|e| format!("Bad Spotify id: {e:?}"))?;
        while self.end_rx.try_recv().is_ok() {}
        let g = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.player
            .load(SpotifyUri::Track { id: sid }, autoplay, start.as_millis() as u32);

        let player = self.player.clone();
        let generation = self.generation.clone();
        let tx = self.end_tx.clone();
        self.rt.spawn(async move {
            player.await_end_of_track().await;
            if generation.load(Ordering::SeqCst) == g {
                let _ = tx.send(());
            }
        });
        Ok(())
    }

    pub fn pause(&self) {
        self.player.pause();
    }

    pub fn resume(&self) {
        self.player.play();
    }

    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.player.stop();
    }

    pub fn finished(&self) -> bool {
        self.end_rx.try_recv().is_ok()
    }
}
