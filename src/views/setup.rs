use gpui::AppContext as _;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit_assets::IconName;
use url::Url;

use crate::api::{self, ApiClient, ApiError, Secret};
use crate::config::{self, Config};
use crate::dates;
use crate::runtime;
use crate::views::widgets;

pub enum SetupEvent {
    Saved(ApiClient),
    Cancelled,
}

enum Status {
    Idle,
    Testing,
    /// The URL and token that passed, so Save stores exactly what was tested.
    Passed {
        url: Url,
        token: Secret,
        summary: SharedString,
    },
    Failed(SharedString),
}

/// First-run setup, and Settings later. The token field may be left blank
/// when a token is already in the Keychain, so changing only the URL does not
/// require pasting the secret again.
pub struct SetupView {
    url_input: Entity<InputState>,
    token_input: Entity<InputState>,
    existing_token: Option<Secret>,
    can_cancel: bool,
    notice: Option<SharedString>,
    status: Status,
    focus: FocusHandle,
    _task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SetupEvent> for SetupView {}

impl Focusable for SetupView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SetupView {
    pub fn new(
        prefill: Option<Config>,
        existing_token: Option<Secret>,
        can_cancel: bool,
        notice: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let url_input = cx.new(|cx| {
            let state = InputState::new(window, cx).placeholder("https://music-api.example.com");
            match &prefill {
                Some(config) => state.default_value(config.worker_url.clone()),
                None => state,
            }
        });
        let token_placeholder = if existing_token.is_some() {
            "Saved in Keychain. Leave blank to keep it"
        } else {
            "READ_TOKEN"
        };
        let token_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(token_placeholder)
                .masked(true)
        });

        let subscriptions = vec![
            cx.subscribe_in(&url_input, window, Self::on_input),
            cx.subscribe_in(&token_input, window, Self::on_input),
        ];
        url_input.update(cx, |input, cx| input.focus(window, cx));

        Self {
            url_input,
            token_input,
            existing_token,
            can_cancel,
            notice,
            status: Status::Idle,
            focus: cx.focus_handle(),
            _task: None,
            _subscriptions: subscriptions,
        }
    }

    fn on_input(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // A passed test only vouches for the values that were tested.
            InputEvent::Change => {
                if !matches!(self.status, Status::Idle | Status::Testing) {
                    self.status = Status::Idle;
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                if matches!(self.status, Status::Passed { .. }) {
                    self.save(window, cx);
                } else {
                    self.test(window, cx);
                }
            }
            _ => {}
        }
    }

    fn test(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let url = match api::parse_base_url(&self.url_input.read(cx).value()) {
            Ok(url) => url,
            Err(message) => return self.fail(message, cx),
        };
        let typed = Secret::new(self.token_input.read(cx).value().to_string());
        let token = match (typed.is_empty(), &self.existing_token) {
            (false, _) => typed,
            (true, Some(existing)) => existing.clone(),
            (true, None) => return self.fail("Enter the READ_TOKEN.".into(), cx),
        };

        self.status = Status::Testing;
        cx.notify();
        let client = ApiClient::new(url.clone(), token.clone());
        self._task = Some(cx.spawn(async move |this, cx| {
            let outcome = runtime::run(async move {
                // Refuse an admin token before anything else: it would pass
                // every /api check and then sit in the Keychain for good.
                if client.token_is_admin().await? {
                    return Ok(None);
                }
                client.plays(1, None).await.map(Some)
            })
            .await;
            this.update(cx, |this, cx| {
                this.status = match outcome {
                    Ok(None) => Status::Failed(
                        "That is the ADMIN_TOKEN. This app only needs READ_TOKEN, which \
                         cannot trigger polls or write rows. Paste READ_TOKEN instead."
                            .into(),
                    ),
                    Ok(Some(plays)) => Status::Passed {
                        url,
                        token,
                        summary: match plays.first() {
                            Some(play) => format!(
                                "Connected. Newest play: {} ({}).",
                                play.track_name.as_deref().unwrap_or("unknown track"),
                                dates::format_played_at(play.played_at_ms, &chrono::Local)
                            )
                            .into(),
                            None => "Connected. The warehouse has no plays yet.".into(),
                        },
                    },
                    Err(ApiError::Unauthorized) => Status::Failed(
                        "The Worker rejected that token. Check it is the Worker's READ_TOKEN."
                            .into(),
                    ),
                    Err(error) => {
                        let copy = widgets::describe(&error);
                        Status::Failed(format!("{}: {}", copy.title, copy.detail).into())
                    }
                };
                cx.notify();
            })
            .ok();
        }));
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = Status::Failed(message.into());
        cx.notify();
    }

    fn save(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Status::Passed { url, token, .. } = &self.status else {
            return;
        };
        let (url, token) = (url.clone(), token.clone());
        let Some(dir) = config::config_dir() else {
            return self.fail("Could not find Application Support.".into(), cx);
        };
        let config = Config {
            worker_url: url.to_string(),
        };
        if let Err(e) = config::save_to(&dir, &config) {
            return self.fail(format!("Could not save settings: {e}"), cx);
        }
        // Only touch the Keychain when a new token was typed, so keeping the
        // saved one does not re-trigger an access prompt.
        if self.existing_token.as_ref() != Some(&token)
            && let Err(message) = config::save_token(&token)
        {
            return self.fail(message, cx);
        }
        cx.emit(SetupEvent::Saved(ApiClient::new(url, token)));
    }
}

impl Render for SetupView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let passed = matches!(self.status, Status::Passed { .. });
        let testing = matches!(self.status, Status::Testing);
        let status_line = match &self.status {
            Status::Idle => None,
            Status::Testing => Some((IconName::LoaderCircle, "Testing…".into(), false)),
            Status::Passed { summary, .. } => Some((IconName::CircleCheck, summary.clone(), false)),
            Status::Failed(message) => Some((IconName::TriangleAlert, message.clone(), true)),
        };

        v_flex()
            .track_focus(&self.focus)
            .size_full()
            .items_center()
            .justify_center()
            .bg(cx.theme().muted)
            .child(
                v_flex()
                    .w(gpui::px(480.))
                    .gap_4()
                    .p_6()
                    .rounded_xl()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(IconName::Disc3).size_6())
                            .child(
                                div()
                                    .text_xl()
                                    .font_semibold()
                                    .child("Connect to music-warehouse"),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "Enter your Worker's URL and its READ_TOKEN. The URL is saved \
                                 under Application Support; the token is stored only in the \
                                 macOS Keychain.",
                            ),
                    )
                    .when_some(self.notice.clone(), |this, notice| {
                        this.child(
                            div()
                                .text_sm()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .bg(cx.theme().danger.opacity(0.1))
                                .child(notice),
                        )
                    })
                    .child(field("Worker URL", Input::new(&self.url_input), cx))
                    .child(field(
                        "Read token",
                        Input::new(&self.token_input).mask_toggle(),
                        cx,
                    ))
                    .when_some(status_line, |this, (icon, text, is_error)| {
                        this.child(
                            h_flex()
                                .gap_2()
                                .items_start()
                                .text_sm()
                                .when(is_error, |this| this.text_color(cx.theme().danger))
                                .child(Icon::new(icon).small().mt_0p5())
                                .child(div().flex_1().min_w_0().child(text)),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .when(self.can_cancel, |this| {
                                this.child(Button::new("cancel").ghost().label("Cancel").on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(SetupEvent::Cancelled)),
                                ))
                            })
                            .child(
                                Button::new("test")
                                    .label("Test connection")
                                    .loading(testing)
                                    .disabled(testing)
                                    .when(!passed, |b| b.primary())
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.test(window, cx)),
                                    ),
                            )
                            .child(
                                Button::new("save")
                                    .label("Save")
                                    .disabled(!passed)
                                    .when(passed, |b| b.primary())
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.save(window, cx)),
                                    ),
                            ),
                    ),
            )
    }
}

fn field(label: &'static str, input: Input, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_sm()
                .font_medium()
                .text_color(cx.theme().foreground)
                .child(label),
        )
        .child(input)
}
