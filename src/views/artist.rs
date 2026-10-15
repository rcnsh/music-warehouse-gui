//! The Artist page (`/api/artist`): one artist's all-time figures, plays per
//! month and most-played songs, exactly as the Worker returns them.
//!
//! The shell calls only `new`, `activate`, `refresh`, `set_artist` and
//! `Focusable::focus_handle`.

use std::rc::Rc;

use gpui::AppContext as _;
use gpui::{
    Action, AnyElement, App, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit_assets::IconName;

use crate::actions::{OpenTrack, ShowPlaysOfArtist};
use crate::api::ApiClient;
use crate::models::{ARTIST_NAME_PREFIX, ArtistHistory, MonthCount};
use crate::state::artist::ArtistStore;
use crate::views::{recent_days, widgets};

pub struct ArtistView {
    store: Entity<ArtistStore>,
    focus: FocusHandle,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl ArtistView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| ArtistStore::new(client, cx));
        let subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            focus: cx.focus_handle(),
            activated: false,
            _subscriptions: subscriptions,
        }
    }

    /// Called every time the page is shown, after the shell focuses it.
    /// `set_artist` has already loaded the artist, so this only matters
    /// for a page shown without one.
    pub fn activate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.activated {
            self.activated = true;
            self.refresh(cx);
        }
    }

    /// ⌘R while the page is showing.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| s.refresh(cx));
    }

    /// Shows another artist: a Spotify artist id or a `name:` placeholder, as carried by
    /// `OpenArtist`. The shell calls this before `activate` each time
    /// the page opens, Back included; the same id again keeps what is loaded.
    pub fn set_artist(&mut self, id: SharedString, cx: &mut Context<Self>) {
        self.activated = true;
        self.store.update(cx, |s, cx| s.set_artist(id, cx));
    }
}

impl Focusable for ArtistView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ArtistView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let daily_retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let store_handle = self.store.downgrade();
        let on_pick: recent_days::PickRange = Rc::new(move |preset, _, cx| {
            store_handle
                .update(cx, |s, cx| s.set_daily_preset(preset, cx))
                .ok();
        });
        let store = self.store.read(cx);
        let page = widgets::page("artist").track_focus(&self.focus);
        let Some(id) = store.id.clone() else {
            return page.child(widgets::page_state(
                Some(IconName::MicVocal),
                "No artist selected",
                Some("Open an artist from Overview, a ranking or a song's page.".into()),
                cx,
            ));
        };

        let data = store.history.data.as_ref();
        let title = artist_title(data.map(|d| d.artist.name.as_str()), &id);
        // The Worker resolves a `name:` placeholder to a real id when it can.
        let spotify = data
            .and_then(|d| d.artist.id.as_deref())
            .and_then(artist_url)
            .or_else(|| artist_url(&id));
        let subtitle: SharedString = if store.history.loading && data.is_some() {
            "Artist · all time · refreshing…".into()
        } else {
            "Artist · all time".into()
        };
        let every_play = ShowPlaysOfArtist::new(id.clone(), title.clone());
        let controls = header_controls(every_play, spotify, "artist-every-play");
        let header = widgets::page_header(title, Some(subtitle), Some(controls), cx);

        let daily = recent_days::card("artist-daily", &store.daily, on_pick, daily_retry, cx);

        page.child(header).child(widgets::page_body(
            "artist-retry",
            &store.history,
            "Loading artist…",
            retry,
            cx,
            |data| render_history(data, daily, cx),
        ))
    }
}

/// "Show every play" (History narrowed to this song or artist), then Open
/// in Spotify when there is a Spotify page.
pub(crate) fn header_controls(
    every_play: impl Action,
    spotify: Option<String>,
    id: &'static str,
) -> AnyElement {
    let every_play: Box<dyn Action> = Box::new(every_play);
    h_flex()
        .gap_2()
        .child(
            Button::new(id)
                .small()
                .icon(IconName::ListMusic)
                .label("Show every play")
                .tooltip("Every stored play, newest first, in History")
                .on_click(move |_, window, cx| {
                    window.dispatch_action(every_play.boxed_clone(), cx)
                }),
        )
        .when_some(spotify, |this, url| this.child(open_in_spotify(url)))
        .into_any_element()
}

fn render_history(data: &ArtistHistory, daily: AnyElement, cx: &App) -> AnyElement {
    if data.plays == 0 {
        return widgets::page_state(
            Some(IconName::Inbox),
            "No plays for this artist",
            Some("The warehouse has no plays where this artist is the primary artist.".into()),
            cx,
        );
    }
    let top = widgets::top_scale(data.top_tracks.iter().map(|t| t.plays)) as f32;
    let songs = data.top_tracks.iter().enumerate().map(|(ix, song)| {
        let row = widgets::RankedRow::new(("artist-song", ix), song.name.clone())
            .rank(ix + 1)
            .art(widgets::album_art(song.image.as_deref(), px(32.), cx))
            .value(widgets::count(song.plays.into()))
            .share(song.plays as f32 / top);
        match &song.id {
            Some(id) => row.on_click_action(OpenTrack::new(id.clone())),
            None => row,
        }
    });
    v_flex()
        .gap_4()
        .child(stat_tiles(
            data.plays,
            data.minutes,
            data.first_played.as_deref(),
            data.last_played.as_deref(),
            cx,
        ))
        .child(daily)
        .child(months_card("artist-months", &data.months, cx))
        .when(!data.top_tracks.is_empty(), |this| {
            this.child(
                widgets::card("Most-played songs · click a song for its page", cx)
                    .child(v_flex().gap_1().children(songs)),
            )
        })
        .into_any_element()
}

/// Plays, minutes, first and last play: the row both the Artist and Track
/// pages open with.
pub(crate) fn stat_tiles(
    plays: u32,
    minutes: u32,
    first: Option<&str>,
    last: Option<&str>,
    cx: &App,
) -> impl IntoElement {
    let when = |iso: Option<&str>| match iso.and_then(widgets::local_day_time) {
        Some((day, time)) => (SharedString::from(day), Some(SharedString::from(time))),
        None => ("—".into(), None),
    };
    let (first_day, first_time) = when(first);
    let (last_day, last_time) = when(last);
    // Tiles with a detail line are taller; keep the row even.
    h_flex()
        .gap_3()
        .items_stretch()
        .child(widgets::stat_tile(
            "Plays",
            widgets::count(plays.into()),
            None,
            cx,
        ))
        .child(widgets::stat_tile(
            "Listening time",
            widgets::minutes(minutes.into()),
            Some("Estimated; skips count for nothing".into()),
            cx,
        ))
        .child(widgets::stat_tile("First play", first_day, first_time, cx))
        .child(widgets::stat_tile("Last play", last_day, last_time, cx))
}

/// The Worker's plays per month, quiet months included, as a bar chart.
pub(crate) fn months_card(id: &'static str, months: &[MonthCount], cx: &App) -> AnyElement {
    let card = widgets::card("Plays per month", cx);
    if months.is_empty() {
        return card.into_any_element();
    }
    card.h(px(280.))
        // The last band label is centred on the final bar and would
        // otherwise be clipped by the card edge, as on Overview.
        .child(div().flex_1().min_h_0().pr_6().child(widgets::bar_chart(
            id,
            month_points(months),
            "Plays",
            cx,
        )))
        .into_any_element()
}

pub(crate) fn open_in_spotify(url: String) -> AnyElement {
    Button::new("open-in-spotify")
        .small()
        .primary()
        .icon(IconName::ExternalLink)
        .label("Open in Spotify")
        .on_click(move |_, _, cx| cx.open_url(&url))
        .into_any_element()
}

fn month_points(months: &[MonthCount]) -> Vec<(SharedString, f64)> {
    months
        .iter()
        .map(|m| (widgets::month_label(&m.month).into(), m.plays.into()))
        .collect()
}

/// A Spotify web link for a real artist id; `name:` placeholders have no page.
/// Web rather than `spotify:` so it works with or without the desktop app.
fn artist_url(id: &str) -> Option<String> {
    is_spotify_id(id).then(|| format!("https://open.spotify.com/artist/{id}"))
}

/// Spotify ids are base62, which also rules out `name:` placeholders.
pub(crate) fn is_spotify_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The Worker's name for the artist, else the name inside a `name:`
/// placeholder (lower-cased by the Worker, but better than nothing).
fn artist_title(name: Option<&str>, id: &str) -> SharedString {
    match name.filter(|n| !n.is_empty()) {
        Some(name) => name.to_owned().into(),
        None => match id.strip_prefix(ARTIST_NAME_PREFIX) {
            Some(name) if !name.is_empty() => name.to_owned().into(),
            _ => "Artist".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_ids_link_to_spotify() {
        assert_eq!(
            artist_url("2YZyLoL8N0Wb9xBt1NhZWg").as_deref(),
            Some("https://open.spotify.com/artist/2YZyLoL8N0Wb9xBt1NhZWg")
        );
        assert_eq!(artist_url("name:kendrick lamar"), None);
        assert_eq!(artist_url(""), None);
    }

    #[test]
    fn title_falls_back_to_the_placeholder_name() {
        assert_eq!(artist_title(Some("SZA"), "abc").as_ref(), "SZA");
        assert_eq!(
            artist_title(Some(""), "name:kendrick lamar").as_ref(),
            "kendrick lamar"
        );
        assert_eq!(
            artist_title(None, "2YZyLoL8N0Wb9xBt1NhZWg").as_ref(),
            "Artist"
        );
    }

    #[test]
    fn months_become_distinct_chart_labels() {
        let months = [
            MonthCount {
                month: "2025-12".into(),
                plays: 0,
            },
            MonthCount {
                month: "2026-01".into(),
                plays: 7,
            },
        ];
        let points = month_points(&months);
        assert_eq!(points[0], ("Dec 2025".into(), 0.));
        assert_eq!(points[1], ("Jan 2026".into(), 7.));
    }
}
