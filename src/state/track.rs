//! Data for the Track page: `GET /api/track` for one song at a time, and its
//! plays per day over a recent range from `GET /api/daily`.

use gpui::{Context, SharedString, Task};

use crate::api::{ApiClient, PlayFilter};
use crate::dates::{self, RangePreset};
use crate::models::TrackHistory;
use crate::state::{self, RecentDays, Remote};

pub struct TrackStore {
    client: ApiClient,
    /// The Spotify track id shown; the Worker folds in its other editions.
    pub id: Option<SharedString>,
    pub history: Remote<TrackHistory>,
    pub daily: RecentDays,
    _task: Option<Task<()>>,
    _daily_task: Option<Task<()>>,
}

impl TrackStore {
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

    /// Switches to another track. The same track again keeps what is
    /// loaded, so Back to a track does not refetch.
    pub fn set_track(&mut self, id: SharedString, cx: &mut Context<Self>) {
        if self.id.as_ref() == Some(&id) {
            return;
        }
        self.id = Some(id);
        self.history.reset();
        self.daily.days.reset();
        self.refresh(cx);
    }

    /// Charts another recent range, keeping the song.
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
        let track = id.clone();
        self._task = Some(state::fetch(
            cx,
            move || {
                let (client, track, tz) = (client.clone(), track.clone(), tz.clone());
                async move { client.track(&track, &tz).await }
            },
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
        let filter = PlayFilter::song(id.to_string());
        self._daily_task = Some(state::fetch(
            cx,
            move || {
                let (client, tz, filter) = (client.clone(), tz.clone(), filter.clone());
                async move { client.daily(&range, &tz, &filter).await }
            },
            |s| &mut s.daily.days,
        ));
    }
}
