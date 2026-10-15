//! Data for the Eras page: `GET /api/stats/eras`.

use gpui::{Context, Task};

use crate::api::ApiClient;
use crate::models::Eras;
use crate::state::{self, Remote};

pub struct ErasStore {
    client: ApiClient,
    pub eras: Remote<Eras>,
    _task: Option<Task<()>>,
}

impl ErasStore {
    /// Fetches nothing until the page first asks, so unvisited pages cost
    /// the Worker nothing.
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            eras: Remote::default(),
            _task: None,
        }
    }

    /// Fetches again, keeping what is shown until the answer arrives.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.eras.start();
        let client = self.client.clone();
        self._task = Some(state::fetch(
            cx,
            move || {
                let client = client.clone();
                async move { client.eras().await }
            },
            |s| &mut s.eras,
        ));
    }
}
