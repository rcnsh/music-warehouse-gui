//! The Wrapped page (`/api/wrapped/ranges`, `/api/wrapped`).
//!
//! A placeholder until this page's own PR: it already loads its data so the
//! whole path (shell → view → store → Worker) is exercised. The shell calls
//! only `new`, `activate`, `refresh` and `Focusable::focus_handle`; keep those
//! signatures and change everything else freely.

use gpui::AppContext as _;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Subscription, Window, div, px,
};

use crate::api::ApiClient;
use crate::state::wrapped::WrappedStore;
use crate::views::widgets;

pub struct WrappedView {
    store: Entity<WrappedStore>,
    focus: FocusHandle,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl WrappedView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| WrappedStore::new(client, cx));
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

impl Focusable for WrappedView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for WrappedView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let retry_summary = cx.listener(|this, _, _, cx| this.refresh(cx));
        let store = self.store.read(cx);
        let ranges = widgets::remote_view(
            "wrapped-ranges-retry",
            &store.ranges,
            "Loading Wrapped ranges…",
            retry,
            cx,
            |ranges| {
                if ranges.ranges.is_empty() {
                    return widgets::coming_soon("No plays to summarise yet", None, cx)
                        .into_any_element();
                }
                let labels: Vec<_> = ranges.ranges.iter().map(|r| r.label.as_str()).collect();
                let summary = widgets::remote_view(
                    "wrapped-retry",
                    &store.wrapped,
                    "Summarising… a range not computed before can take a few seconds",
                    retry_summary,
                    cx,
                    |wrapped| {
                        widgets::coming_soon(
                            "Wrapped is on its way",
                            Some(
                                format!(
                                    "{}: {} listened over {} days. Ranges: {}",
                                    store
                                        .selected
                                        .as_ref()
                                        .map(|p| p.label.as_str())
                                        .unwrap_or_default(),
                                    widgets::minutes(wrapped.totals.minutes.into()),
                                    wrapped.totals.days_listened,
                                    labels.join(", ")
                                )
                                .into(),
                            ),
                            cx,
                        )
                        .into_any_element()
                    },
                );
                div().size_full().child(summary).into_any_element()
            },
        );
        widgets::page("wrapped")
            .track_focus(&self.focus)
            .child(widgets::page_header("Wrapped", None, None, cx))
            .child(div().h(px(320.)).child(ranges))
    }
}
