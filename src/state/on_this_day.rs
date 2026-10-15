//! Data for the On This Day page: `GET /api/on-this-day`.

use chrono::NaiveDate;
use gpui::{Context, Task};

use crate::api::ApiClient;
use crate::dates;
use crate::models::OnThisDay;
use crate::state::{self, Remote};

pub struct OnThisDayStore {
    client: ApiClient,
    /// The calendar date looked back on; `None` follows today.
    pub date: Option<NaiveDate>,
    pub day: Remote<OnThisDay>,
    _task: Option<Task<()>>,
}

impl OnThisDayStore {
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            date: None,
            day: Remote::default(),
            _task: None,
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.day.start();
        let client = self.client.clone();
        let date = self.date;
        let tz = dates::system_timezone();
        self._task = Some(state::fetch(
            cx,
            async move { client.on_this_day(date, &tz).await },
            |s| &mut s.day,
        ));
    }
}
