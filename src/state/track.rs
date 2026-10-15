//! Data for the Track page: `GET /api/track` for one song at a time.

use gpui::{Context, SharedString, Task};

use crate::api::ApiClient;
use crate::dates;
use crate::models::TrackHistory;
use crate::state::{self, Remote};

pub struct TrackStore {
    client: ApiClient,
    /// The Spotify track id shown; the Worker folds in its other editions.
    pub id: Option<SharedString>,
    pub history: Remote<TrackHistory>,
    _task: Option<Task<()>>,
}

impl TrackStore {
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            id: None,
            history: Remote::default(),
            _task: None,
        }
    }

    /// Switches to another track. The same track again keeps what is
    /// loaded, so Back to a track does not refetch.
    pub fn set_track(&mut self, id: SharedString, cx: &mut Context<Self>) {
        if self.id.as_ref() == Some(&id) {
            return;
        }
        self.id = Some(id);
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
            async move { client.track(&id, &tz).await },
            |s| &mut s.history,
        ));
    }
}
