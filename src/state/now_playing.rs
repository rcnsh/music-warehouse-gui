use std::time::Duration;

use chrono::{DateTime, Local};
use gpui::{Context, Task};

use crate::api::{ApiClient, ApiError};
use crate::models::NowPlayingResponse;
use crate::runtime;

const PLAYING_INTERVAL: Duration = Duration::from_secs(20);
const IDLE_INTERVAL: Duration = Duration::from_secs(30);
const FIRST_BACKOFF: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(10 * 60);
/// Neither of these heals without a human, so checking often only spends
/// Worker requests; this is just slow enough to notice a fix on its own.
const NEEDS_HUMAN_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// A 429's Retry-After is honoured, but catalog quotas have answered with a
/// whole day; past an hour the strip would look dead, so cap it there.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

/// How long to wait before the next poll, given the last outcome and how many
/// failures in a row preceded it (including this one).
pub fn next_delay(outcome: &Result<bool, ApiError>, consecutive_failures: u32) -> Duration {
    let backoff = || {
        let doublings = consecutive_failures.saturating_sub(1).min(8);
        (FIRST_BACKOFF * 2u32.pow(doublings)).min(MAX_BACKOFF)
    };
    match outcome {
        Ok(true) => PLAYING_INTERVAL,
        Ok(false) => IDLE_INTERVAL,
        Err(ApiError::NeedsReauth | ApiError::Unauthorized) => NEEDS_HUMAN_INTERVAL,
        Err(ApiError::RateLimited {
            retry_after_seconds,
        }) => {
            let asked = Duration::from_secs(retry_after_seconds.unwrap_or(0)).min(MAX_RETRY_AFTER);
            asked.max(backoff())
        }
        Err(_) => backoff(),
    }
}

/// Polls `/api/now-playing` while the window is visible.
pub struct NowPlayingStore {
    client: ApiClient,
    pub last: Option<NowPlayingResponse>,
    pub error: Option<ApiError>,
    pub loading: bool,
    pub next_poll_at: Option<DateTime<Local>>,
    consecutive_failures: u32,
    visible: bool,
    /// Dropping the task cancels the loop, which is how polling pauses.
    _poll: Option<Task<()>>,
}

impl NowPlayingStore {
    pub fn new(client: ApiClient, cx: &mut Context<Self>) -> Self {
        let mut store = Self {
            client,
            last: None,
            error: None,
            loading: false,
            next_poll_at: None,
            consecutive_failures: 0,
            visible: true,
            _poll: None,
        };
        store.restart(cx);
        store
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if visible {
            // Whatever was playing when the window hid is stale now.
            self.restart(cx);
        } else {
            self._poll = None;
            self.next_poll_at = None;
            cx.notify();
        }
    }

    /// Polls immediately and resumes the normal cadence (cmd-R).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.visible {
            self.restart(cx);
        }
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self._poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let started = this.update(cx, |this, cx| {
                    this.loading = true;
                    cx.notify();
                });
                if started.is_err() {
                    break;
                }
                let request = client.clone();
                let result = runtime::run(async move { request.now_playing().await }).await;
                let Ok(delay) = this.update(cx, |this, cx| {
                    let delay = this.apply(result);
                    cx.notify();
                    delay
                }) else {
                    break;
                };
                cx.background_executor().timer(delay).await;
            }
        }));
    }

    fn apply(&mut self, result: Result<NowPlayingResponse, ApiError>) -> Duration {
        self.loading = false;
        let outcome = match result {
            Ok(response) => {
                let playing = response.item.as_ref().is_some_and(|c| c.is_playing);
                self.last = Some(response);
                self.error = None;
                self.consecutive_failures = 0;
                Ok(playing)
            }
            Err(error) => {
                self.consecutive_failures += 1;
                self.error = Some(error.clone());
                Err(error)
            }
        };
        let delay = next_delay(&outcome, self.consecutive_failures);
        self.next_poll_at = chrono::Duration::from_std(delay)
            .ok()
            .map(|d| Local::now() + d);
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_cadence_is_polite() {
        assert_eq!(next_delay(&Ok(true), 0), Duration::from_secs(20));
        assert_eq!(next_delay(&Ok(false), 0), Duration::from_secs(30));
    }

    #[test]
    fn generic_errors_back_off_exponentially_and_cap() {
        let err = Err(ApiError::Network("down".into()));
        assert_eq!(next_delay(&err, 1), Duration::from_secs(30));
        assert_eq!(next_delay(&err, 2), Duration::from_secs(60));
        assert_eq!(next_delay(&err, 3), Duration::from_secs(120));
        assert_eq!(next_delay(&err, 50), MAX_BACKOFF);
    }

    #[test]
    fn rate_limit_honours_retry_after_within_a_cap() {
        let limited = |s| {
            Err(ApiError::RateLimited {
                retry_after_seconds: s,
            })
        };
        assert_eq!(next_delay(&limited(Some(300)), 1), Duration::from_secs(300));
        // A short Retry-After never undercuts the backoff.
        assert_eq!(next_delay(&limited(Some(1)), 3), Duration::from_secs(120));
        assert_eq!(next_delay(&limited(None), 1), Duration::from_secs(30));
        assert_eq!(next_delay(&limited(Some(86_088)), 1), MAX_RETRY_AFTER);
    }

    #[test]
    fn conditions_needing_a_human_poll_rarely() {
        assert_eq!(
            next_delay(&Err(ApiError::NeedsReauth), 1),
            NEEDS_HUMAN_INTERVAL
        );
        assert_eq!(
            next_delay(&Err(ApiError::Unauthorized), 1),
            NEEDS_HUMAN_INTERVAL
        );
    }
}
