//! Data for the Concerts page: `GET /api/concerts`, every configured region
//! at once. Region filtering happens in the view over this one list; only
//! "new since" goes back to the Worker, which owns `first_seen`'s day rules.

use chrono::NaiveDate;
use gpui::{Context, Task};

use crate::api::ApiClient;
use crate::dates;
use crate::models::ConcertsResponse;
use crate::state::{self, Remote};

/// How far back "New" looks.
pub const NEW_DAYS: i64 = 7;

pub struct ConcertsStore {
    client: ApiClient,
    /// Only events the Worker first saw in the last [`NEW_DAYS`] days.
    pub new_only: bool,
    pub concerts: Remote<ConcertsResponse>,
    _task: Option<Task<()>>,
}

impl ConcertsStore {
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            new_only: false,
            concerts: Remote::default(),
            _task: None,
        }
    }

    pub fn set_new_only(&mut self, new_only: bool, cx: &mut Context<Self>) {
        if new_only == self.new_only {
            return;
        }
        self.new_only = new_only;
        self.concerts.reset();
        self.refresh(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.concerts.start();
        let client = self.client.clone();
        let tz = dates::system_timezone();
        let since = self.new_only.then(|| new_since(dates::today_local()));
        self._task = Some(state::fetch(
            cx,
            move || {
                let (client, tz) = (client.clone(), tz.clone());
                async move { client.concerts(None, since, &tz).await }
            },
            |s| &mut s.concerts,
        ));
    }
}

/// The first day counted as new: today and the six before it.
fn new_since(today: NaiveDate) -> NaiveDate {
    today - chrono::Duration::days(NEW_DAYS - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_covers_a_week_including_today() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        assert_eq!(
            new_since(today),
            NaiveDate::from_ymd_opt(2026, 10, 9).unwrap()
        );
    }
}
