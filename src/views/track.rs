//! The Track page (`/api/track`): one song's all-time figures and plays per
//! month, every edition already folded together by the Worker.
//!
//! The shell calls only `new`, `activate`, `refresh`, `set_track` and
//! `Focusable::focus_handle`.

use gpui::AppContext as _;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div, prelude::FluentBuilder,
    px,
};
use gpui_component::{ActiveTheme, StyledExt, h_flex, v_flex};
use gpui_kit_assets::IconName;

use crate::actions::OpenArtist;
use crate::api::ApiClient;
use crate::models::{Rendition, TrackHistory, album_rendition};
use crate::state::track::TrackStore;
use crate::views::artist::{is_spotify_id, months_card, open_in_spotify, stat_tiles};
use crate::views::{play_detail, widgets};

pub struct TrackView {
    store: Entity<TrackStore>,
    focus: FocusHandle,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl TrackView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| TrackStore::new(client, cx));
        let subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            focus: cx.focus_handle(),
            activated: false,
            _subscriptions: subscriptions,
        }
    }

    /// Called every time the page is shown, after the shell focuses it.
    /// `set_track` has already loaded the track, so this only matters
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

    /// Shows another track: a Spotify track id, as carried by
    /// `OpenTrack`. The shell calls this before `activate` each time
    /// the page opens, Back included; the same id again keeps what is loaded.
    pub fn set_track(&mut self, id: SharedString, cx: &mut Context<Self>) {
        self.activated = true;
        self.store.update(cx, |s, cx| s.set_track(id, cx));
    }
}

impl Focusable for TrackView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TrackView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let store = self.store.read(cx);
        let page = widgets::page("track").track_focus(&self.focus);
        let Some(id) = store.id.clone() else {
            return page.child(widgets::page_state(
                Some(IconName::Music),
                "No song selected",
                Some("Open a song from History, a ranking or an artist's page.".into()),
                cx,
            ));
        };
        let refreshing = store.history.loading && store.history.data.is_some();
        page.child(header(store.history.data.as_ref(), &id, refreshing, cx))
            .child(widgets::page_body(
                "track-retry",
                &store.history,
                "Loading song…",
                retry,
                cx,
                |data| render_history(data, cx),
            ))
    }
}

/// Album art, the song over its artist (a link to the Artist page), and
/// Open in Spotify. Before the first answer only the button shows, since the
/// id alone is enough for it.
fn header(data: Option<&TrackHistory>, id: &str, refreshing: bool, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let title: SharedString = match data {
        Some(d) => d
            .track
            .name
            .clone()
            .unwrap_or_else(|| "Unknown song".into())
            .into(),
        None => "Song".into(),
    };
    let image = data
        .and_then(|d| d.track.image.as_deref())
        .map(|url| SharedString::from(album_rendition(url, Rendition::Px300)));
    let artist = data.and_then(|d| d.track.artist.clone());
    let detail = if refreshing {
        "Song · every edition · refreshing…"
    } else {
        "Song · every edition"
    };
    h_flex()
        .gap_4()
        .items_end()
        .child(widgets::artwork(image, px(96.), false, cx))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_xl().font_semibold().truncate().child(title))
                .when_some(artist, |this, artist| {
                    let open = OpenArtist::by_name(&artist);
                    this.child(div().text_base().child(widgets::link(
                        "track-artist",
                        artist,
                        open,
                        cx,
                    )))
                })
                .child(div().text_sm().text_color(muted).child(detail)),
        )
        .when(is_spotify_id(id), |this| {
            this.child(open_in_spotify(play_detail::track_url(id)))
        })
        .into_any_element()
}

fn render_history(data: &TrackHistory, cx: &App) -> AnyElement {
    if data.plays == 0 {
        let detail = if data.track.name.is_none() {
            "This song is not in the warehouse."
        } else {
            "The warehouse has no plays of this song."
        };
        return widgets::page_state(
            Some(IconName::Inbox),
            "No plays for this song",
            Some(detail.into()),
            cx,
        );
    }
    v_flex()
        .gap_4()
        .child(stat_tiles(
            data.plays,
            data.minutes,
            data.first_played.as_deref(),
            data.last_played.as_deref(),
            cx,
        ))
        .child(months_card("track-months", &data.months, cx))
        .into_any_element()
}
