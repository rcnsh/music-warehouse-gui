//! Response shapes of the music-warehouse read API.
//!
//! Stored-row routes (`/api/plays`, `/api/daily`, `/api/top-artists`) are
//! typed tightly because the Worker owns their SQL. The live routes pass
//! Spotify's payloads through unmodified, so those types keep only what the
//! UI renders and default everything else: Spotify has removed and restored
//! fields before, and a missing field must not blank the strip.

use serde::Deserialize;

/// One row of `GET /api/plays`. Export-era rows carry names but no album,
/// duration or ISRC, and a track missing from `tracks` leaves `track_name`
/// null through the LEFT JOIN, hence the options.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Play {
    pub played_at_ms: i64,
    pub track_id: String,
    #[serde(default)]
    pub context_uri: Option<String>,
    #[serde(default)]
    pub context_type: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub track_name: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub album_name: Option<String>,
    #[serde(default)]
    pub image_url: Option<String>,
    /// Already joined by the Worker as "A, B" in credit order.
    #[serde(default)]
    pub artists: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaysResponse {
    pub plays: Vec<Play>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DayCount {
    /// Local calendar day in the requested zone, `YYYY-MM-DD`.
    pub day: String,
    pub plays: u32,
}

/// `GET /api/daily`. Quiet days are pre-seeded as zero by the Worker, so
/// `days` is already contiguous.
#[derive(Debug, Clone, Deserialize)]
pub struct DailyResponse {
    pub days: Vec<DayCount>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ArtistCount {
    /// A Spotify id, or `name:<artist>` for export-era artists never polled.
    pub artist_id: String,
    pub name: String,
    pub plays: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TopArtistsResponse {
    pub artists: Vec<ArtistCount>,
}

/// `GET /api/now-playing`: `{ item: null }` when Spotify answers 204.
#[derive(Debug, Clone, Deserialize)]
pub struct NowPlayingResponse {
    #[serde(default)]
    pub item: Option<CurrentlyPlaying>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CurrentlyPlaying {
    #[serde(default)]
    pub is_playing: bool,
    /// `track`, `episode`, `ad` or `unknown`.
    #[serde(default)]
    pub currently_playing_type: Option<String>,
    /// Null during ads and for some private sessions.
    #[serde(default)]
    pub item: Option<PlayingItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayingItem {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub artists: Vec<NamedRef>,
    #[serde(default)]
    pub album: Option<NamedRef>,
    /// Present instead of `artists`/`album` when an episode is playing.
    #[serde(default)]
    pub show: Option<NamedRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NamedRef {
    #[serde(default)]
    pub name: Option<String>,
}

/// `GET /api/top`: Spotify's own ranked lists, which the warehouse cannot
/// reconstruct (they weigh signals beyond play counts).
#[derive(Debug, Clone, Deserialize)]
pub struct TopResponse {
    pub tracks: Paging<TopTrack>,
    pub artists: Paging<NamedRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Paging<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TopTrack {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub artists: Vec<NamedRef>,
}

/// Error body shared by every non-2xx JSON response from the Worker.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ErrorBody {
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub needs_reauth: bool,
    #[serde(default)]
    pub retry_after_seconds: Option<u64>,
}

/// Joins artist names the way the Worker does for stored rows, so live and
/// stored credits read the same.
pub fn join_names(names: &[NamedRef]) -> String {
    names
        .iter()
        .filter_map(|a| a.name.as_deref())
        .collect::<Vec<_>>()
        .join(", ")
}
