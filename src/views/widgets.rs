//! Loading, empty and error presentations shared by every view, so each
//! failure reads the same wherever it surfaces, and the building blocks
//! pages are made of (tiles, headings, ranked rows, charts, art, formatting).

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
        ApiError::StillComputing {
            retry_after_seconds,
        } => (
            "Still computing…",
            format!(
                "The Worker is working this out for the first time. It asked to be \
                 asked again in {}; this page does that by itself.",
                human_seconds(*retry_after_seconds)
            ),
        ),
        ApiError::Server { status, message } => {
            ("Worker error", format!("HTTP {status}: {message}"))
        }
        ApiError::Network(message) => ("Can't reach the Worker", message.clone()),
        ApiError::Decode(message) => (
            "Unexpected response",
            format!("The Worker's response did not match what this app expects: {message}"),
        ),
        ApiError::File(message) => ("Could not save the file", message.clone()),
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
    let icon = error_icon(error);
    let button = retry_button(id, error, on_retry).small();
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
        .map(|this| match error {
            // Not a failure: the page fills in by itself once it is computed.
            ApiError::StillComputing { .. } => this.bg(cx.theme().muted),
            _ => this.bg(cx.theme().danger.opacity(0.1)),
        })
        .text_sm()
        .child(Icon::new(error_icon(error)).small())
        .child(div().font_semibold().child(copy.title))
        .child(
            div()
                .flex_1()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(copy.detail),
        )
        .child(retry_button(id, error, on_retry).xsmall())
}

fn error_icon(error: &ApiError) -> IconName {
    match error {
        ApiError::StillComputing { .. } => IconName::LoaderCircle,
        error if error.is_spotify_side() => IconName::CloudOff,
        _ => IconName::TriangleAlert,
    }
}

/// Retry, or "Sign in again" for a rejected token: retrying with it cannot
/// work, signing in again can.
fn retry_button(
    id: impl Into<ElementId>,
    error: &ApiError,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    if *error == ApiError::Unauthorized {
        Button::new(id)
            .label("Sign in again")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx))
    } else {
        Button::new(id).label("Retry").on_click(on_retry)
    }
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
    value_width: Pixels,
    action: Option<Box<dyn Action>>,
}

/// The value column's default width, enough for `12 h 34 min` or `12,345`.
/// Fixed rather than fitted, so the bars of rows with longer and shorter
/// values still line up.
pub const RANKED_VALUE_WIDTH: Pixels = px(84.);

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
            value_width: RANKED_VALUE_WIDTH,
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

    /// Width of the value column, for lists whose values are much shorter
    /// (`7%`) or longer than [`RANKED_VALUE_WIDTH`] allows. Use the same width
    /// for every row of a list.
    pub fn value_width(mut self, width: Pixels) -> Self {
        self.value_width = width;
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
                        .w(self.value_width)
                        .flex_shrink_0()
                        .text_right()
                        .truncate()
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

/// Renders a [`Remote`]'s three states in a pane of fixed size (Overview's
/// panels): a spinner before the first answer, the error panel if that
/// failed, else `content` with a retry banner above it when a later refresh
/// failed. Everything stretches to fill the pane; inside a scrolling [`page`]
/// use [`page_body`] instead.
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

/// How tall a loading, error or empty panel is inside a scrolling [`page`],
/// which has no height of its own for the panel to centre in.
pub const PAGE_PANEL_HEIGHT: Pixels = px(320.);

/// [`remote_view`] for a scrolling [`page`]: the loading and error panels get
/// [`PAGE_PANEL_HEIGHT`] to centre in, and loaded content keeps its natural
/// height (under a retry banner when a refresh failed) rather than being
/// squeezed to fit the window.
pub fn page_body<T>(
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
                this.child(error_banner(id, error, on_retry, cx))
            })
            .child(content(data))
            .into_any_element(),
        (None, Some(error)) => div()
            .h(PAGE_PANEL_HEIGHT)
            .child(error_panel(id, error, on_retry, cx))
            .into_any_element(),
        (None, None) => div()
            .h(PAGE_PANEL_HEIGHT)
            .child(loading_panel(loading_label, cx))
            .into_any_element(),
    }
}

/// An empty or explanatory [`state_panel`] inside a scrolling [`page`], at
/// [`PAGE_PANEL_HEIGHT`].
pub fn page_state(
    icon: Option<IconName>,
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    cx: &App,
) -> AnyElement {
    div()
        .h(PAGE_PANEL_HEIGHT)
        .child(state_panel(icon.map(Icon::new), title, detail, None, cx))
        .into_any_element()
}

/// A muted line of small text: a note under a heading, or what stands in for
/// an empty list inside a card.
pub fn note(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// The value a ranked list's bars are drawn against: its first, since the
/// Worker sends lists sorted, and never below 1 so an empty or all-zero
/// list divides safely. Nothing is re-ranked here.
pub fn top_scale(values: impl IntoIterator<Item = u32>) -> u32 {
    values.into_iter().next().unwrap_or(0).max(1)
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

/// Each credited artist a link to their page, comma-separated. A credit with
/// an id opens that artist; one without (an older Worker's joined names, an
/// artist on a concert bill the warehouse does not know) opens by name.
pub fn credit_links(id: impl Into<SharedString>, credits: &[Credit], cx: &App) -> gpui::Div {
    let id = id.into();
    let mut row = h_flex().flex_wrap();
    for (ix, credit) in credits.iter().enumerate() {
        if ix > 0 {
            row = row.child(div().pr_1().child(","));
        }
        row = row.child(link(
            ElementId::named_usize(id.clone(), ix),
            credit.name.clone(),
            OpenArtist::by_id_or_name(credit.id.as_deref(), &credit.name),
            cx,
        ));
    }
    row
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

pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Chart points for a value per hour of the day, labelled `00`–`23`.
pub fn hour_points(values: impl IntoIterator<Item = f64>) -> Vec<(SharedString, f64)> {
    values
        .into_iter()
        .enumerate()
        .map(|(hour, v)| (format!("{hour:02}").into(), v))
        .collect()
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
