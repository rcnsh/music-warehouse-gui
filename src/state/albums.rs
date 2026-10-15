//! Data for the Top Albums page: `GET /api/stats/albums`. Every year comes
//! in the one response, so switching years needs no request.

use gpui::{Context, Task};

use crate::api::ApiClient;
use crate::models::TopAlbums;
use crate::state::{self, Remote};

pub struct AlbumsStore {
    client: ApiClient,
    pub albums: Remote<TopAlbums>,
    _task: Option<Task<()>>,
}

impl AlbumsStore {
    /// Fetches nothing until the page first asks, so unvisited pages cost
    /// the Worker nothing.
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            albums: Remote::default(),
            _task: None,
        }
    }

    /// Fetches again, keeping what is shown until the answer arrives.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.albums.start();
        let client = self.client.clone();
        self._task = Some(state::fetch(
            cx,
            async move { client.top_albums().await },
            |s| &mut s.albums,
        ));
    }
}
