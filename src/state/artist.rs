//! Data for the Artist page: `GET /api/artist` for one artist at a time.

use gpui::{Context, SharedString, Task};

use crate::api::ApiClient;
use crate::dates;
use crate::models::ArtistHistory;
use crate::state::{self, Remote};

pub struct ArtistStore {
    client: ApiClient,
    /// The artist shown: a Spotify id or a `name:` placeholder.
    pub id: Option<SharedString>,
    pub history: Remote<ArtistHistory>,
    _task: Option<Task<()>>,
}

impl ArtistStore {
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            id: None,
            history: Remote::default(),
            _task: None,
        }
    }

    /// Switches to another artist. The same artist again keeps what is
    /// loaded, so Back to an artist does not refetch.
    pub fn set_artist(&mut self, id: SharedString, cx: &mut Context<Self>) {
        if self.id.as_ref() == Some(&id) {
            return;
        }
        self.id = Some(id);
        // Another artist's figures under this name would mislead.
        self.history.reset();
        self.refresh(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.id.clone() else {
            return;
        };
        self.history.start();
        let client = self.client.clone();
        let tz = dates::system_timezone();
        self._task = Some(state::fetch(
            cx,
            async move { client.artist(&id, &tz).await },
            |s| &mut s.history,
        ));
    }
}
