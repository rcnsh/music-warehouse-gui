//! The On This Day page (`/api/on-this-day`).
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
use crate::state::on_this_day::OnThisDayStore;
use crate::views::widgets;

pub struct OnThisDayView {
    store: Entity<OnThisDayStore>,
    focus: FocusHandle,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl OnThisDayView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| OnThisDayStore::new(client, cx));
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

impl Focusable for OnThisDayView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for OnThisDayView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let store = self.store.read(cx);
        let body = widgets::remote_view(
            "on-this-day-retry",
            &store.day,
            "Loading this day in past years…",
            retry,
            cx,
            |data| {
                let summary = format!(
                    "{} earlier years with plays on {}",
                    data.years.len(),
                    data.date
                );
                widgets::coming_soon("On This Day is on its way", Some(summary.into()), cx)
                    .into_any_element()
            },
        );
        widgets::page("on-this-day")
            .track_focus(&self.focus)
            .child(widgets::page_header("On This Day", None, None, cx))
            .child(div().h(px(320.)).child(body))
    }
}
