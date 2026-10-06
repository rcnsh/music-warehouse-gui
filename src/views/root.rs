use gpui::AppContext as _;
use gpui::{
    Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    Window, div,
};

use crate::api::{self, ApiClient, Secret};
use crate::config::{self, Config};
use crate::ui_state;
use crate::views::setup::{SetupEvent, SetupView};
use crate::views::shell::{Shell, ShellEvent};
use crate::views::widgets;

enum Screen {
    /// Only for the instant between construction and the first screen choice.
    Starting,
    Setup(Entity<SetupView>),
    Main(Entity<Shell>),
}

/// Chooses between first-run setup and the main shell, and swaps them when
/// settings are saved. Rebuilding the shell on save means every store picks
/// up the new client together; none can keep talking to the old Worker.
pub struct AppRoot {
    screen: Screen,
    /// The client the shell was built with, so cancelling Settings restores it.
    client: Option<ApiClient>,
    _subscription: Option<Subscription>,
    _window_bounds: Subscription,
}

impl AppRoot {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root = Self {
            screen: Screen::Starting,
            client: None,
            _subscription: None,
            // Observed here rather than in the shell so the frame is kept
            // from setup and settings too.
            _window_bounds: cx.observe_window_bounds(window, |_, window, cx| {
                let (bounds, content) = (window.window_bounds(), window.viewport_size());
                ui_state::update(cx, |state| {
                    let saved = ui_state::SavedWindow::capture(bounds, content, state.window);
                    state.window = Some(saved);
                });
            }),
        };
        // Reading the Keychain blocks while macOS shows an access prompt
        // (after every rebuild, for an unsigned binary), so it runs off the
        // main thread and the window stays responsive meanwhile.
        cx.spawn_in(window, async move |this, cx| {
            let saved = cx.background_spawn(async { load_saved() }).await;
            this.update_in(cx, |this, window, cx| this.apply_saved(saved, window, cx))
                .ok();
        })
        .detach();
        root
    }

    fn apply_saved(&mut self, saved: Saved, window: &mut Window, cx: &mut Context<Self>) {
        let (config, token, notice) = saved;
        match (&config, &token) {
            (Some(config), Some(token)) if notice.is_none() => {
                match api::parse_base_url(&config.worker_url) {
                    Ok(url) => self.show_main(ApiClient::new(url, token.clone()), window, cx),
                    Err(message) => self.show_setup(
                        Some(config.clone()),
                        Some(token.clone()),
                        Some(format!("The saved Worker URL is invalid: {message}").into()),
                        window,
                        cx,
                    ),
                }
            }
            _ => self.show_setup(config, token, notice, window, cx),
        }
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let (config, token, _) = cx.background_spawn(async { load_saved() }).await;
            this.update_in(cx, |this, window, cx| {
                this.show_setup(config, token, None, window, cx)
            })
            .ok();
        })
        .detach();
    }

    fn show_main(&mut self, client: ApiClient, window: &mut Window, cx: &mut Context<Self>) {
        let shell = cx.new(|cx| Shell::new(client.clone(), window, cx));
        self._subscription = Some(
            cx.subscribe_in(&shell, window, |this, _, event, window, cx| match event {
                ShellEvent::OpenSettings => this.open_settings(window, cx),
            }),
        );
        self.client = Some(client);
        self.screen = Screen::Main(shell);
        cx.notify();
    }

    fn show_setup(
        &mut self,
        config: Option<Config>,
        token: Option<Secret>,
        notice: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let can_cancel = self.client.is_some();
        let setup = cx.new(|cx| SetupView::new(config, token, can_cancel, notice, window, cx));
        self._subscription = Some(
            cx.subscribe_in(&setup, window, |this, _, event, window, cx| match event {
                SetupEvent::Saved(client) => this.show_main(client.clone(), window, cx),
                SetupEvent::Cancelled => {
                    if let Some(client) = this.client.clone() {
                        this.show_main(client, window, cx);
                    }
                }
            }),
        );
        self.screen = Screen::Setup(setup);
        cx.notify();
    }
}

/// Reads saved settings. A failure to read is reported on the setup screen
/// rather than treated as first run, so a locked Keychain is explained.
type Saved = (Option<Config>, Option<Secret>, Option<SharedString>);

fn load_saved() -> Saved {
    let mut notice = None;
    let config = match config::config_dir().map(|dir| config::load_from(&dir)) {
        Some(Ok(config)) => config,
        Some(Err(e)) => {
            notice = Some(format!("Could not read saved settings: {e}").into());
            None
        }
        None => None,
    };
    // Skip the Keychain entirely on first run, so a fresh install asks for
    // nothing before the user has typed anything.
    let token = if config.is_some() {
        match config::load_token() {
            Ok(token) => token,
            Err(message) => {
                notice = Some(message.into());
                None
            }
        }
    } else {
        None
    };
    (config, token, notice)
}

impl Render for AppRoot {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(match &self.screen {
            Screen::Starting => {
                widgets::loading_panel("Reading settings from the Keychain…", cx).into_any_element()
            }
            Screen::Setup(setup) => setup.clone().into_any_element(),
            Screen::Main(shell) => shell.clone().into_any_element(),
        })
    }
}
