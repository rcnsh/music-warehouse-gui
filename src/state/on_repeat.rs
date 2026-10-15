//! Data for the On Repeat page: `GET /api/stats/on-repeat`.

use gpui::{Context, Task};

use crate::api::ApiClient;
use crate::models::OnRepeat;
use crate::state::{self, Remote};

pub struct OnRepeatStore {
    client: ApiClient,
    pub on_repeat: Remote<OnRepeat>,
    _task: Option<Task<()>>,
}

impl OnRepeatStore {
    /// Fetches nothing until the page first asks, so unvisited pages cost
    /// the Worker nothing.
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            on_repeat: Remote::default(),
            _task: None,
        }
    }

    /// Fetches again, keeping what is shown until the answer arrives.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.on_repeat.start();
        let client = self.client.clone();
        self._task = Some(state::fetch(
            cx,
            move || {
                let client = client.clone();
                async move { client.on_repeat().await }
            },
            |s| &mut s.on_repeat,
        ));
    }
}
