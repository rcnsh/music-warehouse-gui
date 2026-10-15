//! The Top Over Time page (`/api/stats/top-over-time`): who led each year or
//! quarter, and when the all-time leaders peaked. Credits the primary artist
//! only, so a period's shares add up.

use gpui::AppContext as _;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Icon, h_flex,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit_assets::IconName;

use crate::actions::OpenArtist;
use crate::api::ApiClient;
use crate::models::{Leader, Period, TopOverTime};
use crate::state::top_over_time::TopOverTimeStore;
use crate::views::albums::as_of_label;
use crate::views::widgets;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Grain {
    Years,
    Quarters,
}

pub struct TopOverTimeView {
    store: Entity<TopOverTimeStore>,
    focus: FocusHandle,
    activated: bool,
    grain: Grain,
    _subscriptions: Vec<Subscription>,
}

impl TopOverTimeView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| TopOverTimeStore::new(client, cx));
        let subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            focus: cx.focus_handle(),
            activated: false,
            grain: Grain::Years,
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

impl Focusable for TopOverTimeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// `2026-Q3` → `Q3 2026`; a year stays as it is.
fn period_label(period: &str) -> String {
    match period.split_once('-') {
        Some((year, quarter)) => format!("{quarter} {year}"),
        None => period.to_owned(),
    }
}

/// The quarter a leader's share was highest in, and that share. Ties go to
/// the earlier quarter, matching the Worker's ordering of leaders by peak.
fn peak<'a>(leader: &Leader, quarters: &'a [Period]) -> Option<(&'a str, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for (i, &share) in leader.share.iter().enumerate() {
        if best.is_none_or(|(_, top)| share > top) {
            best = Some((i, share));
        }
    }
    best.and_then(|(i, share)| quarters.get(i).map(|q| (q.period.as_str(), share)))
}

fn leaders_section(data: &TopOverTime, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let span = match (data.quarters.first(), data.quarters.last()) {
        (Some(first), Some(last)) => Some(
            format!(
                "Share of each quarter's plays, {} to {}",
                period_label(&first.period),
                period_label(&last.period)
            )
            .into(),
        ),
        _ => None,
    };
    v_flex()
        .gap_2()
        .child(widgets::section_heading("Leaders", span, cx))
        .child(
            v_flex()
                .gap_0p5()
                .children(data.leaders.iter().enumerate().map(|(i, leader)| {
                    let shares: Vec<f64> = leader.share.clone();
                    let peaked = peak(leader, &data.quarters).map(|(q, share)| {
                        format!("peaked {} at {}", period_label(q), widgets::percent(share))
                    });
                    h_flex()
                        .id(("leader", i))
                        .gap_3()
                        .px_1()
                        .py_0p5()
                        .rounded_md()
                        .items_center()
                        .text_sm()
                        .cursor_pointer()
                        .hover({
                            let hover = cx.theme().muted.opacity(0.5);
                            move |style| style.bg(hover)
                        })
                        .on_click({
                            let action = OpenArtist::by_name(&leader.name);
                            move |_, window, cx| {
                                window.dispatch_action(Box::new(action.clone()), cx)
                            }
                        })
                        .child(
                            v_flex()
                                .w(px(200.))
                                .flex_shrink_0()
                                .child(div().truncate().child(leader.name.clone()))
                                .when_some(peaked, |this, peaked| {
                                    this.child(
                                        div().truncate().text_xs().text_color(muted).child(peaked),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(widgets::sparkline(&shares, px(24.), cx)),
                        )
                        .child(
                            div()
                                .min_w(px(84.))
                                .flex_shrink_0()
                                .text_right()
                                .child(format!("{} plays", widgets::count(leader.plays.into()))),
                        )
                })),
        )
        .into_any_element()
}

fn period_section(ix: usize, period: &Period, cx: &App) -> AnyElement {
    let mut detail = format!("{} plays", widgets::count(period.plays.into()));
    if period.partial {
        detail.push_str(" · so far");
    }
    let top_track = period.tracks.first().map(|t| t.plays).unwrap_or(0).max(1) as f32;
    let artists =
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_0p5()
            .children(period.artists.iter().enumerate().map(|(i, artist)| {
                widgets::RankedRow::new(
                    SharedString::from(format!("p{ix}-artist-{i}")),
                    artist.name.clone(),
                )
                .rank(i + 1)
                .subtitle(format!("{} of plays", widgets::percent(artist.share)))
                .value(widgets::count(artist.plays.into()))
                .share(artist.share as f32)
                .on_click_action(OpenArtist::by_name(&artist.name))
            }));
    // The Worker sends no track id here, so a song opens its artist.
    let tracks =
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_0p5()
            .children(period.tracks.iter().enumerate().map(|(i, track)| {
                widgets::RankedRow::new(
                    SharedString::from(format!("p{ix}-track-{i}")),
                    track.name.clone(),
                )
                .rank(i + 1)
                .art(widgets::album_art(track.image.as_deref(), px(32.), cx))
                .subtitle(track.artist.clone())
                .value(widgets::count(track.plays.into()))
                .share(track.plays as f32 / top_track)
                .on_click_action(OpenArtist::by_name(&track.artist))
            }));
    v_flex()
        .gap_2()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .child(widgets::section_heading(
            period_label(&period.period),
            Some(detail.into()),
            cx,
        ))
        .child(
            h_flex()
                .gap_4()
                .items_start()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(column_label("Artists", cx))
                        .child(artists),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(column_label("Songs", cx))
                        .child(tracks),
                ),
        )
        .into_any_element()
}

fn column_label(label: &'static str, cx: &App) -> impl IntoElement {
    div()
        .px_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(label)
}

fn content(data: &TopOverTime, grain: Grain, cx: &App) -> AnyElement {
    let periods = match grain {
        Grain::Years => &data.years,
        Grain::Quarters => &data.quarters,
    };
    if periods.is_empty() {
        return widgets::state_panel(
            Some(Icon::new(IconName::Inbox)),
            "No listening yet",
            Some("Top artists appear here once the history has plays.".into()),
            None,
            cx,
        )
        .into_any_element();
    }
    v_flex()
        .gap_4()
        .when(!data.leaders.is_empty(), |this| {
            this.child(leaders_section(data, cx))
        })
        // Newest first: the period the user is living in is the one they look for.
        .children(
            periods
                .iter()
                .enumerate()
                .rev()
                .map(|(i, period)| period_section(i, period, cx)),
        )
        .into_any_element()
}

impl Render for TopOverTimeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let on_grain = cx.listener(|this, ix: &usize, _, cx| {
            this.grain = if *ix == 0 {
                Grain::Years
            } else {
                Grain::Quarters
            };
            cx.notify();
        });
        let grain = self.grain;
        let store = self.store.read(cx);
        let data = store.top.data.as_ref();
        let subtitle = data.map(|d| {
            let mut text = String::from("Primary artist only");
            if let Some(date) = as_of_label(&d.as_of) {
                text.push_str(&format!(" · plays before {date}"));
            }
            SharedString::from(text)
        });
        let toggle = TabBar::new("top-over-time-grain")
            .segmented()
            .selected_index(match grain {
                Grain::Years => 0,
                Grain::Quarters => 1,
            })
            .child(Tab::new().label("Years"))
            .child(Tab::new().label("Quarters"))
            .on_click(on_grain)
            .into_any_element();
        let body = widgets::remote_view(
            "top-over-time-retry",
            &store.top,
            "Loading top artists over time…",
            retry,
            cx,
            |data| content(data, grain, cx),
        );
        let filled = data.is_some();
        widgets::page("top-over-time")
            .track_focus(&self.focus)
            .child(widgets::page_header(
                "Top Over Time",
                subtitle,
                Some(toggle),
                cx,
            ))
            .child(div().when(!filled, |this| this.h(px(320.))).child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> TopOverTime {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/stats-top-over-time.json"
        ))
        .unwrap()
    }

    #[test]
    fn periods_read_naturally() {
        assert_eq!(period_label("2026-Q3"), "Q3 2026");
        assert_eq!(period_label("2026"), "2026");
    }

    #[test]
    fn a_leader_peaks_in_its_highest_quarter() {
        let data = fixture();
        assert!(data.quarters.last().unwrap().partial);
        let twenty_one_pilots = &data.leaders[0];
        assert_eq!(
            peak(twenty_one_pilots, &data.quarters),
            Some(("2026-Q2", 0.11))
        );
        let kendrick = &data.leaders[1];
        assert_eq!(peak(kendrick, &data.quarters), Some(("2026-Q4", 0.14)));
    }

    #[test]
    fn ties_and_gaps_are_handled() {
        let data = fixture();
        let tied = Leader {
            share: vec![0.2, 0.2, 0.1],
            ..Leader::default()
        };
        assert_eq!(peak(&tied, &data.quarters), Some(("2026-Q2", 0.2)));
        let empty = Leader::default();
        assert_eq!(peak(&empty, &data.quarters), None);
    }
}
