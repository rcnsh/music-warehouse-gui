//! The Top Albums page (`/api/stats/albums`): the most-played albums all
//! time and per year. Every year arrives in the one response, so the year
//! tabs switch without a request.

use gpui::AppContext as _;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div, prelude::FluentBuilder,
    px,
};
use gpui_component::{
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit_assets::IconName;

use crate::actions::OpenArtist;
use crate::api::ApiClient;
use crate::models::{TopAlbum, TopAlbums};
use crate::state::albums::AlbumsStore;
use crate::views::widgets;

pub struct AlbumsView {
    store: Entity<AlbumsStore>,
    focus: FocusHandle,
    activated: bool,
    /// Which tab is showing: 0 is all time, then the years newest first.
    tab: usize,
    _subscriptions: Vec<Subscription>,
}

impl AlbumsView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| AlbumsStore::new(client, cx));
        let subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            focus: cx.focus_handle(),
            activated: false,
            tab: 0,
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

impl Focusable for AlbumsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// One tab's worth of albums: what to call it and whether its year is
/// still running.
struct Selection<'a> {
    label: SharedString,
    partial: bool,
    albums: &'a [TopAlbum],
}

/// Tab labels in display order: all time, then each year newest first.
fn tab_labels(data: &TopAlbums) -> Vec<SharedString> {
    std::iter::once(SharedString::from("All time"))
        .chain(data.years.iter().rev().map(|y| y.year.clone().into()))
        .collect()
}

/// The albums under tab `tab`, falling back to all time when a refresh
/// returned fewer years than the tab assumed.
fn selection(data: &TopAlbums, tab: usize) -> Selection<'_> {
    match tab
        .checked_sub(1)
        .and_then(|i| data.years.iter().rev().nth(i))
    {
        Some(year) => Selection {
            label: year.year.clone().into(),
            partial: year.partial,
            albums: &year.albums,
        },
        None => Selection {
            label: "All time".into(),
            partial: false,
            albums: &data.albums,
        },
    }
}

fn album_rows(albums: &[TopAlbum], cx: &App) -> AnyElement {
    let top = albums.first().map(|a| a.plays).unwrap_or(0).max(1) as f32;
    v_flex()
        .gap_0p5()
        .children(albums.iter().enumerate().map(|(i, album)| {
            let subtitle = format!(
                "{} · {} songs · {}",
                album.artist,
                album.songs,
                widgets::month_span(&album.from, Some(&album.to))
            );
            widgets::RankedRow::new(("album", i), album.name.clone())
                .rank(i + 1)
                .art(widgets::album_art(album.image.as_deref(), px(32.), cx))
                .subtitle(subtitle)
                .value(format!("{} plays", widgets::count(album.plays.into())))
                .share(album.plays as f32 / top)
                .on_click_action(OpenArtist::by_name(&album.artist))
        }))
        .into_any_element()
}

impl Render for AlbumsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let on_tab = cx.listener(|this, ix: &usize, _, cx| {
            this.tab = *ix;
            cx.notify();
        });
        let store = self.store.read(cx);
        let data = store.albums.data.as_ref();
        let subtitle = data.map(|d| {
            let mut text = format!(
                "Albums with at least {} different songs played",
                d.min_songs
            );
            if let Some(date) = widgets::local_day(&d.as_of) {
                text.push_str(&format!(" · plays before {date}"));
            }
            SharedString::from(text)
        });
        // A refresh can return fewer years than the tab assumed.
        let tab = match data {
            Some(d) if self.tab <= d.years.len() => self.tab,
            _ => 0,
        };
        let tabs = data.filter(|d| !d.years.is_empty()).map(|d| {
            TabBar::new("albums-years")
                .segmented()
                .selected_index(tab)
                .children(tab_labels(d).into_iter().map(|l| Tab::new().label(l)))
                .on_click(on_tab)
                .into_any_element()
        });
        let body = widgets::page_body(
            "albums-retry",
            &store.albums,
            "Loading top albums…",
            retry,
            cx,
            |data| {
                let selected = selection(data, tab);
                if selected.albums.is_empty() {
                    return widgets::page_state(
                        Some(IconName::Inbox),
                        "No albums yet",
                        Some(
                            format!(
                                "No album in {} has {} different songs played.",
                                selected.label, data.min_songs
                            )
                            .into(),
                        ),
                        cx,
                    );
                }
                let detail = if selected.partial {
                    format!("{} so far", selected.label)
                } else {
                    selected.label.to_string()
                };
                v_flex()
                    .gap_2()
                    .child(widgets::section_heading(
                        "Most played",
                        Some(detail.into()),
                        cx,
                    ))
                    .child(album_rows(selected.albums, cx))
                    .into_any_element()
            },
        );
        widgets::page("albums")
            .track_focus(&self.focus)
            .child(widgets::page_header("Top Albums", subtitle, None, cx))
            .when_some(tabs, |this, tabs| this.child(div().child(tabs)))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> TopAlbums {
        serde_json::from_str(include_str!("../../tests/fixtures/stats-albums.json")).unwrap()
    }

    #[test]
    fn tabs_run_all_time_then_newest_year_first() {
        let data = fixture();
        let labels = tab_labels(&data);
        assert_eq!(labels[0].as_ref(), "All time");
        assert_eq!(labels[1].as_ref(), "2026");
        assert_eq!(labels[2].as_ref(), "2025");

        let year = selection(&data, 1);
        assert_eq!(year.label.as_ref(), "2026");
        assert!(year.partial);
        assert_eq!(selection(&data, 2).label.as_ref(), "2025");
        assert_eq!(selection(&data, 0).albums.len(), data.albums.len());
    }

    #[test]
    fn a_stale_tab_falls_back_to_all_time() {
        let data = fixture();
        let fallback = selection(&data, 99);
        assert_eq!(fallback.label.as_ref(), "All time");
        assert!(!fallback.partial);
    }
}
