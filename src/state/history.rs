use std::collections::HashSet;

use chrono::NaiveDate;
use gpui::{Context, Task};

use crate::api::{ApiClient, ApiError};
use crate::dates;
use crate::models::Play;
use crate::runtime;

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
    exhausted: bool,
}

impl PlayLog {
    pub fn plays(&self) -> &[Play] {
        &self.plays
    }

    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// `before` is exclusive, so asking from one millisecond past the oldest
    /// row re-reads its timestamp: plays sharing it with a different track are
    /// picked up rather than skipped, and `seen` drops the repeat.
    pub fn next_before(&self) -> Option<i64> {
        self.plays.last().map(|p| p.played_at_ms + 1)
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

    pub fn append_page(&mut self, page: Vec<Play>, page_size: u32) {
        let full_page = page.len() as u32 >= page_size;
        let mut added = 0;
        for play in page {
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

/// One list of plays being paged from the Worker, with its request state.
#[derive(Default)]
struct Feed {
    log: PlayLog,
    loading: bool,
    error: Option<ApiError>,
    /// Replacing or dropping the feed cancels its in-flight page, so a stale
    /// response cannot append to a newer list.
    _task: Option<Task<()>>,
}

/// A filter the Worker answers over the whole history (`q`), paged like the
/// unfiltered list.
struct Search {
    query: String,
    feed: Feed,
}

/// Stored play history, extended page by page with `before`. Reads only
/// stored rows, so it keeps working when the Spotify grant is dead.
pub struct HistoryStore {
    client: ApiClient,
    main: Feed,
    /// While set, every accessor reads the search results instead of `main`,
    /// which stays loaded so clearing the filter is instant.
    search: Option<Search>,
    /// The day History was jumped to, if any. While set, the list starts at
    /// the end of that day instead of the newest play, and live updates pause
    /// because newer plays would not join up with the loaded rows.
    anchor: Option<NaiveDate>,
    _newer_task: Option<Task<()>>,
}

impl HistoryStore {
    pub fn new(client: ApiClient, cx: &mut Context<Self>) -> Self {
        let mut store = Self {
            client,
            main: Feed::default(),
            search: None,
            anchor: None,
            _newer_task: None,
        };
        store.fetch(None, cx);
        store
    }

    fn feed(&self) -> &Feed {
        self.search.as_ref().map_or(&self.main, |s| &s.feed)
    }

    fn feed_mut(&mut self) -> &mut Feed {
        match &mut self.search {
            Some(search) => &mut search.feed,
            None => &mut self.main,
        }
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

    pub fn query(&self) -> Option<&str> {
        self.search.as_ref().map(|s| s.query.as_str())
    }

    /// Shows plays matching `query` from across the whole history, or the
    /// unfiltered list again when it is empty.
    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.query().unwrap_or_default() == query {
            return;
        }
        self.search = (!query.is_empty()).then(|| Search {
            query: query.to_owned(),
            feed: Feed::default(),
        });
        if self.search.is_some() {
            self.fetch(self.anchor_before(), cx);
        }
        cx.notify();
    }

    /// Reloads what is on screen: from the newest play, or from the end of
    /// the jumped-to day.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
    }

    pub fn show_newest(&mut self, cx: &mut Context<Self>) {
        self.anchor = None;
        self.reload(cx);
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

    /// Restarts the unfiltered list and any search from the anchor, so both
    /// agree on where history starts.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let before = self.anchor_before();
        let search = self.search.take();
        self.main = Feed::default();
        self._newer_task = None;
        self.fetch(before, cx);
        if let Some(search) = search {
            self.search = Some(Search {
                query: search.query,
                feed: Feed::default(),
            });
            self.fetch(before, cx);
        }
    }

    /// Adds plays ingested since the newest loaded one. Cheap: the Worker
    /// reads only rows after the cursor, usually none. Skipped while jumped
    /// to a day, searching, loading, or failing.
    pub fn check_newer(&mut self, cx: &mut Context<Self>) {
        let Some(after) = self.main.log.newest_ms() else {
            return;
        };
        if self.anchor.is_some()
            || self.search.is_some()
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
                    Some(_) => cx.notify(),
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
        self.fetch(before, cx);
    }

    /// Clears an error and repeats the request that failed.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.feed_mut().error = None;
        let before = self.feed().log.next_before().or(self.anchor_before());
        self.fetch(before, cx);
    }

    fn anchor_before(&self) -> Option<i64> {
        self.anchor
            .and_then(|day| dates::end_of_local_day_ms(day, &chrono::Local))
    }

    /// Fetches the next page of whichever list is showing.
    fn fetch(&mut self, before: Option<i64>, cx: &mut Context<Self>) {
        let query = self.query().map(str::to_owned);
        let page_size = if query.is_some() {
            SEARCH_PAGE_SIZE
        } else {
            PAGE_SIZE
        };
        let client = self.client.clone();
        let task = cx.spawn(async move |this, cx| {
            let q = query.clone();
            let result =
                runtime::run(
                    async move { client.plays(page_size, before, None, q.as_deref()).await },
                )
                .await;
            this.update(cx, |this, cx| {
                // The task belongs to its feed, so a feed that was replaced
                // never sees this; one that is still here is the right one.
                let feed = match (&query, &mut this.search) {
                    (None, _) => &mut this.main,
                    (Some(_), Some(search)) => &mut search.feed,
                    (Some(_), None) => return,
                };
                feed.loading = false;
                match result {
                    Ok(page) => feed.log.append_page(page, page_size),
                    Err(error) => feed.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        });
        let feed = self.feed_mut();
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
            context_uri: None,
            context_type: None,
            source: None,
            track_name: None,
            duration_ms: None,
            album_name: None,
            image_url: None,
            artists: None,
        }
    }

    #[test]
    fn short_page_ends_history() {
        let mut log = PlayLog::default();
        log.append_page(vec![play(30, "a"), play(20, "b")], 3);
        assert!(log.is_exhausted());
        assert_eq!(log.next_before(), Some(21));
    }

    #[test]
    fn overlapping_pages_do_not_duplicate_and_keep_same_millisecond_plays() {
        let mut log = PlayLog::default();
        log.append_page(vec![play(30, "a"), play(20, "b")], 2);
        assert!(!log.is_exhausted());
        // The next request uses before=21, so the boundary row comes back
        // along with a different track that shares its timestamp.
        log.append_page(vec![play(20, "b"), play(20, "c")], 2);
        let ids: Vec<_> = log.plays().iter().map(|p| p.track_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert!(!log.is_exhausted());
    }

    #[test]
    fn full_page_of_repeats_stops_paging() {
        let mut log = PlayLog::default();
        log.append_page(vec![play(20, "a"), play(20, "b")], 2);
        log.append_page(vec![play(20, "a"), play(20, "b")], 2);
        assert!(log.is_exhausted());
        assert_eq!(log.plays().len(), 2);
    }

    #[test]
    fn newer_plays_go_on_top_without_duplicates() {
        let mut log = PlayLog::default();
        log.append_page(vec![play(30, "a"), play(20, "b")], 10);
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
        log.append_page(vec![play(10, "a")], 10);
        assert_eq!(
            log.prepend_page(vec![play(30, "c"), play(20, "b")], 2),
            None
        );
        assert_eq!(log.plays().len(), 1, "nothing merged across a possible gap");
    }

    #[test]
    fn empty_log_starts_from_newest() {
        assert_eq!(PlayLog::default().next_before(), None);
    }
}
