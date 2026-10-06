use gpui::AppContext as _;
use gpui::{
    Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    Window, div,
};

use crate::api::{self, ApiClient, Secret};
use crate::config::{self, Config};
use crate::views::setup::{SetupEvent, SetupView};
use crate::views::shell::{Shell, ShellEvent};

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
}

impl AppRoot {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut root = Self {
            screen: Screen::Starting,
            client: None,
            _subscription: None,
        };
        let (config, token, notice) = load_saved();
        match (&config, &token) {
            (Some(config), Some(token)) if notice.is_none() => {
                match api::parse_base_url(&config.worker_url) {
                    Ok(url) => root.show_main(ApiClient::new(url, token.clone()), window, cx),
                    Err(message) => root.show_setup(
                        Some(config.clone()),
                        Some(token.clone()),
                        Some(format!("The saved Worker URL is invalid: {message}").into()),
                        window,
                        cx,
                    ),
                }
            }
            _ => root.show_setup(config, token, notice, window, cx),
        }
        root
    }

    fn show_main(&mut self, client: ApiClient, window: &mut Window, cx: &mut Context<Self>) {
        let shell = cx.new(|cx| Shell::new(client.clone(), window, cx));
        self._subscription = Some(
            cx.subscribe_in(&shell, window, |this, _, event, window, cx| match event {
                ShellEvent::OpenSettings => {
                    let (config, token, _) = load_saved();
                    this.show_setup(config, token, None, window, cx);
                }
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
fn load_saved() -> (Option<Config>, Option<Secret>, Option<SharedString>) {
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
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(match &self.screen {
            Screen::Starting => div().into_any_element(),
            Screen::Setup(setup) => setup.clone().into_any_element(),
            Screen::Main(shell) => shell.clone().into_any_element(),
        })
    }
}
