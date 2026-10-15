//! Data for the Rhythm page: `GET /api/stats/rhythm`. Every year comes in
//! the one response, so switching years needs no request.

use gpui::{Context, Task};

use crate::api::ApiClient;
use crate::models::Rhythm;
use crate::state::{self, Remote};

pub struct RhythmStore {
    client: ApiClient,
    pub rhythm: Remote<Rhythm>,
    _task: Option<Task<()>>,
}

impl RhythmStore {
    /// Fetches nothing until the page first asks, so unvisited pages cost
    /// the Worker nothing.
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            rhythm: Remote::default(),
            _task: None,
        }
    }

    /// Fetches again, keeping what is shown until the answer arrives.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.rhythm.start();
        let client = self.client.clone();
        self._task = Some(state::fetch(
            cx,
            move || {
                let client = client.clone();
                async move { client.rhythm().await }
            },
            |s| &mut s.rhythm,
        ));
    }
}
