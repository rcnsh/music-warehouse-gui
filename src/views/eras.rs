//! The Eras page (`/api/stats/eras`): how the artist mix changed when the
//! latest era began (usually a move), against how much it changes anyway
//! over the same number of months. Eras are whatever the Worker's ERAS
//! setting names; this page only ever shows their labels.

use gpui::AppContext as _;
use gpui::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable, Hsla, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px, relative,
};
use gpui_component::{ActiveTheme, Icon, h_flex, v_flex};
use gpui_kit_assets::IconName;

use crate::actions::OpenArtist;
use crate::api::ApiClient;
use crate::models::{ArtistShare, Eras, MonthSpan, ShareChange, TypicalChange};
use crate::state::eras::ErasStore;
use crate::views::rhythm::{page_body, span_label};
use crate::views::widgets;

pub struct ErasView {
    store: Entity<ErasStore>,
    focus: FocusHandle,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl ErasView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| ErasStore::new(client, cx));
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

impl Focusable for ErasView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ErasView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let store = self.store.read(cx);
        let subtitle = store.eras.data.as_ref().and_then(subtitle);
        let body = page_body(
            "eras-retry",
            &store.eras,
            "Loading eras…",
            retry,
            cx,
            |data| match comparison(data) {
                Comparison::SingleEra => explain(
                    "Only one era is configured",
                    "This page compares listening before and after the latest era began, \
                     such as a move to another city. Add a second era to the Worker's ERAS \
                     setting to see the comparison."
                        .into(),
                    cx,
                ),
                Comparison::TooEarly => explain("Too early to compare", too_early_detail(data), cx),
                Comparison::Ready => ready_view(data, cx),
            },
        );
        widgets::page("eras")
            .track_focus(&self.focus)
            .child(widgets::page_header("Eras", subtitle, None, cx))
            .child(body)
    }
}

fn subtitle(data: &Eras) -> Option<SharedString> {
    let before = &data.previous_era.as_ref()?.label;
    let latest = &data.latest_era.as_ref()?.label;
    Some(
        match data.window_months {
            0 => format!("{before} → {latest}"),
            1 => format!("{before} → {latest}, one month each side of the change"),
            n => format!("{before} → {latest}, {n} months each side of the change"),
        }
        .into(),
    )
}

fn explain(title: &'static str, detail: String, cx: &App) -> AnyElement {
    div()
        .h(px(360.))
        .child(widgets::state_panel(
            Some(Icon::new(IconName::Info)),
            title,
            Some(detail.into()),
            None,
            cx,
        ))
        .into_any_element()
}

#[derive(Debug, PartialEq)]
enum Comparison {
    SingleEra,
    TooEarly,
    Ready,
}

fn comparison(data: &Eras) -> Comparison {
    if data.latest_era.is_none() || data.previous_era.is_none() {
        Comparison::SingleEra
    } else if !data.ready || data.summary.is_none() {
        Comparison::TooEarly
    } else {
        Comparison::Ready
    }
}

fn too_early_detail(data: &Eras) -> String {
    let latest = data
        .latest_era
        .as_ref()
        .map(|e| e.label.as_str())
        .unwrap_or("The latest era");
    match &data.move_month {
        Some(month) => format!(
            "{latest} began in {}. The comparison appears once its first full month has ended.",
            widgets::month_label(month)
        ),
        None => format!("The comparison appears once {latest} has a full month of listening."),
    }
}

fn period(span: Option<&MonthSpan>) -> String {
    span.map(|s| span_label(&s.from, Some(&s.to)))
        .unwrap_or_default()
}

fn ready_view(data: &Eras, cx: &App) -> AnyElement {
    let (Some(summary), Some(latest), Some(previous)) =
        (&data.summary, &data.latest_era, &data.previous_era)
    else {
        return div().into_any_element();
    };
    let (latest, previous) = (&latest.label, &previous.label);
    let typical = &summary.typical_change;
    let before_span = period(data.before.as_ref());
    let latest_span = period(data.latest.as_ref());
    v_flex()
        .gap_6()
        .child(
            h_flex()
                .gap_3()
                .child(widgets::stat_tile(
                    "Plays to artists already known",
                    widgets::percent(summary.known_artist_share),
                    Some(format!("of {latest}'s plays").into()),
                    cx,
                ))
                .child(widgets::stat_tile(
                    "New artists",
                    widgets::count(summary.new_artists.into()),
                    Some(
                        format!(
                            "of {} played in {latest}",
                            widgets::count(summary.artists.into())
                        )
                        .into(),
                    ),
                    cx,
                ))
                .child(widgets::stat_tile(
                    "Change in artist mix",
                    widgets::percent(summary.change),
                    Some(format!("typically {}", widgets::percent(typical.median)).into()),
                    cx,
                )),
        )
        .child(change_card(summary.change, typical, data.window_months, cx))
        .child(
            v_flex()
                .gap_2()
                .child(widgets::section_heading(
                    "Top artists",
                    Some("Share of each period's plays".into()),
                    cx,
                ))
                .child(
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(top_list(
                            "eras-earlier",
                            format!("Before {latest}"),
                            "All earlier history".into(),
                            &data.top.earlier,
                            cx,
                        ))
                        .child(top_list(
                            "eras-before",
                            format!("End of {previous}"),
                            before_span.clone(),
                            &data.top.before,
                            cx,
                        ))
                        .child(top_list(
                            "eras-latest",
                            latest.clone(),
                            latest_span.clone(),
                            &data.top.latest,
                            cx,
                        )),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .items_start()
                .child(change_list(
                    "eras-risers",
                    "Played more",
                    &data.risers,
                    &before_span,
                    &latest_span,
                    cx,
                ))
                .child(change_list(
                    "eras-fallers",
                    "Played less",
                    &data.fallers,
                    &before_span,
                    &latest_span,
                    cx,
                )),
        )
        .into_any_element()
}

/// Where the usual range and this change sit on one track, as fractions of
/// it: `(min, max, median, this)`. The track runs a little past the largest
/// figure so no marker sits on its end.
fn gauge_positions(change: f64, typical: &TypicalChange) -> (f32, f32, f32, f32) {
    let scale = (typical.max.max(change) * 1.15).clamp(0.01, 1.);
    let at = |v: f64| (v / scale).clamp(0., 1.) as f32;
    (
        at(typical.min),
        at(typical.max),
        at(typical.median),
        at(change),
    )
}

fn change_verdict(change: f64, typical: &TypicalChange) -> String {
    if typical.windows == 0 {
        return "There isn't enough earlier history yet to say what a usual change looks like."
            .into();
    }
    let compared = if change > typical.max {
        "more than any earlier"
    } else if change < typical.min {
        "less than any earlier"
    } else if change >= typical.median {
        "more than usual for a"
    } else {
        "less than usual for a"
    };
    format!(
        "That is {compared} pair of back-to-back periods the same length: {} of {} changed more.",
        typical.exceeded_by, typical.windows
    )
}

fn change_card(change: f64, typical: &TypicalChange, months: u32, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (min, max, median, this) = gauge_positions(change, typical);
    let muted = theme.muted_foreground;
    let span = match months {
        1 => "month".to_owned(),
        n => format!("{n} months"),
    };
    let track = div()
        .relative()
        .h(px(20.))
        .child(
            div()
                .absolute()
                .top(px(7.))
                .left_0()
                .right_0()
                .h(px(6.))
                .rounded_full()
                .bg(theme.muted),
        )
        .when(typical.windows > 0, |this| {
            this.child(
                div()
                    .absolute()
                    .top(px(7.))
                    .h(px(6.))
                    .left(relative(min))
                    .w(relative((max - min).max(0.005)))
                    .rounded_full()
                    .bg(theme.primary.opacity(0.3)),
            )
            .child(
                div()
                    .absolute()
                    .top(px(3.))
                    .h(px(14.))
                    .w(px(2.))
                    .left(relative(median))
                    .bg(muted),
            )
        })
        .child(
            div()
                .absolute()
                .top_0()
                .h(px(20.))
                .w(px(4.))
                .rounded_sm()
                .left(relative(this))
                .bg(theme.primary),
        );
    let legend = h_flex()
        .gap_4()
        .text_xs()
        .text_color(muted)
        .child(format!("This change: {}", widgets::percent(change)))
        .when(typical.windows > 0, |this| {
            this.child(format!(
                "Usual range: {}–{}",
                widgets::percent(typical.min),
                widgets::percent(typical.max)
            ))
            .child(format!("Median: {}", widgets::percent(typical.median)))
        });
    widgets::card("How unusual was the change?", cx)
        .child(div().text_sm().text_color(muted).child(format!(
            "The share of listening that would have to move between artists to turn the \
             {span} before into the {span} after. {}",
            change_verdict(change, typical)
        )))
        .child(track)
        .child(legend)
}

fn top_list(
    id: &'static str,
    title: String,
    detail: String,
    artists: &[ArtistShare],
    cx: &App,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    widgets::card(title, cx)
        .flex_1()
        .min_w_0()
        .child(div().text_xs().text_color(muted).child(detail))
        .children(artists.iter().enumerate().map(|(i, a)| {
            widgets::RankedRow::new(
                ElementId::Name(format!("{id}-{i}").into()),
                a.artist.clone(),
            )
            .rank(i + 1)
            .value(widgets::percent(a.share))
            .on_click_action(OpenArtist::by_name(&a.artist))
        }))
        .when(artists.is_empty(), |this| {
            this.child(div().text_sm().text_color(muted).child("No plays"))
        })
}

fn change_list(
    id: &'static str,
    title: &'static str,
    changes: &[ShareChange],
    before: &str,
    after: &str,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let before_colour = theme.muted_foreground.opacity(0.5);
    let scale = changes
        .iter()
        .flat_map(|c| [c.before, c.after])
        .fold(0., f64::max);
    let width = |v: f64| if scale > 0. { (v / scale) as f32 } else { 0. };
    let bar = |share: f32, colour: Hsla| {
        div()
            .w_full()
            .h(px(5.))
            .rounded_full()
            .bg(theme.muted)
            .child(div().h_full().rounded_full().w(relative(share)).bg(colour))
    };
    let key = |colour: Hsla, label: &str| {
        h_flex()
            .gap_1()
            .child(div().size(px(8.)).rounded_sm().bg(colour))
            .child(label.to_owned())
    };
    widgets::card(title, cx)
        .flex_1()
        .min_w_0()
        .child(
            h_flex()
                .gap_3()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(key(before_colour, before))
                .child(key(theme.primary, after)),
        )
        .children(changes.iter().enumerate().map(|(i, c)| {
            h_flex()
                .gap_3()
                .text_sm()
                .child(div().flex_1().min_w_0().truncate().child(widgets::link(
                    ElementId::Name(format!("{id}-{i}").into()),
                    c.artist.clone(),
                    OpenArtist::by_name(&c.artist),
                    cx,
                )))
                .child(
                    v_flex()
                        .w(px(120.))
                        .flex_shrink_0()
                        .gap(px(2.))
                        .child(bar(width(c.before), before_colour))
                        .child(bar(width(c.after), theme.primary)),
                )
                .child(
                    div()
                        .w(px(100.))
                        .flex_shrink_0()
                        .text_right()
                        .text_xs()
                        .child(format!(
                            "{} → {}",
                            widgets::percent(c.before),
                            widgets::percent(c.after)
                        )),
                )
        }))
        .when(changes.is_empty(), |this| {
            this.child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("No artist moved much"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(json: &str) -> Eras {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn two_eras_compare_and_one_explains_itself() {
        let two = fixture(include_str!("../../tests/fixtures/stats-eras.json"));
        assert_eq!(comparison(&two), Comparison::Ready);
        assert_eq!(two.top.latest.len(), 2);
        assert_eq!(two.risers[0].after, 0.104);
        assert_eq!(
            subtitle(&two).unwrap().as_ref(),
            "City A → City B, 3 months each side of the change"
        );
        let one = fixture(include_str!("../../tests/fixtures/stats-eras-single.json"));
        assert_eq!(comparison(&one), Comparison::SingleEra);
        assert_eq!(subtitle(&one), None);
    }

    #[test]
    fn a_new_era_waits_for_its_first_full_month() {
        let mut data = fixture(include_str!("../../tests/fixtures/stats-eras.json"));
        data.ready = false;
        data.summary = None;
        assert_eq!(comparison(&data), Comparison::TooEarly);
        assert_eq!(
            too_early_detail(&data),
            "City B began in Jun 2026. The comparison appears once its first full month has ended."
        );
    }

    #[test]
    fn the_gauge_fits_every_marker_on_its_track() {
        let typical = TypicalChange {
            median: 0.38,
            min: 0.21,
            max: 0.61,
            windows: 27,
            exceeded_by: 9,
        };
        let (min, max, median, this) = gauge_positions(0.433, &typical);
        assert!(min < median && median < this && this < max && max < 1.);
        let (.., beyond) = gauge_positions(0.9, &typical);
        assert!(beyond < 1.);
        assert!(change_verdict(0.433, &typical).starts_with("That is more than usual for a pair"));
        assert!(change_verdict(0.9, &typical).contains("more than any earlier pair"));
    }
}
