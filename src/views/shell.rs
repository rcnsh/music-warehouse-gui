use std::time::Duration;

use gpui::AppContext as _;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem},
    v_flex,
};
use gpui_kit_assets::IconName;

use crate::actions::{
    ExportCsv, FocusFilter, GoBack, GoToDate, OpenArtist, OpenCommandPalette, OpenSettings,
    OpenTrack, Refresh, ShowAlbums, ShowConcerts, ShowEras, ShowHistory, ShowOnRepeat,
    ShowOnThisDay, ShowOverview, ShowPlaysOfArtist, ShowPlaysOfTrack, ShowRhythm, ShowTopOverTime,
    ShowWrapped,
};
use crate::api::ApiClient;
use crate::appearance;
use crate::state::history::Scope;
use crate::state::now_playing::NowPlayingStore;
use crate::ui_state;
use crate::views::albums::AlbumsView;
use crate::views::artist::ArtistView;
use crate::views::concerts::ConcertsView;
use crate::views::eras::ErasView;
use crate::views::history::HistoryView;
use crate::views::nav::{BackStack, GROUPS, Page, Section};
use crate::views::now_playing::NowPlayingStrip;
use crate::views::on_repeat::OnRepeatView;
use crate::views::on_this_day::OnThisDayView;
use crate::views::overview::{OverviewEvent, OverviewView};
use crate::views::palette::{self, PaletteCommand};
use crate::views::rhythm::RhythmView;
use crate::views::top_over_time::TopOverTimeView;
use crate::views::track::TrackView;
use crate::views::wrapped::WrappedView;

/// How often History asks for plays newer than its top row. The Worker
/// ingests every 30 minutes, so polling faster finds nothing; this keeps the
/// wait after an ingest short while an empty `after` query stays one cheap
/// indexed read.
const LIVE_HISTORY_EVERY: Duration = Duration::from_secs(300);

const SIDEBAR_WIDTH: f32 = 196.;

pub enum ShellEvent {
    OpenSettings,
}

/// Every page's view, created once with the shell. Hidden pages stay alive,
/// so returning to one keeps its scroll position, loaded data and choices.
/// Pages other than History and Overview fetch nothing until first shown
/// (their `activate`).
///
/// Artist and Track are one view each, re-targeted with `set_artist` /
/// `set_track` whenever one opens (Back included). A view per open artist
/// would keep every visited artist's data alive for the whole session; one
/// re-targeted view costs one request when Back returns to a different
/// artist, which is cheap.
struct Pages {
    history: Entity<HistoryView>,
    overview: Entity<OverviewView>,
    on_repeat: Entity<OnRepeatView>,
    on_this_day: Entity<OnThisDayView>,
    albums: Entity<AlbumsView>,
    top_over_time: Entity<TopOverTimeView>,
    rhythm: Entity<RhythmView>,
    eras: Entity<ErasView>,
    wrapped: Entity<WrappedView>,
    concerts: Entity<ConcertsView>,
    artist: Entity<ArtistView>,
    track: Entity<TrackView>,
}

/// The main window once configured: the sidebar, the top bar with Back and
/// the now-playing strip, and the window-wide actions.
pub struct Shell {
    page: Page,
    back: BackStack,
    pages: Pages,
    now_playing_store: Entity<NowPlayingStore>,
    now_playing: Entity<NowPlayingStrip>,
    worker_host: String,
    /// For the ⌘K palette's artist and song search, which lives in a dialog
    /// outside every page.
    client: ApiClient,
    focus: FocusHandle,
    /// Dropped while the window is hidden, which stops the loop.
    live_history: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ShellEvent> for Shell {}

impl Shell {
    pub fn new(client: ApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let worker_host = client.base_url().host_str().unwrap_or_default().to_owned();
        let pages = Pages {
            history: cx.new(|cx| HistoryView::new(client.clone(), window, cx)),
            overview: cx.new(|cx| OverviewView::new(client.clone(), window, cx)),
            on_repeat: cx.new(|cx| OnRepeatView::new(client.clone(), window, cx)),
            on_this_day: cx.new(|cx| OnThisDayView::new(client.clone(), window, cx)),
            albums: cx.new(|cx| AlbumsView::new(client.clone(), window, cx)),
            top_over_time: cx.new(|cx| TopOverTimeView::new(client.clone(), window, cx)),
            rhythm: cx.new(|cx| RhythmView::new(client.clone(), window, cx)),
            eras: cx.new(|cx| ErasView::new(client.clone(), window, cx)),
            wrapped: cx.new(|cx| WrappedView::new(client.clone(), window, cx)),
            concerts: cx.new(|cx| ConcertsView::new(client.clone(), window, cx)),
            artist: cx.new(|cx| ArtistView::new(client.clone(), window, cx)),
            track: cx.new(|cx| TrackView::new(client.clone(), window, cx)),
        };
        let now_playing_store = cx.new(|cx| NowPlayingStore::new(client.clone(), cx));
        let now_playing = cx.new(|cx| NowPlayingStrip::new(now_playing_store.clone(), cx));

        // Polling stops while the window is minimized, on another Space or
        // fully covered, and resumes with an immediate check when it returns.
        let subscriptions = vec![
            cx.observe_window_visibility(window, |this, visibility, _, cx| {
                this.set_visible(visibility.is_visible(), cx);
            }),
            cx.subscribe_in(
                &pages.overview,
                window,
                |this, _, event, window, cx| match event {
                    OverviewEvent::OpenDay(day) => {
                        this.pages.history.update(cx, |h, cx| h.jump_to(*day, cx));
                        this.navigate(Section::History.into(), window, cx);
                    }
                },
            ),
        ];
        let visible = window.visibility().is_visible();

        let mut shell = Self {
            page: Section::History.into(),
            back: BackStack::default(),
            pages,
            now_playing_store,
            now_playing,
            worker_host,
            client,
            focus: cx.focus_handle(),
            live_history: None,
            _subscriptions: subscriptions,
        };
        shell.set_visible(visible, cx);
        let section = ui_state::get(cx)
            .page
            .map(Section::from_saved)
            .unwrap_or(Section::History);
        shell.show(section.into(), window, cx);
        shell
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.now_playing_store
            .update(cx, |s, cx| s.set_visible(visible, cx));
        if !visible {
            self.live_history = None;
            return;
        }
        if self.live_history.is_some() {
            return;
        }
        // Checks straight away on return, so a laptop opened in the morning
        // shows last night's plays without waiting a full interval.
        self.live_history = Some(cx.spawn(async move |this, cx| {
            loop {
                let alive = this
                    .update(cx, |this, cx| {
                        this.pages.history.update(cx, |h, cx| h.check_newer(cx))
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                cx.background_executor().timer(LIVE_HISTORY_EVERY).await;
            }
        }));
    }

    /// Goes to `page`, remembering the current one for Back.
    pub fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if page != self.page {
            self.back.push(self.page.clone());
        }
        self.show(page, window, cx);
    }

    fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.back.pop() {
            self.show(page, window, cx);
        }
    }

    /// Makes `page` visible, focused and loaded, without touching Back.
    fn show(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if let Page::Section(section) = &page {
            ui_state::update(cx, |state| state.page = Some(section.saved()));
        }
        self.page = page.clone();
        let p = &self.pages;
        match page {
            Page::Section(Section::History) => {
                p.history.update(cx, |h, cx| h.focus_table(window, cx));
            }
            Page::Section(Section::Overview) => {
                let handle = p.overview.read(cx).focus_handle(cx);
                handle.focus(window, cx);
            }
            Page::Section(Section::OnRepeat) => {
                focus(&p.on_repeat, window, cx);
                p.on_repeat.update(cx, |v, cx| v.activate(window, cx));
            }
            Page::Section(Section::OnThisDay) => {
                focus(&p.on_this_day, window, cx);
                p.on_this_day.update(cx, |v, cx| v.activate(window, cx));
            }
            Page::Section(Section::Albums) => {
                focus(&p.albums, window, cx);
                p.albums.update(cx, |v, cx| v.activate(window, cx));
            }
            Page::Section(Section::TopOverTime) => {
                focus(&p.top_over_time, window, cx);
                p.top_over_time.update(cx, |v, cx| v.activate(window, cx));
            }
            Page::Section(Section::Rhythm) => {
                focus(&p.rhythm, window, cx);
                p.rhythm.update(cx, |v, cx| v.activate(window, cx));
            }
            Page::Section(Section::Eras) => {
                focus(&p.eras, window, cx);
                p.eras.update(cx, |v, cx| v.activate(window, cx));
            }
            Page::Section(Section::Wrapped) => {
                focus(&p.wrapped, window, cx);
                p.wrapped.update(cx, |v, cx| v.activate(window, cx));
            }
            Page::Section(Section::Concerts) => {
                focus(&p.concerts, window, cx);
                p.concerts.update(cx, |v, cx| v.activate(window, cx));
            }
            Page::Artist(id) => {
                focus(&p.artist, window, cx);
                p.artist.update(cx, |v, cx| {
                    v.set_artist(id, cx);
                    v.activate(window, cx);
                });
            }
            Page::Track(id) => {
                focus(&p.track, window, cx);
                p.track.update(cx, |v, cx| {
                    v.set_track(id, cx);
                    v.activate(window, cx);
                });
            }
        }
        cx.notify();
    }

    fn on_focus_filter(&mut self, _: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Section::History.into(), window, cx);
        self.pages
            .history
            .update(cx, |h, cx| h.focus_filter(window, cx));
    }

    fn on_go_to_date(&mut self, _: &GoToDate, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Section::History.into(), window, cx);
        self.pages
            .history
            .update(cx, |h, cx| h.focus_jump(window, cx));
    }

    /// History narrowed to one song's or artist's plays.
    fn show_plays(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        self.pages
            .history
            .update(cx, |h, cx| h.show_scope(scope, window, cx));
        self.navigate(Section::History.into(), window, cx);
    }

    /// Exports what History shows, so the file matches the list on screen.
    fn on_export_csv(&mut self, _: &ExportCsv, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Section::History.into(), window, cx);
        self.pages
            .history
            .update(cx, |h, cx| h.export_csv(window, cx));
    }

    fn on_open_palette(
        &mut self,
        _: &OpenCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        palette::open(cx.entity().downgrade(), self.client.clone(), window, cx);
    }

    pub fn run_command(
        &mut self,
        command: PaletteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            PaletteCommand::Show(section) => self.navigate(section.into(), window, cx),
            PaletteCommand::Back => self.go_back(&GoBack, window, cx),
            PaletteCommand::FindInHistory => self.on_focus_filter(&FocusFilter, window, cx),
            PaletteCommand::GoToDate => self.on_go_to_date(&GoToDate, window, cx),
            PaletteCommand::BackToNewest => {
                self.pages
                    .history
                    .update(cx, |h, cx| h.back_to_newest(window, cx));
                self.navigate(Section::History.into(), window, cx);
            }
            PaletteCommand::Range(preset) => {
                self.pages
                    .overview
                    .update(cx, |o, cx| o.select_preset(preset, window, cx));
                self.navigate(Section::Overview.into(), window, cx);
            }
            PaletteCommand::Refresh => self.on_refresh(&Refresh, window, cx),
            PaletteCommand::ExportCsv => self.on_export_csv(&ExportCsv, window, cx),
            PaletteCommand::Settings => cx.emit(ShellEvent::OpenSettings),
            PaletteCommand::Appearance(choice) => appearance::set(choice, cx),
        }
    }

    /// Refreshes the visible page and the strip; hidden pages refresh
    /// themselves the next time they are asked, so ⌘R costs only what is on
    /// screen.
    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        let p = &self.pages;
        match &self.page {
            Page::Section(Section::History) => p.history.update(cx, |v, cx| v.refresh(cx)),
            Page::Section(Section::Overview) => p.overview.update(cx, |v, cx| v.refresh(cx)),
            Page::Section(Section::OnRepeat) => p.on_repeat.update(cx, |v, cx| v.refresh(cx)),
            Page::Section(Section::OnThisDay) => p.on_this_day.update(cx, |v, cx| v.refresh(cx)),
            Page::Section(Section::Albums) => p.albums.update(cx, |v, cx| v.refresh(cx)),
            Page::Section(Section::TopOverTime) => {
                p.top_over_time.update(cx, |v, cx| v.refresh(cx))
            }
            Page::Section(Section::Rhythm) => p.rhythm.update(cx, |v, cx| v.refresh(cx)),
            Page::Section(Section::Eras) => p.eras.update(cx, |v, cx| v.refresh(cx)),
            Page::Section(Section::Wrapped) => p.wrapped.update(cx, |v, cx| v.refresh(cx)),
            Page::Section(Section::Concerts) => p.concerts.update(cx, |v, cx| v.refresh(cx)),
            Page::Artist(_) => p.artist.update(cx, |v, cx| v.refresh(cx)),
            Page::Track(_) => p.track.update(cx, |v, cx| v.refresh(cx)),
        }
        self.now_playing_store.update(cx, |s, cx| s.refresh(cx));
    }

    fn on_open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ShellEvent::OpenSettings);
    }

    fn current_view(&self) -> AnyElement {
        let p = &self.pages;
        match &self.page {
            Page::Section(Section::History) => p.history.clone().into_any_element(),
            Page::Section(Section::Overview) => p.overview.clone().into_any_element(),
            Page::Section(Section::OnRepeat) => p.on_repeat.clone().into_any_element(),
            Page::Section(Section::OnThisDay) => p.on_this_day.clone().into_any_element(),
            Page::Section(Section::Albums) => p.albums.clone().into_any_element(),
            Page::Section(Section::TopOverTime) => p.top_over_time.clone().into_any_element(),
            Page::Section(Section::Rhythm) => p.rhythm.clone().into_any_element(),
            Page::Section(Section::Eras) => p.eras.clone().into_any_element(),
            Page::Section(Section::Wrapped) => p.wrapped.clone().into_any_element(),
            Page::Section(Section::Concerts) => p.concerts.clone().into_any_element(),
            Page::Artist(_) => p.artist.clone().into_any_element(),
            Page::Track(_) => p.track.clone().into_any_element(),
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mut groups: Vec<SidebarGroup<SidebarMenu>> = GROUPS
            .iter()
            .map(|group| {
                let items = group.sections.iter().map(|&section| {
                    let hint = SharedString::from(format!("⌘{}", section.shortcut()));
                    SidebarMenuItem::new(section.label())
                        .icon(section.icon())
                        .active(self.page == Page::Section(section))
                        .suffix(move |_, _| div().text_xs().text_color(muted).child(hint.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.navigate(section.into(), window, cx)
                        }))
                });
                SidebarGroup::new(group.label).child(SidebarMenu::new().children(items))
            })
            .collect();
        // A detail page has no fixed place, so it appears while it is open.
        let detail = match &self.page {
            Page::Artist(_) => Some(("Artist", IconName::MicVocal)),
            Page::Track(_) => Some(("Track", IconName::Music)),
            Page::Section(_) => None,
        };
        if let Some((label, icon)) = detail {
            groups.insert(
                0,
                SidebarGroup::new("Viewing").child(
                    SidebarMenu::new().child(SidebarMenuItem::new(label).icon(icon).active(true)),
                ),
            );
        }
        Sidebar::new("nav")
            .collapsible(false)
            .w(px(SIDEBAR_WIDTH))
            .children(groups)
            .footer(
                div()
                    .text_xs()
                    .text_color(muted)
                    .truncate()
                    .child(self.worker_host.clone()),
            )
    }

    fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_3()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("back")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowLeft)
                    .tooltip("Back (⌘[)")
                    .disabled(self.back.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.go_back(&GoBack, window, cx))),
            )
            .child(div().flex_1().min_w_0().child(self.now_playing.clone()))
            .child(
                Button::new("refresh")
                    .ghost()
                    .small()
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh (⌘R)")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.on_refresh(&Refresh, window, cx)),
                    ),
            )
            .child(
                Button::new("settings")
                    .ghost()
                    .small()
                    .icon(IconName::Settings)
                    .tooltip("Settings (⌘,)")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ShellEvent::OpenSettings))),
            )
    }
}

fn focus<V: Focusable>(view: &Entity<V>, window: &mut Window, cx: &mut App) {
    let handle = view.read(cx).focus_handle(cx);
    handle.focus(window, cx);
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .key_context("Shell")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ShowHistory, w, cx| {
                this.navigate(Section::History.into(), w, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowOverview, w, cx| {
                this.navigate(Section::Overview.into(), w, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowOnRepeat, w, cx| {
                this.navigate(Section::OnRepeat.into(), w, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowOnThisDay, w, cx| {
                this.navigate(Section::OnThisDay.into(), w, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowAlbums, w, cx| {
                this.navigate(Section::Albums.into(), w, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowTopOverTime, w, cx| {
                this.navigate(Section::TopOverTime.into(), w, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowRhythm, w, cx| {
                this.navigate(Section::Rhythm.into(), w, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ShowEras, w, cx| this.navigate(Section::Eras.into(), w, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowWrapped, w, cx| {
                this.navigate(Section::Wrapped.into(), w, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowConcerts, w, cx| {
                this.navigate(Section::Concerts.into(), w, cx)
            }))
            .on_action(cx.listener(|this, a: &OpenArtist, w, cx| {
                this.navigate(Page::Artist(a.id.clone()), w, cx)
            }))
            .on_action(cx.listener(|this, a: &OpenTrack, w, cx| {
                this.navigate(Page::Track(a.id.clone()), w, cx)
            }))
            .on_action(cx.listener(|this, a: &ShowPlaysOfTrack, w, cx| {
                let scope = Scope::Song {
                    id: a.id.clone(),
                    name: a.name.clone(),
                };
                this.show_plays(scope, w, cx)
            }))
            .on_action(cx.listener(|this, a: &ShowPlaysOfArtist, w, cx| {
                let scope = Scope::Artist {
                    id: a.id.clone(),
                    name: a.name.clone(),
                };
                this.show_plays(scope, w, cx)
            }))
            .on_action(cx.listener(Self::on_export_csv))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::on_focus_filter))
            .on_action(cx.listener(Self::on_go_to_date))
            .on_action(cx.listener(Self::on_open_palette))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_open_settings))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_sidebar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.render_top_bar(cx))
                    .child(div().flex_1().min_h_0().child(self.current_view())),
            )
    }
}
