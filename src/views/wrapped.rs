//! The Wrapped page (`/api/wrapped/ranges`, `/api/wrapped`): a year in
//! review for one of the ranges the Worker offers a read token.

use gpui::AppContext as _;
use gpui::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

use crate::actions::OpenArtist;
use crate::api::ApiClient;
use crate::models::{ListeningAge, Wrapped, WrappedRanges};
use crate::state::wrapped::WrappedStore;
use crate::views::widgets;

pub struct WrappedView {
    store: Entity<WrappedStore>,
    focus: FocusHandle,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl WrappedView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| WrappedStore::new(client, cx));
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

    fn picker(&self, ranges: &WrappedRanges, cx: &Context<Self>) -> AnyElement {
        let selected = self.store.read(cx).selected.as_ref();
        h_flex()
            .gap_1()
            .flex_wrap()
            .justify_end()
            .children(ranges.ranges.iter().map(|preset| {
                let is_selected = selected == Some(preset);
                let choice = preset.clone();
                Button::new(SharedString::from(format!("wrapped-{}", preset.from)))
                    .small()
                    .label(preset.label.clone())
                    .selected(is_selected)
                    .when(!is_selected, |b| b.ghost())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let choice = choice.clone();
                        this.store.update(cx, |s, cx| s.select(choice, cx));
                    }))
            }))
            .into_any_element()
    }
}

impl Focusable for WrappedView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for WrappedView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let retry_summary = cx.listener(|this, _, _, cx| {
            this.store.update(cx, |s, cx| {
                if let Some(preset) = s.selected.clone() {
                    s.select(preset, cx);
                }
            })
        });
        let store = self.store.read(cx);
        let controls = store
            .ranges
            .data
            .as_ref()
            .filter(|r| !r.ranges.is_empty())
            .map(|r| self.picker(r, cx));
        let subtitle = store.wrapped.data.as_ref().map(|w| {
            SharedString::from(format!(
                "{} – {}, {} days. Minutes are estimates.",
                widgets::day_label(&w.range.from),
                widgets::day_label(&w.range.to),
                w.range.days
            ))
        });
        let loading_label = match &store.selected {
            Some(preset) if !preset.computed => {
                "Summarising… a range not computed before can take a few seconds"
            }
            _ => "Loading Wrapped…",
        };
        let body = widgets::page_body(
            "wrapped-ranges-retry",
            &store.ranges,
            "Loading Wrapped ranges…",
            retry,
            cx,
            |ranges| {
                if ranges.ranges.is_empty() {
                    return widgets::page_state(
                        None,
                        "No plays to summarise yet",
                        Some("Wrapped covers each year with plays, and this year so far.".into()),
                        cx,
                    );
                }
                widgets::page_body(
                    "wrapped-retry",
                    &store.wrapped,
                    loading_label,
                    retry_summary,
                    cx,
                    |wrapped| summary(wrapped, cx),
                )
            },
        );
        widgets::page("wrapped")
            .track_focus(&self.focus)
            .child(widgets::page_header("Wrapped", subtitle, controls, cx))
            .child(body)
    }
}

fn summary(w: &Wrapped, cx: &App) -> AnyElement {
    v_flex()
        .gap_6()
        .child(totals(w, cx))
        .child(highlights(w, cx))
        .child(
            h_flex()
                .gap_3()
                .items_start()
                .child(top_artists(w, cx))
                .child(top_songs(w, cx)),
        )
        .child(
            h_flex()
                .gap_3()
                .items_start()
                .child(top_albums(w, cx))
                .child(discoveries(w, cx)),
        )
        .child(when_you_listened(w, cx))
        .child(months(w, cx))
        .when_some(w.listening_age.as_ref(), |this, age| {
            this.child(decades(age, cx))
        })
        .into_any_element()
}

fn totals(w: &Wrapped, cx: &App) -> impl IntoElement {
    let t = &w.totals;
    let d = &w.discoveries;
    h_flex()
        .gap_3()
        .items_stretch()
        .child(widgets::stat_tile(
            "Listened",
            widgets::minutes(t.minutes.into()),
            Some(format!("{} minutes", widgets::count(t.minutes.into())).into()),
            cx,
        ))
        .child(widgets::stat_tile(
            "Plays",
            widgets::count(t.plays.into()),
            Some(format!("plus {} skipped", widgets::count(t.skips.into())).into()),
            cx,
        ))
        .child(widgets::stat_tile(
            "Days listening",
            widgets::count(t.days_listened.into()),
            Some(format!("of {}", widgets::count(w.range.days.into())).into()),
            cx,
        ))
        .child(widgets::stat_tile(
            "Artists",
            widgets::count(t.artists.into()),
            Some(format!("{} new to you", widgets::count(d.artists.into())).into()),
            cx,
        ))
        .child(widgets::stat_tile(
            "Songs",
            widgets::count(t.songs.into()),
            Some(format!("{} new to you", widgets::count(d.songs.into())).into()),
            cx,
        ))
}

fn highlights(w: &Wrapped, cx: &App) -> impl IntoElement {
    let d = &w.discoveries;
    let (day, day_detail) = match &w.biggest_day {
        Some(day) => (
            widgets::day_label(&day.date),
            Some(widgets::minutes(day.minutes.into()).into()),
        ),
        None => ("None".to_owned(), None),
    };
    let (streak, streak_detail) = match &w.streak {
        Some(s) => (
            widgets::plural(s.days.into(), "day"),
            Some(
                format!(
                    "{} – {}",
                    widgets::day_label(&s.from),
                    widgets::day_label(&s.to)
                )
                .into(),
            ),
        ),
        None => ("None".to_owned(), None),
    };
    let (age, age_detail) = match &w.listening_age {
        Some(age) => (
            age.age.to_string(),
            format!("Peak release years {}–{}", age.window.0, age.window.1),
        ),
        None => (
            "Unknown".to_owned(),
            "Too few release years known".to_owned(),
        ),
    };
    h_flex()
        .gap_3()
        .items_stretch()
        .child(widgets::stat_tile("Biggest day", day, day_detail, cx))
        .child(widgets::stat_tile(
            "Longest streak",
            streak,
            streak_detail,
            cx,
        ))
        .child(widgets::stat_tile(
            "Time on new artists",
            widgets::percent(d.share),
            Some(format!("{} artists first heard", widgets::count(d.artists.into())).into()),
            cx,
        ))
        .child(widgets::stat_tile(
            "Listening age",
            age,
            Some(age_detail.into()),
            cx,
        ))
}

fn share_of(value: u32, max: u32) -> f32 {
    if max == 0 {
        0.
    } else {
        value as f32 / max as f32
    }
}

fn top_artists(w: &Wrapped, cx: &App) -> impl IntoElement {
    let max = w.top_artists.first().map(|a| a.minutes).unwrap_or(0);
    widgets::card("Top artists", cx)
        .flex_1()
        .min_w_0()
        .children(w.top_artists.iter().enumerate().map(|(i, a)| {
            let detail = format!(
                "{} on {}",
                widgets::plural(a.plays.into(), "play"),
                widgets::plural(a.days.into(), "day")
            );
            widgets::RankedRow::new(
                ElementId::Name(format!("wrapped-artist-{i}").into()),
                a.name.clone(),
            )
            .rank(i + 1)
            .art(widgets::album_art(a.image.as_deref(), px(32.), cx))
            .subtitle(detail)
            .value(widgets::minutes(a.minutes.into()))
            .share(share_of(a.minutes, max))
            .on_click_action(OpenArtist::by_name(&a.name))
        }))
        .when(w.top_artists.is_empty(), |this| {
            this.child(widgets::note("No artists", cx))
        })
}

/// Songs carry no track id, so a row opens the song's artist.
fn top_songs(w: &Wrapped, cx: &App) -> impl IntoElement {
    let max = w.top_songs.first().map(|s| s.plays).unwrap_or(0);
    widgets::card("Top songs", cx)
        .flex_1()
        .min_w_0()
        .children(w.top_songs.iter().enumerate().map(|(i, s)| {
            widgets::RankedRow::new(
                ElementId::Name(format!("wrapped-song-{i}").into()),
                s.name.clone(),
            )
            .rank(i + 1)
            .art(widgets::album_art(s.image.as_deref(), px(32.), cx))
            .subtitle(s.artist.clone())
            .value(widgets::plural(s.plays.into(), "play"))
            .share(share_of(s.plays, max))
            .on_click_action(OpenArtist::by_name(&s.artist))
        }))
        .when(w.top_songs.is_empty(), |this| {
            this.child(widgets::note("No songs", cx))
        })
}

fn top_albums(w: &Wrapped, cx: &App) -> impl IntoElement {
    let max = w.top_albums.first().map(|a| a.minutes).unwrap_or(0);
    widgets::card("Top albums", cx)
        .flex_1()
        .min_w_0()
        .children(w.top_albums.iter().enumerate().map(|(i, a)| {
            widgets::RankedRow::new(
                ElementId::Name(format!("wrapped-album-{i}").into()),
                a.name.clone(),
            )
            .rank(i + 1)
            .art(widgets::album_art(a.image.as_deref(), px(32.), cx))
            .subtitle(format!(
                "{}, {}",
                a.artist,
                widgets::plural(a.songs.into(), "song")
            ))
            .value(widgets::minutes(a.minutes.into()))
            .share(share_of(a.minutes, max))
            .on_click_action(OpenArtist::by_name(&a.artist))
        }))
        .when(w.top_albums.is_empty(), |this| {
            this.child(widgets::note(
                "No album had three different songs played",
                cx,
            ))
        })
}

fn discoveries(w: &Wrapped, cx: &App) -> impl IntoElement {
    let top = &w.discoveries.top;
    let max = top.first().map(|d| d.minutes).unwrap_or(0);
    widgets::card("Discoveries", cx)
        .flex_1()
        .min_w_0()
        .child(widgets::note("Artists first played in this range", cx))
        .children(top.iter().enumerate().map(|(i, d)| {
            widgets::RankedRow::new(
                ElementId::Name(format!("wrapped-discovery-{i}").into()),
                d.name.clone(),
            )
            .rank(i + 1)
            .art(widgets::album_art(d.image.as_deref(), px(32.), cx))
            .value(widgets::minutes(d.minutes.into()))
            .share(share_of(d.minutes, max))
            .on_click_action(OpenArtist::by_name(&d.name))
        }))
        .when(top.is_empty(), |this| {
            this.child(widgets::note("No new artists", cx))
        })
}

fn hour_points(hours: &[u32]) -> Vec<(SharedString, f64)> {
    widgets::hour_points(hours.iter().map(|&m| m.into()))
}

fn weekday_points(weekdays: &[u32]) -> Vec<(SharedString, f64)> {
    weekdays
        .iter()
        .zip(widgets::WEEKDAYS)
        .map(|(&m, day)| (day.into(), m.into()))
        .collect()
}

fn when_you_listened(w: &Wrapped, cx: &App) -> impl IntoElement {
    h_flex()
        .gap_3()
        .items_start()
        .child(
            widgets::card("Minutes by hour of day", cx)
                .flex_1()
                .min_w_0()
                .child(div().h(px(200.)).child(widgets::bar_chart(
                    "wrapped-hours",
                    hour_points(&w.hours),
                    "Minutes",
                    cx,
                ))),
        )
        .child(
            widgets::card("Minutes by weekday", cx)
                .w(px(300.))
                .flex_shrink_0()
                .child(div().h(px(200.)).child(widgets::bar_chart(
                    "wrapped-weekdays",
                    weekday_points(&w.weekdays),
                    "Minutes",
                    cx,
                ))),
        )
}

fn months(w: &Wrapped, cx: &App) -> impl IntoElement {
    let max = w.months.iter().map(|m| m.minutes).max().unwrap_or(0);
    widgets::card("Month by month", cx).children(w.months.iter().enumerate().map(|(i, m)| {
        let detail = match (&m.top_artist, &m.top_song) {
            (Some(artist), Some(song)) => format!("{artist} · {song}"),
            (Some(artist), None) => artist.clone(),
            (None, Some(song)) => song.clone(),
            (None, None) => "No standout".into(),
        };
        let row = widgets::RankedRow::new(
            ElementId::Name(format!("wrapped-month-{i}").into()),
            widgets::month_label(&m.month),
        )
        .art(widgets::album_art(m.image.as_deref(), px(32.), cx))
        .subtitle(detail)
        .value(widgets::minutes(m.minutes.into()))
        .share(share_of(m.minutes, max));
        match &m.top_artist {
            Some(artist) => row.on_click_action(OpenArtist::by_name(artist)),
            None => row,
        }
    }))
}

fn decades(age: &ListeningAge, cx: &App) -> impl IntoElement {
    let max = age.decades.iter().map(|d| d.share).fold(0., f64::max);
    widgets::card("Music by decade of release", cx)
        .child(widgets::note(
            "Share of minutes with a known release year, by when the music came out",
            cx,
        ))
        .children(age.decades.iter().enumerate().map(|(i, d)| {
            widgets::RankedRow::new(
                ElementId::Name(format!("wrapped-decade-{i}").into()),
                format!("{}s", d.decade),
            )
            .value(widgets::percent(d.share))
            .value_width(px(48.))
            .share(if max > 0. { (d.share / max) as f32 } else { 0. })
        }))
        .child(widgets::note(
            format!(
                "Release years known for {} of minutes",
                widgets::percent(age.coverage)
            ),
            cx,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_decode() {
        let w: Wrapped = serde_json::from_str(include_str!("../../tests/fixtures/wrapped.json"))
            .expect("wrapped fixture");
        assert_eq!(w.hours.len(), 24);
        assert_eq!(w.weekdays.len(), 7);
        assert_eq!(w.listening_age.as_ref().unwrap().window, (2015, 2019));
        let r: WrappedRanges =
            serde_json::from_str(include_str!("../../tests/fixtures/wrapped-ranges.json"))
                .expect("ranges fixture");
        assert_eq!(r.ranges.len(), 3);
    }

    #[test]
    fn charts_get_distinct_labels() {
        let hours = hour_points(&[1; 24]);
        assert_eq!(hours[9].0.as_ref(), "09");
        let days = weekday_points(&[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(days[6], ("Sun".into(), 7.));
    }

    #[test]
    fn figures_read_naturally() {
        assert_eq!(share_of(5, 0), 0.);
        assert_eq!(share_of(5, 10), 0.5);
    }
}
