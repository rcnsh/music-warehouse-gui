//! The ⌘K command palette: every window command, searchable by name, and
//! below them the stored artists and songs whose names match what is typed.

use std::time::Duration;

use gpui::AppContext as _;
use gpui::{
    Action, App, Context, Entity, IntoElement, ParentElement, SharedString, Styled, Task,
    WeakEntity, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, IndexPath, WindowExt,
    command::{Command, CommandGroup, CommandItem, CommandState},
    h_flex,
};
use gpui_kit_assets::IconName;

use crate::actions;
use crate::api::ApiClient;
use crate::appearance::Appearance;
use crate::dates::RangePreset;
use crate::models::{SearchArtist, SearchTrack, artist_key};
use crate::runtime;
use crate::views::nav::{Page, Section};
use crate::views::shell::Shell;
use crate::views::widgets;

/// How long typing pauses before the query goes to the Worker, so a name
/// typed at a normal pace costs one search rather than one per letter.
const SEARCH_AFTER: Duration = Duration::from_millis(200);

/// Matches per list. The palette shows a few hundred pixels of rows.
const SEARCH_LIMIT: u32 = 6;

/// Sections of the palette's list, as `on_confirm` reports them: the
/// commands are ungrouped (section 0), then one group each.
const SECTION_ARTISTS: usize = 1;
const SECTION_SONGS: usize = 2;

/// Hands out a ticket per query so only the newest query's answer is shown:
/// a slow reply to "ke" must not replace the list for "kendrick".
#[derive(Debug, Default)]
pub struct LatestQuery {
    current: u64,
}

impl LatestQuery {
    pub fn next(&mut self) -> u64 {
        self.current += 1;
        self.current
    }

    pub fn is_latest(&self, ticket: u64) -> bool {
        ticket == self.current
    }
}

/// The answer to one query.
#[derive(Debug, Default)]
struct Matches {
    query: String,
    artists: Vec<SearchArtist>,
    tracks: Vec<SearchTrack>,
}

/// Searches the Worker as the palette's query changes.
struct PaletteSearch {
    client: ApiClient,
    palette: Entity<CommandState>,
    latest: LatestQuery,
    matches: Matches,
    /// Replaced on every keystroke, which drops (and so cancels) the
    /// previous wait or request.
    _task: Option<Task<()>>,
}

impl PaletteSearch {
    fn set_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let query = query.trim().to_owned();
        let ticket = self.latest.next();
        if query.is_empty() {
            self._task = None;
            self.matches = Matches::default();
            self.set_loading(false, window, cx);
            return;
        }
        let client = self.client.clone();
        self.set_loading(true, window, cx);
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SEARCH_AFTER).await;
            let q = query.clone();
            let result =
                runtime::run_cancellable(
                    async move { client.search(&q, None, SEARCH_LIMIT).await },
                )
                .await;
            this.update_in(cx, |this, window, cx| {
                if !this.latest.is_latest(ticket) {
                    return;
                }
                // A failed search leaves just the commands; the palette is
                // no place for an error banner, and the next keystroke
                // tries again.
                let response = result.unwrap_or_default();
                this.matches = Matches {
                    query,
                    artists: response.artists,
                    tracks: response.tracks,
                };
                this.set_loading(false, window, cx);
                cx.notify();
                window.refresh();
            })
            .ok();
        }));
        cx.notify();
    }

    fn set_loading(&self, loading: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.palette
            .update(cx, |state, cx| state.set_loading(loading, window, cx));
    }

    /// The groups to list below the commands. Each item also matches the
    /// query that found it, so the palette's own filter keeps every one the
    /// Worker matched.
    fn groups(&self) -> [CommandGroup; 2] {
        let query = SharedString::from(self.matches.query.clone());
        let artists = self.matches.artists.iter().map(|artist| {
            result_item(
                artist.name.clone(),
                None,
                artist.plays,
                IconName::MicVocal,
                &query,
            )
        });
        let songs = self.matches.tracks.iter().map(|track| {
            result_item(
                track.name.clone(),
                track.artist.clone(),
                track.plays,
                IconName::Music,
                &query,
            )
        });
        [
            CommandGroup::new().label("Artists").items(artists),
            CommandGroup::new().label("Songs").items(songs),
        ]
    }

    /// The page a confirmed result opens.
    fn page_at(&self, ix: IndexPath) -> Option<Page> {
        match ix.section {
            SECTION_ARTISTS => {
                let artist = self.matches.artists.get(ix.row)?;
                Some(Page::Artist(
                    artist_key(artist.id.as_deref(), &artist.name).into(),
                ))
            }
            SECTION_SONGS => {
                let track = self.matches.tracks.get(ix.row)?;
                Some(Page::Track(track.id.clone().into()))
            }
            _ => None,
        }
    }
}

fn plays_label(plays: u32) -> String {
    match plays {
        1 => "1 play".to_owned(),
        n => format!("{} plays", widgets::count(n.into())),
    }
}

/// A row with the name, the song's artist when there is one, and the play
/// count at the right.
fn result_item(
    name: String,
    by: Option<String>,
    plays: u32,
    icon: IconName,
    query: &SharedString,
) -> CommandItem {
    let label = match &by {
        Some(by) => format!("{name} — {by}"),
        None => name.clone(),
    };
    CommandItem::new()
        .label(label)
        .keywords([query.clone()])
        .child(move |_, cx| {
            let muted = cx.theme().muted_foreground;
            h_flex()
                .w_full()
                .gap_2()
                .child(gpui_component::Icon::new(icon).text_color(muted))
                .child(div().truncate().child(name.clone()))
                .when_some(by.clone(), |this, by| {
                    this.child(div().truncate().text_color(muted).child(by))
                })
                .child(
                    div()
                        .ml_auto()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(muted)
                        .child(plays_label(plays)),
                )
                .into_any_element()
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteCommand {
    Show(Section),
    Back,
    FindInHistory,
    GoToDate,
    BackToNewest,
    Range(RangePreset),
    Refresh,
    Settings,
    Appearance(Appearance),
}

impl PaletteCommand {
    /// Every page first, in sidebar order, then the other commands.
    pub fn all() -> Vec<Self> {
        let mut all: Vec<Self> = Section::ALL.into_iter().map(Self::Show).collect();
        all.extend([
            Self::Back,
            Self::FindInHistory,
            Self::GoToDate,
            Self::BackToNewest,
            Self::Range(RangePreset::Last7Days),
            Self::Range(RangePreset::Last30Days),
            Self::Range(RangePreset::LastYear),
            Self::Refresh,
            Self::Settings,
            Self::Appearance(Appearance::System),
            Self::Appearance(Appearance::Light),
            Self::Appearance(Appearance::Dark),
        ]);
        all
    }

    fn label(self) -> SharedString {
        match self {
            Self::Show(section) => format!("Show {}", section.label()).into(),
            Self::Back => "Back".into(),
            Self::FindInHistory => "Filter History".into(),
            Self::GoToDate => "Go to Date in History".into(),
            Self::BackToNewest => "Back to Newest Plays".into(),
            Self::Range(RangePreset::Last7Days) => "Overview: Last 7 Days".into(),
            Self::Range(RangePreset::Last30Days) => "Overview: Last 30 Days".into(),
            Self::Range(RangePreset::LastYear) => "Overview: Last Year".into(),
            Self::Range(RangePreset::Custom) => "Overview: Custom Range".into(),
            Self::Refresh => "Refresh".into(),
            Self::Settings => "Settings".into(),
            Self::Appearance(Appearance::System) => "Appearance: Match System".into(),
            Self::Appearance(Appearance::Light) => "Appearance: Light".into(),
            Self::Appearance(Appearance::Dark) => "Appearance: Dark".into(),
        }
    }

    /// Extra words people search by, beyond the label.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Show(section) => section.keywords(),
            Self::Back => &["previous", "return"],
            Self::FindInHistory => &["search", "find", "track", "album"],
            Self::GoToDate => &["jump", "day", "calendar"],
            Self::BackToNewest => &["today", "latest", "top"],
            Self::Range(RangePreset::Last7Days) => &["range", "week", "7d"],
            Self::Range(RangePreset::Last30Days) => &["range", "month", "30d"],
            Self::Range(RangePreset::LastYear) => &["range", "12 months", "1y"],
            Self::Range(RangePreset::Custom) => &["range"],
            Self::Refresh => &["reload", "update"],
            Self::Settings => &["token", "worker", "url", "preferences"],
            Self::Appearance(_) => &["theme", "mode", "colour", "color"],
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Show(section) => section.icon(),
            Self::Back => IconName::ArrowLeft,
            Self::BackToNewest => IconName::Inbox,
            Self::Range(_) => IconName::ChartColumn,
            Self::FindInHistory => IconName::Search,
            Self::GoToDate => IconName::Calendar,
            Self::Refresh => IconName::RefreshCw,
            Self::Settings => IconName::Settings,
            Self::Appearance(Appearance::Dark) => IconName::Moon,
            Self::Appearance(_) => IconName::Sun,
        }
    }

    /// Only for the shortcut hint the palette draws beside an item; see
    /// `open` for why the palette does not dispatch it.
    fn hint_action(self) -> Option<Box<dyn Action>> {
        Some(match self {
            Self::Show(section) => section.action(),
            Self::Back => Box::new(actions::GoBack),
            Self::FindInHistory => Box::new(actions::FocusFilter),
            Self::GoToDate => Box::new(actions::GoToDate),
            Self::Refresh => Box::new(actions::Refresh),
            Self::Settings => Box::new(actions::OpenSettings),
            Self::BackToNewest | Self::Range(_) | Self::Appearance(_) => return None,
        })
    }
}

/// Opens the palette over the window.
///
/// Confirmed items run through the shell directly instead of by action
/// dispatch: while the palette confirms, focus is inside its dialog, which is
/// not under the shell, so the shell's action handlers would never see them.
/// The palette still dispatches an item's own action first; none of these
/// has a handler on the dialog's path, so that dispatch does nothing.
pub fn open(shell: WeakEntity<Shell>, client: ApiClient, window: &mut Window, cx: &mut App) {
    let state = cx.new(|cx| CommandState::new(window, cx));
    let current = crate::appearance::current(cx);
    let commands = PaletteCommand::all();
    let items: Vec<CommandItem> = commands
        .iter()
        .map(|command| {
            let item = CommandItem::new()
                .checked(*command == PaletteCommand::Appearance(current))
                .label(command.label())
                .icon(command.icon())
                .keywords(command.keywords().iter().copied());
            match command.hint_action() {
                Some(action) => item.action(action),
                None => item,
            }
        })
        .collect();

    let search = cx.new(|_| PaletteSearch {
        client,
        palette: state.clone(),
        latest: LatestQuery::default(),
        matches: Matches::default(),
        _task: None,
    });

    let palette_state = state.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let shell = shell.clone();
        let commands = commands.clone();
        let [artists, songs] = search.read(cx).groups();
        let on_query = search.clone();
        let on_confirm = search.clone();
        dialog.w(px(520.)).p_0().close_button(false).child(
            Command::new(&palette_state)
                .items(items.clone())
                .group(artists)
                .group(songs)
                .placeholder("Type a command, artist or song…")
                .bordered(false)
                .on_query(move |query, window, cx| {
                    on_query.update(cx, |search, cx| search.set_query(query, window, cx));
                })
                .on_confirm(move |ix: IndexPath, window, cx| {
                    window.close_dialog(cx);
                    // Results open pages; the commands run as before. Both go
                    // through the shell for the reason given above.
                    let page = on_confirm.read(cx).page_at(ix);
                    let command = (ix.section == 0)
                        .then(|| commands.get(ix.row).copied())
                        .flatten();
                    shell
                        .update(cx, |shell, cx| match (page, command) {
                            (Some(page), _) => shell.navigate(page, window, cx),
                            (None, Some(command)) => shell.run_command(command, window, cx),
                            (None, None) => {}
                        })
                        .ok();
                }),
        )
    });
    state.update(cx, |state, cx| state.focus(window, cx));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_a_distinct_label() {
        let mut labels: Vec<_> = PaletteCommand::all().iter().map(|c| c.label()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), PaletteCommand::all().len());
    }

    #[test]
    fn only_the_newest_query_is_answered() {
        let mut latest = LatestQuery::default();
        let ke = latest.next();
        let kendrick = latest.next();
        // "ke" answers after "kendrick" was typed: ignored.
        assert!(!latest.is_latest(ke));
        assert!(latest.is_latest(kendrick));
        // Clearing the box takes a ticket too, so a late reply to the last
        // query does not refill an emptied palette.
        latest.next();
        assert!(!latest.is_latest(kendrick));
    }

    #[test]
    fn plays_read_naturally() {
        assert_eq!(plays_label(1), "1 play");
        assert_eq!(plays_label(1234), "1,234 plays");
    }
}
