//! Loading, empty and error presentations shared by every view, so each
//! failure reads the same wherever it surfaces, and the building blocks
//! pages are made of (tiles, headings, ranked rows, charts, art, formatting).

// A shared toolkit: a widget stays here even while no page uses it yet.
#![allow(dead_code)]

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

use crate::actions::OpenSettings;
use crate::api::ApiError;
use crate::models::{ImageRef, Rendition, album_rendition, pick_image};
use crate::state::Remote;

/// What to tell the user about an error: a headline, then what to do.
pub struct ErrorCopy {
    pub title: SharedString,
    pub detail: SharedString,
}

pub fn describe(error: &ApiError) -> ErrorCopy {
    let (title, detail): (&str, String) = match error {
        ApiError::Unauthorized => (
            "Read token rejected",
            "The Worker no longer accepts the saved token. Tokens from signing in expire after \
             90 days. Open Settings (⌘,) to sign in with Spotify again or paste a token."
                .into(),
        ),
        ApiError::NeedsReauth => (
            "Spotify needs re-authorizing",
            "The Worker's Spotify grant has expired. Stored history still works; run \
             `npm run authorize` in music-warehouse to restore live data."
                .into(),
        ),
        ApiError::RateLimited {
            retry_after_seconds,
        } => (
            "Spotify is rate limiting",
            match retry_after_seconds {
                Some(s) => format!("Spotify asked the Worker to wait {}.", human_seconds(*s)),
                None => "Spotify asked the Worker to slow down. It will retry shortly.".into(),
            },
        ),
        ApiError::Upstream(message) => (
            "Spotify is unavailable",
            format!("The Worker could not get an answer from Spotify ({message})."),
        ),
        ApiError::BadRequest(message) => ("Request refused", message.clone()),
        ApiError::Server { status, message } => {
            ("Worker error", format!("HTTP {status}: {message}"))
        }
        ApiError::Network(message) => ("Can't reach the Worker", message.clone()),
        ApiError::Decode(message) => (
            "Unexpected response",
            format!("The Worker's response did not match what this app expects: {message}"),
        ),
    };
    ErrorCopy {
        title: title.into(),
        detail: detail.into(),
    }
}

pub fn human_seconds(seconds: u64) -> String {
    match seconds {
        0..=89 => format!("{seconds}s"),
        90..=5399 => format!("{} min", seconds.div_ceil(60)),
        _ => format!("{} h", seconds.div_ceil(3600)),
    }
}

/// A centred message filling its container: the shape every loading, empty
/// and error state takes.
pub fn state_panel(
    icon: Option<Icon>,
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    action: Option<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .p_6()
        .text_center()
        .when_some(icon, |this, icon| {
            this.child(icon.size_8().text_color(cx.theme().muted_foreground))
        })
        .child(div().font_semibold().child(title.into()))
        .when_some(detail, |this, detail| {
            this.child(
                div()
                    .max_w_96()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail),
            )
        })
        .when_some(action, |this, action| {
            this.child(div().pt_2().child(action))
        })
}

pub fn loading_panel(label: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(Spinner::new().large())
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
}

pub fn error_panel(
    id: impl Into<ElementId>,
    error: &ApiError,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let copy = describe(error);
    let icon = if error.is_spotify_side() {
        IconName::CloudOff
    } else {
        IconName::TriangleAlert
    };
    // Retrying with a rejected token cannot work; signing in again can.
    let button = if *error == ApiError::Unauthorized {
        Button::new(id)
            .small()
            .label("Sign in again")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx))
    } else {
        Button::new(id).small().label("Retry").on_click(on_retry)
    };
    state_panel(
        Some(Icon::new(icon)),
        copy.title,
        Some(copy.detail),
        Some(button.into_any_element()),
        cx,
    )
}

/// A one-line banner for an error that sits above still-useful content.
pub fn error_banner(
    id: impl Into<ElementId>,
    error: &ApiError,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let copy = describe(error);
    h_flex()
        .gap_2()
        .px_3()
        .py_1p5()
        .rounded_md()
        .bg(cx.theme().danger.opacity(0.1))
        .text_sm()
        .child(Icon::new(IconName::TriangleAlert).small())
        .child(div().font_semibold().child(copy.title))
        .child(
            div()
                .flex_1()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(copy.detail),
        )
        .child(Button::new(id).xsmall().label("Retry").on_click(on_retry))
}

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

/// A headline figure: a big value over a muted label, with an optional
/// detail line. Lay several out in an `h_flex().gap_3()`; each takes an equal
/// share of the row.
pub fn stat_tile(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    detail: Option<SharedString>,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .truncate()
                .child(label.into()),
        )
        .child(
            div()
                .text_2xl()
                .font_semibold()
                .truncate()
                .child(value.into()),
        )
        .when_some(detail, |this, detail| {
            this.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(detail),
            )
        })
}

/// One line of a ranked list: rank, optional art, title over a muted
/// subtitle, a bar showing `share` (0–1, relative to the list's top entry or
/// to a total, as the page decides), and the count. With an action the whole
/// row is clickable and dispatches it, e.g. [`crate::actions::OpenArtist`].
#[derive(IntoElement)]
pub struct RankedRow {
    id: ElementId,
    rank: Option<usize>,
    art: Option<AnyElement>,
    title: SharedString,
    subtitle: Option<SharedString>,
    value: Option<SharedString>,
    share: Option<f32>,
    action: Option<Box<dyn Action>>,
}

impl RankedRow {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            rank: None,
            art: None,
            title: title.into(),
            subtitle: None,
            value: None,
            share: None,
            action: None,
        }
    }

    /// 1-based position shown on the left.
    pub fn rank(mut self, rank: usize) -> Self {
        self.rank = Some(rank);
        self
    }

    /// Usually [`album_art`] or [`artwork`] at 32px.
    pub fn art(mut self, art: AnyElement) -> Self {
        self.art = Some(art);
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// The count on the right, already formatted ([`count`], [`minutes`]).
    pub fn value(mut self, value: impl Into<SharedString>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Bar length, 0–1. Values outside are clamped.
    pub fn share(mut self, share: f32) -> Self {
        self.share = Some(share.clamp(0., 1.));
        self
    }

    pub fn on_click_action(mut self, action: impl Action) -> Self {
        self.action = Some(Box::new(action));
        self
    }
}

impl RenderOnce for RankedRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let hover = cx.theme().muted.opacity(0.5);
        let clickable = self.action.is_some();
        h_flex()
            .id(self.id)
            .gap_2()
            .px_1()
            .py_0p5()
            .rounded_md()
            .items_center()
            .text_sm()
            .when_some(self.rank, |this, rank| {
                this.child(
                    div()
                        .w(px(24.))
                        .flex_shrink_0()
                        .text_right()
                        .text_color(muted)
                        .child(rank.to_string()),
                )
            })
            .when_some(self.art, |this, art| this.child(art))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(self.title))
                    .when_some(self.subtitle, |this, subtitle| {
                        this.child(div().truncate().text_xs().text_color(muted).child(subtitle))
                    }),
            )
            .when_some(self.share, |this, share| {
                this.child(
                    div()
                        .w(px(120.))
                        .flex_shrink_0()
                        .child(share_bar(share, cx)),
                )
            })
            .when_some(self.value, |this, value| {
                this.child(
                    div()
                        .min_w(px(56.))
                        .flex_shrink_0()
                        .text_right()
                        .child(value),
                )
            })
            .when_some(self.action, |this, action| {
                this.cursor_pointer()
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            })
            .when(clickable, |this| this.hover(move |style| style.bg(hover)))
    }
}

/// A thin rounded bar filled to `share` (0–1) in the accent colour.
pub fn share_bar(share: f32, cx: &App) -> impl IntoElement {
    div()
        .w_full()
        .h(px(6.))
        .rounded_full()
        .bg(cx.theme().muted)
        .child(
            div()
                .h_full()
                .rounded_full()
                .w(relative(share.clamp(0., 1.)))
                .bg(cx.theme().primary),
        )
}

/// Text that dispatches `action` when clicked, for names inside other text
/// (an artist under a song title, say).
pub fn link(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: impl Action,
    cx: &App,
) -> impl IntoElement {
    let action: Box<dyn Action> = Box::new(action);
    let hover = cx.theme().primary;
    div()
        .id(id)
        .cursor_pointer()
        .hover(move |style| style.text_color(hover))
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        .child(label.into())
}

/// A labelled bar chart over a series (months, hours, quarters), styled like
/// Overview's daily chart. Labels must be distinct; `name` is what the
/// tooltip calls the value ("Plays", "Minutes"). Give it a fixed-height
/// parent, e.g. `div().h(px(220.)).child(...)`.
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
        .value_tick_format(|v| format!("{v:.0}"))
        .band_tick_count(bands.clamp(1, 8))
        .grid_dashed(false)
        .value_axis_label_placement(AxisLabelPlacement::Inside)
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

/// Renders a [`Remote`]'s three states the same way everywhere: a spinner
/// before the first answer, the error panel if that failed, else `content`
/// with a retry banner above it when a later refresh failed.
pub fn remote_view<T>(
    id: impl Into<ElementId>,
    remote: &Remote<T>,
    loading_label: impl Into<SharedString>,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
    content: impl FnOnce(&T) -> AnyElement,
) -> AnyElement {
    match (&remote.data, &remote.error) {
        (None, None) => loading_panel(loading_label, cx).into_any_element(),
        (None, Some(error)) => error_panel(id, error, on_retry, cx).into_any_element(),
        (Some(data), None) => content(data),
        (Some(data), Some(error)) => v_flex()
            .size_full()
            .gap_2()
            .child(error_banner(id, error, on_retry, cx))
            .child(div().flex_1().min_h_0().child(content(data)))
            .into_any_element(),
    }
}

/// What an unfinished page shows until its own PR lands.
pub fn coming_soon(
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    cx: &App,
) -> impl IntoElement {
    state_panel(
        Some(Icon::new(IconName::Construction)),
        title,
        detail,
        None,
        cx,
    )
}

/// `12,345`.
pub fn count(n: u64) -> String {
    crate::views::history::group(n as usize)
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
    chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
        .map(|d| d.format("%b %Y").to_string())
        .unwrap_or_else(|_| month.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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
