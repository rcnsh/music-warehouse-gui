use std::time::Duration;

use chrono::NaiveDate;
use gpui::AppContext as _;
use gpui::{
    AnyElement, App, ClipboardItem, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Icon, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    table::{Column, DataTable, TableDelegate, TableEvent, TableState},
    v_flex,
};
use gpui_kit_assets::IconName;

use crate::actions::{CopyPlay, OpenInSpotify};
use crate::api::ApiClient;
use crate::dates;
use crate::models::{Play, Rendition, album_rendition};
use crate::state::history::HistoryStore;
use crate::views::{play_detail, widgets};

const COL_ART: usize = 0;
const COL_TIME: usize = 1;
const COL_TRACK: usize = 2;
const COL_ARTIST: usize = 3;

/// How long typing has to pause before the filter goes to the Worker. Every
/// search sent costs Worker reads even if the user has typed on, so a word
/// typed at a normal pace should cost one search, not one per letter.
const SEARCH_AFTER: Duration = Duration::from_millis(450);

/// One letter matches most of the history and can't use the Worker's name
/// index, so a search waits for a second.
const MIN_SEARCH_CHARS: usize = 2;

/// The filter text to send to the Worker: empty to show everything, `None`
/// while there is too little to search yet.
fn searchable(text: &str) -> Option<&str> {
    match text.chars().count() {
        0 => Some(""),
        n if n < MIN_SEARCH_CHARS => None,
        _ => Some(text),
    }
}

/// Reads the "search from date" box: empty for no start bound. The range
/// ends at the go-to date, so a start after it could match nothing.
fn parse_since(
    text: &str,
    anchor: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<Option<NaiveDate>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let day = dates::parse_jump(text, today)?;
    match anchor {
        Some(end) if day > end => Err(format!(
            "The search start is after the go-to date ({}).",
            end.format("%Y-%m-%d")
        )),
        _ => Ok(Some(day)),
    }
}

/// A table row: an index into one of the store's lists, so the store stays
/// the single owner of fetched data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    /// A play Spotify reports that the warehouse has not stored yet.
    Unsaved(usize),
    Stored(usize),
}

/// The Worker does the filtering (`q`), so every loaded row is shown, with
/// any unsaved plays above them.
pub struct PlaysTable {
    store: Entity<HistoryStore>,
    rows: Vec<Row>,
}

impl PlaysTable {
    fn rebuild(&mut self, cx: &App) {
        let store = self.store.read(cx);
        self.rows = (0..store.unsaved().len())
            .map(Row::Unsaved)
            .chain((0..store.plays().len()).map(Row::Stored))
            .collect();
    }

    fn play<'a>(&self, row_ix: usize, cx: &'a App) -> Option<&'a Play> {
        let store = self.store.read(cx);
        match *self.rows.get(row_ix)? {
            Row::Unsaved(ix) => store.unsaved().get(ix),
            Row::Stored(ix) => store.plays().get(ix),
        }
    }

    fn is_unsaved(&self, row_ix: usize) -> bool {
        matches!(self.rows.get(row_ix), Some(Row::Unsaved(_)))
    }
}

/// The time column for an unsaved play: a label saying so, then a short
/// time, since such plays are at most a day or so old.
fn unsaved_time(play: &Play, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let when = chrono::DateTime::from_timestamp_millis(play.played_at_ms)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%a %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    h_flex()
        .gap_1p5()
        .overflow_hidden()
        .child(
            div()
                .flex_shrink_0()
                .px_1()
                .rounded_sm()
                .border_1()
                .border_color(cx.theme().border)
                .text_xs()
                .text_color(muted)
                .child(UNSAVED_LABEL),
        )
        .child(div().truncate().text_color(muted).child(when))
        .into_any_element()
}

const UNSAVED_LABEL: &str = "Not saved yet";

impl TableDelegate for PlaysTable {
    fn columns_count(&self, _: &App) -> usize {
        5
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            COL_ART => Column::new("art", "").width(px(44.)).resizable(false),
            COL_TIME => Column::new("time", "Played").width(px(190.)),
            COL_TRACK => Column::new("track", "Track").width(px(320.)),
            COL_ARTIST => Column::new("artist", "Artist").width(px(240.)),
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
            return div().into_any_element();
        };
        if col_ix == COL_ART {
            let url = play
                .image_url
                .as_deref()
                .map(|url| SharedString::from(album_rendition(url, Rendition::Px64)));
            return widgets::artwork(url, px(22.), false, cx);
        }
        let muted = cx.theme().muted_foreground;
        let unsaved = self.is_unsaved(row_ix);
        if unsaved && col_ix == COL_TIME {
            return unsaved_time(play, cx);
        }
        // Export-era rows lack album names; say so rather than leave a hole
        // that looks like a rendering bug.
        let (text, is_missing): (SharedString, bool) = match col_ix {
            COL_TIME => (
                dates::format_played_at(play.played_at_ms, &chrono::Local).into(),
                false,
            ),
            COL_TRACK => text_or(&play.track_name, "Unknown track"),
            COL_ARTIST => text_or(&play.artists, "Unknown artist"),
            _ => text_or(&play.album_name, "No album info"),
        };
        div()
            .truncate()
            // Unsaved rows are muted throughout, so they read as provisional.
            .when(col_ix == COL_TIME || unsaved, |this| this.text_color(muted))
            .when(is_missing, |this| this.text_color(muted).italic())
            .child(text)
            .into_any_element()
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, cx: &App) -> String {
        let Some(play) = self.play(row_ix, cx) else {
            return String::new();
        };
        match col_ix {
            COL_ART => String::new(),
            COL_TIME if self.is_unsaved(row_ix) => format!(
                "{} ({UNSAVED_LABEL})",
                dates::format_played_at(play.played_at_ms, &chrono::Local)
            ),
            COL_TIME => dates::format_played_at(play.played_at_ms, &chrono::Local),
            COL_TRACK => play.track_name.clone().unwrap_or_default(),
            COL_ARTIST => play.artists.clone().unwrap_or_default(),
            _ => play.album_name.clone().unwrap_or_default(),
        }
    }

    fn loading(&self, cx: &App) -> bool {
        let store = self.store.read(cx);
        store.is_loading() && store.plays().is_empty()
    }

    fn has_more(&self, cx: &App) -> bool {
        let store = self.store.read(cx);
        store.has_more() && !store.is_loading() && store.error().is_none()
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
        let store = self.store.read(cx);
        let Some(query) = store.query().map(str::to_owned) else {
            return widgets::state_panel(
                Some(Icon::new(IconName::Inbox)),
                "No plays stored yet",
                Some("The warehouse is empty. Plays appear after the Worker's next poll.".into()),
                None,
                cx,
            )
            .into_any_element();
        };
        let day = widgets::day;
        let scope = match (store.since(), store.anchor()) {
            (Some(from), Some(to)) => {
                format!("Searched every play from {} to {}.", day(from), day(to))
            }
            (Some(from), None) => format!("Searched every play since {}.", day(from)),
            (None, Some(to)) => format!("Searched every play up to {}.", day(to)),
            (None, None) => "Searched every stored play.".to_owned(),
        };
        widgets::state_panel(
            Some(Icon::new(IconName::Search)),
            format!("No plays match “{query}”"),
            Some(format!("{scope} Track, artist and album names are matched.").into()),
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
    jump: Entity<InputState>,
    jump_error: Option<SharedString>,
    /// The first day a search covers; the go-to date is the last.
    since: Entity<InputState>,
    since_error: Option<SharedString>,
    /// Set while the filter holds too little text to search.
    filter_too_short: bool,
    _search_debounce: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl HistoryView {
    pub fn new(client: ApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| HistoryStore::new(client, cx));
        let delegate = PlaysTable {
            store: store.clone(),
            rows: Vec::new(),
        };
        let table = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .col_movable(false)
                .col_selectable(false)
                .sortable(false)
        });
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search all plays by track, artist or album")
        });

        let jump = cx.new(|cx| InputState::new(window, cx).placeholder("Go to date"));
        let since = cx.new(|cx| InputState::new(window, cx).placeholder("Search from date"));

        let subscriptions = vec![
            cx.observe(&store, |this, _, cx| this.rebuild_rows(cx)),
            // The details panel follows the table's selection.
            cx.subscribe(&table, |_, _, event: &TableEvent, cx| {
                if matches!(event, TableEvent::SelectRow(_) | TableEvent::ClearSelection) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&filter, window, Self::on_filter_event),
            cx.subscribe_in(&jump, window, Self::on_jump_event),
            cx.subscribe_in(&since, window, Self::on_since_event),
        ];
        Self {
            store,
            table,
            filter,
            jump,
            jump_error: None,
            since,
            since_error: None,
            filter_too_short: false,
            _search_debounce: None,
            _subscriptions: subscriptions,
        }
    }

    /// Rows are positions, so new plays arriving on top would silently move
    /// the selection to a different play. Follow the play itself instead.
    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected_play(cx).map(|p| (p.played_at_ms, p.track_id));
        self.table.update(cx, |table, cx| {
            let before = table.selected_row();
            table.delegate_mut().rebuild(cx);
            // An unsaved play keeps its selection when the Worker stores it:
            // the stored row has the same time and track.
            let after = selected.and_then(|(ms, id)| {
                let delegate = table.delegate();
                (0..delegate.rows.len()).position(|row_ix| {
                    delegate
                        .play(row_ix, cx)
                        .is_some_and(|play| play.played_at_ms == ms && play.track_id == id)
                })
            });
            match (before, after) {
                (Some(old), Some(new)) if old != new => table.set_selected_row(new, cx),
                (Some(_), None) => table.clear_selection(cx),
                _ => {}
            }
            cx.notify();
        });
        cx.notify();
    }

    fn on_jump_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => {
                let text = input.read(cx).value().to_string();
                match dates::parse_jump(&text, dates::today_local()) {
                    Ok(day) => {
                        self.jump_error = None;
                        self.jump_to(day, cx);
                        self.focus_table(window, cx);
                    }
                    Err(message) => self.jump_error = Some(message.into()),
                }
                cx.notify();
            }
            InputEvent::Change if self.jump_error.is_some() => {
                self.jump_error = None;
                cx.notify();
            }
            _ => {}
        }
    }

    /// Enter applies the start date; clearing the box lifts it at once.
    fn on_since_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = input.read(cx).value().trim().to_owned();
        match event {
            InputEvent::PressEnter { .. } => {
                let anchor = self.store.read(cx).anchor();
                match parse_since(&text, anchor, dates::today_local()) {
                    Ok(day) => {
                        self.since_error = None;
                        self.store.update(cx, |store, cx| store.set_since(day, cx));
                        self.reset_table(cx);
                        if self.store.read(cx).query().is_some() {
                            self.focus_table(window, cx);
                        } else {
                            // A start date alone filters nothing; the search
                            // box is where it takes effect.
                            self.focus_filter(window, cx);
                        }
                    }
                    Err(message) => self.since_error = Some(message.into()),
                }
                cx.notify();
            }
            InputEvent::Change => {
                self.since_error = None;
                if text.is_empty() && self.store.read(cx).since().is_some() {
                    self.store.update(cx, |store, cx| store.set_since(None, cx));
                    self.reset_table(cx);
                }
                cx.notify();
            }
            _ => {}
        }
    }

    pub fn focus_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.jump.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Public so other views (the Overview chart) can open History at a day.
    pub fn jump_to(&mut self, day: chrono::NaiveDate, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.jump_to(day, cx));
        self.reset_table(cx);
    }

    pub fn back_to_newest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.jump
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.jump_error = None;
        self.store.update(cx, |store, cx| store.show_newest(cx));
        self.reset_table(cx);
    }

    /// Clears what belonged to the old list: a selected row index and the
    /// scroll offset would otherwise land somewhere arbitrary in the new one.
    fn reset_table(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.clear_selection(cx);
            table.scroll_to_row(0, cx);
        });
    }

    /// Called on a timer by the shell while the window is visible.
    pub fn check_newer(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.check_newer(cx));
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
                self._search_debounce = None;
                self.filter_too_short = searchable(&text).is_none();
                match searchable(&text) {
                    // Clearing, or too little to search yet: show everything, at
                    // once, since that needs no request.
                    Some("") | None => self.search("", cx),
                    Some(_) => {
                        self._search_debounce = Some(cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(SEARCH_AFTER).await;
                            this.update(cx, |this, cx| this.search(&text, cx)).ok();
                        }));
                    }
                }
                cx.notify();
            }
            // Enter searches at once rather than waiting out the pause, then
            // hands the keyboard to the results, where j/k take over.
            InputEvent::PressEnter { .. } => {
                let text = input.read(cx).value().trim().to_owned();
                if let Some(query) = searchable(&text) {
                    self._search_debounce = None;
                    self.search(query, cx);
                }
                self.focus_table(window, cx);
            }
            _ => {}
        }
    }

    fn search(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.store.read(cx).query().unwrap_or_default() == query {
            return;
        }
        self.store
            .update(cx, |store, cx| store.set_query(query, cx));
        self.reset_table(cx);
    }

    fn selected_play(&self, cx: &App) -> Option<Play> {
        let table = self.table.read(cx);
        let row = table.selected_row()?;
        table.delegate().play(row, cx).cloned()
    }

    fn open_selected(&mut self, _: &OpenInSpotify, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(play) = self.selected_play(cx) {
            cx.open_url(&play_detail::track_url(&play.track_id));
        }
    }

    fn copy_selected(&mut self, _: &CopyPlay, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(play) = self.selected_play(cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(play_detail::copy_text(&play)));
            window.push_notification(Notification::success("Copied track and link"), cx);
        }
    }

    fn close_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| table.clear_selection(cx));
        self.focus_table(window, cx);
        // focus_table selects the first row when nothing is selected; closing
        // the panel should leave nothing selected.
        self.table.update(cx, |table, cx| table.clear_selection(cx));
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
        self.reset_table(cx);
    }

    fn status_text(&self, cx: &App) -> SharedString {
        let store = self.store.read(cx);
        let loaded = store.plays().len();
        let mut prefix = match store.anchor() {
            Some(day) => format!("From {} back · ", day.format("%a %-d %b %Y")),
            None => String::new(),
        };
        match (store.since(), store.query()) {
            (Some(day), Some(_)) => {
                prefix.insert_str(0, &format!("Since {} · ", day.format("%a %-d %b %Y")))
            }
            (Some(_), None) => prefix.insert_str(0, "The start date applies to searches · "),
            (None, _) => {}
        }
        if self.filter_too_short {
            prefix.insert_str(0, "Type 2 or more characters to search · ");
        }
        let unsaved = store.unsaved().len();
        let suffix = match unsaved {
            0 => String::new(),
            1 => " · 1 play not saved yet".to_owned(),
            n => format!(" · {n} plays not saved yet"),
        };
        let (noun, all) = match store.query() {
            Some(_) => ("matching plays", "matching"),
            None => ("plays", "stored"),
        };
        let n = widgets::count(loaded as u64);
        let text = match (store.is_loading(), store.has_more()) {
            (true, _) if loaded > 0 => format!("{n} {noun} loaded · loading older…"),
            (true, _) if store.query().is_some() => "Searching…".to_owned(),
            (true, _) => "Loading…".to_owned(),
            (false, true) => format!("{n} {noun} loaded · scroll for older"),
            (false, false) if store.anchor().is_some() => {
                format!("{n} {noun} loaded · reached the oldest")
            }
            (false, false) => format!("All {n} {all} plays loaded"),
        };
        format!("{prefix}{text}{suffix}").into()
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
        let anchored = store.anchor().is_some();
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

        let selected_unsaved = {
            let table = self.table.read(cx);
            table
                .selected_row()
                .is_some_and(|row| table.delegate().is_unsaved(row))
        };
        let detail = self.selected_play(cx).map(|play| {
            let actions = play_detail::DetailActions {
                on_open: Box::new(cx.listener(|this, _, window, cx| {
                    this.open_selected(&OpenInSpotify, window, cx)
                })),
                on_copy: Box::new(
                    cx.listener(|this, _, window, cx| this.copy_selected(&CopyPlay, window, cx)),
                ),
                on_close: Box::new(
                    cx.listener(|this, _, window, cx| this.close_details(window, cx)),
                ),
            };
            play_detail::render(&play, selected_unsaved, actions, cx)
        });

        v_flex()
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::copy_selected))
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
                        div().w(px(170.)).child(
                            Input::new(&self.since)
                                .prefix(Icon::new(IconName::CalendarDays).small())
                                .cleanable(true)
                                .small(),
                        ),
                    )
                    .child(
                        div().w(px(150.)).child(
                            Input::new(&self.jump)
                                .prefix(Icon::new(IconName::Calendar).small())
                                .small(),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("⌘F search · ⌘G date · j/k move"),
                    ),
            )
            .when_some(
                self.since_error.clone().or(self.jump_error.clone()),
                |this, message| {
                    this.child(
                        div()
                            .px_3()
                            .py_1()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(message),
                    )
                },
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
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(div().flex_1().min_w_0().h_full().child(body))
                    .when_some(detail, |this, detail| this.child(detail)),
            )
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
                    .when(anchored, |this| {
                        this.child(
                            Button::new("back-to-newest")
                                .xsmall()
                                .ghost()
                                .icon(IconName::ArrowUp)
                                .label("Back to newest")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.back_to_newest(window, cx)
                                })),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_waits_for_two_characters() {
        assert_eq!(searchable(""), Some(""));
        assert_eq!(searchable("a"), None);
        assert_eq!(searchable("é"), None);
        assert_eq!(searchable("u2"), Some("u2"));
        assert_eq!(searchable("divinity"), Some("divinity"));
    }

    #[test]
    fn search_start_date() {
        let day = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let today = day("2026-10-15");
        assert_eq!(parse_since("  ", None, today), Ok(None));
        assert_eq!(
            parse_since("2024-01-31", None, today),
            Ok(Some(day("2024-01-31")))
        );
        // A future start means "today", as the go-to box treats it.
        assert_eq!(parse_since("2030-01-01", None, today), Ok(Some(today)));
        assert!(parse_since("yesterday", None, today).is_err());
        // The range ends at the go-to date, so a later start is refused.
        let end = Some(day("2024-06-01"));
        assert!(parse_since("2024-06-02", end, today).is_err());
        assert_eq!(
            parse_since("2024-06-01", end, today),
            Ok(Some(day("2024-06-01")))
        );
    }
}
