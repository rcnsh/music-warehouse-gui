use std::collections::HashSet;

use gpui::{Context, Task};

use crate::api::{ApiClient, ApiError};
use crate::models::Play;
use crate::runtime;

/// Rows per request. The Worker caps `limit` at 500; 200 fills a tall window
/// several times over while keeping each response small.
pub const PAGE_SIZE: u32 = 200;

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

/// Stored play history, extended page by page with `before`. Reads only
/// stored rows, so it keeps working when the Spotify grant is dead.
pub struct HistoryStore {
    client: ApiClient,
    log: PlayLog,
    loading: bool,
    error: Option<ApiError>,
    _task: Option<Task<()>>,
}

impl HistoryStore {
    pub fn new(client: ApiClient, cx: &mut Context<Self>) -> Self {
        let mut store = Self {
            client,
            log: PlayLog::default(),
            loading: false,
            error: None,
            _task: None,
        };
        store.fetch(None, cx);
        store
    }

    pub fn plays(&self) -> &[Play] {
        self.log.plays()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&ApiError> {
        self.error.as_ref()
    }

    pub fn has_more(&self) -> bool {
        !self.log.is_exhausted()
    }

    /// Starts over from the newest play. Replacing the task cancels any
    /// in-flight page, so a stale response cannot append to the new list.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.log = PlayLog::default();
        self.error = None;
        self.fetch(None, cx);
    }

    /// Requests the page before the oldest loaded play. A no-op while a page
    /// is in flight, after the last page, or after an error until the user
    /// retries, so scroll-driven calls cannot hammer a failing Worker.
    pub fn load_older(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.log.is_exhausted() || self.error.is_some() {
            return;
        }
        self.fetch(self.log.next_before(), cx);
    }

    /// Clears an error and repeats the request that failed.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.fetch(self.log.next_before(), cx);
    }

    fn fetch(&mut self, before: Option<i64>, cx: &mut Context<Self>) {
        self.loading = true;
        cx.notify();
        let client = self.client.clone();
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = runtime::run(async move { client.plays(PAGE_SIZE, before).await }).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(page) => this.log.append_page(page, PAGE_SIZE),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        }));
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
    fn empty_log_starts_from_newest() {
        assert_eq!(PlayLog::default().next_before(), None);
    }
}
