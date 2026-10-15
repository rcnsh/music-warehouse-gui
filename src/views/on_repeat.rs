//! The On Repeat page (`/api/stats/on-repeat`): songs and albums played far
//! more than usual over the last whole days, each song with its day-by-day
//! plays.

use chrono::{Days, NaiveDate};
use gpui::AppContext as _;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use gpui_kit_assets::IconName;

use crate::actions::{OpenArtist, OpenTrack};
use crate::api::ApiClient;
use crate::models::{OnRepeat, RepeatAlbum, RepeatTrack};
use crate::state::on_repeat::OnRepeatStore;
use crate::views::widgets;

pub struct OnRepeatView {
    store: Entity<OnRepeatStore>,
    focus: FocusHandle,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl OnRepeatView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| OnRepeatStore::new(client, cx));
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

impl Focusable for OnRepeatView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The window's first and last days. `as_of` is the midnight that started
/// today, so the window ends yesterday.
fn window_dates(last_midnight: NaiveDate, days: u32) -> (NaiveDate, NaiveDate) {
    let last = last_midnight - Days::new(1);
    let first = last_midnight - Days::new(days.max(1).into());
    (first, last)
}

fn window_subtitle(data: &OnRepeat) -> SharedString {
    match widgets::local_date(&data.as_of) {
        Some(midnight) => {
            let (first, last) = window_dates(midnight, data.window_days);
            format!(
                "Played far more than usual over the last {} days, {} – {}",
                data.window_days,
                first.format("%-d %b"),
                widgets::day(last)
            )
        }
        None => format!(
            "Played far more than usual over the last {} days",
            data.window_days
        ),
    }
    .into()
}

/// `8 of 14 days`.
fn days_label(track: &RepeatTrack, window_days: u32) -> String {
    format!("{} of {} days", track.days, window_days)
}

fn track_row(ix: usize, track: &RepeatTrack, window_days: u32, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let hover = cx.theme().muted.opacity(0.5);
    let subtitle = match &track.album {
        Some(album) => format!("{} · {album}", track.artists),
        None => track.artists.clone(),
    };
    let daily: Vec<f64> = track.daily.iter().map(|&n| n.into()).collect();
    let action = OpenTrack::new(track.track_id.clone());
    h_flex()
        .id(("repeat-track", ix))
        .gap_2()
        .px_1()
        .py_0p5()
        .rounded_md()
        .items_center()
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .on_click(move |_, window, cx| window.dispatch_action(Box::new(action.clone()), cx))
        .child(
            div()
                .w(px(24.))
                .flex_shrink_0()
                .text_right()
                .text_color(muted)
                .child((ix + 1).to_string()),
        )
        .child(widgets::album_art(track.image.as_deref(), px(32.), cx))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().truncate().child(track.name.clone()))
                .child(div().truncate().text_xs().text_color(muted).child(subtitle)),
        )
        .child(
            div()
                .flex_shrink_0()
                .child(widgets::sparkline(&daily, px(24.), cx)),
        )
        .child(
            v_flex()
                .min_w(px(84.))
                .flex_shrink_0()
                .items_end()
                .child(format!("{} plays", widgets::count(track.plays.into())))
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(days_label(track, window_days)),
                ),
        )
        .into_any_element()
}

fn album_rows(albums: &[RepeatAlbum], cx: &App) -> AnyElement {
    let top = widgets::top_scale(albums.iter().map(|a| a.plays)) as f32;
    v_flex()
        .gap_0p5()
        .children(albums.iter().enumerate().map(|(i, album)| {
            let songs = match album.tracks_on_repeat {
                1 => "1 song on repeat".to_owned(),
                n => format!("{n} songs on repeat"),
            };
            widgets::RankedRow::new(("repeat-album", i), album.name.clone())
                .rank(i + 1)
                .art(widgets::album_art(album.image.as_deref(), px(32.), cx))
                .subtitle(format!("{} · {songs}", album.artist))
                .value(format!("{} plays", widgets::count(album.plays.into())))
                .share(album.plays as f32 / top)
                .on_click_action(OpenArtist::by_name(&album.artist))
        }))
        .into_any_element()
}

fn content(data: &OnRepeat, cx: &App) -> AnyElement {
    if data.tracks.is_empty() && data.albums.is_empty() {
        return widgets::page_state(
            Some(IconName::Repeat),
            "Nothing on repeat",
            Some(
                format!(
                    "No song stood out against your usual listening over the last {} days.",
                    data.window_days
                )
                .into(),
            ),
            cx,
        );
    }
    v_flex()
        .gap_4()
        .when(!data.tracks.is_empty(), |this| {
            this.child(
                v_flex()
                    .gap_2()
                    .child(widgets::section_heading(
                        "Songs",
                        Some("Bars show plays per day, oldest first".into()),
                        cx,
                    ))
                    .child(
                        v_flex().gap_0p5().children(
                            data.tracks
                                .iter()
                                .enumerate()
                                .map(|(i, track)| track_row(i, track, data.window_days, cx)),
                        ),
                    ),
            )
        })
        .when(!data.albums.is_empty(), |this| {
            this.child(
                v_flex()
                    .gap_2()
                    .child(widgets::section_heading("Albums", None, cx))
                    .child(album_rows(&data.albums, cx)),
            )
        })
        .into_any_element()
}

impl Render for OnRepeatView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let store = self.store.read(cx);
        let data = store.on_repeat.data.as_ref();
        let subtitle = data.map(window_subtitle);
        let body = widgets::page_body(
            "on-repeat-retry",
            &store.on_repeat,
            "Loading what's on repeat…",
            retry,
            cx,
            |data| content(data, cx),
        );
        widgets::page("on-repeat")
            .track_focus(&self.focus)
            .child(widgets::page_header("On Repeat", subtitle, None, cx))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> OnRepeat {
        serde_json::from_str(include_str!("../../tests/fixtures/stats-on-repeat.json")).unwrap()
    }

    #[test]
    fn fixture_carries_a_day_per_window_day() {
        let data = fixture();
        assert_eq!(data.window_days, 14);
        assert!(
            data.tracks
                .iter()
                .all(|t| t.daily.len() == data.window_days as usize)
        );
        assert_eq!(
            days_label(&data.tracks[0], data.window_days),
            "8 of 14 days"
        );
    }

    #[test]
    fn the_window_ends_the_day_before_as_of() {
        let midnight = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let (first, last) = window_dates(midnight, 14);
        assert_eq!(first, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
        assert_eq!(last, NaiveDate::from_ymd_opt(2026, 10, 14).unwrap());
    }
}
