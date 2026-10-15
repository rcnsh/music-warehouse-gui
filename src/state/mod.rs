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
use std::time::Duration;

use gpui::{Context, Task};

use crate::api::ApiError;
use crate::dates::{self, DateRange, RangePreset};
use crate::models::DailyResponse;
use crate::runtime;

/// Runs `request` on the HTTP runtime and stores its result in the
/// [`Remote`] that `slot` picks out of the entity, then notifies. Call
/// `start()` on that `Remote` first, and keep the returned task in the
/// entity: dropping it (say, by starting a newer request) cancels the
/// request and the update.
///
/// `request` builds the request, so it can be sent again: while the Worker
/// answers "still computing" the error is shown, loading stays on, and the
/// request is repeated after the wait it asked for (see [`compute_retry`]).
///
/// ```ignore
/// self.albums.start();
/// let client = self.client.clone();
/// self._task = Some(state::fetch(cx, move || {
///     let client = client.clone();
///     async move { client.top_albums().await }
/// }, |s| &mut s.albums));
/// ```
pub fn fetch<S, T, F>(
    cx: &mut Context<S>,
    request: impl Fn() -> F + 'static,
    slot: fn(&mut S) -> &mut Remote<T>,
) -> Task<()>
where
    S: 'static,
    T: Send + 'static,
    F: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    fetch_then(cx, request, slot, |_, _| {})
}

/// [`fetch`], then `after` once the result is stored.
pub fn fetch_then<S, T, F>(
    cx: &mut Context<S>,
    request: impl Fn() -> F + 'static,
    slot: fn(&mut S) -> &mut Remote<T>,
    after: fn(&mut S, &mut Context<S>),
) -> Task<()>
where
    S: 'static,
    T: Send + 'static,
    F: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    cx.notify();
    cx.spawn(async move |this, cx| {
        let mut retries = 0;
        loop {
            let result = runtime::run(request()).await;
            let wait = result
                .as_ref()
                .err()
                .and_then(|e| compute_retry(e, retries));
            let Ok(()) = this.update(cx, |this, cx| {
                let remote = slot(this);
                match wait {
                    // Shown as "still computing" while the wait runs.
                    Some(_) => remote.error = result.err(),
                    None => {
                        remote.finish(result);
                        after(this, cx);
                    }
                }
                cx.notify();
            }) else {
                return;
            };
            let Some(wait) = wait else { return };
            retries += 1;
            cx.background_executor().timer(wait).await;
        }
    })
}

/// How many times a "still computing" answer is asked again by itself.
const COMPUTE_RETRIES: u32 = 6;
/// The longest wait that is retried by itself. The Worker asks for at most
/// its two-minute compute claim while a compute runs; a longer wait means a
/// compute failed and is backing off, which is left to a manual retry.
const MAX_COMPUTE_WAIT: Duration = Duration::from_secs(120);

/// How long to wait before asking again after `error`, when it is the
/// Worker's "still computing" and fewer than [`COMPUTE_RETRIES`] retries
/// have been made. `None` means show the error and stop.
pub fn compute_retry(error: &ApiError, retries: u32) -> Option<Duration> {
    let ApiError::StillComputing {
        retry_after_seconds,
    } = error
    else {
        return None;
    };
    let wait = Duration::from_secs((*retry_after_seconds).max(1));
    (retries < COMPUTE_RETRIES && wait <= MAX_COMPUTE_WAIT).then_some(wait)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn computing(seconds: u64) -> ApiError {
        ApiError::StillComputing {
            retry_after_seconds: seconds,
        }
    }

    #[test]
    fn still_computing_is_retried_after_the_wait_it_asks_for() {
        assert_eq!(
            compute_retry(&computing(30), 0),
            Some(Duration::from_secs(30))
        );
        // The Worker never asks for zero, but a zero must not spin.
        assert_eq!(
            compute_retry(&computing(0), 0),
            Some(Duration::from_secs(1))
        );
    }

    #[test]
    fn compute_retries_are_bounded() {
        assert!(compute_retry(&computing(5), COMPUTE_RETRIES - 1).is_some());
        assert_eq!(compute_retry(&computing(5), COMPUTE_RETRIES), None);
        // An hour is a failed compute backing off: left to the user.
        assert_eq!(compute_retry(&computing(3600), 0), None);
    }

    #[test]
    fn other_errors_are_not_retried() {
        assert_eq!(compute_retry(&ApiError::Network("down".into()), 0), None);
        let generic = ApiError::Server {
            status: 503,
            message: "x".into(),
        };
        assert_eq!(compute_retry(&generic, 0), None);
    }
}
