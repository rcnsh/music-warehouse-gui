use std::collections::VecDeque;
use std::future::Future;

use chrono::NaiveDate;
use gpui::{Context, Task};

use crate::api::{ApiClient, ApiError, PlayFilter, TopRange};
use crate::dates::{self, DateRange, RangePreset};
use crate::models::{DailyResponse, TopArtistsResponse, TopResponse, TopTracksResponse};
use crate::runtime;
use crate::state::{self, Remote};
use crate::ui_state;

pub const TOP_ARTISTS_LIMIT: u32 = 50;
pub const TOP_TRACKS_LIMIT: u32 = 50;
pub const SPOTIFY_TOP_LIMIT: u32 = 10;

/// Ranges kept for the session. An entry is at most a year of days plus two
/// top-50 lists, so this bounds a session's memory loosely while covering
/// every preset and a few custom ranges.
const RANGE_CACHE_CAPACITY: usize = 12;

/// Data for the Overview: the daily chart and the artist and song counts
/// share one range, so they are always fetched together. Spotify's own top
/// lists use its fixed windows instead and are fetched independently,
/// because they are live and can fail while the stored-row routes keep
/// working.
pub struct OverviewStore {
    client: ApiClient,
    pub tz: String,
    pub preset: RangePreset,
    pub range: DateRange,
    pub daily: Remote<DailyResponse>,
    pub artists: Remote<TopArtistsResponse>,
    pub tracks: Remote<TopTracksResponse>,
    pub spotify_range: TopRange,
    pub spotify_top: Remote<TopResponse>,
    /// A year view reads tens of thousands of rows on the Worker, so going
    /// back to a range already seen this session costs no request.
    cache: RangeCache<RangeData>,
    /// The range the in-flight requests belong to, cached once all three land.
    pending: Option<RangeKey>,
    _range_tasks: Vec<Task<()>>,
    _spotify_task: Option<Task<()>>,
}

/// Everything the Overview shows for one range.
#[derive(Clone)]
pub struct RangeData {
    pub daily: DailyResponse,
    pub artists: TopArtistsResponse,
    pub tracks: TopTracksResponse,
}

impl OverviewStore {
    pub fn new(client: ApiClient, cx: &mut Context<Self>) -> Self {
        let saved = ui_state::get(cx);
        let (preset, range) = saved
            .range
            .as_ref()
            .and_then(|r| r.restore(dates::today_local()))
            .unwrap_or_else(|| {
                let preset = RangePreset::Last30Days;
                let range = preset
                    .resolve(dates::today_local())
                    .expect("30-day preset always resolves");
                (preset, range)
            });
        let mut store = Self {
            client,
            tz: dates::system_timezone(),
            preset,
            range,
            daily: Remote::default(),
            artists: Remote::default(),
            tracks: Remote::default(),
            spotify_range: ui_state::spotify_range(&saved).unwrap_or(TopRange::Short),
            spotify_top: Remote::default(),
            cache: RangeCache::new(RANGE_CACHE_CAPACITY),
            pending: None,
            _range_tasks: Vec::new(),
            _spotify_task: None,
        };
        store.fetch_range(cx);
        store.fetch_spotify_top(cx);
        store
    }

    /// Switches to a preset, re-resolving against today so a window left open
    /// overnight still ends on the current day.
    pub fn select_preset(&mut self, preset: RangePreset, cx: &mut Context<Self>) {
        self.preset = preset;
        if let Some(range) = preset.resolve(dates::today_local()) {
            self.set_range(range, cx);
        } else {
            cx.notify();
        }
    }

    pub fn set_custom_range(&mut self, range: DateRange, cx: &mut Context<Self>) {
        self.preset = RangePreset::Custom;
        self.set_range(range, cx);
    }

    fn set_range(&mut self, range: DateRange, cx: &mut Context<Self>) {
        let changed = range != self.range;
        self.range = range;
        if self.show_cached(cx) {
            return;
        }
        if changed {
            self.reset_range_data();
        }
        self.fetch_range(cx);
    }

    /// Keeping the old range's bars under a new title would mislead.
    fn reset_range_data(&mut self) {
        self.daily.reset();
        self.artists.reset();
        self.tracks.reset();
    }

    pub fn select_spotify_range(&mut self, range: TopRange, cx: &mut Context<Self>) {
        if range != self.spotify_range {
            self.spotify_range = range;
            self.spotify_top.reset();
        }
        self.fetch_spotify_top(cx);
    }

    /// Refetches the range on screen. Other cached ranges that reach today
    /// or yesterday are dropped too, since they may have gained plays (the
    /// Worker stores plays up to half an hour late); ranges that ended before
    /// that cannot change, so a year already seen stays free to revisit.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let today = dates::today_local();
        self.cache.remove(&self.key());
        self.cache.drop_open(today);
        // The system zone can change while the app runs (travel).
        self.tz = dates::system_timezone();
        if self.preset != RangePreset::Custom
            && let Some(range) = self.preset.resolve(today)
            && range != self.range
        {
            self.range = range;
            self.reset_range_data();
        }
        self.fetch_range(cx);
        self.fetch_spotify_top(cx);
    }

    fn key(&self) -> RangeKey {
        RangeKey::new(&self.range, &self.tz)
    }

    /// Shows the current range from the cache, cancelling any requests for
    /// the range it replaces. Returns false on a miss.
    fn show_cached(&mut self, cx: &mut Context<Self>) -> bool {
        let key = self.key();
        let Some(data) = self.cache.get(&key, dates::today_local()).cloned() else {
            return false;
        };
        self._range_tasks.clear();
        self.pending = None;
        self.daily.finish(Ok(data.daily));
        self.artists.finish(Ok(data.artists));
        self.tracks.finish(Ok(data.tracks));
        cx.notify();
        true
    }

    /// Caches the range once all three requests have succeeded. A failed
    /// part leaves it uncached, so the next visit asks again.
    fn cache_if_complete(&mut self) {
        let done = !self.daily.loading && !self.artists.loading && !self.tracks.loading;
        let failed = self.daily.error.is_some()
            || self.artists.error.is_some()
            || self.tracks.error.is_some();
        if !done || failed {
            return;
        }
        let (Some(key), Some(daily), Some(artists), Some(tracks)) = (
            self.pending.take(),
            self.daily.data.clone(),
            self.artists.data.clone(),
            self.tracks.data.clone(),
        ) else {
            return;
        };
        let data = RangeData {
            daily,
            artists,
            tracks,
        };
        self.cache.insert(key, dates::today_local(), data);
    }

    fn fetch_range(&mut self, cx: &mut Context<Self>) {
        self.daily.start();
        self.artists.start();
        self.tracks.start();
        self.pending = Some(self.key());
        let range = self.range;

        let (client, tz) = (self.client.clone(), self.tz.clone());
        let daily = spawn_range_request(
            cx,
            move || {
                let (client, tz) = (client.clone(), tz.clone());
                async move { client.daily(&range, &tz, &PlayFilter::default()).await }
            },
            |s| &mut s.daily,
        );
        let (client, tz) = (self.client.clone(), self.tz.clone());
        let artists = spawn_range_request(
            cx,
            move || {
                let (client, tz) = (client.clone(), tz.clone());
                async move { client.top_artists(&range, &tz, TOP_ARTISTS_LIMIT).await }
            },
            |s| &mut s.artists,
        );
        let (client, tz) = (self.client.clone(), self.tz.clone());
        let tracks = spawn_range_request(
            cx,
            move || {
                let (client, tz) = (client.clone(), tz.clone());
                async move { client.top_tracks(&range, &tz, TOP_TRACKS_LIMIT).await }
            },
            |s| &mut s.tracks,
        );
        // Replacing the tasks cancels requests for a range no longer shown.
        self._range_tasks = vec![daily, artists, tracks];
    }

    fn fetch_spotify_top(&mut self, cx: &mut Context<Self>) {
        self.spotify_top.start();
        cx.notify();
        let client = self.client.clone();
        let range = self.spotify_range;
        self._spotify_task = Some(cx.spawn(async move |this, cx| {
            let result: Result<TopResponse, ApiError> =
                runtime::run(async move { client.top(range, SPOTIFY_TOP_LIMIT).await }).await;
            this.update(cx, |this, cx| {
                this.spotify_top.finish(result);
                cx.notify();
            })
            .ok();
        }));
    }
}

/// Like `state::fetch`, but offers the range to the cache after each answer.
fn spawn_range_request<T, F>(
    cx: &mut Context<OverviewStore>,
    request: impl Fn() -> F + 'static,
    slot: fn(&mut OverviewStore) -> &mut Remote<T>,
) -> Task<()>
where
    T: Send + 'static,
    F: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    state::fetch_then(cx, request, slot, |this, _| this.cache_if_complete())
}

/// A range as the Worker resolves it: the same days in another zone are
/// different buckets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeKey {
    range: DateRange,
    tz: String,
}

impl RangeKey {
    pub fn new(range: &DateRange, tz: &str) -> Self {
        Self {
            range: *range,
            tz: tz.to_owned(),
        }
    }
}

struct CacheEntry<V> {
    key: RangeKey,
    fetched_on: NaiveDate,
    value: V,
}

impl<V> CacheEntry<V> {
    /// Whether the range could still gain plays: it reaches the day before
    /// it was fetched, allowing for plays the Worker stores after midnight.
    fn is_open(&self) -> bool {
        self.fetched_on
            .pred_opt()
            .is_none_or(|yesterday| self.key.range.to() >= yesterday)
    }

    /// An open range fetched on an earlier day is missing that day's later
    /// plays, so it no longer counts as a hit.
    fn is_stale(&self, today: NaiveDate) -> bool {
        self.fetched_on != today && self.is_open()
    }
}

/// Least-recently-used, bounded store of results per resolved range.
pub struct RangeCache<V> {
    entries: VecDeque<CacheEntry<V>>,
    capacity: usize,
}

impl<V> RangeCache<V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn get(&mut self, key: &RangeKey, today: NaiveDate) -> Option<&V> {
        let ix = self.entries.iter().position(|e| &e.key == key)?;
        let entry = self.entries.remove(ix)?;
        if entry.is_stale(today) {
            return None;
        }
        self.entries.push_back(entry);
        self.entries.back().map(|e| &e.value)
    }

    pub fn insert(&mut self, key: RangeKey, today: NaiveDate, value: V) {
        self.remove(&key);
        self.entries.push_back(CacheEntry {
            key,
            fetched_on: today,
            value,
        });
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    pub fn remove(&mut self, key: &RangeKey) {
        self.entries.retain(|e| &e.key != key);
    }

    /// Drops every range that could have gained plays since it was fetched.
    pub fn drop_open(&mut self, today: NaiveDate) {
        self.entries.retain(|e| !e.is_open() && !e.is_stale(today));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn key(from: &str, to: &str, tz: &str) -> RangeKey {
        RangeKey::new(&DateRange::new(day(from), day(to)).unwrap(), tz)
    }

    #[test]
    fn a_range_seen_today_is_a_hit() {
        let mut cache = RangeCache::new(4);
        let today = day("2026-10-15");
        cache.insert(key("2026-09-16", "2026-10-15", "UTC"), today, 1);
        let hit = cache.get(&key("2026-09-16", "2026-10-15", "UTC"), today);
        assert_eq!(hit, Some(&1));
    }

    #[test]
    fn the_same_days_in_another_zone_miss() {
        let mut cache = RangeCache::new(4);
        let today = day("2026-10-15");
        cache.insert(key("2026-09-16", "2026-10-15", "UTC"), today, 1);
        let other = key("2026-09-16", "2026-10-15", "Europe/London");
        assert_eq!(cache.get(&other, today), None);
    }

    #[test]
    fn a_range_reaching_today_goes_stale_overnight() {
        let mut cache = RangeCache::new(4);
        let k = key("2026-10-01", "2026-10-15", "UTC");
        cache.insert(k.clone(), day("2026-10-15"), 1);
        assert_eq!(cache.get(&k, day("2026-10-16")), None);
        // The stale entry is gone, not just hidden.
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn a_range_ending_yesterday_also_goes_stale_overnight() {
        // Plays from just before midnight can be stored after it.
        let mut cache = RangeCache::new(4);
        let k = key("2026-10-01", "2026-10-14", "UTC");
        cache.insert(k.clone(), day("2026-10-15"), 1);
        assert_eq!(cache.get(&k, day("2026-10-16")), None);
    }

    #[test]
    fn a_settled_range_survives_day_changes() {
        let mut cache = RangeCache::new(4);
        let k = key("2025-01-01", "2025-12-31", "UTC");
        cache.insert(k.clone(), day("2026-10-15"), 1);
        assert_eq!(cache.get(&k, day("2026-11-02")), Some(&1));
    }

    #[test]
    fn the_least_recently_used_range_is_evicted() {
        let mut cache = RangeCache::new(2);
        let today = day("2026-10-15");
        let a = key("2026-01-01", "2026-01-31", "UTC");
        let b = key("2026-02-01", "2026-02-28", "UTC");
        let c = key("2026-03-01", "2026-03-31", "UTC");
        cache.insert(a.clone(), today, 1);
        cache.insert(b.clone(), today, 2);
        // Touching `a` makes `b` the oldest.
        assert!(cache.get(&a, today).is_some());
        cache.insert(c.clone(), today, 3);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&b, today), None);
        assert_eq!(cache.get(&a, today), Some(&1));
        assert_eq!(cache.get(&c, today), Some(&3));
    }

    #[test]
    fn inserting_again_replaces_the_entry() {
        let mut cache = RangeCache::new(4);
        let today = day("2026-10-15");
        let k = key("2026-10-09", "2026-10-15", "UTC");
        cache.insert(k.clone(), today, 1);
        cache.insert(k.clone(), today, 2);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&k, today), Some(&2));
    }

    #[test]
    fn refresh_drops_open_ranges_and_keeps_settled_ones() {
        let mut cache = RangeCache::new(4);
        let today = day("2026-10-15");
        let open = key("2026-10-09", "2026-10-15", "UTC");
        let settled = key("2025-01-01", "2025-12-31", "UTC");
        cache.insert(open.clone(), today, 1);
        cache.insert(settled.clone(), today, 2);
        cache.drop_open(today);
        assert_eq!(cache.get(&open, today), None);
        assert_eq!(cache.get(&settled, today), Some(&2));
    }
}
