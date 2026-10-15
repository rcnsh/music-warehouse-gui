//! The Rhythm page (`/api/stats/rhythm`).
//!
//! A placeholder until this page's own PR: it already loads its data so the
//! whole path (shell → view → store → Worker) is exercised. The shell calls
//! only `new`, `activate`, `refresh` and `Focusable::focus_handle`;
//! keep those signatures and change everything else freely.

use gpui::AppContext as _;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Subscription, Window, div, px,
};

use crate::api::ApiClient;
use crate::state::rhythm::RhythmStore;
use crate::views::widgets;

pub struct RhythmView {
    store: Entity<RhythmStore>,
    focus: FocusHandle,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl RhythmView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| RhythmStore::new(client, cx));
        let subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            focus: cx.focus_handle(),
            activated: false,
            _subscriptions: subscriptions,
        }
    }

    /// Called every time the page is shown, after the shell focuses it.
    /// Loads on the first call only, so a page never visited costs no request.
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
}

impl Focusable for RhythmView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RhythmView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let store = self.store.read(cx);
        let body = widgets::remote_view(
            "rhythm-retry",
            &store.rhythm,
            "Loading listening rhythm…",
            retry,
            cx,
            |data| {
                let summary = format!(
                    "{} streak thresholds, {} eras and {} years, {} to {}",
                    data.streaks.len(),
                    data.eras.len(),
                    data.years.len(),
                    data.range.from,
                    data.range.to
                );
                widgets::coming_soon("Rhythm is on its way", Some(summary.into()), cx)
                    .into_any_element()
            },
        );
        widgets::page("rhythm")
            .track_focus(&self.focus)
            .child(widgets::page_header("Rhythm", None, None, cx))
            .child(div().h(px(320.)).child(body))
    }
}
