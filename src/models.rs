//! Response shapes of the music-warehouse read API.
//!
//! Stored-row routes (`/api/plays`, `/api/daily`, `/api/top-artists`) are
//! typed tightly because the Worker owns their SQL. The live routes pass
//! Spotify's payloads through unmodified, so those types keep only what the
//! UI renders and default everything else: Spotify has removed and restored
//! fields before, and a missing field must not blank the strip.
//!
//! The history, stats, Wrapped and concert shapes further down follow the
//! Worker's `main` branch. A response's own fields, which the Worker has
//! sent since the route existed, are required: a renamed one fails loudly
//! as "Unexpected response" rather than quietly blanking a page. Fields
//! added later, and the fields of rows inside a response, default, so a
//! Worker a version behind still decodes and only the missing figure goes
//! blank. `Option` fields are null or absent without a default.

use std::collections::BTreeMap;

use serde::Deserialize;

/// One row of `GET /api/plays`. Export-era rows carry names but no album,
/// duration or ISRC, and a track missing from `tracks` leaves `track_name`
/// null through the LEFT JOIN, hence the options.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
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
    /// The same credits with ids, in credit order. Absent from older
    /// Workers; preferred over splitting `artists`, which breaks on a name
    /// that contains a comma.
    #[serde(default)]
    pub artist_list: Option<Vec<Credit>>,
    /// The playlist's, album's or artist's name. Null when there is no
    /// context or the Worker has not resolved it, and from older Workers.
    #[serde(default)]
    pub context_name: Option<String>,
    /// Which play of the song this was, every edition counted. Null until
    /// the Worker has numbered it.
    #[serde(default)]
    pub play_number: Option<u32>,
    #[serde(default)]
    pub first_play: Option<bool>,
}

impl Play {
    /// The credits with ids when the Worker sent them, else names split out
    /// of `artists` without ids.
    pub fn credits(&self) -> Vec<Credit> {
        match &self.artist_list {
            Some(list) if !list.is_empty() => list.clone(),
            _ => split_credits(self.artists.as_deref().unwrap_or_default()),
        }
    }
}

/// A credited artist. `id` is a Spotify id or a `name:` placeholder; null on
/// a concert bill for an artist not in the warehouse, and on credits split
/// from a joined string.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Credit {
    pub id: Option<String>,
    pub name: String,
}

/// Credits from a string the Worker joined as "A, B", for responses without
/// a credit list. A name that itself contains ", " is split too, which is
/// why the list is preferred.
pub fn split_credits(artists: &str) -> Vec<Credit> {
    artists
        .split(", ")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| Credit {
            id: None,
            name: name.to_owned(),
        })
        .collect()
}

/// The filters `/api/plays` and `/api/daily` applied, echoed at the top
/// level. A Worker older than a filter ignores it and echoes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct FilterEcho {
    pub q: Option<String>,
    pub track_id: Option<String>,
    pub artist_id: Option<String>,
    pub artist_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaysResponse {
    pub plays: Vec<Play>,
    /// The `before` for the next page, opaque. The Worker reads one row past
    /// the page, so an explicit `null` means there is no page after this one
    /// (`Some(None)`); a Worker older than cursors sends nothing (`None`).
    #[serde(default, deserialize_with = "nullable")]
    pub next_before: Option<Option<String>>,
    #[serde(flatten)]
    pub filter: FilterEcho,
}

/// Keeps `null` apart from absent, which `Option` alone folds together.
fn nullable<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
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
    #[serde(flatten)]
    pub filter: FilterEcho,
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
    /// Spotify track or episode id, usable with `OpenTrack` for tracks.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub artists: Vec<NamedRef>,
    #[serde(default)]
    pub album: Option<NamedRef>,
    /// Present instead of `artists`/`album` when an episode is playing.
    #[serde(default)]
    pub show: Option<NamedRef>,
    /// Episodes carry their own artwork; tracks use the album's.
    #[serde(default)]
    pub images: Vec<ImageRef>,
}

impl PlayingItem {
    pub fn artwork(&self) -> &[ImageRef] {
        match &self.album {
            Some(album) if !album.images.is_empty() => &album.images,
            _ => &self.images,
        }
    }
}

/// A Spotify album, artist or show. `images` is empty on the simplified
/// artist objects nested inside tracks, which carry no artwork.
#[derive(Debug, Clone, Deserialize)]
pub struct NamedRef {
    /// Spotify id; for artists, usable with `OpenArtist`.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub images: Vec<ImageRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageRef {
    pub url: String,
    #[serde(default)]
    pub width: Option<u32>,
}

/// The smallest image at least `min_px` wide, so thumbnails never download
/// the 640px original; falls back to the largest when none is big enough.
pub fn pick_image(images: &[ImageRef], min_px: u32) -> Option<&str> {
    let width = |i: &ImageRef| i.width.unwrap_or(0);
    images
        .iter()
        .filter(|i| width(i) >= min_px)
        .min_by_key(|i| width(i))
        .or_else(|| images.iter().max_by_key(|i| width(i)))
        .map(|i| i.url.as_str())
}

/// Sizes Spotify's CDN serves for album art.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rendition {
    Px64,
    Px300,
}

/// Stored rows keep only the 640px album image. Spotify's CDN encodes the size
/// in the URL's hash prefix, so swap in the rendition a view needs: a 64px
/// table thumbnail is 2.7 KB instead of ~96 KB. The prefix is a CDN
/// convention, not a documented API, so anything unrecognised is left as is.
pub fn album_rendition(url: &str, size: Rendition) -> String {
    const PREFIXES: [&str; 3] = [
        "/image/ab67616d0000b273",
        "/image/ab67616d00001e02",
        "/image/ab67616d00004851",
    ];
    let wanted = match size {
        Rendition::Px64 => "/image/ab67616d00004851",
        Rendition::Px300 => "/image/ab67616d00001e02",
    };
    if url.starts_with("https://i.scdn.co/")
        && let Some(prefix) = PREFIXES.iter().find(|p| url.contains(**p))
    {
        return url.replacen(prefix, wanted, 1);
    }
    url.to_owned()
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
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub artists: Vec<NamedRef>,
    #[serde(default)]
    pub album: Option<NamedRef>,
}

/// `GET /api/recently-played`: Spotify's payload, newest first. Unlike
/// stored plays it is never behind the Worker's polling interval.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RecentlyPlayedResponse {
    pub items: Vec<RecentlyPlayedItem>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RecentlyPlayedItem {
    /// Null for local files and tracks Spotify has since removed.
    pub track: Option<PlayingItem>,
    /// ISO 8601 instant the play ended.
    pub played_at: Option<String>,
    pub context: Option<PlayContext>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PlayContext {
    pub uri: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

// ---------------------------------------------------------------------------
// One artist's or song's whole history: /api/artist, /api/track,
// /api/top-tracks, /api/on-this-day, /api/search.
// ---------------------------------------------------------------------------

/// Plays in one local calendar month, `YYYY-MM`. History routes send every
/// month from the first play to the last, quiet ones as zero.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct MonthCount {
    pub month: String,
    pub plays: u32,
}

/// A song in a ranked list, with its editions (single, album, remaster)
/// folded together by the Worker. `id` is the most-played edition's.
/// `artist` is absent inside `/api/artist`, where it is the artist itself.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct RankedSong {
    pub id: Option<String>,
    pub name: String,
    pub artist: Option<String>,
    pub plays: u32,
    /// Stored 640px album art; shrink it with [`album_rendition`].
    pub image: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ArtistName {
    /// A Spotify id, a `name:` placeholder, or null when only a name matched.
    pub id: Option<String>,
    pub name: String,
}

/// `GET /api/artist`. Credit is the primary artist's only, as in every stat.
/// `minutes` is the Worker's estimate (skips count for nothing).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArtistHistory {
    pub artist: ArtistName,
    pub plays: u32,
    pub minutes: u32,
    /// ISO 8601 instants; null when the artist has no plays.
    pub first_played: Option<String>,
    pub last_played: Option<String>,
    pub months: Vec<MonthCount>,
    pub top_tracks: Vec<RankedSong>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TrackName {
    pub id: String,
    /// Null when the id is not in the warehouse.
    pub name: Option<String>,
    pub artist: Option<String>,
    pub image: Option<String>,
}

/// `GET /api/track`, every edition of the song folded together.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TrackHistory {
    pub track: TrackName,
    pub plays: u32,
    pub minutes: u32,
    pub first_played: Option<String>,
    pub last_played: Option<String>,
    pub months: Vec<MonthCount>,
}

/// `GET /api/top-tracks`: the songs counterpart of `/api/top-artists`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TopTracksResponse {
    pub tracks: Vec<RankedSong>,
}

/// `GET /api/on-this-day`: the same calendar date in each earlier year,
/// newest year first; years with no plays are left out.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OnThisDay {
    pub date: String,
    pub years: Vec<OnThisDayYear>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OnThisDayYear {
    pub year: i32,
    pub plays: u32,
    pub top_tracks: Vec<RankedSong>,
}

/// `GET /api/search`: stored artists and songs whose name contains `q`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchResponse {
    pub artists: Vec<SearchArtist>,
    pub tracks: Vec<SearchTrack>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SearchArtist {
    /// Null for an artist known only by name from the imported export;
    /// open it with [`artist_key`] and the name.
    pub id: Option<String>,
    pub name: String,
    pub plays: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SearchTrack {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub plays: u32,
}

/// The id to open an artist by. Most stats name artists without an id, and
/// the export keys artists `name:<name>`; the client sends both the id and
/// that name, so the Worker folds such a key into the real artist by name.
pub fn artist_key(id: Option<&str>, name: &str) -> String {
    match id {
        Some(id) if !id.is_empty() => id.to_owned(),
        _ => format!("{ARTIST_NAME_PREFIX}{name}"),
    }
}

/// Prefix of the Worker's placeholder ids for artists known only by name.
pub const ARTIST_NAME_PREFIX: &str = "name:";

// ---------------------------------------------------------------------------
// /api/stats/*: aggregates over the whole history, recomputed at most daily.
// ---------------------------------------------------------------------------

/// `GET /api/stats/on-repeat`: songs played far more than usual over the
/// last `window_days` whole local days.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OnRepeat {
    pub as_of: String,
    pub window_days: u32,
    pub tracks: Vec<RepeatTrack>,
    pub albums: Vec<RepeatAlbum>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RepeatTrack {
    pub track_id: String,
    pub name: String,
    /// Every credited artist, joined `"A, B"`.
    pub artists: String,
    pub album: Option<String>,
    pub image: Option<String>,
    pub plays: u32,
    pub days: u32,
    /// Plays per local day across the window, oldest first.
    pub daily: Vec<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RepeatAlbum {
    pub name: String,
    pub artist: String,
    pub image: Option<String>,
    pub tracks_on_repeat: u32,
    pub plays: u32,
}

/// `{ from, to }` as `YYYY-MM` months.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct MonthSpan {
    pub from: String,
    pub to: String,
}

/// `GET /api/stats/rhythm`: listening streaks and time of day, in each
/// era's local time, overall and per calendar year.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rhythm {
    pub range: MonthSpan,
    /// One set per play-count threshold (1, 10 and 25 plays a day).
    pub streaks: Vec<StreakSet>,
    pub eras: Vec<EraRhythm>,
    pub years: Vec<RhythmYear>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct StreakSet {
    pub threshold: u32,
    pub current_weeks: u32,
    pub longest: Option<Streak>,
    pub top: Vec<Streak>,
    /// Share (0–1) of days with at least `threshold` plays.
    pub active_share: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Streak {
    /// Exact length once finished; null while still running.
    pub days: Option<u32>,
    pub weeks: u32,
    /// `YYYY-MM` months.
    pub from: String,
    pub to: String,
    pub running: bool,
}

/// Rhythm within one era (a configured span of time, usually a place lived).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EraRhythm {
    pub key: String,
    pub label: String,
    pub from: String,
    /// Null while the era is still running.
    pub to: Option<String>,
    /// Share of plays in each local hour 0–23; null until the era is long enough.
    pub hour_share: Option<Vec<f64>>,
    /// Monday-first 7 × 24 shares; null until the era is long enough.
    pub grid: Option<Vec<Vec<f64>>>,
    pub hours_available_from: Option<String>,
    pub grid_available_from: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RhythmYear {
    pub year: String,
    /// The current year, so far.
    pub partial: bool,
    pub streaks: Vec<StreakSet>,
    pub eras: Vec<EraRhythm>,
}

/// `GET /api/stats/top-over-time`: leaders per UTC quarter and per year,
/// credited to the primary artist only.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TopOverTime {
    pub as_of: String,
    pub quarters: Vec<Period>,
    pub years: Vec<Period>,
    /// The most-played artists overall, ordered by when they peaked.
    pub leaders: Vec<Leader>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Period {
    /// `YYYY-Qn` or `YYYY`.
    pub period: String,
    pub partial: bool,
    pub plays: u32,
    pub artists: Vec<PeriodArtist>,
    pub tracks: Vec<PeriodTrack>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PeriodArtist {
    pub name: String,
    pub plays: u32,
    /// Share (0–1) of the period's plays.
    pub share: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PeriodTrack {
    /// The song's most-played edition in the period, which the Track page
    /// folds back into the whole song. Absent from older Workers.
    pub track_id: Option<String>,
    pub name: String,
    pub artist: String,
    pub plays: u32,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Leader {
    pub name: String,
    pub plays: u32,
    /// Share of each quarter's plays, aligned with `TopOverTime::quarters`.
    pub share: Vec<f64>,
}

/// `GET /api/stats/eras`: how the artist mix changed since the latest era
/// began, against how much it usually changes over the same span.
///
/// Modelled on the Worker's v2 shape (`latest`, `top.earlier/latest`, era
/// names). An older Worker's city-named keys decode as empty lists.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Eras {
    pub as_of: String,
    /// Null with a single era configured.
    pub latest_era: Option<EraName>,
    pub previous_era: Option<EraName>,
    /// The UTC month the latest era began.
    pub move_month: Option<String>,
    /// Complete months on each side of the move; 0 until the first one ends.
    pub window_months: u32,
    pub latest: Option<MonthSpan>,
    pub before: Option<MonthSpan>,
    pub ready: bool,
    pub summary: Option<EraSummary>,
    pub top: EraTop,
    pub risers: Vec<ShareChange>,
    pub fallers: Vec<ShareChange>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct EraName {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EraSummary {
    /// Share of the latest era's plays that went to artists played before it.
    pub known_artist_share: f64,
    pub new_artists: u32,
    pub artists: u32,
    /// Total variation distance (0–1) between the two artist mixes.
    pub change: f64,
    pub typical_change: TypicalChange,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TypicalChange {
    pub median: f64,
    pub min: f64,
    pub max: f64,
    pub windows: u32,
    /// How many past windows changed more than this one.
    pub exceeded_by: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EraTop {
    /// All history before the latest era began.
    pub earlier: Vec<ArtistShare>,
    /// The same number of months just before it.
    pub before: Vec<ArtistShare>,
    pub latest: Vec<ArtistShare>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ArtistShare {
    pub artist: String,
    pub share: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ShareChange {
    pub artist: String,
    pub before: f64,
    pub after: f64,
}

/// `GET /api/stats/albums`: most-played albums with at least `min_songs`
/// different songs played, all time and per UTC year.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TopAlbums {
    pub as_of: String,
    pub min_songs: u32,
    pub albums: Vec<TopAlbum>,
    pub years: Vec<TopAlbumsYear>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TopAlbum {
    pub name: String,
    pub artist: String,
    pub image: Option<String>,
    pub plays: u32,
    pub songs: u32,
    /// Months of the first and latest play.
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TopAlbumsYear {
    pub year: String,
    pub partial: bool,
    pub albums: Vec<TopAlbum>,
}

// ---------------------------------------------------------------------------
// /api/wrapped and /api/wrapped/ranges
// ---------------------------------------------------------------------------

/// `GET /api/wrapped/ranges`: what a read token may ask `/api/wrapped` for.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WrappedRanges {
    /// Newest first: this year so far, then each closed year with plays.
    pub ranges: Vec<WrappedPreset>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct WrappedPreset {
    /// Local `YYYY-MM-DD` days, inclusive.
    pub from: String,
    pub to: String,
    pub label: String,
    /// `year` or `year_to_date`.
    pub kind: String,
    /// Whether the Worker already holds this summary. One that is not
    /// computes on request, which can take several seconds.
    #[serde(default)]
    pub computed: bool,
}

/// `GET /api/wrapped`: a year-in-review for a closed range. Minutes are
/// estimates; songs rank by plays, artists and albums by minutes.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Wrapped {
    pub range: WrappedRange,
    pub totals: WrappedTotals,
    pub biggest_day: Option<BiggestDay>,
    /// Minutes in each local hour, 0–23.
    pub hours: Vec<u32>,
    /// Minutes on each weekday, Monday first.
    pub weekdays: Vec<u32>,
    pub top_artists: Vec<WrappedArtist>,
    pub top_songs: Vec<WrappedSong>,
    pub top_albums: Vec<WrappedAlbum>,
    pub months: Vec<WrappedMonth>,
    pub discoveries: Discoveries,
    pub listening_age: Option<ListeningAge>,
    pub streak: Option<DayStreak>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct WrappedRange {
    pub from: String,
    pub to: String,
    pub days: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WrappedTotals {
    pub minutes: u32,
    pub plays: u32,
    pub skips: u32,
    pub days_listened: u32,
    pub artists: u32,
    pub songs: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BiggestDay {
    pub date: String,
    pub minutes: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WrappedArtist {
    /// A Spotify id or a `name:` placeholder. Wrapped's ids are absent from
    /// older Workers, so every one is optional.
    pub id: Option<String>,
    pub name: String,
    pub minutes: u32,
    pub plays: u32,
    pub share: f64,
    pub days: u32,
    pub longest_run_days: u32,
    pub top_song: Option<String>,
    /// The top song's most-listened edition, for the Track page.
    pub top_song_id: Option<String>,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WrappedSong {
    /// The song's most-listened edition.
    pub track_id: Option<String>,
    pub name: String,
    pub artist: String,
    pub artist_id: Option<String>,
    pub album: Option<String>,
    pub image: Option<String>,
    pub plays: u32,
    pub minutes: u32,
    /// `YYYY-MM` with the most minutes.
    pub peak_month: String,
    /// First local day it was played in the range.
    pub first_played: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WrappedAlbum {
    pub name: String,
    pub artist: String,
    pub artist_id: Option<String>,
    pub image: Option<String>,
    pub plays: u32,
    pub minutes: u32,
    pub songs: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WrappedMonth {
    pub month: String,
    pub minutes: u32,
    pub top_artist: Option<String>,
    pub top_artist_id: Option<String>,
    pub top_song: Option<String>,
    pub top_song_id: Option<String>,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Discoveries {
    /// Artists and songs first played ever inside the range.
    pub artists: u32,
    pub songs: u32,
    /// Share of the range's minutes that went to those new artists.
    pub share: f64,
    pub top: Vec<Discovery>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Discovery {
    pub id: Option<String>,
    pub name: String,
    pub minutes: u32,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ListeningAge {
    pub age: u32,
    pub median_year: i32,
    /// First and last release year of the five-year window with most minutes.
    pub window: (i32, i32),
    pub coverage: f64,
    pub decades: Vec<DecadeShare>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DecadeShare {
    pub decade: i32,
    pub share: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DayStreak {
    pub days: u32,
    pub from: String,
    pub to: String,
}

// ---------------------------------------------------------------------------
// /api/concerts
// ---------------------------------------------------------------------------

/// `GET /api/concerts`: upcoming events for the most-played artists, in
/// each region the Worker is configured to watch. Region keys are whatever
/// the deployment chose; label them through `regions`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConcertsResponse {
    /// Absent from Workers older than configurable regions.
    #[serde(default)]
    pub regions: Vec<ConcertRegion>,
    /// Region key → ISO instant it was last fetched in full.
    pub checked: BTreeMap<String, String>,
    /// Soonest first; undated events last.
    pub concerts: Vec<Concert>,
}

impl Concert {
    /// The bill with ids when the Worker sent it, else split from `artists`.
    pub fn credits(&self) -> Vec<Credit> {
        match &self.artist_list {
            Some(list) if !list.is_empty() => list.clone(),
            _ => split_credits(&self.artists),
        }
    }
}

impl ConcertsResponse {
    /// A region's label, falling back to its key for Workers that send none.
    pub fn region_label<'a>(&'a self, key: &'a str) -> &'a str {
        self.regions
            .iter()
            .find(|r| r.key == key)
            .map(|r| r.label.as_str())
            .unwrap_or(key)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ConcertRegion {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Concert {
    pub region: String,
    /// Local `YYYY-MM-DD`; null until announced.
    pub date: Option<String>,
    /// Local `HH:MM`.
    pub time: Option<String>,
    /// The matched artists on the bill, joined `"A, B"`.
    pub artists: String,
    /// The bill with ids where the artist is in the warehouse. Null on rows
    /// the Worker has not rewritten since it began sending it.
    pub artist_list: Option<Vec<Credit>>,
    pub name: String,
    pub venue: Option<String>,
    pub city: Option<String>,
    /// Ticketmaster's status code, e.g. `onsale`, `offsale`, `cancelled`.
    pub status: Option<String>,
    pub url: Option<String>,
    /// ISO instant the Worker first saw the event, for "new since".
    pub first_seen: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_renamed_response_field_fails_rather_than_blanking() {
        // `years` renamed: the page must say the response is unexpected.
        assert!(serde_json::from_str::<OnThisDay>(r#"{"date":"2026-10-15","yrs":[]}"#).is_err());
        assert!(serde_json::from_str::<OnThisDay>(r#"{"date":"2026-10-15","years":[]}"#).is_ok());
        // A field added later still defaults for an older Worker.
        let preset: WrappedPreset = serde_json::from_str(
            r#"{"from":"2025-01-01","to":"2025-12-31","label":"2025","kind":"year"}"#,
        )
        .unwrap();
        assert!(!preset.computed);
        let concerts: ConcertsResponse =
            serde_json::from_str(r#"{"checked":{},"concerts":[]}"#).unwrap();
        assert!(concerts.regions.is_empty());
    }

    fn image(width: Option<u32>, url: &str) -> ImageRef {
        ImageRef {
            url: url.into(),
            width,
        }
    }

    #[test]
    fn joined_credits_split_into_names_without_ids() {
        let names =
            |s: &str| -> Vec<String> { split_credits(s).into_iter().map(|c| c.name).collect() };
        assert_eq!(
            names("Kendrick Lamar, Taylour Paige"),
            ["Kendrick Lamar", "Taylour Paige"]
        );
        assert_eq!(names("SZA"), ["SZA"]);
        assert!(names("").is_empty());
        assert!(split_credits("A, B").iter().all(|c| c.id.is_none()));
    }

    #[test]
    fn picks_smallest_image_that_is_large_enough() {
        let images = [
            image(Some(640), "l"),
            image(Some(300), "m"),
            image(Some(64), "s"),
        ];
        assert_eq!(pick_image(&images, 40), Some("s"));
        assert_eq!(pick_image(&images, 100), Some("m"));
        // Nothing is wide enough: take the biggest rather than nothing.
        assert_eq!(pick_image(&images, 1000), Some("l"));
        assert_eq!(pick_image(&[], 40), None);
    }

    #[test]
    fn stored_album_urls_shrink_to_the_64px_rendition() {
        assert_eq!(
            album_rendition(
                "https://i.scdn.co/image/ab67616d0000b2732e02117d76426a08ac7c174f",
                Rendition::Px64
            ),
            "https://i.scdn.co/image/ab67616d000048512e02117d76426a08ac7c174f"
        );
        assert_eq!(
            album_rendition(
                "https://i.scdn.co/image/ab67616d00001e022e02117d76426a08ac7c174f",
                Rendition::Px64
            ),
            "https://i.scdn.co/image/ab67616d000048512e02117d76426a08ac7c174f"
        );
        // Artist images and other hosts use different prefixes; leave them alone.
        let artist = "https://i.scdn.co/image/ab6761610000e5eb0123";
        assert_eq!(album_rendition(artist, Rendition::Px64), artist);
        let other = "https://example.com/image/ab67616d0000b273x";
        assert_eq!(album_rendition(other, Rendition::Px300), other);
        assert_eq!(
            album_rendition(
                "https://i.scdn.co/image/ab67616d000048512e02",
                Rendition::Px300
            ),
            "https://i.scdn.co/image/ab67616d00001e022e02"
        );
    }
}
