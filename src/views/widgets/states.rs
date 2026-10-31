//! Loading, empty and error states, so each failure reads the same wherever
//! it surfaces.

use super::*;

/// What to tell the user about an error: a headline, then what to do.
pub struct ErrorCopy {
    pub title: SharedString,
    pub detail: SharedString,
}

pub fn describe(error: &ApiError) -> ErrorCopy {
    let (title, detail): (&str, String) = match error {
        ApiError::Unauthorized => (
            "Read token rejected",
            format!(
                "The Worker no longer accepts the saved token. Tokens from signing in expire \
                 after 90 days. Open Settings ({}) to sign in with Spotify again or paste a \
                 token.",
                super::shortcut(',')
            ),
        ),
        ApiError::NeedsReauth => (
            "Spotify needs re-authorizing",
            "The Worker's Spotify grant has expired. Stored history still works; run \
             `npm run authorize` in music-warehouse to restore live data."
                .into(),
        ),
        ApiError::RateLimited {
            retry_after_seconds,
        } => (
            "Spotify is rate limiting",
            match retry_after_seconds {
                Some(s) => format!("Spotify asked the Worker to wait {}.", human_seconds(*s)),
                None => "Spotify asked the Worker to slow down. It will retry shortly.".into(),
            },
        ),
        ApiError::Upstream(message) => (
            "Spotify is unavailable",
            format!("The Worker could not get an answer from Spotify ({message})."),
        ),
        ApiError::BadRequest(message) => ("Request refused", message.clone()),
        ApiError::StillComputing {
            retry_after_seconds,
        } => (
            "Still computing…",
            format!(
                "The Worker is working this out for the first time. It asked to be \
                 asked again in {}; this page does that by itself.",
                human_seconds(*retry_after_seconds)
            ),
        ),
        ApiError::Server { status, message } => {
            ("Worker error", format!("HTTP {status}: {message}"))
        }
        ApiError::Network(message) => ("Can't reach the Worker", message.clone()),
        ApiError::Decode(message) => (
            "Unexpected response",
            format!("The Worker's response did not match what this app expects: {message}"),
        ),
        ApiError::File(message) => ("Could not save the file", message.clone()),
    };
    ErrorCopy {
        title: title.into(),
        detail: detail.into(),
    }
}

pub fn human_seconds(seconds: u64) -> String {
    match seconds {
        0..=89 => format!("{seconds}s"),
        90..=5399 => format!("{} min", seconds.div_ceil(60)),
        _ => format!("{} h", seconds.div_ceil(3600)),
    }
}

/// A centred message filling its container: the shape every loading, empty
/// and error state takes.
pub fn state_panel(
    icon: Option<Icon>,
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    action: Option<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .p_6()
        .text_center()
        .when_some(icon, |this, icon| {
            this.child(icon.size_8().text_color(cx.theme().muted_foreground))
        })
        .child(div().font_semibold().child(title.into()))
        .when_some(detail, |this, detail| {
            this.child(
                div()
                    .max_w_96()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail),
            )
        })
        .when_some(action, |this, action| {
            this.child(div().pt_2().child(action))
        })
}

pub fn loading_panel(label: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(Spinner::new().large())
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
}

pub fn error_panel(
    id: impl Into<ElementId>,
    error: &ApiError,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let copy = describe(error);
    let icon = error_icon(error);
    let button = retry_button(id, error, on_retry).small();
    state_panel(
        Some(Icon::new(icon)),
        copy.title,
        Some(copy.detail),
        Some(button.into_any_element()),
        cx,
    )
}

/// A one-line banner for an error that sits above still-useful content.
pub fn error_banner(
    id: impl Into<ElementId>,
    error: &ApiError,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let copy = describe(error);
    h_flex()
        .gap_2()
        .px_3()
        .py_1p5()
        .rounded_md()
        .map(|this| match error {
            // Not a failure: the page fills in by itself once it is computed.
            ApiError::StillComputing { .. } => this.bg(cx.theme().muted),
            _ => this.bg(cx.theme().danger.opacity(0.1)),
        })
        .text_sm()
        .child(Icon::new(error_icon(error)).small())
        .child(div().font_semibold().child(copy.title))
        .child(
            div()
                .flex_1()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(copy.detail),
        )
        .child(retry_button(id, error, on_retry).xsmall())
}

fn error_icon(error: &ApiError) -> IconName {
    match error {
        ApiError::StillComputing { .. } => IconName::LoaderCircle,
        error if error.is_spotify_side() => IconName::CloudOff,
        _ => IconName::TriangleAlert,
    }
}

/// Retry, or "Sign in again" for a rejected token: retrying with it cannot
/// work, signing in again can.
fn retry_button(
    id: impl Into<ElementId>,
    error: &ApiError,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    if *error == ApiError::Unauthorized {
        Button::new(id)
            .label("Sign in again")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx))
    } else {
        Button::new(id).label("Retry").on_click(on_retry)
    }
}

/// Renders a [`Remote`]'s three states in a pane of fixed size (Overview's
/// panels): a spinner before the first answer, the error panel if that
/// failed, else `content` with a retry banner above it when a later refresh
/// failed. Everything stretches to fill the pane; inside a scrolling [`page`]
/// use [`page_body`] instead.
pub fn remote_view<T>(
    id: impl Into<ElementId>,
    remote: &Remote<T>,
    loading_label: impl Into<SharedString>,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
    content: impl FnOnce(&T) -> AnyElement,
) -> AnyElement {
    match (&remote.data, &remote.error) {
        (None, None) => loading_panel(loading_label, cx).into_any_element(),
        (None, Some(error)) => error_panel(id, error, on_retry, cx).into_any_element(),
        (Some(data), None) => content(data),
        (Some(data), Some(error)) => v_flex()
            .size_full()
            .gap_2()
            .child(error_banner(id, error, on_retry, cx))
            .child(div().flex_1().min_h_0().child(content(data)))
            .into_any_element(),
    }
}

/// How tall a loading, error or empty panel is inside a scrolling [`page`],
/// which has no height of its own for the panel to centre in.
pub const PAGE_PANEL_HEIGHT: Pixels = px(320.);

/// [`remote_view`] for a scrolling [`page`]: the loading and error panels get
/// [`PAGE_PANEL_HEIGHT`] to centre in, and loaded content keeps its natural
/// height (under a retry banner when a refresh failed) rather than being
/// squeezed to fit the window.
pub fn page_body<T>(
    id: impl Into<ElementId>,
    remote: &Remote<T>,
    loading_label: impl Into<SharedString>,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
    content: impl FnOnce(&T) -> AnyElement,
) -> AnyElement {
    match (&remote.data, &remote.error) {
        (Some(data), error) => v_flex()
            .gap_3()
            .when_some(error.as_ref(), |this, error| {
                this.child(error_banner(id, error, on_retry, cx))
            })
            .child(content(data))
            .into_any_element(),
        (None, Some(error)) => div()
            .h(PAGE_PANEL_HEIGHT)
            .child(error_panel(id, error, on_retry, cx))
            .into_any_element(),
        (None, None) => div()
            .h(PAGE_PANEL_HEIGHT)
            .child(loading_panel(loading_label, cx))
            .into_any_element(),
    }
}

/// An empty or explanatory [`state_panel`] inside a scrolling [`page`], at
/// [`PAGE_PANEL_HEIGHT`].
pub fn page_state(
    icon: Option<IconName>,
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    cx: &App,
) -> AnyElement {
    div()
        .h(PAGE_PANEL_HEIGHT)
        .child(state_panel(icon.map(Icon::new), title, detail, None, cx))
        .into_any_element()
}
