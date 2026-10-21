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
    FilterEcho, NowPlayingResponse, OnRepeat, OnThisDay, Play, PlaysResponse,
    RecentlyPlayedResponse, Rhythm, SearchResponse, TopAlbums, TopArtistsResponse, TopOverTime,
    TopResponse, TopTracksResponse, TrackHistory, Wrapped, WrappedRanges,
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
    /// A 503 with `Retry-After` and no `needs_reauth`: the Worker is
    /// computing a summary for the first time and another request holds
    /// the claim. Asking again after the wait answers it.
    #[error("The Worker is still computing this.")]
    StillComputing { retry_after_seconds: u64 },
    #[error("The Worker returned HTTP {status}: {message}")]
    Server { status: u16, message: String },
    #[error("Could not reach the Worker: {0}")]
    Network(String),
    #[error("Unexpected response from the Worker: {0}")]
    Decode(String),
    /// A download could not be written to disk.
    #[error("Could not write the file: {0}")]
    File(String),
}

impl ApiError {
    /// Maps a non-2xx status, its `Retry-After` header and its body to an
    /// error. Kept free of I/O so the Worker's documented failure table can
    /// be tested against fixtures.
    pub fn from_response(status: u16, retry_after: Option<u64>, body: &str) -> Self {
        let parsed: ErrorBody = serde_json::from_str(body).unwrap_or_default();
        let message = parsed
            .error
            .clone()
            .unwrap_or_else(|| body.chars().take(200).collect());
        match (status, retry_after) {
            (401, _) => Self::Unauthorized,
            (400, _) => Self::BadRequest(message),
            (429, _) => Self::RateLimited {
                retry_after_seconds: parsed.retry_after_seconds,
            },
            (503, _) if parsed.needs_reauth => Self::NeedsReauth,
            (503, Some(retry_after_seconds)) => Self::StillComputing {
                retry_after_seconds,
            },
            (502, _) => Self::Upstream(message),
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

    /// The token this client sends, so Settings can show it as the saved one
    /// without reading the Keychain again.
    pub fn token(&self) -> &Secret {
        &self.token
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
        self.get_within(url, None).await
    }

    /// [`get`](Self::get) with its own time limit in place of the client's
    /// 20 s, for a route that may compute before it answers.
    async fn get_within<T: DeserializeOwned>(
        &self,
        url: Url,
        timeout: Option<Duration>,
    ) -> Result<T, ApiError> {
        let mut request = self.http.get(url).bearer_auth(self.token.expose());
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let response = request
            .send()
            .await
            // reqwest's error text includes the URL but never headers, so it
            // is safe to surface; the token only travels in a header.
            .map_err(|e| ApiError::Network(e.without_url().to_string()))?;
        let status = response.status();
        let retry_after = retry_after_header(&response);
        let body = response
            .text()
            .await
            .map_err(|e| ApiError::Network(e.without_url().to_string()))?;
        if status != StatusCode::OK {
            return Err(ApiError::from_response(status.as_u16(), retry_after, &body));
        }
        decode(&body)
    }

    /// Newest-first page of plays strictly older than `before` and newer
    /// than `after_ms`, limited to those `filter` allows. With both bounds,
    /// a filtered list stays inside a date range.
    pub async fn plays(
        &self,
        limit: u32,
        before: Option<Before>,
        after_ms: Option<i64>,
        filter: &PlayFilter,
    ) -> Result<PlaysPage, ApiError> {
        let query = plays_query(limit, before, after_ms, filter);
        let response: PlaysResponse = self.get(self.endpoint("plays", &query)).await?;
        filter.check_applied(&response.filter)?;
        Ok(PlaysPage {
            plays: response.plays,
            next_before: response.next_before.clone().flatten(),
            last: response.next_before == Some(None),
        })
    }

    /// The URL of every play `filter` allows between the bounds as one CSV
    /// download, for [`ApiClient::download`]. Same bounds as [`plays`].
    ///
    /// [`plays`]: ApiClient::plays
    pub fn plays_csv_url(
        &self,
        before_ms: Option<i64>,
        after_ms: Option<i64>,
        filter: &PlayFilter,
    ) -> Url {
        self.endpoint("plays.csv", &bounds_query(before_ms, after_ms, filter))
    }

    /// Streams `url` (one of this client's endpoints) into `file` a chunk at
    /// a time, so a long history never sits in memory, calling `progress`
    /// with the bytes written so far. Returns the total bytes.
    pub async fn download(
        &self,
        url: Url,
        file: &mut (impl tokio::io::AsyncWrite + Unpin),
        progress: impl Fn(&[u8]),
    ) -> Result<u64, ApiError> {
        use tokio::io::AsyncWriteExt as _;
        let mut response = self
            .http
            .get(url)
            .bearer_auth(self.token.expose())
            // The client's 20 s limit covers the whole body, and a whole
            // history can take minutes to stream.
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|e| ApiError::Network(e.without_url().to_string()))?;
        let status = response.status();
        if status != StatusCode::OK {
            let retry_after = retry_after_header(&response);
            let body = response.text().await.unwrap_or_default();
            return Err(ApiError::from_response(status.as_u16(), retry_after, &body));
        }
        let mut written = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ApiError::Network(e.without_url().to_string()))?
        {
            file.write_all(&chunk)
                .await
                .map_err(|e| ApiError::File(e.to_string()))?;
            written += chunk.len() as u64;
            progress(&chunk);
        }
        file.flush()
            .await
            .map_err(|e| ApiError::File(e.to_string()))?;
        Ok(written)
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

    /// Plays per local day, everyone's or (with a song or artist in
    /// `filter`) one song's or artist's. `/api/daily` takes no `q`.
    pub async fn daily(
        &self,
        range: &DateRange,
        tz: &str,
        filter: &PlayFilter,
    ) -> Result<DailyResponse, ApiError> {
        debug_assert!(filter.q.is_none(), "/api/daily has no text search");
        let query = daily_query(range, tz, filter);
        let response: DailyResponse = self.get(self.endpoint("daily", &query)).await?;
        filter.check_applied(&response.filter)?;
        Ok(response)
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
        // A range never computed is summarised inline, which over years of
        // plays can outlast the usual limit.
        self.get_within(self.endpoint("wrapped", &query), Some(WRAPPED_TIMEOUT))
            .await
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

/// Which plays `/api/plays`, `/api/plays.csv` and `/api/daily` count: a text
/// search, one song (every edition), one artist (primary credits), or a
/// search inside a song or artist. Empty means everyone's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayFilter {
    pub q: Option<String>,
    pub track_id: Option<String>,
    pub artist_id: Option<String>,
    /// Sent with a `name:` placeholder, so the Worker can still find the
    /// artist by name when the placeholder row itself is gone.
    pub artist_name: Option<String>,
}

impl PlayFilter {
    /// Every play of the song `track_id` is one edition of.
    pub fn song(track_id: impl Into<String>) -> Self {
        Self {
            track_id: Some(track_id.into()),
            ..Self::default()
        }
    }

    /// Every play crediting the artist first. `id` is a Spotify id or a
    /// `name:` placeholder, as [`crate::actions::OpenArtist`] carries.
    pub fn artist(id: &str) -> Self {
        Self {
            artist_id: Some(id.to_owned()),
            artist_name: id.strip_prefix(ARTIST_NAME_PREFIX).map(str::to_owned),
            ..Self::default()
        }
    }

    /// Narrows to plays whose track, album or artist name contains `q`.
    pub fn search(mut self, q: Option<&str>) -> Self {
        self.q = q.filter(|q| !q.is_empty()).map(str::to_owned);
        self
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn fields(&self) -> [(&'static str, Option<&str>); 4] {
        [
            ("q", self.q.as_deref()),
            ("track_id", self.track_id.as_deref()),
            ("artist_id", self.artist_id.as_deref()),
            ("artist_name", self.artist_name.as_deref()),
        ]
    }

    fn query(&self) -> Vec<(&'static str, String)> {
        self.fields()
            .into_iter()
            .filter_map(|(name, value)| Some((name, value?.to_owned())))
            .collect()
    }

    /// A Worker older than a filter ignores it and returns everyone's plays,
    /// which would look like a filter that matches everything. Responses
    /// echo each filter applied, so a missing or different echo means one
    /// was not.
    fn check_applied(&self, echo: &FilterEcho) -> Result<(), ApiError> {
        let echoed = [
            echo.q.as_deref(),
            echo.track_id.as_deref(),
            echo.artist_id.as_deref(),
            echo.artist_name.as_deref(),
        ];
        for ((name, sent), echoed) in self.fields().into_iter().zip(echoed) {
            // The Worker trims what it is sent.
            if let Some(sent) = sent
                && echoed != Some(sent.trim())
            {
                let what = if name == "q" {
                    "search history"
                } else {
                    "list one song's or artist's plays"
                };
                return Err(ApiError::Decode(format!(
                    "this Worker cannot {what}. Deploy the latest music-warehouse."
                )));
            }
        }
        Ok(())
    }
}

/// The longest a Wrapped summary may take, computing included.
const WRAPPED_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// The longest a CSV download may take. A whole history is a few hundred
/// thousand rows, which the Worker streams in a minute or two.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Where a page of plays ends (exclusive): a time, or the cursor the Worker
/// returned with the previous page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Before {
    Ms(i64),
    /// Opaque; sent back exactly as received.
    Cursor(String),
}

/// One page of `/api/plays`, and the cursor for the page after it when the
/// Worker sends one (older Workers do not, and none is sent after the last).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaysPage {
    pub plays: Vec<Play>,
    pub next_before: Option<String>,
    /// The Worker said no page follows (`next_before: null`). False from a
    /// Worker older than cursors, which says nothing either way.
    pub last: bool,
}

fn plays_query(
    limit: u32,
    before: Option<Before>,
    after_ms: Option<i64>,
    filter: &PlayFilter,
) -> Vec<(&'static str, String)> {
    let mut query = vec![("limit", limit.to_string())];
    match before {
        Some(Before::Ms(ms)) => query.extend(bounds_query(Some(ms), after_ms, filter)),
        Some(Before::Cursor(cursor)) => {
            query.push(("before", cursor));
            query.extend(bounds_query(None, after_ms, filter));
        }
        None => query.extend(bounds_query(None, after_ms, filter)),
    }
    query
}

fn bounds_query(
    before_ms: Option<i64>,
    after_ms: Option<i64>,
    filter: &PlayFilter,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(before) = before_ms {
        query.push(("before", before.to_string()));
    }
    if let Some(after) = after_ms {
        query.push(("after", after.to_string()));
    }
    query.extend(filter.query());
    query
}

fn daily_query(range: &DateRange, tz: &str, filter: &PlayFilter) -> Vec<(&'static str, String)> {
    let mut query = range_query(range, tz);
    query.extend(filter.query());
    query
}

fn range_query(range: &DateRange, tz: &str) -> Vec<(&'static str, String)> {
    vec![
        ("from", range.query_from()),
        ("to", range.query_to()),
        ("tz", tz.to_owned()),
    ]
}

/// `Retry-After` in seconds. The Worker only sends the delay form, never an
/// HTTP date, so anything else is treated as absent.
fn retry_after_header(response: &reqwest::Response) -> Option<u64> {
    parse_retry_after(
        response
            .headers()
            .get(reqwest::header::RETRY_AFTER)?
            .to_str()
            .ok()?,
    )
}

fn parse_retry_after(value: &str) -> Option<u64> {
    value.trim().parse().ok()
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
            ApiError::from_response(503, None, &fixture("error-503-needs-reauth.json")),
            ApiError::NeedsReauth
        );
        assert_eq!(
            ApiError::from_response(429, None, &fixture("error-429-rate-limited.json")),
            ApiError::RateLimited {
                retry_after_seconds: Some(30)
            }
        );
        assert_eq!(
            ApiError::from_response(502, None, &fixture("error-502-upstream.json")),
            ApiError::Upstream("Spotify upstream: http".into())
        );
        assert_eq!(
            ApiError::from_response(401, None, &fixture("error-401.json")),
            ApiError::Unauthorized
        );
        assert_eq!(
            ApiError::from_response(400, None, &fixture("error-400-bad-request.json")),
            ApiError::BadRequest("Unknown IANA timezone: Mars/Base".into())
        );
    }

    #[test]
    fn a_503_with_retry_after_is_still_computing() {
        let body = r#"{"error":"Stats are being computed; try again shortly."}"#;
        assert_eq!(
            ApiError::from_response(503, Some(42), body),
            ApiError::StillComputing {
                retry_after_seconds: 42
            }
        );
        // needs_reauth wins over a Retry-After, and a bare 503 stays generic.
        assert_eq!(
            ApiError::from_response(503, Some(5), &fixture("error-503-needs-reauth.json")),
            ApiError::NeedsReauth
        );
        assert!(matches!(
            ApiError::from_response(503, None, body),
            ApiError::Server { status: 503, .. }
        ));
        assert_eq!(parse_retry_after(" 120 "), Some(120));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    }

    #[test]
    fn non_json_error_bodies_still_classify() {
        // A Cloudflare edge page in front of the Worker returns HTML, not JSON.
        let error = ApiError::from_response(500, None, "<html>oops</html>");
        assert_eq!(
            error,
            ApiError::Server {
                status: 500,
                message: "<html>oops</html>".into()
            }
        );
        // A 503 without the flag is a plain server error, not a reauth prompt.
        assert!(matches!(
            ApiError::from_response(503, None, "{}"),
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

    fn search(q: &str) -> PlayFilter {
        PlayFilter::default().search(Some(q))
    }

    fn echo(filter: &PlayFilter) -> FilterEcho {
        FilterEcho {
            q: filter.q.clone(),
            track_id: filter.track_id.clone(),
            artist_id: filter.artist_id.clone(),
            artist_name: filter.artist_name.clone(),
        }
    }

    #[test]
    fn a_filter_that_was_not_echoed_is_an_error() {
        let none = PlayFilter::default();
        assert!(none.check_applied(&FilterEcho::default()).is_ok());
        assert!(search("rosé").check_applied(&echo(&search("rosé"))).is_ok());
        assert!(matches!(
            search("rosé").check_applied(&FilterEcho::default()),
            Err(ApiError::Decode(_))
        ));
        assert!(
            search("rosé")
                .check_applied(&echo(&search("rose")))
                .is_err()
        );

        // An older Worker echoes the search but not the song it ignored.
        let both = PlayFilter::song("t1").search(Some("dna"));
        assert!(both.check_applied(&echo(&both)).is_ok());
        let Err(ApiError::Decode(message)) = both.check_applied(&echo(&search("dna"))) else {
            panic!("an ignored song filter must fail");
        };
        assert!(message.contains("song"), "{message}");
    }

    #[test]
    fn plays_search_is_encoded_into_q() {
        let url = client().endpoint(
            "plays",
            &plays_query(200, Some(Before::Ms(17)), None, &search("Sad & 100%")),
        );
        assert_eq!(
            url.as_str(),
            "https://music.example.com/api/plays?limit=200&before=17&q=Sad+%26+100%25"
        );
        let plain = client().endpoint(
            "plays",
            &plays_query(50, None, None, &PlayFilter::default()),
        );
        assert_eq!(
            plain.as_str(),
            "https://music.example.com/api/plays?limit=50"
        );
        // Searching for nothing is no search.
        assert!(PlayFilter::default().search(Some("")).is_empty());
    }

    #[test]
    fn a_cursor_from_the_worker_is_sent_back_as_before() {
        let c = client();
        assert_eq!(
            c.endpoint(
                "plays",
                &plays_query(
                    200,
                    Some(Before::Cursor("1791331200000.abc".into())),
                    None,
                    &PlayFilter::default()
                )
            )
            .as_str(),
            "https://music.example.com/api/plays?limit=200&before=1791331200000.abc"
        );
    }

    #[test]
    fn the_last_page_is_told_apart_from_an_older_worker() {
        let page = |body: &str| {
            let response: PlaysResponse = decode(body).unwrap();
            PlaysPage {
                plays: response.plays,
                next_before: response.next_before.clone().flatten(),
                last: response.next_before == Some(None),
            }
        };
        // A cursor: more follows.
        let more = page(r#"{"plays":[],"next_before":"1791331200000.abc"}"#);
        assert_eq!(more.next_before.as_deref(), Some("1791331200000.abc"));
        assert!(!more.last);
        // The Worker read one row past the page and found none.
        let done = page(r#"{"plays":[],"next_before":null}"#);
        assert_eq!(done.next_before, None);
        assert!(done.last);
        // A Worker older than cursors says nothing, so paging falls back to timestamps.
        let old = page(r#"{"plays":[]}"#);
        assert_eq!(old.next_before, None);
        assert!(!old.last);
    }

    #[test]
    fn plays_search_can_stay_inside_a_date_range() {
        let url = client().endpoint(
            "plays",
            &plays_query(100, Some(Before::Ms(20)), Some(10), &search("dna")),
        );
        assert_eq!(
            url.as_str(),
            "https://music.example.com/api/plays?limit=100&before=20&after=10&q=dna"
        );
        let response: PlaysResponse = decode(&fixture("plays-search-after.json")).unwrap();
        assert_eq!(response.filter.q.as_deref(), Some("kendrick"));
        assert!(!response.plays.is_empty());
    }

    #[test]
    fn scoped_pages_ask_for_one_song_or_artist() {
        let c = client();
        let song = PlayFilter::song("67XC51nlZncNpHmZ8rOU9a");
        assert_eq!(
            c.endpoint(
                "plays",
                &plays_query(100, Some(Before::Ms(20)), None, &song)
            )
            .as_str(),
            "https://music.example.com/api/plays?limit=100&before=20&track_id=67XC51nlZncNpHmZ8rOU9a"
        );
        let artist = PlayFilter::artist("2YZyLoL8N0Wb9xBt1NhZWg").search(Some("we cry"));
        assert_eq!(
            c.endpoint("plays", &plays_query(100, None, Some(5), &artist))
                .as_str(),
            "https://music.example.com/api/plays?limit=100&after=5&q=we+cry&artist_id=2YZyLoL8N0Wb9xBt1NhZWg"
        );
        // A placeholder also sends its name, as the Artist page does.
        let placeholder = PlayFilter::artist("name:Sigur Rós");
        assert_eq!(
            c.endpoint("plays", &plays_query(10, None, None, &placeholder))
                .as_str(),
            "https://music.example.com/api/plays?limit=10&artist_id=name%3ASigur+R%C3%B3s&artist_name=Sigur+R%C3%B3s"
        );

        let response: PlaysResponse = decode(&fixture("plays-scoped.json")).unwrap();
        assert!(song.check_applied(&response.filter).is_ok());
        assert!(
            response
                .plays
                .iter()
                .all(|p| p.track_id == "67XC51nlZncNpHmZ8rOU9a")
        );
    }

    #[test]
    fn csv_url_carries_the_view_filters_and_bounds_but_no_limit() {
        let c = client();
        assert_eq!(
            c.plays_csv_url(None, None, &PlayFilter::default()).as_str(),
            "https://music.example.com/api/plays.csv"
        );
        let filter = PlayFilter::artist("name:Sigur Rós").search(Some("hopp"));
        assert_eq!(
            c.plays_csv_url(Some(200), Some(100), &filter).as_str(),
            "https://music.example.com/api/plays.csv?before=200&after=100&q=hopp&artist_id=name%3ASigur+R%C3%B3s&artist_name=Sigur+R%C3%B3s"
        );
        let prefixed = ApiClient::new(
            parse_base_url("https://example.com/warehouse").unwrap(),
            Secret::new("t"),
        );
        assert_eq!(
            prefixed
                .plays_csv_url(None, None, &PlayFilter::song("abc"))
                .as_str(),
            "https://example.com/warehouse/api/plays.csv?track_id=abc"
        );
    }

    #[test]
    fn new_play_fields_decode_and_old_rows_still_do() {
        let response: PlaysResponse = decode(&fixture("plays-scoped.json")).unwrap();
        let latest = &response.plays[0];
        assert_eq!(latest.play_number, Some(6));
        assert_eq!(latest.first_play, Some(false));
        assert_eq!(
            latest.context_name.as_deref(),
            Some("Mr. Morale & The Big Steppers")
        );
        let credits = latest.credits();
        assert_eq!(credits.len(), 2);
        assert_eq!(credits[1].id.as_deref(), Some("7nkr1fevjIVmgx5biwAWVo"));

        let first = response.plays.last().unwrap();
        assert_eq!((first.play_number, first.first_play), (Some(1), Some(true)));
        assert_eq!(first.context_name, None);
        assert_eq!(first.credits()[1].id.as_deref(), Some("name:taylour paige"));

        // Rows from a Worker before these fields fall back to the joined names.
        let old: PlaysResponse = decode(&fixture("plays.json")).unwrap();
        let row = &old.plays[0];
        assert_eq!((row.play_number, row.first_play), (None, None));
        assert_eq!(row.artist_list, None);
        let names: Vec<_> = row.credits().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["Kendrick Lamar", "Taylour Paige"]);
        assert!(row.credits().iter().all(|c| c.id.is_none()));
        assert_eq!(old.filter, FilterEcho::default());
    }

    #[test]
    fn scoped_daily_counts_decode_with_their_echo() {
        let response: DailyResponse = decode(&fixture("daily-artist.json")).unwrap();
        assert_eq!(response.days.len(), 7);
        let filter = PlayFilter::artist("2YZyLoL8N0Wb9xBt1NhZWg");
        assert!(filter.check_applied(&response.filter).is_ok());
        let range = DateRange::new(
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(),
        )
        .unwrap();
        let query = daily_query(&range, "UTC", &PlayFilter::song("abc"));
        assert_eq!(
            client().endpoint("daily", &query).as_str(),
            "https://music.example.com/api/daily?from=2026-10-01&to=2026-10-07&tz=UTC&track_id=abc"
        );
        // Everyone's days need no echo, so Overview works on any Worker.
        let all: DailyResponse = decode(&fixture("daily.json")).unwrap();
        assert!(PlayFilter::default().check_applied(&all.filter).is_ok());
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
        assert!(top.quarters[0].tracks[0].track_id.is_some());
        let last_year = top.years.last().unwrap();
        assert_eq!(last_year.tracks.last().unwrap().track_id, None);

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
        assert_eq!(wrapped.top_artists[1].top_song_id, None);
        assert!(wrapped.top_artists[0].top_song_id.is_some());
        assert!(wrapped.months[0].top_song_id.is_some());
        assert_eq!(wrapped.streak.as_ref().unwrap().days, 64);

        let artist = &wrapped.top_artists[0];
        assert_eq!(artist.id.as_deref(), Some("2YZyLoL8N0Wb9xBt1NhZWg"));
        assert!(wrapped.top_songs[0].track_id.is_some());
        assert!(wrapped.top_albums[0].artist_id.is_some());
        assert!(wrapped.months[0].top_artist_id.is_some());
        assert_eq!(wrapped.months[1].top_artist_id, None);
        assert!(wrapped.discoveries.top[0].id.is_some());
        // A summary computed before the ids were added still decodes.
        let old: Wrapped = decode(
            r#"{"range":{},"totals":{},"biggest_day":null,"hours":[],"weekdays":[],
                "top_artists":[],"top_albums":[],"discoveries":{},"listening_age":null,
                "streak":null,"top_songs":[{"name":"DNA.","artist":"Kendrick Lamar","plays":1}],
                "months":[{"month":"2025-01","top_artist":"x"}]}"#,
        )
        .unwrap();
        assert_eq!(old.top_songs[0].track_id, None);
        assert_eq!(old.months[0].top_artist_id, None);
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

        // An artist not in the warehouse is billed without an id.
        let bill = response.concerts[1].credits();
        assert_eq!(bill[0].id.as_deref(), Some("04gDigrS5kc9YWfZHwBETP"));
        assert_eq!(
            (bill[1].id.as_deref(), bill[1].name.as_str()),
            (None, "Kanye West")
        );
        // A row the Worker has not rewritten has no list: split the names.
        assert_eq!(undated.artist_list, None);
        assert_eq!(undated.credits()[0].name, "Kendrick Lamar");

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
    fn plain_http_is_only_for_this_machine() {
        for local in [
            "http://localhost:8787",
            "http://127.0.0.1",
            "http://[::1]:8787",
        ] {
            assert!(parse_base_url(local).is_ok(), "{local}");
        }
        // Names that only start like loopback still go over the network.
        for remote in [
            "http://localhost.example.com",
            "http://127.0.0.1.example.com",
            "http://192.168.1.10:8787",
        ] {
            assert!(parse_base_url(remote).is_err(), "{remote}");
        }
    }

    #[test]
    fn a_saved_base_url_keeps_its_prefix_and_drops_the_rest() {
        // A pasted link's query or fragment must not ride along on every
        // request, and trailing slashes must not double up before /api.
        let url = parse_base_url("https://example.com/warehouse//?token=abc#top").unwrap();
        assert_eq!(url.as_str(), "https://example.com/warehouse");
        let client = ApiClient::new(url, Secret::new("t"));
        assert_eq!(
            client.endpoint("daily", &[]).as_str(),
            "https://example.com/warehouse/api/daily"
        );
    }

    #[test]
    fn only_spotify_failures_are_spotify_side() {
        let spotify = [
            ApiError::NeedsReauth,
            ApiError::RateLimited {
                retry_after_seconds: None,
            },
            ApiError::Upstream("x".into()),
        ];
        assert!(spotify.iter().all(ApiError::is_spotify_side));
        let worker = [
            ApiError::Unauthorized,
            ApiError::BadRequest("x".into()),
            ApiError::StillComputing {
                retry_after_seconds: 5,
            },
            ApiError::Server {
                status: 500,
                message: "x".into(),
            },
            ApiError::Network("x".into()),
            ApiError::Decode("x".into()),
            ApiError::File("x".into()),
        ];
        assert!(!worker.iter().any(ApiError::is_spotify_side));
    }

    #[test]
    fn long_error_pages_are_cut_short_without_splitting_a_character() {
        let page = "é".repeat(500);
        let ApiError::Server { message, .. } = ApiError::from_response(500, None, &page) else {
            panic!("a 500 is a server error");
        };
        assert_eq!(message.chars().count(), 200);
        // The Worker's own message is kept whole, however long.
        let long = format!(r#"{{"error":"{}"}}"#, "x".repeat(300));
        assert_eq!(
            ApiError::from_response(400, None, &long),
            ApiError::BadRequest("x".repeat(300))
        );
    }

    #[test]
    fn a_search_echoed_trimmed_was_applied() {
        let sent = PlayFilter::default().search(Some(" dna "));
        let echoed = FilterEcho {
            q: Some("dna".into()),
            ..FilterEcho::default()
        };
        assert!(sent.check_applied(&echoed).is_ok());
    }

    #[test]
    fn an_artist_placeholder_filters_by_its_name_too() {
        let placeholder = PlayFilter::artist("name:Sigur Rós");
        assert_eq!(placeholder.artist_name.as_deref(), Some("Sigur Rós"));
        let real = PlayFilter::artist("6UUrUCIZtQeOf8tC0WuzRy");
        assert_eq!(real.artist_name, None);
        assert_eq!(
            client().endpoint("daily", &placeholder.query()).as_str(),
            "https://music.example.com/api/daily?artist_id=name%3ASigur+R%C3%B3s&artist_name=Sigur+R%C3%B3s"
        );
    }

    #[test]
    fn spotify_top_ranges_use_its_parameter_names() {
        let params: Vec<_> = TopRange::ALL.iter().map(|r| r.as_param()).collect();
        assert_eq!(params, ["short_term", "medium_term", "long_term"]);
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
