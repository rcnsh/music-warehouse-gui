//! Data for the Artist page: `GET /api/artist` for one artist at a time, and
//! their plays per day over a recent range from `GET /api/daily`.

use gpui::{Context, SharedString, Task};

use crate::api::{ApiClient, PlayFilter};
use crate::dates::{self, RangePreset};
use crate::models::ArtistHistory;
use crate::state::{self, RecentDays, Remote};

pub struct ArtistStore {
    client: ApiClient,
    /// The artist shown: a Spotify id or a `name:` placeholder.
    pub id: Option<SharedString>,
    pub history: Remote<ArtistHistory>,
    pub daily: RecentDays,
    _task: Option<Task<()>>,
    _daily_task: Option<Task<()>>,
}

impl ArtistStore {
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            id: None,
            history: Remote::default(),
            daily: RecentDays::default(),
            _task: None,
            _daily_task: None,
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
        self.daily.days.reset();
        self.refresh(cx);
    }

    /// Charts another recent range, keeping the artist.
    pub fn set_daily_preset(&mut self, preset: RangePreset, cx: &mut Context<Self>) {
        if self.daily.preset == preset {
            return;
        }
        self.daily.preset = preset;
        self.daily.days.reset();
        self.fetch_daily(cx);
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
        self.fetch_daily(cx);
    }

    fn fetch_daily(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.id.clone() else {
            return;
        };
        let Some(range) = self.daily.start() else {
            return;
        };
        let client = self.client.clone();
        let tz = dates::system_timezone();
        let filter = PlayFilter::artist(&id);
        self._daily_task = Some(state::fetch(
            cx,
            async move { client.daily(&range, &tz, &filter).await },
            |s| &mut s.daily.days,
        ));
    }
}
