use gpui::AppContext as _;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, Subscription, Window, div, prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit_assets::IconName;

use crate::actions::{FocusFilter, OpenSettings, Refresh, ShowHistory, ShowOverview};
use crate::api::ApiClient;
use crate::state::now_playing::NowPlayingStore;
use crate::views::history::HistoryView;
use crate::views::now_playing::NowPlayingStrip;
use crate::views::overview::OverviewView;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    History,
    Overview,
}

pub enum ShellEvent {
    OpenSettings,
}

/// The main window once configured: tabs, the now-playing strip, and the
/// window-wide actions. Both views stay alive when hidden so switching tabs
/// keeps scroll position, loaded pages and the chosen range.
pub struct Shell {
    page: Page,
    history: Entity<HistoryView>,
    overview: Entity<OverviewView>,
    now_playing_store: Entity<NowPlayingStore>,
    now_playing: Entity<NowPlayingStrip>,
    worker_host: String,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ShellEvent> for Shell {}

impl Shell {
    pub fn new(client: ApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let worker_host = client.base_url().host_str().unwrap_or_default().to_owned();
        let history = cx.new(|cx| HistoryView::new(client.clone(), window, cx));
        let overview = cx.new(|cx| OverviewView::new(client.clone(), window, cx));
        let now_playing_store = cx.new(|cx| NowPlayingStore::new(client, cx));
        let now_playing = cx.new(|cx| NowPlayingStrip::new(now_playing_store.clone(), cx));

        // Polling stops while the window is minimized, on another Space or
        // fully covered, and resumes with an immediate check when it returns.
        let subscriptions =
            vec![
                cx.observe_window_visibility(window, |this, visibility, _, cx| {
                    this.now_playing_store
                        .update(cx, |s, cx| s.set_visible(visibility.is_visible(), cx));
                }),
            ];
        let visible = window.visibility().is_visible();
        now_playing_store.update(cx, |s, cx| s.set_visible(visible, cx));

        let shell = Self {
            page: Page::History,
            history,
            overview,
            now_playing_store,
            now_playing,
            worker_host,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        shell.history.update(cx, |h, cx| h.focus_table(window, cx));
        shell
    }

    fn show(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        match page {
            Page::History => self.history.update(cx, |h, cx| h.focus_table(window, cx)),
            Page::Overview => {
                let handle = self.overview.read(cx).focus_handle(cx);
                handle.focus(window, cx);
            }
        }
        cx.notify();
    }

    fn on_show_history(&mut self, _: &ShowHistory, window: &mut Window, cx: &mut Context<Self>) {
        self.show(Page::History, window, cx);
    }

    fn on_show_overview(&mut self, _: &ShowOverview, window: &mut Window, cx: &mut Context<Self>) {
        self.show(Page::Overview, window, cx);
    }

    fn on_focus_filter(&mut self, _: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::History {
            self.show(Page::History, window, cx);
        }
        self.history.update(cx, |h, cx| h.focus_filter(window, cx));
    }

    /// Refreshes the visible page and the strip; the hidden page refreshes
    /// itself the next time it is asked, so cmd-R costs only what is on screen.
    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::History => self.history.update(cx, |h, cx| h.refresh(cx)),
            Page::Overview => self.overview.update(cx, |o, cx| o.refresh(cx)),
        }
        self.now_playing_store.update(cx, |s, cx| s.refresh(cx));
    }

    fn on_open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ShellEvent::OpenSettings);
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = match self.page {
            Page::History => 0,
            Page::Overview => 1,
        };
        v_flex()
            .key_context("Shell")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_show_history))
            .on_action(cx.listener(Self::on_show_overview))
            .on_action(cx.listener(Self::on_focus_filter))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_open_settings))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .gap_4()
                    .px_3()
                    .py_1p5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        TabBar::new("pages")
                            .segmented()
                            .small()
                            .selected_index(selected)
                            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                let page = if *ix == 0 {
                                    Page::History
                                } else {
                                    Page::Overview
                                };
                                this.show(page, window, cx);
                            }))
                            .child(Tab::new().label("History  ⌘1"))
                            .child(Tab::new().label("Overview  ⌘2")),
                    )
                    .child(div().flex_1().min_w_0().child(self.now_playing.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.worker_host.clone()),
                    )
                    .child(
                        Button::new("refresh")
                            .ghost()
                            .small()
                            .icon(IconName::RefreshCw)
                            .tooltip("Refresh (⌘R)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_refresh(&Refresh, window, cx)
                            })),
                    )
                    .child(
                        Button::new("settings")
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .tooltip("Settings (⌘,)")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ShellEvent::OpenSettings))),
                    ),
            )
            .child(div().flex_1().min_h_0().map(|this| match self.page {
                Page::History => this.child(self.history.clone()),
                Page::Overview => this.child(self.overview.clone()),
            }))
    }
}
