//! The On This Day page (`/api/on-this-day`): what was played on one
//! calendar date in each earlier year, today's by default.

use chrono::{Datelike, NaiveDate};
use gpui::AppContext as _;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div, prelude::FluentBuilder,
    px,
};
use gpui_component::{
    ActiveTheme, Icon, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit_assets::IconName;

use crate::actions::OpenTrack;
use crate::api::ApiClient;
use crate::dates;
use crate::models::{OnThisDay, OnThisDayYear};
use crate::state::on_this_day::OnThisDayStore;
use crate::views::widgets;

pub struct OnThisDayView {
    store: Entity<OnThisDayStore>,
    date_input: Entity<InputState>,
    date_error: Option<SharedString>,
    focus: FocusHandle,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl OnThisDayView {
    pub fn new(client: ApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| OnThisDayStore::new(client, cx));
        let date_input = cx.new(|cx| InputState::new(window, cx).placeholder("Go to date"));
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe_in(&date_input, window, Self::on_date_event),
        ];
        Self {
            store,
            date_input,
            date_error: None,
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

    fn go_to(&mut self, date: NaiveDate, window: &mut Window, cx: &mut Context<Self>) {
        self.date_error = None;
        self.date_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.store.update(cx, |s, cx| s.set_date(date, cx));
        cx.notify();
    }

    fn step(&mut self, days: i64, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.store.read(cx).shown_date();
        if let Some(date) = shown.checked_add_signed(chrono::Duration::days(days)) {
            self.go_to(date, window, cx);
        }
    }

    fn on_date_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => {
                let text = input.read(cx).value().to_string();
                match parse_day(&text) {
                    Ok(date) => {
                        self.go_to(date, window, cx);
                        window.focus(&self.focus, cx);
                    }
                    Err(message) => self.date_error = Some(message.into()),
                }
                cx.notify();
            }
            InputEvent::Change if self.date_error.is_some() => {
                self.date_error = None;
                cx.notify();
            }
            _ => {}
        }
    }

    fn render_controls(&self, shown: NaiveDate, cx: &mut Context<Self>) -> AnyElement {
        let is_today = shown == dates::today_local();
        h_flex()
            .gap_1()
            .child(
                Button::new("on-this-day-prev")
                    .small()
                    .ghost()
                    .icon(IconName::ChevronLeft)
                    .tooltip("Previous day")
                    .on_click(cx.listener(|this, _, window, cx| this.step(-1, window, cx))),
            )
            .child(
                Button::new("on-this-day-today")
                    .small()
                    .label("Today")
                    .selected(is_today)
                    .when(!is_today, |b| b.ghost())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.go_to(dates::today_local(), window, cx)
                    })),
            )
            .child(
                Button::new("on-this-day-next")
                    .small()
                    .ghost()
                    .icon(IconName::ChevronRight)
                    .tooltip("Next day")
                    .on_click(cx.listener(|this, _, window, cx| this.step(1, window, cx))),
            )
            .child(
                div().w(px(150.)).child(
                    Input::new(&self.date_input)
                        .prefix(Icon::new(IconName::Calendar).small())
                        .small(),
                ),
            )
            .into_any_element()
    }
}

impl Focusable for OnThisDayView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for OnThisDayView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.store.read(cx).shown_date();
        let controls = self.render_controls(shown, cx);
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let store = self.store.read(cx);
        let body = widgets::page_body(
            "on-this-day-retry",
            &store.day,
            "Looking back at this day…",
            retry,
            cx,
            |data| render_day(data, cx),
        );
        widgets::page("on-this-day")
            .track_focus(&self.focus)
            .child(widgets::page_header(
                "On This Day",
                Some(subtitle(store.described_date(), dates::today_local()).into()),
                Some(controls),
                cx,
            ))
            .when_some(self.date_error.clone(), |this, message| {
                this.child(div().text_xs().text_color(cx.theme().danger).child(message))
            })
            .child(body)
    }
}

fn render_day(data: &OnThisDay, cx: &App) -> AnyElement {
    let Ok(date) = NaiveDate::parse_from_str(&data.date, "%Y-%m-%d") else {
        return widgets::page_state(None, "Unexpected date from the Worker", None, cx);
    };
    if data.years.is_empty() {
        return widgets::page_state(
            Some(IconName::CalendarDays),
            format!("Nothing played on {} in earlier years", day_month(date)),
            Some("Try another day with the arrows or the date box.".into()),
            cx,
        );
    }
    v_flex()
        .gap_5()
        .children(
            data.years
                .iter()
                .map(|year| render_year(year, date.year(), cx)),
        )
        .into_any_element()
}

fn render_year(year: &OnThisDayYear, this_year: i32, cx: &App) -> AnyElement {
    let top = year.top_tracks.first().map(|t| t.plays).unwrap_or(0).max(1);
    let detail = format!(
        "{} {} · {}",
        widgets::count(year.plays.into()),
        if year.plays == 1 { "play" } else { "plays" },
        years_ago(this_year - year.year)
    );
    v_flex()
        .gap_1()
        .child(widgets::section_heading(
            year.year.to_string(),
            Some(detail.into()),
            cx,
        ))
        .children(year.top_tracks.iter().enumerate().map(|(i, track)| {
            let row = widgets::RankedRow::new(
                SharedString::from(format!("otd-{}-{i}", year.year)),
                track.name.clone(),
            )
            .rank(i + 1)
            .art(widgets::album_art(track.image.as_deref(), px(32.), cx))
            .share(track.plays as f32 / top as f32)
            .value(widgets::count(track.plays.into()));
            let row = match &track.artist {
                Some(artist) => row.subtitle(artist.clone()),
                None => row,
            };
            match &track.id {
                Some(id) => row.on_click_action(OpenTrack::new(id.clone())),
                None => row,
            }
        }))
        .into_any_element()
}

/// `15 October`, the part of the date every earlier year shares.
fn day_month(date: NaiveDate) -> String {
    date.format("%-d %B").to_string()
}

fn subtitle(shown: NaiveDate, today: NaiveDate) -> String {
    if shown == today {
        format!("Today, {}, in earlier years", day_month(shown))
    } else {
        format!(
            "{} in the years before {}",
            day_month(shown),
            shown.format("%Y")
        )
    }
}

fn years_ago(years: i32) -> String {
    format!("{} ago", widgets::plural(years.max(0) as u64, "year"))
}

/// Any real date, past or future: a future date still looks back over the
/// years before it, so unlike History's box nothing is clamped to today.
fn parse_day(text: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|_| "Enter a date like 2024-05-31.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn date_box_takes_any_real_day() {
        assert_eq!(parse_day(" 2024-02-29 "), Ok(d(2024, 2, 29)));
        assert_eq!(parse_day("2030-12-25"), Ok(d(2030, 12, 25)));
        assert!(parse_day("2025-02-29").is_err());
        assert!(parse_day("15 Oct").is_err());
    }

    #[test]
    fn labels_read_naturally() {
        let today = d(2026, 10, 15);
        assert_eq!(
            subtitle(today, today),
            "Today, 15 October, in earlier years"
        );
        assert_eq!(
            subtitle(d(2026, 2, 3), today),
            "3 February in the years before 2026"
        );
        assert_eq!(years_ago(1), "1 year ago");
        assert_eq!(years_ago(4), "4 years ago");
    }

    #[test]
    fn fixture_years_rank_their_songs() {
        let data: OnThisDay = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/on-this-day.json"
        )))
        .unwrap();
        assert_eq!(data.years[0].year, 2025);
        assert_eq!(data.years[0].plays, 29);
        assert!(
            data.years[0]
                .top_tracks
                .windows(2)
                .all(|w| w[0].plays >= w[1].plays)
        );
    }
}
