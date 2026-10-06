use gpui::{Context, Task};

use crate::api::{ApiClient, ApiError, TopRange};
use crate::dates::{self, DateRange, RangePreset};
use crate::models::{DailyResponse, TopArtistsResponse, TopResponse};
use crate::runtime;
use crate::state::Remote;
use crate::ui_state;

pub const TOP_ARTISTS_LIMIT: u32 = 50;
pub const SPOTIFY_TOP_LIMIT: u32 = 10;

/// Data for the Overview: the daily chart and artist counts share one range,
/// so they are always fetched together. Spotify's own top lists use its fixed
/// windows instead and are fetched independently, because they are live and
/// can fail while the stored-row routes keep working.
pub struct OverviewStore {
    client: ApiClient,
    pub tz: String,
    pub preset: RangePreset,
    pub range: DateRange,
    pub daily: Remote<DailyResponse>,
    pub artists: Remote<TopArtistsResponse>,
    pub spotify_range: TopRange,
    pub spotify_top: Remote<TopResponse>,
    _range_tasks: Vec<Task<()>>,
    _spotify_task: Option<Task<()>>,
}

impl OverviewStore {
    pub fn new(client: ApiClient, cx: &mut Context<Self>) -> Self {
        let saved = ui_state::get(cx);
        let (preset, range) = saved
            .range
            .as_ref()
            .and_then(|r| r.restore(dates::today_local()))
            .unwrap_or_else(|| {
                let preset = RangePreset::Last30Days;
                let range = preset
                    .resolve(dates::today_local())
                    .expect("30-day preset always resolves");
                (preset, range)
            });
        let mut store = Self {
            client,
            tz: dates::system_timezone(),
            preset,
            range,
            daily: Remote::default(),
            artists: Remote::default(),
            spotify_range: ui_state::spotify_range(&saved).unwrap_or(TopRange::Short),
            spotify_top: Remote::default(),
            _range_tasks: Vec::new(),
            _spotify_task: None,
        };
        store.fetch_range(cx);
        store.fetch_spotify_top(cx);
        store
    }

    /// Switches to a preset, re-resolving against today so a window left open
    /// overnight still ends on the current day.
    pub fn select_preset(&mut self, preset: RangePreset, cx: &mut Context<Self>) {
        self.preset = preset;
        if let Some(range) = preset.resolve(dates::today_local()) {
            self.set_range(range, cx);
        } else {
            cx.notify();
        }
    }

    pub fn set_custom_range(&mut self, range: DateRange, cx: &mut Context<Self>) {
        self.preset = RangePreset::Custom;
        self.set_range(range, cx);
    }

    fn set_range(&mut self, range: DateRange, cx: &mut Context<Self>) {
        if range != self.range {
            // Keeping the old range's bars under a new title would mislead.
            self.daily.reset();
            self.artists.reset();
        }
        self.range = range;
        self.fetch_range(cx);
    }

    pub fn select_spotify_range(&mut self, range: TopRange, cx: &mut Context<Self>) {
        if range != self.spotify_range {
            self.spotify_range = range;
            self.spotify_top.reset();
        }
        self.fetch_spotify_top(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        // The system zone can change while the app runs (travel).
        self.tz = dates::system_timezone();
        if self.preset == RangePreset::Custom {
            self.fetch_range(cx);
        } else {
            self.select_preset(self.preset, cx);
        }
        self.fetch_spotify_top(cx);
    }

    fn fetch_range(&mut self, cx: &mut Context<Self>) {
        self.daily.start();
        self.artists.start();
        cx.notify();
        let range = self.range;
        let tz = self.tz.clone();

        let client = self.client.clone();
        let tz_daily = tz.clone();
        let daily = cx.spawn(async move |this, cx| {
            let result = runtime::run(async move { client.daily(&range, &tz_daily).await }).await;
            this.update(cx, |this, cx| {
                this.daily.finish(result);
                cx.notify();
            })
            .ok();
        });

        let client = self.client.clone();
        let artists = cx.spawn(async move |this, cx| {
            let result =
                runtime::run(
                    async move { client.top_artists(&range, &tz, TOP_ARTISTS_LIMIT).await },
                )
                .await;
            this.update(cx, |this, cx| {
                this.artists.finish(result);
                cx.notify();
            })
            .ok();
        });
        // Replacing the tasks cancels requests for a range no longer shown.
        self._range_tasks = vec![daily, artists];
    }

    fn fetch_spotify_top(&mut self, cx: &mut Context<Self>) {
        self.spotify_top.start();
        cx.notify();
        let client = self.client.clone();
        let range = self.spotify_range;
        self._spotify_task = Some(cx.spawn(async move |this, cx| {
            let result: Result<TopResponse, ApiError> =
                runtime::run(async move { client.top(range, SPOTIFY_TOP_LIMIT).await }).await;
            this.update(cx, |this, cx| {
                this.spotify_top.finish(result);
                cx.notify();
            })
            .ok();
        }));
    }
}
