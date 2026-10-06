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

/// Stored rows keep only the 640px album image. Spotify's CDN encodes the size
/// in the URL's hash prefix, so swap to the 64px rendition of the same image
/// for table thumbnails (2.7 KB instead of ~96 KB). The prefix is a CDN
/// convention, not a documented API, so anything unrecognised is left as is.
pub fn album_thumbnail(url: &str) -> String {
    const LARGE: &str = "/image/ab67616d0000b273";
    const MEDIUM: &str = "/image/ab67616d00001e02";
    const SMALL: &str = "/image/ab67616d00004851";
    if url.starts_with("https://i.scdn.co/") {
        for prefix in [LARGE, MEDIUM] {
            if url.contains(prefix) {
                return url.replacen(prefix, SMALL, 1);
            }
        }
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
    pub name: Option<String>,
    #[serde(default)]
    pub artists: Vec<NamedRef>,
    #[serde(default)]
    pub album: Option<NamedRef>,
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

    fn image(width: Option<u32>, url: &str) -> ImageRef {
        ImageRef {
            url: url.into(),
            width,
        }
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
            album_thumbnail("https://i.scdn.co/image/ab67616d0000b2732e02117d76426a08ac7c174f"),
            "https://i.scdn.co/image/ab67616d000048512e02117d76426a08ac7c174f"
        );
        assert_eq!(
            album_thumbnail("https://i.scdn.co/image/ab67616d00001e022e02117d76426a08ac7c174f"),
            "https://i.scdn.co/image/ab67616d000048512e02117d76426a08ac7c174f"
        );
        // Artist images and other hosts use different prefixes; leave them alone.
        let artist = "https://i.scdn.co/image/ab6761610000e5eb0123";
        assert_eq!(album_thumbnail(artist), artist);
        let other = "https://example.com/image/ab67616d0000b273x";
        assert_eq!(album_thumbnail(other), other);
    }
}
