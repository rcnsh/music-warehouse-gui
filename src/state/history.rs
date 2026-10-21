use std::collections::HashSet;

use chrono::{DateTime, NaiveDate};
use gpui::{Context, SharedString, Task};

use crate::api::{ApiClient, ApiError, Before, PlayFilter, PlaysPage};
use crate::dates;
use crate::models::{Credit, Play, RecentlyPlayedItem, pick_image};
use crate::runtime;
use crate::state::now_playing;

/// Rows per request. The Worker caps `limit` at 500; 200 fills a tall window
/// several times over while keeping each response small.
pub const PAGE_SIZE: u32 = 200;

/// Search pages are smaller: the Worker's cost per page grows with rows
/// returned (names are joined per row, and a broad search walks further to
/// fill a page), and a search is usually after one play near the top.
pub const SEARCH_PAGE_SIZE: u32 = 100;

/// Loaded plays, newest first, plus what is needed to ask for the next page.
/// Plain data so the paging rules are testable without a GPUI app.
#[derive(Debug, Default)]
pub struct PlayLog {
    plays: Vec<Play>,
    /// The warehouse's primary key, so overlapping pages never duplicate rows.
    seen: HashSet<(i64, String)>,
    /// The Worker's cursor for the next page, when it sent one.
    cursor: Option<String>,
    exhausted: bool,
}

impl PlayLog {
    pub fn plays(&self) -> &[Play] {
        &self.plays
    }

    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Where the next page starts: the Worker's cursor when it sent one.
    /// Otherwise, since `before` is exclusive, one millisecond past the
    /// oldest row, which re-reads its timestamp: plays sharing it with a
    /// different track are picked up rather than skipped, and `seen` drops
    /// the repeat.
    pub fn next_before(&self) -> Option<Before> {
        if let Some(cursor) = &self.cursor {
            return Some(Before::Cursor(cursor.clone()));
        }
        self.plays.last().map(|p| Before::Ms(p.played_at_ms + 1))
    }

    /// Merges plays newer than the newest loaded one (an `after` query) onto
    /// the top. Returns how many rows were added, or `None` when the page was
    /// full: more plays may sit between it and the loaded rows, and stitching
    /// across that gap would silently hide them, so the caller reloads instead.
    pub fn prepend_page(&mut self, page: Vec<Play>, page_size: u32) -> Option<usize> {
        if page.len() as u32 >= page_size {
            return None;
        }
        let fresh: Vec<Play> = page
            .into_iter()
            .filter(|p| self.seen.insert((p.played_at_ms, p.track_id.clone())))
            .collect();
        let added = fresh.len();
        self.plays.splice(0..0, fresh);
        Some(added)
    }

    pub fn newest_ms(&self) -> Option<i64> {
        self.plays.first().map(|p| p.played_at_ms)
    }

    pub fn contains(&self, play: &Play) -> bool {
        self.seen
            .contains(&(play.played_at_ms, play.track_id.clone()))
    }

    pub fn append_page(&mut self, page: PlaysPage, page_size: u32) {
        let PlaysPage {
            plays,
            next_before,
            last,
        } = page;
        self.cursor = next_before;
        // The Worker reads one row past the page, so a page it calls the
        // last is, even when it is full; without that word, a short page is
        // the only sign.
        let full_page = plays.len() as u32 >= page_size && !last;
        let mut added = 0;
        for play in plays {
            if self.seen.insert((play.played_at_ms, play.track_id.clone())) {
                self.plays.push(play);
                added += 1;
            }
        }
        // A full page of nothing new would otherwise re-request the same
        // window forever.
        self.exhausted = !full_page || added == 0;
    }
}

/// Spotify's last plays, as many as it returns (its maximum).
const LIVE_LIMIT: u32 = 50;

/// A play from Spotify's live recently-played list, in the stored row's
/// shape so the table, detail panel, copy and "open in Spotify" treat both
/// alike. `None` for items that cannot be a row: local files and removed
/// tracks have no track, and a play without a parsable time can't be placed.
pub fn live_play(item: &RecentlyPlayedItem) -> Option<Play> {
    let track = item.track.as_ref()?;
    let track_id = track.id.clone().filter(|id| !id.is_empty())?;
    let played_at_ms = DateTime::parse_from_rfc3339(item.played_at.as_deref()?)
        .ok()?
        .timestamp_millis();
    let artists: Vec<&str> = track
        .artists
        .iter()
        .filter_map(|a| a.name.as_deref())
        .collect();
    let credits = track
        .artists
        .iter()
        .filter_map(|a| {
            Some(Credit {
                id: a.id.clone().filter(|id| !id.is_empty()),
                name: a.name.clone()?,
            })
        })
        .collect();
    let context = item.context.as_ref();
    Some(Play {
        played_at_ms,
        track_id,
        context_uri: context.and_then(|c| c.uri.clone()),
        context_type: context.and_then(|c| c.kind.clone()),
        track_name: track.name.clone(),
        album_name: track.album.as_ref().and_then(|a| a.name.clone()),
        // The largest size, as stored rows have: renditions are picked per
        // use from that URL.
        image_url: pick_image(track.artwork(), 640).map(str::to_owned),
        artists: (!artists.is_empty()).then(|| artists.join(", ")),
        artist_list: Some(credits),
        // Spotify's payload names no context and numbers no plays.
        ..Play::default()
    })
}

/// One song's or one artist's plays, which History can be narrowed to
/// instead of (or as well as) a text search. `name` is only for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Every edition of the song, as the Track page counts it.
    Song {
        id: SharedString,
        name: SharedString,
    },
    /// Plays crediting the artist first, as the Artist page counts them. The
    /// id may be a `name:` placeholder.
    Artist {
        id: SharedString,
        name: SharedString,
    },
}

impl Scope {
    pub fn name(&self) -> &str {
        match self {
            Self::Song { name, .. } | Self::Artist { name, .. } => name,
        }
    }

    /// The name for sentences, "this song" or "this artist" when unknown.
    pub fn display_name(&self) -> &str {
        match (self, self.name()) {
            (Self::Song { .. }, "") => "this song",
            (Self::Artist { .. }, "") => "this artist",
            (_, name) => name,
        }
    }

    /// "Song: DNA." for the chip that shows it.
    pub fn label(&self) -> String {
        let kind = match self {
            Self::Song { .. } => "Song",
            Self::Artist { .. } => "Artist",
        };
        format!("{kind}: {}", self.display_name())
    }

    fn filter(&self) -> PlayFilter {
        match self {
            Self::Song { id, .. } => PlayFilter::song(id.to_string()),
            Self::Artist { id, .. } => PlayFilter::artist(id),
        }
    }
}

/// What the Worker is asked for: the scope narrowed by the search text.
pub fn list_filter(scope: Option<&Scope>, query: &str) -> PlayFilter {
    scope
        .map(Scope::filter)
        .unwrap_or_default()
        .search(Some(query))
}

/// Text searches walk further and join more names per page than one song's
/// or artist's plays, so their pages are smaller.
fn page_size(filter: &PlayFilter) -> u32 {
    if filter.q.is_some() {
        SEARCH_PAGE_SIZE
    } else {
        PAGE_SIZE
    }
}

/// The plays a list shows, as a CSV download asks for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportRequest {
    pub filter: PlayFilter,
    pub before: Option<i64>,
    pub after: Option<i64>,
}

/// The live plays the warehouse has not stored yet, newest first: those
/// newer than the newest stored play. Anything at or before it has either
/// been stored or never will be, so nothing shows twice; `contains` still
/// catches a second track stamped the same millisecond as a stored one.
pub fn unsaved_plays(live: &[Play], stored: &PlayLog) -> Vec<Play> {
    let newest = stored.newest_ms().unwrap_or(i64::MIN);
    let mut seen = HashSet::new();
    let mut unsaved: Vec<Play> = live
        .iter()
        .filter(|p| p.played_at_ms > newest && !stored.contains(p))
        .filter(|p| seen.insert((p.played_at_ms, p.track_id.clone())))
        .cloned()
        .collect();
    unsaved.sort_by_key(|p| std::cmp::Reverse(p.played_at_ms));
    unsaved
}

/// One list of plays being paged from the Worker, with its request state.
#[derive(Default)]
struct Feed {
    log: PlayLog,
    loading: bool,
    error: Option<ApiError>,
    /// Failed requests in a row, for the backoff before trying again.
    failures: u32,
    /// The in-flight page, or the wait before retrying a failed one.
    /// Replacing or dropping the feed cancels it, so a stale response cannot
    /// append to a newer list.
    _task: Option<Task<()>>,
}

/// Stored play history, extended page by page with `before`. Reads only
/// stored rows, so it keeps working when the Spotify grant is dead.
pub struct HistoryStore {
    client: ApiClient,
    main: Feed,
    /// The search text, applied by the Worker over the whole history.
    query: String,
    /// One song or artist the list is narrowed to.
    scope: Option<Scope>,
    /// Present while `query` or `scope` narrows the list. Every accessor
    /// then reads it instead of `main`, which stays loaded so clearing the
    /// filter is instant.
    filtered: Option<Feed>,
    /// The day History was jumped to, if any. While set, the list starts at
    /// the end of that day instead of the newest play, and live updates pause
    /// because newer plays would not join up with the loaded rows.
    anchor: Option<NaiveDate>,
    /// The first day a search covers, if limited. The end is the anchor.
    since: Option<NaiveDate>,
    /// Spotify's live last plays as fetched; `unsaved` is the part of them
    /// the warehouse has not stored yet.
    live: Vec<Play>,
    unsaved: Vec<Play>,
    _newer_task: Option<Task<()>>,
    _live_task: Option<Task<()>>,
}

impl HistoryStore {
    pub fn new(client: ApiClient, cx: &mut Context<Self>) -> Self {
        let mut store = Self {
            client,
            main: Feed::default(),
            query: String::new(),
            scope: None,
            filtered: None,
            anchor: None,
            since: None,
            live: Vec::new(),
            unsaved: Vec::new(),
            _newer_task: None,
            _live_task: None,
        };
        store.fetch(false, None, cx);
        store.fetch_live(cx);
        store
    }

    fn feed(&self) -> &Feed {
        self.filtered.as_ref().unwrap_or(&self.main)
    }

    pub fn plays(&self) -> &[Play] {
        self.feed().log.plays()
    }

    pub fn is_loading(&self) -> bool {
        self.feed().loading
    }

    pub fn error(&self) -> Option<&ApiError> {
        self.feed().error.as_ref()
    }

    pub fn has_more(&self) -> bool {
        !self.feed().log.is_exhausted()
    }

    pub fn anchor(&self) -> Option<NaiveDate> {
        self.anchor
    }

    pub fn since(&self) -> Option<NaiveDate> {
        self.since
    }

    /// Plays Spotify reports that the warehouse has not stored yet, to show
    /// above the stored rows. Only on the newest, unfiltered list: they are
    /// newer than any jumped-to day, and no filter was applied to them.
    pub fn unsaved(&self) -> &[Play] {
        if self.filtered.is_some() || self.anchor.is_some() {
            return &[];
        }
        &self.unsaved
    }

    /// Limits a filtered list to plays from the start of `day` on, or lifts
    /// the limit with `None`. Restarts a filtered list that is showing.
    pub fn set_since(&mut self, day: Option<NaiveDate>, cx: &mut Context<Self>) {
        if self.since == day {
            return;
        }
        self.since = day;
        self.restart_filtered(cx);
        cx.notify();
    }

    /// The search text, if any.
    pub fn query(&self) -> Option<&str> {
        Some(self.query.as_str()).filter(|q| !q.is_empty())
    }

    pub fn scope(&self) -> Option<&Scope> {
        self.scope.as_ref()
    }

    pub fn is_filtered(&self) -> bool {
        self.filtered.is_some()
    }

    /// Shows plays matching `query` from across the whole history (inside
    /// the scope, if one is set), or drops the search when it is empty.
    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.query == query {
            return;
        }
        self.query = query.to_owned();
        self.restart_filtered(cx);
        cx.notify();
    }

    /// Narrows the list to one song or artist, keeping any search and date
    /// bounds, or widens it again with `None`.
    pub fn set_scope(&mut self, scope: Option<Scope>, cx: &mut Context<Self>) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.restart_filtered(cx);
        cx.notify();
    }

    /// Every play of one song or artist, newest first, with nothing else
    /// narrowing it: how another page opens History.
    pub fn show_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.query.clear();
        self.since = None;
        self.scope = Some(scope);
        if self.anchor.take().is_some() {
            self.reload(cx);
            self.fetch_live(cx);
        } else {
            self.restart_filtered(cx);
        }
        cx.notify();
    }

    /// What a CSV of this list asks for: its filter and date bounds. The
    /// unsaved plays above it are Spotify's, not the warehouse's, so they
    /// are not part of it.
    pub fn export_request(&self) -> ExportRequest {
        let filter = list_filter(self.scope.as_ref(), &self.query);
        let after = if filter.is_empty() {
            None
        } else {
            self.since_after()
        };
        ExportRequest {
            filter,
            before: self.anchor_before_ms(),
            after,
        }
    }

    /// Reloads what is on screen: from the newest play, or from the end of
    /// the jumped-to day.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
        self.fetch_live(cx);
    }

    pub fn show_newest(&mut self, cx: &mut Context<Self>) {
        self.anchor = None;
        self.reload(cx);
        self.fetch_live(cx);
    }

    /// Shows plays from the end of `day` (local time) backwards, using the
    /// Worker's `before` cursor rather than paging through everything newer.
    pub fn jump_to(&mut self, day: NaiveDate, cx: &mut Context<Self>) {
        // Today's end is in the future, so that list is the newest one; keep
        // it unpinned so new plays still arrive on top.
        if day >= dates::today_local() {
            return self.show_newest(cx);
        }
        self.anchor = Some(day);
        self.reload(cx);
    }

    /// Restarts the unfiltered list and any filtered one from the anchor, so
    /// both agree on where history starts.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let before = self.anchor_before();
        self.main = Feed::default();
        self._newer_task = None;
        self.fetch(false, before, cx);
        self.update_unsaved();
        self.restart_filtered(cx);
    }

    /// Starts the filtered list over for the current search, scope and
    /// bounds, or drops it when nothing narrows the list.
    fn restart_filtered(&mut self, cx: &mut Context<Self>) {
        let filtered = !list_filter(self.scope.as_ref(), &self.query).is_empty();
        self.filtered = filtered.then(Feed::default);
        if filtered {
            self.fetch(true, self.anchor_before(), cx);
        }
    }

    /// Adds plays ingested since the newest loaded one. Cheap: the Worker
    /// reads only rows after the cursor, usually none. Skipped while jumped
    /// to a day, filtering, loading, or failing; a failed list is retried
    /// instead, so returning to the window recovers it straight away.
    pub fn check_newer(&mut self, cx: &mut Context<Self>) {
        if self.anchor.is_none() {
            self.fetch_live(cx);
        }
        self.recover(false, cx);
        self.recover(true, cx);
        let Some(after) = self.main.log.newest_ms() else {
            return;
        };
        if self.anchor.is_some()
            || self.filtered.is_some()
            || self.main.loading
            || self.main.error.is_some()
        {
            return;
        }
        // One millisecond early: `after` is exclusive, and a second play
        // stamped the same millisecond would otherwise never show up. The
        // dedupe drops the one already loaded.
        let after = after - 1;
        let client = self.client.clone();
        self._newer_task = Some(cx.spawn(async move |this, cx| {
            let result =
                runtime::run(async move { client.plays_after(PAGE_SIZE, after).await }).await;
            this.update(cx, |this, cx| {
                // A failed background check is not worth an error banner;
                // the next check or a manual refresh will try again.
                let Ok(page) = result else { return };
                if page.is_empty() {
                    return;
                }
                match this.main.log.prepend_page(page, PAGE_SIZE) {
                    Some(0) => {}
                    Some(_) => {
                        this.update_unsaved();
                        cx.notify();
                    }
                    None => this.show_newest(cx),
                }
            })
            .ok();
        }));
    }

    /// Requests the page before the oldest loaded play. A no-op while a page
    /// is in flight, after the last page, or after an error until the user
    /// retries, so scroll-driven calls cannot hammer a failing Worker.
    pub fn load_older(&mut self, cx: &mut Context<Self>) {
        let feed = self.feed();
        if feed.loading || feed.log.is_exhausted() || feed.error.is_some() {
            return;
        }
        let before = feed.log.next_before().or(self.anchor_before());
        self.fetch(self.filtered.is_some(), before, cx);
    }

    /// Clears an error and repeats the request that failed.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.retry_feed(self.filtered.is_some(), cx);
    }

    fn retry_feed(&mut self, filtered: bool, cx: &mut Context<Self>) {
        let anchor_before = self.anchor_before();
        let Some(feed) = self.feed_at(filtered) else {
            return;
        };
        feed.error = None;
        let before = feed.log.next_before().or(anchor_before);
        self.fetch(filtered, before, cx);
    }

    /// Retries a list whose last request failed, unless one is in flight.
    fn recover(&mut self, filtered: bool, cx: &mut Context<Self>) {
        if self
            .feed_at(filtered)
            .is_some_and(|feed| feed.error.is_some() && !feed.loading)
        {
            self.retry_feed(filtered, cx);
        }
    }

    fn feed_at(&mut self, filtered: bool) -> Option<&mut Feed> {
        if filtered {
            self.filtered.as_mut()
        } else {
            Some(&mut self.main)
        }
    }

    /// Asks Spotify for its last plays. They are an optional extra: when
    /// Spotify is not connected or the call fails, there are no unsaved rows
    /// and no error banner.
    fn fetch_live(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self._live_task = Some(cx.spawn(async move |this, cx| {
            let result =
                runtime::run(async move { client.recently_played(LIVE_LIMIT).await }).await;
            this.update(cx, |this, cx| {
                this.live = result
                    .map(|r| r.items.iter().filter_map(live_play).collect())
                    .unwrap_or_default();
                this.update_unsaved();
                cx.notify();
            })
            .ok();
        }));
    }

    /// Recomputes the unsaved rows after either list changes. None show
    /// until the first stored page is in, or every live play would flash up
    /// as unsaved while it loads.
    fn update_unsaved(&mut self) {
        self.unsaved = if self.main.loading || self.main.error.is_some() {
            Vec::new()
        } else {
            unsaved_plays(&self.live, &self.main.log)
        };
    }

    /// The exclusive `after` bound for the start of `since`: one millisecond
    /// before its local midnight, so a play at exactly midnight is included.
    fn since_after(&self) -> Option<i64> {
        let day = self.since?.pred_opt()?;
        dates::end_of_local_day_ms(day, &chrono::Local).map(|ms| ms - 1)
    }

    fn anchor_before(&self) -> Option<Before> {
        self.anchor_before_ms().map(Before::Ms)
    }

    fn anchor_before_ms(&self) -> Option<i64> {
        self.anchor
            .and_then(|day| dates::end_of_local_day_ms(day, &chrono::Local))
    }

    /// Fetches the next page of the unfiltered list or the filtered one.
    fn fetch(&mut self, filtered: bool, before: Option<Before>, cx: &mut Context<Self>) {
        let filter = if filtered {
            list_filter(self.scope.as_ref(), &self.query)
        } else {
            PlayFilter::default()
        };
        // The start bound narrows filtered lists; the unfiltered list always
        // runs back to the first play.
        let after = if filtered { self.since_after() } else { None };
        let page_size = page_size(&filter);
        let client = self.client.clone();
        let task = cx.spawn(async move |this, cx| {
            let result =
                runtime::run(async move { client.plays(page_size, before, after, &filter).await })
                    .await;
            this.update(cx, |this, cx| {
                // The task belongs to its feed, so a feed that was replaced
                // never sees this; one that is still here is the right one.
                let feed = match (filtered, &mut this.filtered) {
                    (false, _) => &mut this.main,
                    (true, Some(feed)) => feed,
                    (true, None) => return,
                };
                feed.loading = false;
                match result {
                    Ok(page) => {
                        feed.failures = 0;
                        feed.log.append_page(page, page_size);
                    }
                    Err(error) => {
                        // Tries again by itself, backing off as the
                        // now-playing strip does, so a list that failed
                        // to load (asleep, offline) fills in once the
                        // Worker answers.
                        feed.failures += 1;
                        let wait = now_playing::next_delay(&Err(error.clone()), feed.failures);
                        feed.error = Some(error);
                        feed._task = Some(cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(wait).await;
                            this.update(cx, |this, cx| this.recover(filtered, cx)).ok();
                        }));
                    }
                }
                this.update_unsaved();
                cx.notify();
            })
            .ok();
        });
        let feed = if filtered {
            self.filtered.get_or_insert_with(Feed::default)
        } else {
            &mut self.main
        };
        feed.loading = true;
        feed._task = Some(task);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(ms: i64, track: &str) -> Play {
        Play {
            played_at_ms: ms,
            track_id: track.into(),
            ..Play::default()
        }
    }

    fn page(plays: Vec<Play>) -> PlaysPage {
        PlaysPage {
            plays,
            next_before: None,
            last: false,
        }
    }

    fn song() -> Scope {
        Scope::Song {
            id: "67XC51nlZncNpHmZ8rOU9a".into(),
            name: "We Cry Together".into(),
        }
    }

    #[test]
    fn a_scope_pages_one_song_or_artist_and_narrows_a_search() {
        assert!(list_filter(None, "").is_empty());
        assert_eq!(
            list_filter(None, "dna"),
            PlayFilter::default().search(Some("dna"))
        );
        assert_eq!(
            list_filter(Some(&song()), ""),
            PlayFilter::song("67XC51nlZncNpHmZ8rOU9a")
        );
        let artist = Scope::Artist {
            id: "name:sigur rós".into(),
            name: "Sigur Rós".into(),
        };
        let filter = list_filter(Some(&artist), "hopp");
        assert_eq!(filter.artist_id.as_deref(), Some("name:sigur rós"));
        assert_eq!(filter.artist_name.as_deref(), Some("sigur rós"));
        assert_eq!(filter.q.as_deref(), Some("hopp"));
        // Searches get the smaller pages; a song's plays the usual ones.
        assert_eq!(page_size(&filter), SEARCH_PAGE_SIZE);
        assert_eq!(page_size(&list_filter(Some(&song()), "")), PAGE_SIZE);
    }

    #[test]
    fn scopes_label_their_chip() {
        assert_eq!(song().label(), "Song: We Cry Together");
        let unnamed = Scope::Artist {
            id: "abc".into(),
            name: "".into(),
        };
        assert_eq!(unnamed.label(), "Artist: this artist");
    }

    #[test]
    fn live_plays_keep_their_credit_ids() {
        let path = format!(
            "{}/tests/fixtures/recently-played.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let body = std::fs::read_to_string(path).unwrap();
        let response: crate::models::RecentlyPlayedResponse = serde_json::from_str(&body).unwrap();
        let play = live_play(&response.items[0]).unwrap();
        let credits = play.credits();
        assert_eq!(credits[0].name, "Kendrick Lamar");
        assert!(credits[0].id.is_some());
        assert_eq!(play.play_number, None);
    }

    #[test]
    fn short_page_ends_history() {
        let mut log = PlayLog::default();
        log.append_page(page(vec![play(30, "a"), play(20, "b")]), 3);
        assert!(log.is_exhausted());
        assert_eq!(log.next_before(), Some(Before::Ms(21)));
    }

    #[test]
    fn overlapping_pages_do_not_duplicate_and_keep_same_millisecond_plays() {
        let mut log = PlayLog::default();
        log.append_page(page(vec![play(30, "a"), play(20, "b")]), 2);
        assert!(!log.is_exhausted());
        // The next request uses before=21, so the boundary row comes back
        // along with a different track that shares its timestamp.
        log.append_page(page(vec![play(20, "b"), play(20, "c")]), 2);
        let ids: Vec<_> = log.plays().iter().map(|p| p.track_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert!(!log.is_exhausted());
    }

    #[test]
    fn full_page_of_repeats_stops_paging() {
        let mut log = PlayLog::default();
        log.append_page(page(vec![play(20, "a"), play(20, "b")]), 2);
        log.append_page(page(vec![play(20, "a"), play(20, "b")]), 2);
        assert!(log.is_exhausted());
        assert_eq!(log.plays().len(), 2);
    }

    #[test]
    fn newer_plays_go_on_top_without_duplicates() {
        let mut log = PlayLog::default();
        log.append_page(page(vec![play(30, "a"), play(20, "b")]), 10);
        assert_eq!(log.newest_ms(), Some(30));
        // The store asks from one millisecond early, so a repeat is normal.
        assert_eq!(
            log.prepend_page(vec![play(50, "d"), play(40, "c"), play(30, "a")], 10),
            Some(2)
        );
        let ids: Vec<_> = log.plays().iter().map(|p| p.track_id.as_str()).collect();
        assert_eq!(ids, ["d", "c", "a", "b"]);
    }

    #[test]
    fn full_page_of_newer_plays_asks_for_a_reload() {
        let mut log = PlayLog::default();
        log.append_page(page(vec![play(10, "a")]), 10);
        assert_eq!(
            log.prepend_page(vec![play(30, "c"), play(20, "b")], 2),
            None
        );
        assert_eq!(log.plays().len(), 1, "nothing merged across a possible gap");
    }

    fn ids(plays: &[Play]) -> Vec<&str> {
        plays.iter().map(|p| p.track_id.as_str()).collect()
    }

    #[test]
    fn only_plays_newer_than_the_newest_stored_one_are_unsaved() {
        let mut log = PlayLog::default();
        log.append_page(page(vec![play(30, "a"), play(20, "b")]), 10);
        // Spotify's list overlaps the stored rows: 30/a is already stored,
        // and 25/x is older than the newest stored play, so it is either
        // stored already under the Worker's own dedupe or never will be.
        let live = [play(50, "d"), play(40, "c"), play(30, "a"), play(25, "x")];
        assert_eq!(ids(&unsaved_plays(&live, &log)), ["d", "c"]);
    }

    #[test]
    fn unsaved_plays_are_deduped_and_newest_first() {
        let log = PlayLog::default();
        let live = [play(40, "c"), play(50, "d"), play(40, "c"), play(40, "e")];
        // Nothing stored yet: every live play is unsaved, once each, and a
        // different track in the same millisecond is kept.
        assert_eq!(ids(&unsaved_plays(&live, &log)), ["d", "c", "e"]);
    }

    #[test]
    fn a_play_once_stored_stops_being_unsaved() {
        let mut log = PlayLog::default();
        log.append_page(page(vec![play(30, "a")]), 10);
        let live = [play(40, "c"), play(30, "a")];
        assert_eq!(ids(&unsaved_plays(&live, &log)), ["c"]);
        // The next check finds the Worker stored it.
        log.prepend_page(vec![play(40, "c")], 10);
        assert!(unsaved_plays(&live, &log).is_empty());
    }

    #[test]
    fn live_items_become_rows_like_stored_ones() {
        let path = format!(
            "{}/tests/fixtures/recently-played.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let body = std::fs::read_to_string(path).unwrap();
        let response: crate::models::RecentlyPlayedResponse = serde_json::from_str(&body).unwrap();
        let plays: Vec<Play> = response.items.iter().filter_map(live_play).collect();
        assert_eq!(plays.len(), 2);
        let first = &plays[0];
        assert_eq!(first.track_id, "1QPreu0BNOrUfEb8HTd2qG");
        assert_eq!(first.track_name.as_deref(), Some("Rich Spirit"));
        assert_eq!(first.artists.as_deref(), Some("Kendrick Lamar"));
        assert_eq!(first.context_type.as_deref(), Some("album"));
        assert_eq!(
            first.played_at_ms,
            DateTime::parse_from_rfc3339("2026-10-15T08:35:58.853Z")
                .unwrap()
                .timestamp_millis()
        );
        assert!(first.image_url.as_deref().unwrap().contains("b273"));
    }

    #[test]
    fn live_items_without_a_track_or_time_are_skipped() {
        assert!(live_play(&RecentlyPlayedItem::default()).is_none());
        let item = RecentlyPlayedItem {
            played_at: Some("not a time".into()),
            ..RecentlyPlayedItem::default()
        };
        assert!(live_play(&item).is_none());
    }

    #[test]
    fn the_workers_cursor_is_used_when_it_sends_one() {
        let mut log = PlayLog::default();
        log.append_page(
            PlaysPage {
                plays: vec![play(30, "a"), play(20, "b")],
                next_before: Some("20.b".into()),
                last: false,
            },
            2,
        );
        assert_eq!(log.next_before(), Some(Before::Cursor("20.b".into())));
        // A Worker without cursors falls back to the timestamp.
        log.append_page(page(vec![play(10, "c"), play(5, "d")]), 2);
        assert_eq!(log.next_before(), Some(Before::Ms(6)));
    }

    #[test]
    fn a_full_page_the_worker_calls_the_last_ends_the_list() {
        // The Worker's last page can be exactly full: it read one row past
        // it and found none, so asking for more would only re-read the end.
        let mut log = PlayLog::default();
        log.append_page(
            PlaysPage {
                plays: vec![play(30, "a"), play(20, "b")],
                next_before: None,
                last: true,
            },
            2,
        );
        assert!(log.is_exhausted());
        // A Worker older than cursors leaves a full page open.
        let mut old = PlayLog::default();
        old.append_page(page(vec![play(30, "a"), play(20, "b")]), 2);
        assert!(!old.is_exhausted());
    }

    #[test]
    fn plays_responses_decode_with_and_without_a_cursor() {
        let with: crate::models::PlaysResponse =
            serde_json::from_str(r#"{"plays":[],"next_before":"abc"}"#).unwrap();
        assert_eq!(with.next_before, Some(Some("abc".into())));
        let done: crate::models::PlaysResponse =
            serde_json::from_str(r#"{"plays":[],"next_before":null}"#).unwrap();
        assert_eq!(done.next_before, Some(None));
        let old: crate::models::PlaysResponse = serde_json::from_str(r#"{"plays":[]}"#).unwrap();
        assert_eq!(old.next_before, None);
    }

    #[test]
    fn empty_log_starts_from_newest() {
        assert_eq!(PlayLog::default().next_before(), None);
    }
}
