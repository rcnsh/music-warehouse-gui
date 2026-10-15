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

    /// The day shown: the chosen one, or today's.
    pub fn shown_date(&self) -> NaiveDate {
        self.date.unwrap_or_else(dates::today_local)
    }

    /// The day the loaded years belong to, which is what the page describes.
    /// Following today, that is the Worker's answer rather than the clock,
    /// so the page does not name tomorrow at midnight before its data moves.
    pub fn described_date(&self) -> NaiveDate {
        if self.date.is_none()
            && let Some(day) = self.day.data.as_ref().and_then(|d| d.date.parse().ok())
        {
            return day;
        }
        self.shown_date()
    }

    /// Moves to another day. Today's date is stored as `None` so the page
    /// keeps following the calendar if it stays open past midnight.
    pub fn set_date(&mut self, date: NaiveDate, cx: &mut Context<Self>) {
        let date = (date != dates::today_local()).then_some(date);
        if date == self.date && self.day.data.is_some() {
            return;
        }
        self.date = date;
        // The old day's years would read as the new day's while it loads.
        self.day.reset();
        self.refresh(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.day.start();
        let client = self.client.clone();
        let date = self.date;
        let tz = dates::system_timezone();
        self._task = Some(state::fetch(
            cx,
            move || {
                let (client, tz) = (client.clone(), tz.clone());
                async move { client.on_this_day(date, &tz).await }
            },
            |s| &mut s.day,
        ));
    }
}
