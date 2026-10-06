//! HTTP client for the music-warehouse read API.
//!
//! Only `READ_TOKEN` routes are reachable from here by design: the client has
//! no method for `/health`, `/login` or any real `/admin/*` route, so even an
//! admin credential could not trigger a poll or write rows through this app.
//! The one request outside `/api/` is [`ApiClient::token_is_admin`], which
//! asks for an admin path that does not exist so setup can refuse that token.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use url::Url;

use crate::dates::DateRange;
use crate::models::{
    DailyResponse, ErrorBody, NowPlayingResponse, Play, PlaysResponse, TopArtistsResponse,
    TopResponse,
};

/// Bearer token wrapper whose `Debug` never prints the value, so a stray
/// `{:?}` on any struct holding it cannot leak it into logs.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Arc<str>);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Arc::from(value.into().trim()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

/// Every way a request can fail, split by what the UI should say about it.
/// The live routes' documented failures (503 `needs_reauth`, 429, 502) get
/// their own variants because each needs a different message and retry policy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    #[error("The Worker rejected the read token.")]
    Unauthorized,
    #[error("The Worker rejected the request: {0}")]
    BadRequest(String),
    #[error("Spotify needs re-authorizing on the Worker.")]
    NeedsReauth,
    #[error("Spotify is rate limiting the Worker.")]
    RateLimited { retry_after_seconds: Option<u64> },
    #[error("Spotify returned an error: {0}")]
    Upstream(String),
    #[error("The Worker returned HTTP {status}: {message}")]
    Server { status: u16, message: String },
    #[error("Could not reach the Worker: {0}")]
    Network(String),
    #[error("Unexpected response from the Worker: {0}")]
    Decode(String),
}

impl ApiError {
    /// Maps a non-2xx status and its body to an error. Kept free of I/O so the
    /// Worker's documented failure table can be tested against fixtures.
    pub fn from_response(status: u16, body: &str) -> Self {
        let parsed: ErrorBody = serde_json::from_str(body).unwrap_or_default();
        let message = parsed
            .error
            .clone()
            .unwrap_or_else(|| body.chars().take(200).collect());
        match status {
            401 => Self::Unauthorized,
            400 => Self::BadRequest(message),
            429 => Self::RateLimited {
                retry_after_seconds: parsed.retry_after_seconds,
            },
            503 if parsed.needs_reauth => Self::NeedsReauth,
            502 => Self::Upstream(message),
            _ => Self::Server { status, message },
        }
    }

    /// Whether this error came from Spotify being unavailable to the Worker,
    /// as opposed to the Worker itself or the network.
    pub fn is_spotify_side(&self) -> bool {
        matches!(
            self,
            Self::NeedsReauth | Self::RateLimited { .. } | Self::Upstream(_)
        )
    }
}

/// Spotify's three fixed windows for its top lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopRange {
    Short,
    Medium,
    Long,
}

impl TopRange {
    pub const ALL: [TopRange; 3] = [Self::Short, Self::Medium, Self::Long];

    pub fn as_param(self) -> &'static str {
        match self {
            Self::Short => "short_term",
            Self::Medium => "medium_term",
            Self::Long => "long_term",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Short => "4 weeks",
            Self::Medium => "6 months",
            Self::Long => "1 year",
        }
    }
}

/// Normalizes a user-entered Worker URL. Plain HTTP is only accepted for
/// loopback, so a token is never sent in clear over a real network.
pub fn parse_base_url(input: &str) -> Result<Url, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Enter the Worker URL.".into());
    }
    let mut url = Url::parse(trimmed).map_err(|e| format!("Not a valid URL: {e}"))?;
    let host = url.host_str().unwrap_or_default().to_owned();
    if host.is_empty() {
        return Err("The URL needs a host name.".into());
    }
    let loopback = host == "localhost" || host == "127.0.0.1" || host == "[::1]";
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => return Err("Use https:// (plain http is only allowed for localhost).".into()),
        other => return Err(format!("Unsupported scheme {other}://")),
    }
    url.set_query(None);
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url)
}

#[derive(Clone)]
pub struct ApiClient {
    base: Url,
    token: Secret,
    http: reqwest::Client,
}

impl fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiClient")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

impl ApiClient {
    pub fn new(base: Url, token: Secret) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("music-warehouse-gui/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client with static configuration");
        Self { base, token, http }
    }

    pub fn base_url(&self) -> &Url {
        &self.base
    }

    /// Builds `<base>/api/<route>?<query>`, preserving any path prefix on the
    /// base URL in case the Worker is mounted below the root.
    pub fn endpoint(&self, route: &str, query: &[(&str, String)]) -> Url {
        let mut url = self.base.clone();
        let path = format!("{}/api/{route}", self.base.path().trim_end_matches('/'));
        url.set_path(&path);
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        url
    }

    async fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T, ApiError> {
        let response = self
            .http
            .get(url)
            .bearer_auth(self.token.expose())
            .send()
            .await
            // reqwest's error text includes the URL but never headers, so it
            // is safe to surface; the token only travels in a header.
            .map_err(|e| ApiError::Network(e.without_url().to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| ApiError::Network(e.without_url().to_string()))?;
        if status != StatusCode::OK {
            return Err(ApiError::from_response(status.as_u16(), &body));
        }
        decode(&body)
    }

    /// Newest-first page of plays strictly older than `before_ms`.
    pub async fn plays(&self, limit: u32, before_ms: Option<i64>) -> Result<Vec<Play>, ApiError> {
        let mut query = vec![("limit", limit.to_string())];
        if let Some(before) = before_ms {
            query.push(("before", before.to_string()));
        }
        let response: PlaysResponse = self.get(self.endpoint("plays", &query)).await?;
        Ok(response.plays)
    }

    pub async fn daily(&self, range: &DateRange, tz: &str) -> Result<DailyResponse, ApiError> {
        let query = range_query(range, tz);
        self.get(self.endpoint("daily", &query)).await
    }

    pub async fn top_artists(
        &self,
        range: &DateRange,
        tz: &str,
        limit: u32,
    ) -> Result<TopArtistsResponse, ApiError> {
        let mut query = range_query(range, tz);
        query.push(("limit", limit.to_string()));
        self.get(self.endpoint("top-artists", &query)).await
    }

    pub async fn now_playing(&self) -> Result<NowPlayingResponse, ApiError> {
        self.get(self.endpoint("now-playing", &[])).await
    }

    /// Whether the token is the Worker's ADMIN_TOKEN, which also opens
    /// `/api/*` and so would pass every other check. Asks for an `/admin/`
    /// path that does not exist: the Worker answers 401 to a read token
    /// before routing, and 404 only once a token has admin access. Nothing
    /// is read, written or polled either way.
    pub async fn token_is_admin(&self) -> Result<bool, ApiError> {
        let mut url = self.base.clone();
        let path = format!(
            "{}/admin/mwgui-token-probe",
            self.base.path().trim_end_matches('/')
        );
        url.set_path(&path);
        let response = self
            .http
            .get(url)
            .bearer_auth(self.token.expose())
            .send()
            .await
            .map_err(|e| ApiError::Network(e.without_url().to_string()))?;
        Ok(admin_probe_verdict(response.status().as_u16()))
    }

    pub async fn top(&self, range: TopRange, limit: u32) -> Result<TopResponse, ApiError> {
        let query = [
            ("range", range.as_param().to_owned()),
            ("limit", limit.to_string()),
        ];
        self.get(self.endpoint("top", &query)).await
    }
}

fn range_query(range: &DateRange, tz: &str) -> Vec<(&'static str, String)> {
    vec![
        ("from", range.query_from()),
        ("to", range.query_to()),
        ("tz", tz.to_owned()),
    ]
}

/// Only a 404 proves admin access; anything else (401, an edge error page)
/// is treated as "not admin" so a flaky probe never blocks setup.
fn admin_probe_verdict(status: u16) -> bool {
    status == 404
}

pub fn decode<T: DeserializeOwned>(body: &str) -> Result<T, ApiError> {
    serde_json::from_str(body).map_err(|e| ApiError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use chrono::NaiveDate;

    fn fixture(name: &str) -> String {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn client() -> ApiClient {
        ApiClient::new(
            parse_base_url("https://music.example.com/").unwrap(),
            Secret::new("t"),
        )
    }

    #[test]
    fn plays_fixture_decodes_including_export_rows_with_nulls() {
        let response: PlaysResponse = decode(&fixture("plays.json")).unwrap();
        assert_eq!(response.plays.len(), 5);
        let first = &response.plays[0];
        assert_eq!(first.track_name.as_deref(), Some("We Cry Together"));
        assert_eq!(
            first.artists.as_deref(),
            Some("Kendrick Lamar, Taylour Paige")
        );
        assert_eq!(first.source.as_deref(), Some("api"));
        let export = &response.plays[4];
        assert_eq!(export.source.as_deref(), Some("export"));
        assert_eq!(export.album_name, None);
        assert_eq!(export.duration_ms, None);
        assert!(
            response
                .plays
                .windows(2)
                .all(|w| w[0].played_at_ms > w[1].played_at_ms),
            "the Worker returns newest first"
        );
    }

    #[test]
    fn daily_fixture_decodes() {
        let response: DailyResponse = decode(&fixture("daily.json")).unwrap();
        assert_eq!(response.days.len(), 7);
        assert_eq!(response.days[0].day, "2026-09-30");
        assert_eq!(response.days[0].plays, 9);
    }

    #[test]
    fn top_artists_fixture_decodes() {
        let response: TopArtistsResponse = decode(&fixture("top-artists.json")).unwrap();
        assert_eq!(response.artists.len(), 5);
        assert_eq!(response.artists[0].name, "Kendrick Lamar");
        assert_eq!(response.artists[0].plays, 424);
    }

    #[test]
    fn now_playing_fixture_decodes() {
        let response: NowPlayingResponse = decode(&fixture("now-playing.json")).unwrap();
        let current = response.item.expect("something was playing at capture");
        assert!(current.is_playing);
        assert_eq!(current.currently_playing_type.as_deref(), Some("track"));
        let item = current.item.unwrap();
        assert_eq!(join_names(&item.artists), "Kendrick Lamar");
        let thumb = pick_image(item.artwork(), 48).unwrap();
        assert!(
            thumb.contains("ab67616d00004851"),
            "64px rendition: {thumb}"
        );
        assert!(item.album.unwrap().name.is_some());
    }

    #[test]
    fn idle_now_playing_is_none() {
        let response: NowPlayingResponse = decode(&fixture("now-playing-idle.json")).unwrap();
        assert!(response.item.is_none());
    }

    #[test]
    fn top_fixture_decodes() {
        let response: TopResponse = decode(&fixture("top.json")).unwrap();
        assert_eq!(response.tracks.items.len(), 2);
        assert_eq!(response.artists.items.len(), 2);
        assert!(response.tracks.items[0].name.is_some());
        assert_eq!(response.artists.items[0].images.len(), 3);
        assert!(response.tracks.items[0].album.is_some());
    }

    #[test]
    fn worker_failure_table_maps_to_distinct_errors() {
        assert_eq!(
            ApiError::from_response(503, &fixture("error-503-needs-reauth.json")),
            ApiError::NeedsReauth
        );
        assert_eq!(
            ApiError::from_response(429, &fixture("error-429-rate-limited.json")),
            ApiError::RateLimited {
                retry_after_seconds: Some(30)
            }
        );
        assert_eq!(
            ApiError::from_response(502, &fixture("error-502-upstream.json")),
            ApiError::Upstream("Spotify upstream: http".into())
        );
        assert_eq!(
            ApiError::from_response(401, &fixture("error-401.json")),
            ApiError::Unauthorized
        );
        assert_eq!(
            ApiError::from_response(400, &fixture("error-400-bad-request.json")),
            ApiError::BadRequest("Unknown IANA timezone: Mars/Base".into())
        );
    }

    #[test]
    fn non_json_error_bodies_still_classify() {
        // A Cloudflare edge page in front of the Worker returns HTML, not JSON.
        let error = ApiError::from_response(500, "<html>oops</html>");
        assert_eq!(
            error,
            ApiError::Server {
                status: 500,
                message: "<html>oops</html>".into()
            }
        );
        // A 503 without the flag is a plain server error, not a reauth prompt.
        assert!(matches!(
            ApiError::from_response(503, "{}"),
            ApiError::Server { status: 503, .. }
        ));
    }

    #[test]
    fn endpoint_builds_query_and_keeps_path_prefix() {
        let url = client().endpoint("plays", &[("limit", "200".into()), ("before", "17".into())]);
        assert_eq!(
            url.as_str(),
            "https://music.example.com/api/plays?limit=200&before=17"
        );

        let prefixed = ApiClient::new(
            parse_base_url("https://example.com/warehouse/").unwrap(),
            Secret::new("t"),
        );
        assert_eq!(
            prefixed.endpoint("now-playing", &[]).as_str(),
            "https://example.com/warehouse/api/now-playing"
        );
    }

    #[test]
    fn range_query_encodes_timezone() {
        let range = DateRange::new(
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
        )
        .unwrap();
        let url = client().endpoint("daily", &range_query(&range, "America/Los_Angeles"));
        assert_eq!(
            url.as_str(),
            "https://music.example.com/api/daily?from=2026-09-01&to=2026-09-30&tz=America%2FLos_Angeles"
        );
    }

    #[test]
    fn base_url_rules() {
        assert_eq!(
            parse_base_url(" https://music-api.example.com/ ")
                .unwrap()
                .as_str(),
            "https://music-api.example.com/"
        );
        assert!(parse_base_url("http://music-api.example.com").is_err());
        assert!(parse_base_url("http://127.0.0.1:8787").is_ok());
        assert!(parse_base_url("ftp://example.com").is_err());
        assert!(parse_base_url("").is_err());
        assert!(parse_base_url("not a url").is_err());
    }

    #[test]
    fn admin_probe_only_flags_routed_requests() {
        assert!(admin_probe_verdict(404));
        assert!(!admin_probe_verdict(401));
        assert!(!admin_probe_verdict(502));
    }

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::new("super-secret-value");
        assert!(!format!("{secret:?}").contains("super"));
        let client = ApiClient::new(parse_base_url("https://x.example").unwrap(), secret);
        assert!(!format!("{client:?}").contains("super"));
    }
}
