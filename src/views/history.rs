use gpui::AppContext as _;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    table::{Column, DataTable, TableDelegate, TableState},
    v_flex,
};
use gpui_kit_assets::IconName;

use crate::api::ApiClient;
use crate::dates;
use crate::models::Play;
use crate::state::history::{HistoryStore, PAGE_SIZE};
use crate::views::widgets;

/// Case-insensitive match against the three text columns a person would
/// remember a play by.
pub fn matches_filter(play: &Play, needle_lower: &str) -> bool {
    [&play.track_name, &play.artists, &play.album_name]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase().contains(needle_lower))
}

/// Table rows are indices into the store's plays, so filtering never copies
/// rows and the store stays the single owner of fetched data.
pub struct PlaysTable {
    store: Entity<HistoryStore>,
    rows: Vec<usize>,
    filter: String,
}

impl PlaysTable {
    fn rebuild(&mut self, cx: &App) {
        let plays = self.store.read(cx).plays();
        let needle = self.filter.to_lowercase();
        self.rows = if needle.is_empty() {
            (0..plays.len()).collect()
        } else {
            plays
                .iter()
                .enumerate()
                .filter(|(_, play)| matches_filter(play, &needle))
                .map(|(ix, _)| ix)
                .collect()
        };
    }

    fn play<'a>(&self, row_ix: usize, cx: &'a App) -> Option<&'a Play> {
        let ix = *self.rows.get(row_ix)?;
        self.store.read(cx).plays().get(ix)
    }
}

impl TableDelegate for PlaysTable {
    fn columns_count(&self, _: &App) -> usize {
        4
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("time", "Played").width(px(190.)),
            1 => Column::new("track", "Track").width(px(320.)),
            2 => Column::new("artist", "Artist").width(px(240.)),
            _ => Column::new("album", "Album").width(px(280.)),
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(play) = self.play(row_ix, cx) else {
            return div();
        };
        let muted = cx.theme().muted_foreground;
        // Export-era rows lack album names; say so rather than leave a hole
        // that looks like a rendering bug.
        let (text, is_missing): (SharedString, bool) = match col_ix {
            0 => (
                dates::format_played_at(play.played_at_ms, &chrono::Local).into(),
                false,
            ),
            1 => text_or(&play.track_name, "Unknown track"),
            2 => text_or(&play.artists, "Unknown artist"),
            _ => text_or(&play.album_name, "No album info"),
        };
        div()
            .truncate()
            .when(col_ix == 0, |this| this.text_color(muted))
            .when(is_missing, |this| this.text_color(muted).italic())
            .child(text)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, cx: &App) -> String {
        let Some(play) = self.play(row_ix, cx) else {
            return String::new();
        };
        match col_ix {
            0 => dates::format_played_at(play.played_at_ms, &chrono::Local),
            1 => play.track_name.clone().unwrap_or_default(),
            2 => play.artists.clone().unwrap_or_default(),
            _ => play.album_name.clone().unwrap_or_default(),
        }
    }

    fn loading(&self, cx: &App) -> bool {
        let store = self.store.read(cx);
        store.is_loading() && store.plays().is_empty()
    }

    /// Paging stops while filtering: a filter that matches little would
    /// otherwise keep the table "near the bottom" and walk the whole history.
    fn has_more(&self, cx: &App) -> bool {
        let store = self.store.read(cx);
        self.filter.is_empty() && store.has_more() && !store.is_loading() && store.error().is_none()
    }

    fn load_more_threshold(&self) -> usize {
        50
    }

    fn load_more(&mut self, _: &mut Window, cx: &mut Context<TableState<Self>>) {
        self.store.update(cx, |store, cx| store.load_older(cx));
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if self.filter.is_empty() {
            return widgets::state_panel(
                Some(Icon::new(IconName::Inbox)),
                "No plays stored yet",
                Some("The warehouse is empty. Plays appear after the Worker's next poll.".into()),
                None,
                cx,
            )
            .into_any_element();
        }
        widgets::state_panel(
            Some(Icon::new(IconName::Search)),
            format!("No loaded plays match “{}”", self.filter),
            Some("The filter only searches plays already loaded in this window.".into()),
            None,
            cx,
        )
        .into_any_element()
    }
}

fn text_or(value: &Option<String>, fallback: &'static str) -> (SharedString, bool) {
    match value.as_deref().filter(|s| !s.is_empty()) {
        Some(text) => (text.to_owned().into(), false),
        None => (fallback.into(), true),
    }
}

pub struct HistoryView {
    store: Entity<HistoryStore>,
    table: Entity<TableState<PlaysTable>>,
    filter: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl HistoryView {
    pub fn new(client: ApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| HistoryStore::new(client, cx));
        let delegate = PlaysTable {
            store: store.clone(),
            rows: Vec::new(),
            filter: String::new(),
        };
        let table = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .col_movable(false)
                .col_selectable(false)
                .sortable(false)
        });
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter loaded plays by track, artist or album")
        });

        let subscriptions = vec![
            cx.observe(&store, |this, _, cx| this.rebuild_rows(cx)),
            cx.subscribe_in(&filter, window, Self::on_filter_event),
        ];
        Self {
            store,
            table,
            filter,
            _subscriptions: subscriptions,
        }
    }

    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().rebuild(cx);
            cx.notify();
        });
        cx.notify();
    }

    fn on_filter_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = input.read(cx).value().trim().to_owned();
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().filter = text;
                    table.delegate_mut().rebuild(cx);
                    // Row indices now point into a different list.
                    table.clear_selection(cx);
                    table.scroll_to_row(0, cx);
                    cx.notify();
                });
                cx.notify();
            }
            // Enter hands the keyboard to the results, where j/k take over.
            InputEvent::PressEnter { .. } => self.focus_table(window, cx),
            _ => {}
        }
    }

    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn focus_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.table.read(cx).focus_handle(cx);
        handle.focus(window, cx);
        // Select the first row so j/k have somewhere to start.
        self.table.update(cx, |table, cx| {
            if table.selected_row().is_none() && table.delegate().rows_count(cx) > 0 {
                table.set_selected_row(0, cx);
            }
        });
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.refresh(cx));
        self.table.update(cx, |table, cx| {
            table.clear_selection(cx);
            table.scroll_to_row(0, cx);
        });
    }

    fn status_text(&self, cx: &App) -> SharedString {
        let store = self.store.read(cx);
        let loaded = store.plays().len();
        let filter = &self.table.read(cx).delegate().filter;
        let shown = self.table.read(cx).delegate().rows.len();
        if !filter.is_empty() {
            return format!(
                "{} of {} loaded plays match. The filter only searches loaded rows.",
                group(shown),
                group(loaded)
            )
            .into();
        }
        match (store.is_loading(), store.has_more()) {
            (true, _) if loaded > 0 => format!("{} plays loaded · loading older…", group(loaded)),
            (true, _) => "Loading…".to_owned(),
            (false, true) => format!("{} plays loaded · scroll for older", group(loaded)),
            (false, false) => format!("All {} stored plays loaded", group(loaded)),
        }
        .into()
    }
}

impl Focusable for HistoryView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.table.read(cx).focus_handle(cx)
    }
}

impl Render for HistoryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let error = store.error().cloned();
        let has_rows = !store.plays().is_empty();
        let can_page_manually = store.has_more() && !store.is_loading() && error.is_none();
        let filtering = !self.table.read(cx).delegate().filter.is_empty();
        let retry = cx.listener(|this, _, _, cx| this.store.update(cx, |s, cx| s.retry(cx)));

        let body = match (&error, has_rows) {
            // Nothing to show at all: the error is the page.
            (Some(error), false) => {
                widgets::error_panel("history-retry", error, retry, cx).into_any_element()
            }
            _ => DataTable::new(&self.table)
                .stripe(true)
                .bordered(false)
                .into_any_element(),
        };

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div().flex_1().child(
                            Input::new(&self.filter)
                                .prefix(Icon::new(IconName::Search).small())
                                .cleanable(true)
                                .small(),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("⌘F filter · Enter to list · j/k move"),
                    ),
            )
            .when_some(error.clone().filter(|_| has_rows), |this, error| {
                let retry =
                    cx.listener(|this, _, _, cx| this.store.update(cx, |s, cx| s.retry(cx)));
                this.child(div().px_3().pt_2().child(widgets::error_banner(
                    "history-banner-retry",
                    &error,
                    retry,
                    cx,
                )))
            })
            .child(div().flex_1().min_h_0().child(body))
            .child(
                h_flex()
                    .gap_3()
                    .px_3()
                    .py_1p5()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().flex_1().child(self.status_text(cx)))
                    // Scrolling cannot page while a filter is active, so give
                    // an explicit way to widen what the filter searches.
                    .when(filtering && can_page_manually, |this| {
                        this.child(
                            Button::new("load-older")
                                .xsmall()
                                .ghost()
                                .label(format!("Load {PAGE_SIZE} older"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.store.update(cx, |s, cx| s.load_older(cx))
                                })),
                        )
                    }),
            )
    }
}

/// Thousands separators for counts in status text.
pub fn group(n: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn play(track: Option<&str>, artists: Option<&str>, album: Option<&str>) -> Play {
        Play {
            played_at_ms: 0,
            track_id: "t".into(),
            context_uri: None,
            context_type: None,
            source: None,
            track_name: track.map(Into::into),
            duration_ms: None,
            album_name: album.map(Into::into),
            image_url: None,
            artists: artists.map(Into::into),
        }
    }

    #[test]
    fn filter_matches_any_text_column_case_insensitively() {
        let p = play(
            Some("Rich Spirit"),
            Some("Kendrick Lamar"),
            Some("Mr. Morale"),
        );
        assert!(matches_filter(&p, "spirit"));
        assert!(matches_filter(&p, "kendrick"));
        assert!(matches_filter(&p, "morale"));
        assert!(!matches_filter(&p, "drake"));
    }

    #[test]
    fn filter_tolerates_export_rows_without_album() {
        let p = play(Some("211"), Some("Rukkus"), None);
        assert!(matches_filter(&p, "rukkus"));
        assert!(!matches_filter(&p, "album"));
    }

    #[test]
    fn grouping() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1000), "1,000");
        assert_eq!(group(1234567), "1,234,567");
    }
}
