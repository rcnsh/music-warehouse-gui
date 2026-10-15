//! The Track page (`/api/track`).
//!
//! A placeholder until this page's own PR: it already loads its data so the
//! whole path (shell → view → store → Worker) is exercised. The shell calls
//! only `new`, `activate`, `refresh`, `set_track` and `Focusable::focus_handle`;
//! keep those signatures and change everything else freely.

use gpui::AppContext as _;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Styled, Subscription, Window, div, px,
};

use crate::api::ApiClient;
use crate::state::track::TrackStore;
use crate::views::widgets;

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
        let Some(id) = store.id.clone() else {
            return widgets::page("track")
                .track_focus(&self.focus)
                .child(widgets::coming_soon("No track selected", None, cx));
        };
        let title: SharedString = store
            .history
            .data
            .as_ref()
            .map(|data| SharedString::from(data.track.name.clone().unwrap_or_default()))
            .unwrap_or_else(|| "Track".into());
        let body = widgets::remote_view(
            "track-retry",
            &store.history,
            "Loading track…",
            retry,
            cx,
            |data| {
                let summary = format!(
                    "{} plays, {} listened, over {} months",
                    widgets::count(data.plays.into()),
                    widgets::minutes(data.minutes.into()),
                    data.months.len()
                );
                widgets::coming_soon("The track page is on its way", Some(summary.into()), cx)
                    .into_any_element()
            },
        );
        widgets::page("track")
            .track_focus(&self.focus)
            .child(widgets::page_header(title, Some(id), None, cx))
            .child(div().h(px(320.)).child(body))
    }
}
