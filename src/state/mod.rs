//! Entities that own fetched data. Views observe these and render; they never
//! talk to the API directly.
//!
//! Each page has its own module here, owned with its view in `views/`.

pub mod albums;
pub mod artist;
pub mod concerts;
pub mod eras;
pub mod export;
pub mod history;
pub mod now_playing;
pub mod on_repeat;
pub mod on_this_day;
pub mod overview;
pub mod rhythm;
pub mod top_over_time;
pub mod track;
pub mod wrapped;

use std::future::Future;

use gpui::{Context, Task};

use crate::api::ApiError;
use crate::dates::{self, DateRange, RangePreset};
use crate::models::DailyResponse;
use crate::runtime;

/// Runs `request` on the HTTP runtime and stores its result in the
/// [`Remote`] that `slot` picks out of the entity, then notifies. Call
/// `start()` on that `Remote` first, and keep the returned task in the
/// entity: dropping it (say, by starting a newer request) cancels the update.
///
/// ```ignore
/// self.albums.start();
/// let client = self.client.clone();
/// self._task = Some(state::fetch(cx, async move { client.top_albums().await }, |s| &mut s.albums));
/// ```
pub fn fetch<S, T, F>(
    cx: &mut Context<S>,
    request: F,
    slot: fn(&mut S) -> &mut Remote<T>,
) -> Task<()>
where
    S: 'static,
    T: Send + 'static,
    F: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    cx.notify();
    cx.spawn(async move |this, cx| {
        let result = runtime::run(request).await;
        this.update(cx, |this, cx| {
            slot(this).finish(result);
            cx.notify();
        })
        .ok();
    })
}

/// One song's or artist's plays per day over a recent range, as the Artist
/// and Track pages chart them beside their all-time months.
#[derive(Debug)]
pub struct RecentDays {
    pub preset: RangePreset,
    /// The days last asked for, so the chart's caption matches its bars.
    pub range: Option<DateRange>,
    pub days: Remote<DailyResponse>,
}

impl Default for RecentDays {
    fn default() -> Self {
        Self {
            preset: RangePreset::Last90Days,
            range: None,
            days: Remote::default(),
        }
    }
}

impl RecentDays {
    /// Marks a fetch started and returns the range to ask for, ending today.
    pub fn start(&mut self) -> Option<DateRange> {
        let range = self.preset.resolve(dates::today_local())?;
        self.range = Some(range);
        self.days.start();
        Some(range)
    }
}

/// A remote value that keeps its last good data while reloading or after a
/// failed refresh, so a transient error does not blank a populated view.
#[derive(Debug, Clone)]
pub struct Remote<T> {
    pub data: Option<T>,
    pub loading: bool,
    pub error: Option<ApiError>,
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self {
            data: None,
            loading: false,
            error: None,
        }
    }
}

impl<T> Remote<T> {
    pub fn start(&mut self) {
        self.loading = true;
    }

    pub fn finish(&mut self, result: Result<T, ApiError>) {
        self.loading = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Clears data that belongs to a different query, such as a new range.
    pub fn reset(&mut self) {
        self.data = None;
        self.error = None;
    }
}
