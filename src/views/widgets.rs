//! Loading, empty and error presentations shared by every view, so each
//! failure reads the same wherever it surfaces.

use gpui::{
    AnyElement, App, ClickEvent, ElementId, IntoElement, ParentElement, SharedString, Styled,
    Window, div, prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme, Icon, Sizable, StyledExt, button::Button, h_flex, spinner::Spinner, v_flex,
};
use gpui_kit_assets::IconName;

use crate::api::ApiError;

/// What to tell the user about an error: a headline, then what to do.
pub struct ErrorCopy {
    pub title: SharedString,
    pub detail: SharedString,
}

pub fn describe(error: &ApiError) -> ErrorCopy {
    let (title, detail): (&str, String) = match error {
        ApiError::Unauthorized => (
            "Read token rejected",
            "The Worker did not accept the saved READ_TOKEN. Open Settings (⌘,) to enter it again."
                .into(),
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
        ApiError::Server { status, message } => {
            ("Worker error", format!("HTTP {status}: {message}"))
        }
        ApiError::Network(message) => ("Can't reach the Worker", message.clone()),
        ApiError::Decode(message) => (
            "Unexpected response",
            format!("The Worker's response did not match what this app expects: {message}"),
        ),
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
    let icon = if error.is_spotify_side() {
        IconName::CloudOff
    } else {
        IconName::TriangleAlert
    };
    state_panel(
        Some(Icon::new(icon)),
        copy.title,
        Some(copy.detail),
        Some(
            Button::new(id)
                .small()
                .label("Retry")
                .on_click(on_retry)
                .into_any_element(),
        ),
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
        .bg(cx.theme().danger.opacity(0.1))
        .text_sm()
        .child(Icon::new(IconName::TriangleAlert).small())
        .child(div().font_semibold().child(copy.title))
        .child(
            div()
                .flex_1()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(copy.detail),
        )
        .child(Button::new(id).xsmall().label("Retry").on_click(on_retry))
}

pub fn card(title: impl Into<SharedString>, cx: &App) -> gpui::Div {
    v_flex()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .bg(cx.theme().background)
        .p_4()
        .gap_3()
        .child(div().font_semibold().child(title.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_seconds_rounds_up() {
        assert_eq!(human_seconds(30), "30s");
        assert_eq!(human_seconds(91), "2 min");
        assert_eq!(human_seconds(3875), "65 min");
        assert_eq!(human_seconds(86_088), "24 h");
    }

    #[test]
    fn live_failures_get_their_own_copy() {
        assert_eq!(
            describe(&ApiError::NeedsReauth).title.as_ref(),
            "Spotify needs re-authorizing"
        );
        assert!(
            describe(&ApiError::RateLimited {
                retry_after_seconds: Some(120)
            })
            .detail
            .contains("2 min")
        );
        assert_eq!(
            describe(&ApiError::Upstream("x".into())).title.as_ref(),
            "Spotify is unavailable"
        );
    }
}
