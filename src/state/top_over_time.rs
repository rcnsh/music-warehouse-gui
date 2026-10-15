//! Data for the Top Over Time page: `GET /api/stats/top-over-time`.

use gpui::{Context, Task};

use crate::api::ApiClient;
use crate::models::TopOverTime;
use crate::state::{self, Remote};

pub struct TopOverTimeStore {
    client: ApiClient,
    pub top: Remote<TopOverTime>,
    _task: Option<Task<()>>,
}

impl TopOverTimeStore {
    /// Fetches nothing until the page first asks, so unvisited pages cost
    /// the Worker nothing.
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            top: Remote::default(),
            _task: None,
        }
    }

    /// Fetches again, keeping what is shown until the answer arrives.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.top.start();
        let client = self.client.clone();
        self._task = Some(state::fetch(
            cx,
            move || {
                let client = client.clone();
                async move { client.top_over_time().await }
            },
            |s| &mut s.top,
        ));
    }
}
