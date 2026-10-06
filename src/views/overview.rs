use gpui::AppContext as _;
use std::cell::Cell;
use std::rc::Rc;

use chrono::NaiveDate;
use gpui::{
    AnyElement, App, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Render,
    SharedString, Styled, Subscription, Window, canvas, div, prelude::FluentBuilder, px, relative,
};
use gpui_component::{
    ActiveTheme, Icon, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    chart::BarChart,
    h_flex,
    input::{Input, InputEvent, InputState},
    plot::{
        AxisLabelPlacement,
        scale::{Scale, ScaleBand},
    },
    scroll::ScrollableElement,
    table::{Column, DataTable, TableDelegate, TableState},
    v_flex,
};
use gpui_kit_assets::IconName;

use crate::api::{ApiClient, ApiError, TopRange};
use crate::dates::{self, RangePreset};
use crate::models::{DayCount, ImageRef, join_names, pick_image};
use crate::state::overview::OverviewStore;
use crate::views::history::group;
use crate::views::widgets;

pub struct ArtistsTable {
    store: Entity<OverviewStore>,
}

impl ArtistsTable {
    fn rows<'a>(&self, cx: &'a App) -> &'a [crate::models::ArtistCount] {
        self.store
            .read(cx)
            .artists
            .data
            .as_ref()
            .map(|r| r.artists.as_slice())
            .unwrap_or_default()
    }
}

impl TableDelegate for ArtistsTable {
    fn columns_count(&self, _: &App) -> usize {
        3
    }

    fn rows_count(&self, cx: &App) -> usize {
        self.rows(cx).len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("rank", "#").width(px(44.)).text_right(),
            1 => Column::new("artist", "Artist").width(px(260.)),
            _ => Column::new("plays", "Plays").width(px(220.)),
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let rows = self.rows(cx);
        let Some(artist) = rows.get(row_ix) else {
            return div().into_any_element();
        };
        match col_ix {
            0 => div()
                .text_color(cx.theme().muted_foreground)
                .child((row_ix + 1).to_string())
                .into_any_element(),
            1 => div()
                .truncate()
                .child(artist.name.clone())
                .into_any_element(),
            _ => {
                // The Worker sorts by plays, so the first row is the scale;
                // nothing is summed or re-ranked here.
                let top = rows.first().map(|a| a.plays).unwrap_or(1).max(1);
                let share = artist.plays as f32 / top as f32;
                h_flex()
                    .gap_2()
                    .w_full()
                    .child(
                        div()
                            .w(px(48.))
                            .text_right()
                            .child(group(artist.plays as usize)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(6.))
                            .rounded_full()
                            .bg(cx.theme().muted)
                            .child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .w(relative(share))
                                    .bg(cx.theme().primary),
                            ),
                    )
                    .into_any_element()
            }
        }
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, cx: &App) -> String {
        let Some(artist) = self.rows(cx).get(row_ix) else {
            return String::new();
        };
        match col_ix {
            0 => (row_ix + 1).to_string(),
            1 => artist.name.clone(),
            _ => artist.plays.to_string(),
        }
    }

    fn loading(&self, cx: &App) -> bool {
        let artists = &self.store.read(cx).artists;
        artists.loading && artists.data.is_none()
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        widgets::state_panel(
            Some(Icon::new(IconName::Inbox)),
            "No artists in this range",
            None,
            None,
            cx,
        )
    }
}

/// Band layout shared by the chart and the click hit-test, so a click opens
/// the bar the chart's hover is highlighting. BarChart has no click callback.
const BAR_PADDING_INNER: f32 = 0.4;
const BAR_PADDING_OUTER: f32 = 0.2;
const BAR_MAX_WIDTH: f32 = 30.;

/// The day a click lands on, given its distance from the chart's left edge.
/// Mirrors BarChart's own hover hit-test, which only lines up because the
/// value labels sit inside the plot instead of in a measured gutter.
pub fn day_index_at(x: f32, width: f32, days: usize) -> Option<usize> {
    if days == 0 || !(0.0..=width).contains(&x) {
        return None;
    }
    let scale = ScaleBand::new(0..days, [0., width])
        .padding_inner(BAR_PADDING_INNER)
        .padding_outer(BAR_PADDING_OUTER)
        .max_band_width(BAR_MAX_WIDTH);
    Some(scale.nearest_index(x))
}

pub enum OverviewEvent {
    OpenDay(NaiveDate),
}

pub struct OverviewView {
    store: Entity<OverviewStore>,
    /// Where the chart was last painted, for turning clicks into days.
    chart_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    artists_table: Entity<TableState<ArtistsTable>>,
    from_input: Entity<InputState>,
    to_input: Entity<InputState>,
    custom_error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl OverviewView {
    pub fn new(client: ApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| OverviewStore::new(client, cx));
        let artists_table = cx.new(|cx| {
            TableState::new(
                ArtistsTable {
                    store: store.clone(),
                },
                window,
                cx,
            )
            .col_movable(false)
            .col_selectable(false)
            .sortable(false)
        });
        let range = store.read(cx).range;
        let from_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("YYYY-MM-DD")
                .default_value(range.query_from())
        });
        let to_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("YYYY-MM-DD")
                .default_value(range.query_to())
        });
        let subscriptions = vec![
            cx.observe(&store, |this, _, cx| {
                this.artists_table.update(cx, |_, cx| cx.notify());
                cx.notify();
            }),
            cx.subscribe_in(&from_input, window, Self::on_custom_input),
            cx.subscribe_in(&to_input, window, Self::on_custom_input),
        ];
        Self {
            store,
            chart_bounds: Rc::default(),
            artists_table,
            from_input,
            to_input,
            custom_error: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.refresh(cx));
    }

    fn on_chart_click(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.chart_bounds.get() else {
            return;
        };
        let Some(days) = self.store.read(cx).daily.data.as_ref().map(|r| &r.days) else {
            return;
        };
        let x = (event.position.x - bounds.origin.x).as_f32();
        let day = day_index_at(x, bounds.size.width.as_f32(), days.len())
            .and_then(|ix| days.get(ix))
            .and_then(|d| NaiveDate::parse_from_str(&d.day, "%Y-%m-%d").ok());
        if let Some(day) = day {
            cx.emit(OverviewEvent::OpenDay(day));
        }
    }

    fn on_custom_input(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::PressEnter { .. } = event {
            self.apply_custom(cx);
        }
    }

    fn apply_custom(&mut self, cx: &mut Context<Self>) {
        let from = self.from_input.read(cx).value();
        let to = self.to_input.read(cx).value();
        match dates::parse_custom(&from, &to, dates::today_local()) {
            Ok(range) => {
                self.custom_error = None;
                self.store
                    .update(cx, |store, cx| store.set_custom_range(range, cx));
            }
            Err(message) => self.custom_error = Some(message.into()),
        }
        cx.notify();
    }

    fn select_preset(&mut self, preset: RangePreset, window: &mut Window, cx: &mut Context<Self>) {
        self.custom_error = None;
        if preset == RangePreset::Custom {
            // Seed the fields with the range on screen, which is the most
            // likely starting point for an adjustment.
            let range = self.store.read(cx).range;
            self.from_input
                .update(cx, |i, cx| i.set_value(range.query_from(), window, cx));
            self.to_input
                .update(cx, |i, cx| i.set_value(range.query_to(), window, cx));
            self.from_input.update(cx, |i, cx| i.focus(window, cx));
        }
        self.store
            .update(cx, |store, cx| store.select_preset(preset, cx));
    }

    fn render_range_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let preset = store.preset;
        let caption = format!("{} · {}", store.range.describe(), store.tz);
        h_flex()
            .gap_3()
            .flex_wrap()
            .child(h_flex().gap_1().children(RangePreset::ALL.iter().map(|&p| {
                Button::new(SharedString::from(format!("preset-{}", p.label())))
                    .small()
                    .label(p.label())
                    .selected(p == preset)
                    .when(p != preset, |b| b.ghost())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.select_preset(p, window, cx)),
                    )
            })))
            .when(preset == RangePreset::Custom, |this| {
                this.child(
                    h_flex()
                        .gap_1()
                        .child(
                            div()
                                .w(px(120.))
                                .child(Input::new(&self.from_input).small()),
                        )
                        .child("–")
                        .child(div().w(px(120.)).child(Input::new(&self.to_input).small()))
                        .child(
                            Button::new("apply-custom")
                                .small()
                                .primary()
                                .label("Apply")
                                .on_click(cx.listener(|this, _, _, cx| this.apply_custom(cx))),
                        ),
                )
            })
            .when_some(self.custom_error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(caption),
            )
    }

    fn render_chart(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let daily = &store.daily;
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        match (&daily.data, &daily.error) {
            (_, Some(error)) => {
                widgets::error_panel("daily-retry", error, retry, cx).into_any_element()
            }
            (None, None) => widgets::loading_panel("Loading daily plays…", cx).into_any_element(),
            (Some(response), None) if response.days.iter().all(|d| d.plays == 0) => {
                widgets::state_panel(
                    Some(Icon::new(IconName::ChartColumn)),
                    "No plays in this range",
                    Some(
                        "Nothing was stored for these days. Plays under about 30 seconds \
                         may never be recorded by Spotify."
                            .into(),
                    ),
                    None,
                    cx,
                )
                .into_any_element()
            }
            (Some(response), None) => {
                let range_days = response.days.len() as u64;
                let accent = cx.theme().primary;
                let chart = BarChart::new(response.days.clone())
                    .band(move |d: &DayCount| dates::day_label(&d.day, range_days))
                    .value(|d: &DayCount| d.plays as f64)
                    .name("Plays")
                    .fill(move |_, _, _, _| accent)
                    .value_axis(true)
                    .value_tick_count(4)
                    .value_tick_format(|v| format!("{v:.0}"))
                    .band_tick_count(if range_days <= 7 { 7 } else { 6 })
                    .grid_dashed(false)
                    .value_axis_label_placement(AxisLabelPlacement::Inside)
                    .padding_inner(BAR_PADDING_INNER)
                    .padding_outer(BAR_PADDING_OUTER)
                    .max_band_width(px(BAR_MAX_WIDTH))
                    .id("daily-chart");
                let bounds = self.chart_bounds.clone();
                div()
                    .relative()
                    .size_full()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_chart_click))
                    .child(chart)
                    .child(
                        canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_artists(&self, cx: &mut Context<Self>) -> AnyElement {
        let artists = &self.store.read(cx).artists;
        match &artists.error {
            Some(error) => {
                let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
                widgets::error_panel("artists-retry", error, retry, cx).into_any_element()
            }
            None => DataTable::new(&self.artists_table)
                .bordered(false)
                .into_any_element(),
        }
    }

    fn render_spotify_top(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let selected = store.spotify_range;
        let top = &store.spotify_top;
        let muted = cx.theme().muted_foreground;

        let body: AnyElement = match (&top.data, &top.error) {
            (_, Some(error)) => {
                let retry = cx.listener(move |this, _, _, cx| {
                    this.store
                        .update(cx, |s, cx| s.select_spotify_range(selected, cx))
                });
                live_error(error, retry, cx)
            }
            (None, None) => widgets::loading_panel("Asking Spotify…", cx).into_any_element(),
            (Some(response), None) => {
                // Lines are 32px tall at 2x, so 64px renditions stay sharp.
                let artists = response.artists.items.iter().enumerate().map(|(ix, a)| {
                    let art = widgets::artwork(image_url(&a.images), px(32.), true, cx);
                    ranked_line(ix, art, a.name.clone().unwrap_or_default(), None, muted)
                });
                let tracks = response.tracks.items.iter().enumerate().map(|(ix, t)| {
                    let images = t
                        .album
                        .as_ref()
                        .map(|a| a.images.as_slice())
                        .unwrap_or_default();
                    let art = widgets::artwork(image_url(images), px(32.), false, cx);
                    ranked_line(
                        ix,
                        art,
                        t.name.clone().unwrap_or_default(),
                        Some(join_names(&t.artists)),
                        muted,
                    )
                });
                v_flex()
                    .gap_3()
                    .child(section_label("Artists", cx))
                    .children(artists)
                    .child(section_label("Tracks", cx))
                    .children(tracks)
                    .into_any_element()
            }
        };

        widgets::card("Spotify's top lists", cx)
            .size_full()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Live from Spotify, ranked by Spotify. Not from the warehouse."),
            )
            .child(h_flex().gap_1().children(TopRange::ALL.iter().map(|&r| {
                Button::new(SharedString::from(format!("top-{}", r.as_param())))
                    .xsmall()
                    .label(r.label())
                    .selected(r == selected)
                    .when(r != selected, |b| b.ghost())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.store.update(cx, |s, cx| s.select_spotify_range(r, cx))
                    }))
            })))
            .child(div().flex_1().min_h_0().overflow_y_scrollbar().child(body))
    }
}

/// The live route's documented failures get specific wording; anything else
/// falls back to the shared error panel.
fn live_error(
    error: &ApiError,
    retry: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    match error {
        ApiError::NeedsReauth => widgets::state_panel(
            Some(Icon::new(IconName::KeyRound)),
            "Spotify needs re-authorizing",
            Some(
                "Live lists are unavailable until the Worker is re-authorized. The chart and \
                 artist counts come from stored plays and are unaffected."
                    .into(),
            ),
            None,
            cx,
        )
        .into_any_element(),
        _ => widgets::error_panel("spotify-top-retry", error, retry, cx).into_any_element(),
    }
}

fn section_label(text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

fn image_url(images: &[ImageRef]) -> Option<SharedString> {
    pick_image(images, 64).map(|url| SharedString::from(url.to_owned()))
}

fn ranked_line(
    ix: usize,
    art: AnyElement,
    title: String,
    subtitle: Option<String>,
    muted: gpui::Hsla,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .items_center()
        .text_sm()
        .child(
            div()
                .w(px(20.))
                .text_right()
                .text_color(muted)
                .child((ix + 1).to_string()),
        )
        .child(art)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().truncate().child(title))
                .when_some(subtitle, |this, subtitle| {
                    this.child(div().truncate().text_xs().text_color(muted).child(subtitle))
                }),
        )
}

impl EventEmitter<OverviewEvent> for OverviewView {}

impl Focusable for OverviewView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // The artist table is the view's only keyboard-navigable list.
        self.artists_table.read(cx).focus_handle(cx)
    }
}

impl Render for OverviewView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let artists_loading_more = {
            let artists = &self.store.read(cx).artists;
            artists.loading && artists.data.is_some()
        };
        let daily_refreshing = {
            let daily = &self.store.read(cx).daily;
            daily.loading && daily.data.is_some()
        };
        v_flex()
            .size_full()
            .gap_4()
            .p_4()
            .child(self.render_range_bar(cx))
            .child(
                widgets::card(
                    if daily_refreshing {
                        "Plays per day · refreshing…"
                    } else {
                        "Plays per day · click a day to see its plays"
                    },
                    cx,
                )
                .h(px(300.))
                // The last band label is centred on the final bar and
                // would otherwise be clipped by the card edge.
                .child(div().flex_1().min_h_0().pr_6().child(self.render_chart(cx))),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .items_stretch()
                    .child(
                        widgets::card(
                            if artists_loading_more {
                                "Top artists · refreshing…"
                            } else {
                                "Top artists"
                            },
                            cx,
                        )
                        .flex_1()
                        .min_w_0()
                        .child(div().flex_1().min_h_0().child(self.render_artists(cx))),
                    )
                    .child(
                        div()
                            .w(px(340.))
                            .h_full()
                            .child(self.render_spotify_top(cx)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_map_to_the_nearest_day() {
        // 30 days over 1000px: outer padding is 0.2 of a 33.3px slot each side.
        assert_eq!(day_index_at(0., 1000., 30), Some(0));
        assert_eq!(day_index_at(15., 1000., 30), Some(0));
        assert_eq!(day_index_at(500., 1000., 30), Some(15));
        assert_eq!(day_index_at(1000., 1000., 30), Some(29));
    }

    #[test]
    fn clicks_outside_the_chart_or_without_data_are_ignored() {
        assert_eq!(day_index_at(-1., 1000., 30), None);
        assert_eq!(day_index_at(1001., 1000., 30), None);
        assert_eq!(day_index_at(10., 1000., 0), None);
        assert_eq!(day_index_at(999., 1000., 1), Some(0));
    }
}
