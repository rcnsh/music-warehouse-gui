//! Loading, empty and error presentations shared by every view, so each
//! failure reads the same wherever it surfaces, and the building blocks
//! pages are made of (tiles, headings, ranked rows, charts, art, formatting).
//! States, rows and charts have their own files; pages use them all through
//! `widgets::`.

mod charts;
mod rows;
mod states;

pub use charts::*;
pub use rows::*;
pub use states::*;

use chrono::{DateTime, Local, NaiveDate, TimeZone};
use gpui::{
    Action, AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, Pixels, RenderOnce, SharedString, StatefulInteractiveElement, Styled,
    StyledImage, Window, div, img, prelude::FluentBuilder, px, relative,
};
use gpui_component::{
    ActiveTheme, Icon, Sizable, StyledExt, button::Button, chart::BarChart, h_flex,
    plot::AxisLabelPlacement, spinner::Spinner, v_flex,
};
use gpui_kit_assets::IconName;

use crate::actions::{OpenArtist, OpenSettings};
use crate::api::ApiError;
use crate::models::{Credit, ImageRef, Rendition, album_rendition, pick_image};
use crate::state::Remote;

/// Album or artist art at a fixed size. Missing art and failed downloads
/// both show the same placeholder, so a dead image URL never leaves a hole
/// or shifts the row.
pub fn artwork(url: Option<SharedString>, size: Pixels, round: bool, cx: &App) -> AnyElement {
    let placeholder_bg = cx.theme().muted;
    let placeholder_fg = cx.theme().muted_foreground;
    let placeholder = move || {
        div()
            .size(size)
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(placeholder_bg)
            .when(round, |this| this.rounded_full())
            .when(!round, |this| this.rounded_sm())
            .child(
                Icon::new(IconName::Music)
                    .size(size * 0.5)
                    .text_color(placeholder_fg),
            )
            .into_any_element()
    };
    match url {
        Some(url) => img(url)
            .size(size)
            .flex_shrink_0()
            .object_fit(ObjectFit::Cover)
            .when(round, |this| this.rounded_full())
            .when(!round, |this| this.rounded_sm())
            .with_fallback(placeholder)
            .with_loading(placeholder)
            .into_any_element(),
        None => placeholder(),
    }
}

pub fn card(title: impl Into<SharedString>, cx: &App) -> gpui::Div {
    v_flex()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .bg(cx.theme().background)
        .p_4()
        .gap_3()
        .child(div().font_semibold().child(title.into()))
}

// ---------------------------------------------------------------------------
// Page building blocks. Generic on purpose: pages compose these rather than
// restyling the same shapes, so every page reads as one app.
// ---------------------------------------------------------------------------

/// A page's scrolling column, padded like Overview. Add sections as children.
/// `id` must be unique within the page (the page's name is fine).
pub fn page(id: impl Into<ElementId>) -> gpui::Stateful<gpui::Div> {
    v_flex()
        .id(id)
        .size_full()
        .overflow_y_scroll()
        .p_4()
        .gap_4()
}

/// A page's title line: a large title, an optional muted subtitle (what the
/// figures cover, when they were computed), and optional controls on the right.
pub fn page_header(
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    controls: Option<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .gap_4()
        .items_end()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_xl()
                        .font_semibold()
                        .truncate()
                        .child(title.into()),
                )
                .when_some(subtitle, |this, subtitle| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(subtitle),
                    )
                }),
        )
        .when_some(controls, |this, controls| this.child(controls))
}

/// A small heading above a group of rows or tiles, with optional muted detail.
pub fn section_heading(
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .items_baseline()
        .child(div().font_semibold().child(title.into()))
        .when_some(detail, |this, detail| {
            this.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail),
            )
        })
}

/// Album art from a stored row's 640px URL, fetched at the smallest CDN
/// rendition that stays sharp at `size` on a 2x display.
pub fn album_art(url: Option<&str>, size: Pixels, cx: &App) -> AnyElement {
    let rendition = if size.as_f32() <= 32. {
        Rendition::Px64
    } else {
        Rendition::Px300
    };
    let url = url.map(|u| SharedString::from(album_rendition(u, rendition)));
    artwork(url, size, false, cx)
}

/// Art from a live Spotify payload's image list (artists round, albums square),
/// choosing the smallest image that stays sharp at `size` on a 2x display.
pub fn spotify_art(images: &[ImageRef], size: Pixels, round: bool, cx: &App) -> AnyElement {
    let wanted = (size.as_f32() * 2.).ceil() as u32;
    let url = pick_image(images, wanted).map(|u| SharedString::from(u.to_owned()));
    artwork(url, size, round, cx)
}

/// A muted line of small text: a note under a heading, or what stands in for
/// an empty list inside a card.
pub fn note(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// `12,345`.
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `1 play`, `1,204 plays`.
pub fn plural(n: u64, unit: &str) -> String {
    match n {
        1 => format!("1 {unit}"),
        n => format!("{} {unit}s", count(n)),
    }
}

/// Listening time for a figure in minutes: `45 min`, `12 h 5 min`, `1,204 h`.
pub fn minutes(total: u64) -> String {
    match total {
        0..=59 => format!("{total} min"),
        60..=5999 => match total % 60 {
            0 => format!("{} h", total / 60),
            m => format!("{} h {m} min", total / 60),
        },
        _ => format!("{} h", count(total / 60)),
    }
}

/// A 0–1 share as a percentage: `12.8%`, or `<0.1%` for a sliver.
pub fn percent(share: f64) -> String {
    let pct = share * 100.;
    if pct > 0. && pct < 0.1 {
        "<0.1%".into()
    } else if pct >= 10. {
        format!("{pct:.0}%")
    } else {
        format!("{pct:.1}%")
    }
}

/// `2026-07` → `Jul 2026`; anything else unchanged.
pub fn month_label(month: &str) -> String {
    NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
        .map(|d| d.format("%b %Y").to_string())
        .unwrap_or_else(|_| month.to_owned())
}

/// A span of `YYYY-MM` months: `Feb 2024 – Jun 2024`, `Feb 2024` when it is
/// one month (or the end is blank), `Sep 2026 – now` when `to` is `None`
/// because it is still running.
pub fn month_span(from: &str, to: Option<&str>) -> String {
    match to {
        None => format!("{} – now", month_label(from)),
        Some(to) if to == from || to.is_empty() => month_label(from),
        Some(to) => format!("{} – {}", month_label(from), month_label(to)),
    }
}

/// `16 Jul 2017`: how every page writes a calendar day.
pub fn day(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// A `YYYY-MM-DD` day from the Worker as [`day`] writes it; anything else
/// unchanged.
pub fn day_label(text: &str) -> String {
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map(day)
        .unwrap_or_else(|_| text.to_owned())
}

/// The local calendar day of an ISO 8601 instant (a stat's `as_of`, a first
/// play).
pub fn local_date(iso: &str) -> Option<NaiveDate> {
    instant_in(iso, &Local).map(|at| at.date_naive())
}

/// An ISO 8601 instant as a local day, `12 Oct 2026`.
pub fn local_day(iso: &str) -> Option<String> {
    local_date(iso).map(day)
}

/// An ISO 8601 instant as a local day and time: `("16 Jul 2017", "16:32")`.
pub fn local_day_time(iso: &str) -> Option<(String, String)> {
    day_time_in(iso, &Local)
}

fn instant_in<Tz: TimeZone>(iso: &str, tz: &Tz) -> Option<DateTime<Tz>> {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|at| at.with_timezone(tz))
}

fn day_time_in<Tz: TimeZone>(iso: &str, tz: &Tz) -> Option<(String, String)>
where
    Tz::Offset: std::fmt::Display,
{
    let at = instant_in(iso, tz)?;
    Some((day(at.date_naive()), at.format("%H:%M").to_string()))
}

/// "1st", "2nd", "3rd", "4th", with 11th to 13th as English has them.
pub fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{}{suffix}", count(n.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_scale_to_the_first_value_and_never_zero() {
        assert_eq!(top_scale([40, 12, 3]), 40);
        assert_eq!(top_scale([]), 1);
        assert_eq!(top_scale([0, 0]), 1);
    }

    #[test]
    fn figures_format_for_reading() {
        assert_eq!(count(1_234_567), "1,234,567");
        assert_eq!(minutes(45), "45 min");
        assert_eq!(minutes(120), "2 h");
        assert_eq!(minutes(725), "12 h 5 min");
        assert_eq!(minutes(72_240), "1,204 h");
        assert_eq!(percent(0.128), "13%");
        assert_eq!(percent(0.0734), "7.3%");
        assert_eq!(percent(0.0004), "<0.1%");
        assert_eq!(percent(0.), "0.0%");
        assert_eq!(month_label("2026-07"), "Jul 2026");
        assert_eq!(month_label("2026-Q3"), "2026-Q3");
        assert_eq!(plural(1, "day"), "1 day");
        assert_eq!(plural(1204, "play"), "1,204 plays");
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1000), "1,000");
    }

    #[test]
    fn spans_and_days_read_naturally() {
        assert_eq!(
            month_span("2024-02", Some("2024-06")),
            "Feb 2024 – Jun 2024"
        );
        assert_eq!(month_span("2024-02", Some("2024-02")), "Feb 2024");
        assert_eq!(month_span("2024-02", Some("")), "Feb 2024");
        assert_eq!(month_span("2026-09", None), "Sep 2026 – now");
        assert_eq!(day_label("2025-08-04"), "4 Aug 2025");
        assert_eq!(day_label("soon"), "soon");
    }

    #[test]
    fn instants_read_in_the_local_zone() {
        use chrono::{FixedOffset, Utc};
        let singapore = FixedOffset::east_opt(8 * 3600).unwrap();
        assert_eq!(
            day_time_in("2017-07-16T16:32:15.000Z", &singapore),
            Some(("17 Jul 2017".into(), "00:32".into()))
        );
        assert_eq!(
            instant_in("2026-10-11T16:00:00.000Z", &Utc).map(|t| t.date_naive()),
            NaiveDate::from_ymd_opt(2026, 10, 11)
        );
        assert_eq!(day_time_in("not a date", &singapore), None);
    }

    #[test]
    fn ordinals() {
        let cases = [
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (4, "4th"),
            (11, "11th"),
            (12, "12th"),
            (13, "13th"),
            (21, "21st"),
            (22, "22nd"),
            (23, "23rd"),
            (101, "101st"),
            (111, "111th"),
            (112, "112th"),
            (1001, "1,001st"),
        ];
        for (n, want) in cases {
            assert_eq!(ordinal(n), want);
        }
    }

    #[test]
    fn hours_are_labelled() {
        let points = hour_points([0.5, 2.]);
        assert_eq!(points[0], ("00".into(), 0.5));
        assert_eq!(points[1], ("01".into(), 2.));
    }

    #[test]
    fn human_seconds_rounds_up() {
        assert_eq!(human_seconds(30), "30s");
        assert_eq!(human_seconds(91), "2 min");
        assert_eq!(human_seconds(3875), "65 min");
        assert_eq!(human_seconds(86_088), "24 h");
    }

    #[test]
    fn live_failures_get_their_own_copy() {
        assert_eq!(
            describe(&ApiError::NeedsReauth).title.as_ref(),
            "Spotify needs re-authorizing"
        );
        assert!(
            describe(&ApiError::RateLimited {
                retry_after_seconds: Some(120)
            })
            .detail
            .contains("2 min")
        );
        assert_eq!(
            describe(&ApiError::Upstream("x".into())).title.as_ref(),
            "Spotify is unavailable"
        );
    }
}
