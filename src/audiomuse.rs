//! AudioMuse-AI API: sessions, sonic recommendations and Song Alchemy.
use crate::Song;
use reqwest::{
    blocking::{Client as Http, RequestBuilder, Response},
    header,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, time::Duration};
use url::Url;

#[derive(Clone)]
pub enum Authentication {
    None,
    Login { username: String, password: String },
    Token(String),
}
#[derive(Clone)]
pub struct AudioMuse {
    base: Url,
    http: Http,
    auth: Option<(header::HeaderName, header::HeaderValue)>,
    server: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Mood {
    pub index: usize,
    #[serde(default)]
    pub top_tags: Vec<String>,
    #[serde(default)]
    pub n_songs: usize,
}
#[derive(Deserialize)]
struct Track {
    item_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    album: Option<String>,
}
impl From<Track> for Song {
    fn from(t: Track) -> Self {
        Self {
            id: t.item_id,
            title: t.title.unwrap_or_default(),
            artist: t.author.unwrap_or_default(),
            album: t.album.unwrap_or_default(),
            ..Default::default()
        }
    }
}
#[derive(Clone, Copy)]
pub struct SimilarOptions {
    pub count: usize,
    pub artist_limit: bool,
    pub radius: bool,
    pub mood_similarity: bool,
}
impl AudioMuse {
    pub fn connect(
        base: &str,
        auth: Authentication,
        server: &str,
    ) -> Result<(Self, BTreeMap<String, Vec<Mood>>), String> {
        let mut base = Url::parse(base.trim()).map_err(|_| "Некорректный адрес AudioMuse-AI")?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(
                "Укажите HTTP(S)-адрес AudioMuse-AI без логина, параметров и фрагмента".into(),
            );
        }
        base.set_path(&format!("{}/", base.path().trim_end_matches('/')));
        let http = Http::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        let mut client = Self {
            base,
            http,
            auth: None,
            server: if server.trim().is_empty() {
                None
            } else {
                Some(server.trim().into())
            },
        };
        match auth {
            Authentication::None => (),
            Authentication::Token(token) => {
                if token.trim().is_empty() {
                    return Err("Введите API-токен AudioMuse-AI".into());
                }
                let mut value = header::HeaderValue::from_str(&format!("Bearer {}", token.trim()))
                    .map_err(|_| "Некорректный API-токен")?;
                value.set_sensitive(true);
                client.auth = Some((header::AUTHORIZATION, value));
            }
            Authentication::Login { username, password } => {
                if username.trim().is_empty() || password.is_empty() {
                    return Err("Введите логин и пароль AudioMuse-AI".into());
                }
                let resp = client
                    .http
                    .post(client.endpoint("auth")?)
                    .header("X-Requested-With", "XMLHttpRequest")
                    .json(&json!({"user":username, "password":password}))
                    .send()
                    .map_err(|e| e.without_url().to_string())?;
                if !resp.status().is_success() {
                    return Err(Self::error(resp));
                }
                let cookie = resp
                    .headers()
                    .get_all(header::SET_COOKIE)
                    .iter()
                    .filter_map(|h| h.to_str().ok())
                    .find_map(|s| {
                        s.split(';').next().filter(|s| {
                            s.starts_with("audiomuse_jwt=") && s.len() > "audiomuse_jwt=".len()
                        })
                    })
                    .ok_or("AudioMuse-AI не вернул сессию")?;
                let mut value = header::HeaderValue::from_str(cookie)
                    .map_err(|_| "Некорректная сессия AudioMuse-AI")?;
                value.set_sensitive(true);
                client.auth = Some((header::COOKIE, value));
            }
        }
        let moods = client.moods()?;
        Ok((client, moods))
    }
    fn endpoint(&self, path: &str) -> Result<Url, String> {
        self.base
            .join(path)
            .map_err(|_| "Некорректный адрес AudioMuse-AI".into())
    }
    fn authorized(&self, mut req: RequestBuilder) -> RequestBuilder {
        if let Some((name, value)) = &self.auth {
            req = req.header(name.clone(), value.clone());
        }
        req
    }
    fn error(resp: Response) -> String {
        let status = resp.status();
        let detail = resp.json::<Value>().ok().and_then(|v| {
            v.get("error_message")
                .or_else(|| v.get("error"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        match detail {
            Some(s) => format!("AudioMuse-AI ({status}): {s}"),
            None => format!("AudioMuse-AI вернул HTTP {status}"),
        }
    }
    fn json(&self, req: RequestBuilder) -> Result<Value, String> {
        let response = self
            .authorized(req)
            .send()
            .map_err(|e| e.without_url().to_string())?;
        if !response.status().is_success() {
            return Err(Self::error(response));
        }
        response
            .json()
            .map_err(|e| format!("Некорректный ответ AudioMuse-AI: {}", e.without_url()))
    }
    pub fn moods(&self) -> Result<BTreeMap<String, Vec<Mood>>, String> {
        let req = self.http.get(self.endpoint("api/mood_centroids")?);
        serde_json::from_value(self.json(req)?)
            .map_err(|e| format!("Некорректный список настроений: {e}"))
    }
    fn tracks(value: Value) -> Result<Vec<Song>, String> {
        serde_json::from_value::<Vec<Track>>(value)
            .map(|v| v.into_iter().map(Song::from).collect())
            .map_err(|e| format!("Некорректный список треков AudioMuse-AI: {e}"))
    }
    pub fn similar(
        &self,
        seed: &str,
        mood: Option<(&str, usize)>,
        opts: SimilarOptions,
    ) -> Result<Vec<Song>, String> {
        let mut params = vec![
            ("n", opts.count.clamp(1, 100).to_string()),
            ("eliminate_duplicates", opts.artist_limit.to_string()),
            ("radius_similarity", opts.radius.to_string()),
            ("mood_similarity", opts.mood_similarity.to_string()),
        ];
        if let Some((m, i)) = mood {
            params.push(("mood", m.into()));
            params.push(("centroid_index", i.to_string()));
        } else if !seed.is_empty() {
            params.push(("item_id", seed.into()));
        } else {
            return Err("Выберите трек".into());
        }
        if let Some(server) = &self.server {
            params.push(("server", server.clone()));
        }
        let req = self
            .http
            .get(self.endpoint("api/similar_tracks")?)
            .query(&params);
        Self::tracks(self.json(req)?)
    }
    pub fn artist_mix(
        &self,
        artist: &str,
        count: usize,
        temperature: f32,
    ) -> Result<Vec<Song>, String> {
        if artist.trim().is_empty() {
            return Err("Введите имя исполнителя".into());
        }
        let mut payload = json!({"items":[{"type":"artist", "id":artist.trim(), "op":"ADD"}], "n":count.clamp(1, 100), "temperature":temperature.clamp(0.0, 10.0)});
        if let Some(server) = &self.server {
            payload["server"] = json!(server);
        }
        let req = self.http.post(self.endpoint("api/alchemy")?).json(&payload);
        let value = self.json(req)?;
        Self::tracks(
            value
                .get("results")
                .cloned()
                .ok_or("AudioMuse-AI не вернул список треков")?,
        )
    }
}
