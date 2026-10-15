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

use chrono::NaiveDate;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use url::Url;

use crate::dates::DateRange;
use crate::models::{
    ARTIST_NAME_PREFIX, ArtistHistory, ConcertsResponse, DailyResponse, Eras, ErrorBody,
    NowPlayingResponse, OnRepeat, OnThisDay, Play, PlaysResponse, RecentlyPlayedResponse, Rhythm,
    SearchResponse, TopAlbums, TopArtistsResponse, TopOverTime, TopResponse, TopTracksResponse,
    TrackHistory, Wrapped, WrappedRanges,
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

    /// Newest-first page of plays strictly older than `before_ms` and newer
    /// than `after_ms`, limited to those whose track, album or artist name
    /// contains `search`. With both bounds, a search stays inside a date range.
    pub async fn plays(
        &self,
        limit: u32,
        before_ms: Option<i64>,
        after_ms: Option<i64>,
        search: Option<&str>,
    ) -> Result<Vec<Play>, ApiError> {
        let query = plays_query(limit, before_ms, after_ms, search);
        let response: PlaysResponse = self.get(self.endpoint("plays", &query)).await?;
        check_search_applied(search, response.q.as_deref())?;
        Ok(response.plays)
    }

    /// Plays strictly newer than `after_ms`, still newest first.
    pub async fn plays_after(&self, limit: u32, after_ms: i64) -> Result<Vec<Play>, ApiError> {
        let query = [
            ("limit", limit.to_string()),
            ("after", after_ms.to_string()),
        ];
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

    /// Spotify's own last plays, including any the Worker has not stored yet.
    pub async fn recently_played(&self, limit: u32) -> Result<RecentlyPlayedResponse, ApiError> {
        let query = [("limit", limit.clamp(1, 50).to_string())];
        self.get(self.endpoint("recently-played", &query)).await
    }

    /// One artist's whole history. `id` is a Spotify id or a `name:`
    /// placeholder (see [`crate::models::artist_key`]), passed through as is.
    pub async fn artist(&self, id: &str, tz: &str) -> Result<ArtistHistory, ApiError> {
        self.get(self.endpoint("artist", &artist_query(id, tz)))
            .await
    }

    /// One song's whole history, every edition folded together.
    pub async fn track(&self, id: &str, tz: &str) -> Result<TrackHistory, ApiError> {
        let query = [("id", id.to_owned()), ("tz", tz.to_owned())];
        self.get(self.endpoint("track", &query)).await
    }

    /// Stored artists and songs whose name contains `q`; `artist` narrows
    /// the songs to those whose primary artist contains it.
    pub async fn search(
        &self,
        q: &str,
        artist: Option<&str>,
        limit: u32,
    ) -> Result<SearchResponse, ApiError> {
        self.get(self.endpoint("search", &search_query(q, artist, limit)))
            .await
    }

    /// Plays per song over a local-calendar range, most played first.
    pub async fn top_tracks(
        &self,
        range: &DateRange,
        tz: &str,
        limit: u32,
    ) -> Result<TopTracksResponse, ApiError> {
        let mut query = range_query(range, tz);
        query.push(("limit", limit.to_string()));
        self.get(self.endpoint("top-tracks", &query)).await
    }

    /// The most-played songs on `date` (default today) in each earlier year.
    pub async fn on_this_day(
        &self,
        date: Option<NaiveDate>,
        tz: &str,
    ) -> Result<OnThisDay, ApiError> {
        let mut query = vec![("tz", tz.to_owned())];
        if let Some(date) = date {
            query.push(("date", date.format("%Y-%m-%d").to_string()));
        }
        self.get(self.endpoint("on-this-day", &query)).await
    }

    pub async fn on_repeat(&self) -> Result<OnRepeat, ApiError> {
        self.get(self.endpoint(Stat::OnRepeat.route(), &[])).await
    }

    pub async fn rhythm(&self) -> Result<Rhythm, ApiError> {
        self.get(self.endpoint(Stat::Rhythm.route(), &[])).await
    }

    pub async fn top_over_time(&self) -> Result<TopOverTime, ApiError> {
        self.get(self.endpoint(Stat::TopOverTime.route(), &[]))
            .await
    }

    pub async fn eras(&self) -> Result<Eras, ApiError> {
        self.get(self.endpoint(Stat::Eras.route(), &[])).await
    }

    pub async fn top_albums(&self) -> Result<TopAlbums, ApiError> {
        self.get(self.endpoint(Stat::Albums.route(), &[])).await
    }

    /// The ranges `wrapped` will compute for a read token.
    pub async fn wrapped_ranges(&self) -> Result<WrappedRanges, ApiError> {
        self.get(self.endpoint("wrapped/ranges", &[])).await
    }

    /// The summary for a closed range of local days, `YYYY-MM-DD` inclusive.
    /// A read token gets only ranges from `wrapped_ranges` (or ones already
    /// computed); anything else is a 403, [`ApiError::Server`].
    pub async fn wrapped(&self, from: &str, to: &str) -> Result<Wrapped, ApiError> {
        let query = [("from", from.to_owned()), ("to", to.to_owned())];
        self.get(self.endpoint("wrapped", &query)).await
    }

    /// Upcoming concerts, optionally one region's (a key from the
    /// response's `regions`) or only those first seen on or after `since`.
    pub async fn concerts(
        &self,
        region: Option<&str>,
        since: Option<NaiveDate>,
        tz: &str,
    ) -> Result<ConcertsResponse, ApiError> {
        self.get(self.endpoint("concerts", &concerts_query(region, since, tz)))
            .await
    }
}

/// The `/api/stats/*` aggregates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stat {
    OnRepeat,
    Rhythm,
    TopOverTime,
    Eras,
    Albums,
}

impl Stat {
    fn route(self) -> &'static str {
        match self {
            Self::OnRepeat => "stats/on-repeat",
            Self::Rhythm => "stats/rhythm",
            Self::TopOverTime => "stats/top-over-time",
            Self::Eras => "stats/eras",
            Self::Albums => "stats/albums",
        }
    }
}

/// A `name:` placeholder also sends its name, so the Worker can fold it
/// into the real artist when the placeholder row itself is gone.
fn artist_query(id: &str, tz: &str) -> Vec<(&'static str, String)> {
    let mut query = vec![("id", id.to_owned())];
    if let Some(name) = id.strip_prefix(ARTIST_NAME_PREFIX) {
        query.push(("name", name.to_owned()));
    }
    query.push(("tz", tz.to_owned()));
    query
}

fn search_query(q: &str, artist: Option<&str>, limit: u32) -> Vec<(&'static str, String)> {
    let mut query = vec![("q", q.to_owned())];
    if let Some(artist) = artist {
        query.push(("artist", artist.to_owned()));
    }
    query.push(("limit", limit.clamp(1, 50).to_string()));
    query
}

fn concerts_query(
    region: Option<&str>,
    since: Option<NaiveDate>,
    tz: &str,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(region) = region {
        query.push(("region", region.to_owned()));
    }
    if let Some(since) = since {
        query.push(("since", since.format("%Y-%m-%d").to_string()));
        query.push(("tz", tz.to_owned()));
    }
    query
}

/// A Worker older than search ignores `q` and returns the unfiltered list,
/// which would look like a filter that matches everything. Search responses
/// echo `q`, so a missing or different echo means it was not applied.
fn check_search_applied(sent: Option<&str>, echoed: Option<&str>) -> Result<(), ApiError> {
    match sent {
        Some(q) if echoed != Some(q) => Err(ApiError::Decode(
            "this Worker cannot search history. Deploy the latest music-warehouse.".into(),
        )),
        _ => Ok(()),
    }
}

fn plays_query(
    limit: u32,
    before_ms: Option<i64>,
    after_ms: Option<i64>,
    search: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut query = vec![("limit", limit.to_string())];
    if let Some(before) = before_ms {
        query.push(("before", before.to_string()));
    }
    if let Some(after) = after_ms {
        query.push(("after", after.to_string()));
    }
    if let Some(q) = search {
        query.push(("q", q.to_owned()));
    }
    query
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
    fn search_that_was_not_echoed_is_an_error() {
        assert!(check_search_applied(None, None).is_ok());
        assert!(check_search_applied(Some("rosé"), Some("rosé")).is_ok());
        assert!(matches!(
            check_search_applied(Some("rosé"), None),
            Err(ApiError::Decode(_))
        ));
        assert!(check_search_applied(Some("rosé"), Some("rose")).is_err());
    }

    #[test]
    fn plays_search_is_encoded_into_q() {
        let url = client().endpoint(
            "plays",
            &plays_query(200, Some(17), None, Some("Sad & 100%")),
        );
        assert_eq!(
            url.as_str(),
            "https://music.example.com/api/plays?limit=200&before=17&q=Sad+%26+100%25"
        );
        let plain = client().endpoint("plays", &plays_query(50, None, None, None));
        assert_eq!(
            plain.as_str(),
            "https://music.example.com/api/plays?limit=50"
        );
    }

    #[test]
    fn plays_search_can_stay_inside_a_date_range() {
        let url = client().endpoint("plays", &plays_query(100, Some(20), Some(10), Some("dna")));
        assert_eq!(
            url.as_str(),
            "https://music.example.com/api/plays?limit=100&before=20&after=10&q=dna"
        );
        let response: PlaysResponse = decode(&fixture("plays-search-after.json")).unwrap();
        assert_eq!(response.q.as_deref(), Some("kendrick"));
        assert!(!response.plays.is_empty());
    }

    #[test]
    fn history_urls() {
        let c = client();
        assert_eq!(
            c.endpoint(
                "artist",
                &artist_query("2YZyLoL8N0Wb9xBt1NhZWg", "Europe/London")
            )
            .as_str(),
            "https://music.example.com/api/artist?id=2YZyLoL8N0Wb9xBt1NhZWg&tz=Europe%2FLondon"
        );
        // A placeholder also sends its name, so the Worker can fold it.
        assert_eq!(
            c.endpoint("artist", &artist_query("name:Sigur Rós", "UTC"))
                .as_str(),
            "https://music.example.com/api/artist?id=name%3ASigur+R%C3%B3s&name=Sigur+R%C3%B3s&tz=UTC"
        );
        assert_eq!(
            c.endpoint("search", &search_query("a&b", Some("x"), 99))
                .as_str(),
            "https://music.example.com/api/search?q=a%26b&artist=x&limit=50"
        );
        assert_eq!(
            c.endpoint("search", &search_query("dna", None, 10))
                .as_str(),
            "https://music.example.com/api/search?q=dna&limit=10"
        );
    }

    #[test]
    fn stat_and_concert_urls() {
        let c = client();
        assert_eq!(
            c.endpoint(Stat::TopOverTime.route(), &[]).as_str(),
            "https://music.example.com/api/stats/top-over-time"
        );
        assert_eq!(
            c.endpoint(Stat::OnRepeat.route(), &[]).as_str(),
            "https://music.example.com/api/stats/on-repeat"
        );
        assert_eq!(
            c.endpoint("concerts", &concerts_query(None, None, "UTC"))
                .as_str(),
            "https://music.example.com/api/concerts"
        );
        let since = NaiveDate::from_ymd_opt(2026, 10, 1);
        assert_eq!(
            c.endpoint(
                "concerts",
                &concerts_query(Some("pt"), since, "Europe/Lisbon")
            )
            .as_str(),
            "https://music.example.com/api/concerts?region=pt&since=2026-10-01&tz=Europe%2FLisbon"
        );
    }

    #[test]
    fn history_fixtures_decode() {
        let artist: ArtistHistory = decode(&fixture("artist.json")).unwrap();
        assert_eq!(artist.artist.name, "Kendrick Lamar");
        assert!(artist.plays > 0 && artist.minutes > 0);
        assert_eq!(artist.months.len(), 6);
        assert_eq!(artist.top_tracks.len(), 3);
        assert!(artist.top_tracks[0].artist.is_none(), "implied by the page");
        assert!(artist.top_tracks[0].id.is_some());

        let empty: ArtistHistory = decode(&fixture("artist-placeholder.json")).unwrap();
        assert_eq!(empty.plays, 0);
        assert!(empty.first_played.is_none());

        let track: TrackHistory = decode(&fixture("track.json")).unwrap();
        assert_eq!(track.track.name.as_deref(), Some("We Cry Together"));
        assert_eq!(track.months[0].month, "2026-10");

        let top: TopTracksResponse = decode(&fixture("top-tracks.json")).unwrap();
        assert_eq!(top.tracks.len(), 3);
        assert!(top.tracks[0].plays >= top.tracks[1].plays);

        let day: OnThisDay = decode(&fixture("on-this-day.json")).unwrap();
        assert_eq!(day.date, "2026-10-15");
        assert!(day.years[0].year > day.years[1].year, "newest year first");
        assert!(day.years[0].top_tracks[0].artist.is_some());

        let search: SearchResponse = decode(&fixture("search.json")).unwrap();
        assert_eq!(search.artists[0].name, "Kendrick Lamar");
        assert_eq!(search.tracks.len(), 3);

        let recent: RecentlyPlayedResponse = decode(&fixture("recently-played.json")).unwrap();
        assert_eq!(recent.items.len(), 2);
        let first = &recent.items[0];
        assert!(first.played_at.is_some());
        let track = first.track.as_ref().unwrap();
        assert!(track.id.is_some());
        assert!(track.artists[0].id.is_some());
        assert_eq!(
            first.context.as_ref().unwrap().kind.as_deref(),
            Some("album")
        );
    }

    #[test]
    fn stats_fixtures_decode() {
        let repeat: OnRepeat = decode(&fixture("stats-on-repeat.json")).unwrap();
        assert_eq!(repeat.window_days, 14);
        assert_eq!(repeat.tracks[0].daily.len(), 14);
        assert_eq!(repeat.albums[0].tracks_on_repeat, 5);

        let rhythm: Rhythm = decode(&fixture("stats-rhythm.json")).unwrap();
        assert_eq!(rhythm.streaks.len(), 3);
        assert!(rhythm.streaks[0].top[1].days.is_none(), "running streak");
        assert_eq!(rhythm.eras[0].grid.as_ref().unwrap().len(), 7);
        assert_eq!(rhythm.eras[0].hour_share.as_ref().unwrap().len(), 24);
        assert!(rhythm.eras[1].grid.is_none());
        assert!(rhythm.years[1].partial);

        let top: TopOverTime = decode(&fixture("stats-top-over-time.json")).unwrap();
        assert_eq!(top.quarters.len(), 3);
        assert_eq!(top.leaders[0].share.len(), top.quarters.len());
        assert!(top.quarters[2].partial);

        let albums: TopAlbums = decode(&fixture("stats-albums.json")).unwrap();
        assert_eq!(albums.min_songs, 4);
        assert_eq!(albums.years[1].albums[1].image, None);
    }

    #[test]
    fn eras_fixture_decodes_in_main_shape() {
        let eras: Eras = decode(&fixture("stats-eras.json")).unwrap();
        assert_eq!(eras.latest_era.as_ref().unwrap().label, "City B");
        assert_eq!(eras.previous_era.as_ref().unwrap().key, "city-a");
        assert!(eras.ready);
        assert_eq!(eras.summary.as_ref().unwrap().typical_change.windows, 27);
        assert_eq!(eras.top.latest[0].artist, "Kendrick Lamar");
        assert_eq!(eras.risers.len(), 1);

        let single: Eras = decode(&fixture("stats-eras-single.json")).unwrap();
        assert!(single.latest_era.is_none() && single.summary.is_none());

        // The deployed Worker may still send v1 keys; those must not fail.
        let v1: Eras = decode(
            r#"{"as_of":"x","singapore":{"from":"a","to":"b"},"top":{"newcastle":[],"singapore":[]}}"#,
        )
        .unwrap();
        assert!(v1.latest.is_none() && v1.top.latest.is_empty());
    }

    #[test]
    fn wrapped_fixtures_decode() {
        let ranges: WrappedRanges = decode(&fixture("wrapped-ranges.json")).unwrap();
        assert_eq!(ranges.ranges[0].kind, "year_to_date");
        assert!(!ranges.ranges[0].computed);

        let wrapped: Wrapped = decode(&fixture("wrapped.json")).unwrap();
        assert_eq!(wrapped.range.days, 365);
        assert_eq!(wrapped.hours.len(), 24);
        assert_eq!(wrapped.weekdays.len(), 7);
        assert_eq!(wrapped.listening_age.as_ref().unwrap().window, (2015, 2019));
        assert_eq!(wrapped.top_artists[1].top_song, None);
        assert_eq!(wrapped.streak.as_ref().unwrap().days, 64);
    }

    #[test]
    fn concerts_fixture_decodes_with_free_form_regions() {
        let response: ConcertsResponse = decode(&fixture("concerts.json")).unwrap();
        assert_eq!(response.concerts.len(), 3);
        assert_eq!(response.region_label("pt"), "Portugal");
        assert_eq!(response.region_label("unknown"), "unknown");
        assert!(response.checked.contains_key("berlin"));
        let undated = &response.concerts[2];
        assert_eq!(undated.date, None);
        assert_eq!(response.concerts[1].url, None);

        // Workers older than configurable regions send no `regions` list.
        let old: ConcertsResponse =
            decode(r#"{"checked":{},"concerts":[{"region":"x","artists":"a","name":"n","first_seen":"t"}]}"#)
                .unwrap();
        assert!(old.regions.is_empty());
        assert_eq!(old.region_label("x"), "x");
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
