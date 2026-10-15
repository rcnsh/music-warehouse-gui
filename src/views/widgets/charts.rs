//! Bar charts, sparklines and the series they draw.

use super::*;

/// The widest a chart bar gets, so a short series across a wide card (one
/// month of plays, seven weekdays) keeps slim bars like Overview's.
pub const CHART_BAR_MAX_WIDTH: f32 = 30.;

/// A labelled bar chart over a series (months, hours, quarters), styled like
/// Overview's daily chart. Labels must be distinct; `name` is what the
/// tooltip calls the value ("Plays", "Minutes"). Give it a fixed-height
/// parent, e.g. `div().h(px(220.)).child(...)`.
///
/// Unlike Overview's, the value labels sit in a gutter left of the bars
/// rather than over them: nothing here hit-tests clicks, so the gutter costs
/// nothing, and inside the plot they cover the first bars.
pub fn bar_chart(
    id: impl Into<ElementId>,
    points: Vec<(SharedString, f64)>,
    name: impl Into<SharedString>,
    cx: &App,
) -> impl IntoElement {
    let accent = cx.theme().primary;
    let bands = points.len();
    BarChart::new(points)
        .band(|p: &(SharedString, f64)| p.0.clone())
        .value(|p: &(SharedString, f64)| p.1)
        .name(name)
        .fill(move |_, _, _, _| accent)
        .value_axis(true)
        .value_tick_count(4)
        // The zero line needs no label, and in the gutter it would touch
        // the first band label below it.
        .value_tick_format(|v| {
            if v == 0. {
                String::new()
            } else {
                format!("{v:.0}")
            }
        })
        .band_tick_count(bands.clamp(1, 8))
        .grid_dashed(false)
        .value_axis_label_placement(AxisLabelPlacement::Outside)
        .max_band_width(px(CHART_BAR_MAX_WIDTH))
        .id(id)
}

/// A tiny inline bar series (on-repeat's 14 days, a year's months) with no
/// axes: each bar's height is its share of the largest value.
pub fn sparkline(values: &[f64], height: Pixels, cx: &App) -> impl IntoElement {
    let max = values.iter().copied().fold(0., f64::max);
    let accent = cx.theme().primary;
    let empty = cx.theme().muted;
    h_flex()
        .h(height)
        .gap(px(2.))
        .items_end()
        .children(values.iter().map(move |&v| {
            let share = if max > 0. { (v / max) as f32 } else { 0. };
            div()
                .w(px(4.))
                .h(relative(share.max(0.04)))
                .rounded_sm()
                .bg(if v > 0. { accent } else { empty })
        }))
}

/// The value a ranked list's bars are drawn against: its first, since the
/// Worker sends lists sorted, and never below 1 so an empty or all-zero
/// list divides safely. Nothing is re-ranked here.
pub fn top_scale(values: impl IntoIterator<Item = u32>) -> u32 {
    values.into_iter().next().unwrap_or(0).max(1)
}

pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Chart points for a value per hour of the day, labelled `00`–`23`.
pub fn hour_points(values: impl IntoIterator<Item = f64>) -> Vec<(SharedString, f64)> {
    values
        .into_iter()
        .enumerate()
        .map(|(hour, v)| (format!("{hour:02}").into(), v))
        .collect()
}
