//! The Rhythm page (`/api/stats/rhythm`): listening streaks, and when in the
//! day and week the listening happens, in each era's local time. All time or
//! one calendar year, from the one response.

use gpui::AppContext as _;
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

use crate::api::ApiClient;
use crate::models::{EraRhythm, Rhythm, Streak, StreakSet};
use crate::state::Remote;
use crate::state::rhythm::RhythmStore;
use crate::views::widgets;

pub struct RhythmView {
    store: Entity<RhythmStore>,
    /// The calendar year shown, or all time.
    year: Option<String>,
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
            year: None,
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

    fn year_picker(&self, data: &Rhythm, cx: &Context<Self>) -> AnyElement {
        let choices = std::iter::once(None).chain(data.years.iter().rev().map(Some));
        h_flex()
            .gap_1()
            .flex_wrap()
            .justify_end()
            .children(choices.map(|year| {
                let key = year.map(|y| y.year.clone());
                let selected = self.year == key;
                let label: SharedString = match year {
                    None => "All time".into(),
                    Some(y) if y.partial => format!("{} so far", y.year).into(),
                    Some(y) => y.year.clone().into(),
                };
                Button::new(SharedString::from(format!(
                    "rhythm-year-{}",
                    key.as_deref().unwrap_or("all")
                )))
                .small()
                .label(label)
                .selected(selected)
                .when(!selected, |b| b.ghost())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.year = key.clone();
                    cx.notify();
                }))
            }))
            .into_any_element()
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
        let data = store.rhythm.data.as_ref();
        let subtitle = data.map(|d| {
            SharedString::from(format!(
                "{}, in each era's local time. Updated weekly.",
                span_label(&d.range.from, Some(&d.range.to))
            ))
        });
        let controls = data
            .filter(|d| !d.years.is_empty())
            .map(|d| self.year_picker(d, cx));
        let body = page_body(
            "rhythm-retry",
            &store.rhythm,
            "Loading listening rhythm…",
            retry,
            cx,
            |data| {
                let (streaks, eras) = scope(data, self.year.as_deref());
                v_flex()
                    .gap_6()
                    .child(streaks_section(streaks, cx))
                    .children(eras.iter().map(|era| era_section(era, eras.len(), cx)))
                    .when(eras.is_empty(), |this| {
                        this.child(div().h(px(200.)).child(widgets::state_panel(
                            None,
                            "No listening in this period",
                            None,
                            None,
                            cx,
                        )))
                    })
                    .into_any_element()
            },
        );
        widgets::page("rhythm")
            .track_focus(&self.focus)
            .child(widgets::page_header("Rhythm", subtitle, controls, cx))
            .child(body)
    }
}

/// The streaks and eras for a year, or for all time when `year` is `None`
/// or names a year the response no longer has.
fn scope<'a>(data: &'a Rhythm, year: Option<&str>) -> (&'a [StreakSet], &'a [EraRhythm]) {
    year.and_then(|year| data.years.iter().find(|y| y.year == year))
        .map(|y| (y.streaks.as_slice(), y.eras.as_slice()))
        .unwrap_or((data.streaks.as_slice(), data.eras.as_slice()))
}

fn streaks_section(sets: &[StreakSet], cx: &App) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(widgets::section_heading(
            "Streaks",
            Some("Runs of consecutive days at each level of listening".into()),
            cx,
        ))
        .child(
            h_flex()
                .gap_3()
                .items_start()
                .children(sets.iter().map(|set| streak_card(set, cx))),
        )
}

fn streak_card(set: &StreakSet, cx: &App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let longest = set.longest.as_ref();
    let max_days = set.top.iter().map(approx_days).max().unwrap_or(0).max(1);
    let mut card = widgets::card(threshold_label(set.threshold), cx)
        .flex_1()
        .min_w_0()
        .child(
            v_flex()
                .child(
                    div()
                        .text_2xl()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(longest.map(streak_length).unwrap_or_else(|| "None".into())),
                )
                .child(div().text_xs().text_color(muted).child(match longest {
                    Some(s) => format!("Longest, {}", streak_span(s)),
                    None => "No day reached this level".into(),
                })),
        )
        .child(div().text_xs().text_color(muted).child(activity_line(set)));
    if set.top.len() > 1 {
        card = card.child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Top runs"),
                )
                .children(set.top.iter().enumerate().map(|(i, streak)| {
                    widgets::RankedRow::new(
                        ElementId::Name(format!("streak-{}-{i}", set.threshold).into()),
                        streak_length(streak),
                    )
                    .subtitle(streak_span(streak))
                    .share(approx_days(streak) as f32 / max_days as f32)
                })),
        );
    }
    card
}

fn threshold_label(threshold: u32) -> String {
    match threshold {
        1 => "Any listening".into(),
        n => format!("{n}+ plays a day"),
    }
}

fn activity_line(set: &StreakSet) -> String {
    let active = format!("{} of days reach it", widgets::percent(set.active_share));
    match set.current_weeks {
        0 => active,
        1 => format!("{active}. Current run: 1 week"),
        n => format!("{active}. Current run: {n} weeks"),
    }
}

/// A finished run's exact length; a running one only in whole weeks, which
/// is all the Worker publishes for it.
fn streak_length(streak: &Streak) -> String {
    match (streak.days, streak.weeks) {
        (Some(1), _) => "1 day".into(),
        (Some(days), _) => format!("{days} days"),
        (None, 0) => "Under a week so far".into(),
        (None, 1) => "1 week so far".into(),
        (None, weeks) => format!("{weeks} weeks so far"),
    }
}

fn streak_span(streak: &Streak) -> String {
    span_label(
        &streak.from,
        (!streak.running).then_some(streak.to.as_str()),
    )
}

/// Comparable length for the bars: running runs only come in weeks.
fn approx_days(streak: &Streak) -> u32 {
    streak.days.unwrap_or(streak.weeks * 7)
}

/// `Feb 2024 – Jun 2024`, `Feb 2024` for one month, `Sep 2026 – now` while
/// still running. Months are `YYYY-MM`.
pub(crate) fn span_label(from: &str, to: Option<&str>) -> String {
    match to {
        None => format!("{} – now", widgets::month_label(from)),
        Some(to) if to == from => widgets::month_label(from),
        Some(to) => format!(
            "{} – {}",
            widgets::month_label(from),
            widgets::month_label(to)
        ),
    }
}

fn era_section(era: &EraRhythm, eras: usize, cx: &App) -> impl IntoElement {
    let title = if eras > 1 {
        era.label.clone()
    } else {
        "When you listen".into()
    };
    let span = span_label(&era.from, era.to.as_deref());
    v_flex()
        .gap_3()
        .child(widgets::section_heading(title, Some(span.into()), cx))
        .child(
            widgets::card("By hour of day", cx).child(match &era.hour_share {
                Some(shares) => div()
                    .h(px(180.))
                    .child(widgets::bar_chart(
                        ElementId::Name(format!("rhythm-hours-{}", era.key).into()),
                        hour_points(shares),
                        "% of plays",
                        cx,
                    ))
                    .into_any_element(),
                None => not_yet(era.hours_available_from.as_deref(), cx),
            }),
        )
        .child(
            widgets::card("By weekday and hour", cx).child(match &era.grid {
                Some(grid) => heatmap(grid, cx).into_any_element(),
                None => not_yet(era.grid_available_from.as_deref(), cx),
            }),
        )
}

fn hour_points(shares: &[f64]) -> Vec<(SharedString, f64)> {
    shares
        .iter()
        .enumerate()
        .map(|(hour, share)| (format!("{hour:02}").into(), share * 100.))
        .collect()
}

/// Why a figure is missing: the Worker holds each back until an era has
/// enough days to show a routine rather than a few particular nights.
fn not_yet(available_from: Option<&str>, cx: &App) -> AnyElement {
    let text = match available_from {
        Some(month) => format!(
            "Not enough days here yet to show a routine. Expected from {}.",
            widgets::month_label(month)
        ),
        None => "Too few days in this period to show a routine.".into(),
    };
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Each cell's strength, 0–1, relative to the busiest cell, so the pattern
/// shows whatever the total.
fn heat_levels(grid: &[Vec<f64>]) -> Vec<Vec<f32>> {
    let max = grid.iter().flatten().copied().fold(0., f64::max);
    grid.iter()
        .map(|row| {
            row.iter()
                .map(|&v| if max > 0. { (v / max) as f32 } else { 0. })
                .collect()
        })
        .collect()
}

fn heatmap(grid: &[Vec<f64>], cx: &App) -> impl IntoElement {
    let accent = cx.theme().primary;
    let empty = cx.theme().muted;
    let muted = cx.theme().muted_foreground;
    let label_w = px(36.);
    let levels = heat_levels(grid);
    let hour_labels = h_flex()
        .gap(px(2.))
        .child(div().w(label_w).flex_shrink_0())
        .children((0..24).map(move |hour| {
            div()
                .flex_1()
                .text_xs()
                .text_color(muted)
                .when(hour % 3 == 0, |this| this.child(format!("{hour:02}")))
        }));
    v_flex()
        .gap(px(2.))
        .child(hour_labels)
        .children(levels.into_iter().zip(WEEKDAYS).map(move |(row, day)| {
            h_flex()
                .gap(px(2.))
                .child(
                    div()
                        .w(label_w)
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(muted)
                        .child(day),
                )
                .children(row.into_iter().map(move |level| {
                    div().flex_1().h(px(18.)).rounded_sm().bg(if level > 0. {
                        accent.opacity(0.12 + 0.88 * level)
                    } else {
                        empty
                    })
                }))
        }))
}

/// Like [`widgets::remote_view`], but for content in a scrolling page: the
/// loading and error panels get a fixed height to centre in, and loaded
/// content keeps its natural height (with the retry banner above it when a
/// refresh failed) instead of being squeezed to fill the page.
pub(crate) fn page_body<T>(
    id: impl Into<ElementId>,
    remote: &Remote<T>,
    loading_label: impl Into<SharedString>,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
    content: impl FnOnce(&T) -> AnyElement,
) -> AnyElement {
    match (&remote.data, &remote.error) {
        (Some(data), error) => v_flex()
            .gap_3()
            .when_some(error.as_ref(), |this, error| {
                this.child(widgets::error_banner(id, error, on_retry, cx))
            })
            .child(content(data))
            .into_any_element(),
        (None, _) => div()
            .h(px(360.))
            .child(widgets::remote_view(
                id,
                remote,
                loading_label,
                on_retry,
                cx,
                |_| div().into_any_element(),
            ))
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Rhythm {
        serde_json::from_str(include_str!("../../tests/fixtures/stats-rhythm.json")).unwrap()
    }

    #[test]
    fn fixture_decodes_with_every_era() {
        let data = fixture();
        assert_eq!(data.streaks.len(), 3);
        assert_eq!(data.eras.len(), 2);
        assert_eq!(data.eras[0].grid.as_ref().unwrap().len(), 7);
        assert!(data.eras[1].grid.is_none());
        assert!(data.eras[1].to.is_none());
    }

    #[test]
    fn a_year_scopes_streaks_and_eras_and_an_unknown_one_falls_back() {
        let data = fixture();
        let (_, all) = scope(&data, None);
        assert_eq!(all.len(), 2);
        let (streaks, eras) = scope(&data, Some("2025"));
        assert_eq!((streaks.len(), eras.len()), (1, 1));
        let (_, gone) = scope(&data, Some("1999"));
        assert_eq!(gone.len(), 2);
    }

    #[test]
    fn streaks_read_as_lengths_and_spans() {
        let done = Streak {
            days: Some(140),
            weeks: 20,
            from: "2024-02".into(),
            to: "2024-06".into(),
            running: false,
        };
        assert_eq!(streak_length(&done), "140 days");
        assert_eq!(streak_span(&done), "Feb 2024 – Jun 2024");
        let running = Streak {
            days: None,
            weeks: 2,
            from: "2026-09".into(),
            to: "2026-10".into(),
            running: true,
        };
        assert_eq!(streak_length(&running), "2 weeks so far");
        assert_eq!(streak_span(&running), "Sep 2026 – now");
        assert_eq!(approx_days(&running), 14);
        assert_eq!(span_label("2024-02", Some("2024-02")), "Feb 2024");
        assert_eq!(threshold_label(1), "Any listening");
        assert_eq!(threshold_label(25), "25+ plays a day");
    }

    #[test]
    fn heat_is_relative_to_the_busiest_cell() {
        let levels = heat_levels(&[vec![0.0, 0.01], vec![0.02, 0.005]]);
        assert_eq!(levels, vec![vec![0.0, 0.5], vec![1.0, 0.25]]);
        assert_eq!(heat_levels(&[vec![0.0]]), vec![vec![0.0]]);
    }

    #[test]
    fn hours_are_labelled_and_in_percent() {
        let points = hour_points(&[0.02, 0.5]);
        assert_eq!(points[0].0.as_ref(), "00");
        assert_eq!(points[1].0.as_ref(), "01");
        assert!((points[1].1 - 50.).abs() < 1e-9);
    }
}
