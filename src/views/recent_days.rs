//! The "Plays per day" card on the Artist and Track pages: one song's or
//! artist's daily counts from `/api/daily` over a recent range, which the
//! all-time monthly chart is too coarse to show.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, IntoElement, ParentElement, SharedString, Styled, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
};

use crate::dates::{self, RangePreset};
use crate::models::DayCount;
use crate::state::RecentDays;
use crate::views::widgets;

const CHART_HEIGHT: f32 = 220.;

pub type PickRange = Rc<dyn Fn(RangePreset, &mut Window, &mut App)>;

/// `id` keeps the Artist and Track pages' elements apart.
pub fn card(
    id: &'static str,
    daily: &RecentDays,
    on_pick: PickRange,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let caption = daily
        .range
        .map(|range| range.describe())
        .unwrap_or_default();
    let presets = h_flex().gap_1().children(RangePreset::RECENT.map(|preset| {
        let on_pick = on_pick.clone();
        Button::new(SharedString::from(format!("{id}-{}", preset.label())))
            .xsmall()
            .label(preset.label())
            .selected(preset == daily.preset)
            .when(preset != daily.preset, |b| b.ghost())
            .on_click(move |_, window, cx| on_pick(preset, window, cx))
    }));
    let body = widgets::remote_view(
        SharedString::from(format!("{id}-retry")),
        &daily.days,
        "Loading plays per day…",
        on_retry,
        cx,
        |response| chart(id, &response.days, cx),
    );
    widgets::card("Plays per day", cx)
        .child(
            h_flex().gap_3().child(presets).child(div().flex_1()).child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(caption),
            ),
        )
        .child(div().h(px(CHART_HEIGHT)).child(body))
        .into_any_element()
}

fn chart(id: &'static str, days: &[DayCount], cx: &App) -> AnyElement {
    if days.iter().all(|d| d.plays == 0) {
        return widgets::state_panel(
            None,
            "No plays in this range",
            Some("Try a longer range; the monthly chart below covers all time.".into()),
            None,
            cx,
        )
        .into_any_element();
    }
    // The last band label is centred on the final bar and would otherwise be
    // clipped by the card edge, as on the monthly chart.
    div()
        .size_full()
        .pr_6()
        .child(widgets::bar_chart(
            SharedString::from(format!("{id}-chart")),
            day_points(days),
            "Plays",
            cx,
        ))
        .into_any_element()
}

/// Bar labels from the Worker's days, unique across the range as the chart
/// needs (`dates::day_label` keeps the year on long ranges).
fn day_points(days: &[DayCount]) -> Vec<(SharedString, f64)> {
    let range_days = days.len() as u64;
    days.iter()
        .map(|d| (dates::day_label(&d.day, range_days).into(), d.plays.into()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_labels_stay_unique_over_a_year() {
        let days: Vec<DayCount> = (0..365)
            .map(|i| {
                let day =
                    chrono::NaiveDate::from_ymd_opt(2025, 10, 16).unwrap() + chrono::Days::new(i);
                DayCount {
                    day: day.format("%Y-%m-%d").to_string(),
                    plays: i as u32,
                }
            })
            .collect();
        let points = day_points(&days);
        let labels: std::collections::HashSet<_> = points.iter().map(|p| p.0.clone()).collect();
        assert_eq!(labels.len(), 365);
        assert_eq!(points[364].1, 364.);
    }
}
