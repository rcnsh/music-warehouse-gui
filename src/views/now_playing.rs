use gpui::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{ActiveTheme, Icon, Sizable, StyledExt, h_flex, spinner::Spinner};
use gpui_kit_assets::IconName;

use crate::api::ApiError;
use crate::models::{CurrentlyPlaying, join_names, pick_image};
use crate::state::now_playing::NowPlayingStore;
use crate::views::widgets::{self, human_seconds};

/// One line of status at the top of the window. Live failures are expected
/// states of the Worker's proxy, so they read as status rather than alarms.
pub struct NowPlayingStrip {
    store: Entity<NowPlayingStore>,
}

impl NowPlayingStrip {
    pub fn new(store: Entity<NowPlayingStore>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self { store }
    }
}

enum Tone {
    Normal,
    Muted,
    Warning,
}

struct Line {
    icon: IconName,
    art: Option<SharedString>,
    title: SharedString,
    detail: Option<SharedString>,
    tone: Tone,
}

fn describe_playing(current: &CurrentlyPlaying) -> Line {
    let icon = if current.is_playing {
        IconName::Play
    } else {
        IconName::Pause
    };
    let state = if current.is_playing { "" } else { "Paused · " };
    let Some(item) = &current.item else {
        // Spotify sends no item during ads and some private sessions.
        let title = match current.currently_playing_type.as_deref() {
            Some("ad") => "Spotify is playing an ad",
            _ => "Playing something Spotify won't describe",
        };
        return Line {
            icon,
            art: None,
            title: title.into(),
            detail: None,
            tone: Tone::Muted,
        };
    };
    let name = item.name.clone().unwrap_or_else(|| "Unknown".into());
    let detail = match &item.show {
        Some(show) => show.name.clone(),
        None => {
            let artists = join_names(&item.artists);
            let album = item.album.as_ref().and_then(|a| a.name.clone());
            match (artists.is_empty(), album) {
                (false, Some(album)) => Some(format!("{artists} · {album}")),
                (false, None) => Some(artists),
                (true, album) => album,
            }
        }
    };
    Line {
        icon,
        art: pick_image(item.artwork(), 48).map(|url| url.to_owned().into()),
        title: format!("{state}{name}").into(),
        detail: detail.map(Into::into),
        tone: if current.is_playing {
            Tone::Normal
        } else {
            Tone::Muted
        },
    }
}

fn describe_error(error: &ApiError, retry_at: Option<String>) -> Line {
    let retry = retry_at.map(|t| format!("Next check {t}"));
    let (icon, title, detail): (IconName, &str, Option<String>) = match error {
        ApiError::NeedsReauth => (
            IconName::KeyRound,
            "Spotify needs re-authorizing",
            Some("History and charts still work".into()),
        ),
        ApiError::RateLimited {
            retry_after_seconds,
        } => (
            IconName::Hourglass,
            "Spotify rate limit",
            match (retry_after_seconds, retry) {
                (Some(s), _) => Some(format!("Spotify asked for {}", human_seconds(*s))),
                (None, retry) => retry,
            },
        ),
        ApiError::Upstream(_) => (IconName::CloudOff, "Spotify unavailable", retry),
        ApiError::Unauthorized => (
            IconName::KeyRound,
            "Read token rejected",
            Some("Open Settings (⌘,)".into()),
        ),
        ApiError::Network(_) => (IconName::WifiOff, "Can't reach the Worker", retry),
        _ => (IconName::TriangleAlert, "Now playing unavailable", retry),
    };
    Line {
        icon,
        art: None,
        title: title.into(),
        detail: detail.map(Into::into),
        tone: Tone::Warning,
    }
}

impl Render for NowPlayingStrip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let retry_at = store
            .next_poll_at
            .map(|t| format!("at {}", t.format("%H:%M")));

        let line = match (&store.error, &store.last) {
            (Some(error), _) => Some(describe_error(error, retry_at)),
            (None, Some(response)) => Some(match &response.item {
                Some(current) => describe_playing(current),
                None => Line {
                    icon: IconName::Music,
                    art: None,
                    title: "Nothing playing".into(),
                    detail: None,
                    tone: Tone::Muted,
                },
            }),
            (None, None) => None,
        };

        let theme = cx.theme();
        let (fg, muted, warn) = (theme.foreground, theme.muted_foreground, theme.warning);
        h_flex()
            .id("now-playing")
            .gap_2()
            .min_w_0()
            .text_sm()
            .map(|this| match line {
                None => this
                    .child(Spinner::new().small())
                    .child(div().text_color(muted).child("Checking Spotify…")),
                Some(line) => {
                    let color = match line.tone {
                        Tone::Normal => fg,
                        Tone::Muted => muted,
                        Tone::Warning => warn,
                    };
                    let art = line
                        .art
                        .map(|url| widgets::artwork(Some(url), px(22.), false, cx));
                    this.when_some(art, |this, art| this.child(art))
                        .child(Icon::new(line.icon).small().text_color(color))
                        .child(
                            div()
                                .flex_shrink_0()
                                .max_w(gpui::px(360.))
                                .truncate()
                                .font_medium()
                                .text_color(color)
                                .child(line.title),
                        )
                        .when_some(line.detail, |this, detail| {
                            this.child(div().truncate().text_color(muted).child(detail))
                        })
                }
            })
            .when(store.loading && store.last.is_some(), |this| {
                this.child(Spinner::new().xsmall().color(muted))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::decode;
    use crate::models::NowPlayingResponse;

    #[test]
    fn playing_track_reads_title_then_credits() {
        let body = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/now-playing.json"
        ))
        .unwrap();
        let response: NowPlayingResponse = decode(&body).unwrap();
        let line = describe_playing(response.item.as_ref().unwrap());
        assert_eq!(line.title.as_ref(), "Count Me Out");
        assert_eq!(
            line.detail.as_deref(),
            Some("Kendrick Lamar · Mr. Morale & The Big Steppers")
        );
    }

    #[test]
    fn ad_without_item_is_described() {
        let current = CurrentlyPlaying {
            is_playing: true,
            currently_playing_type: Some("ad".into()),
            item: None,
        };
        assert_eq!(
            describe_playing(&current).title.as_ref(),
            "Spotify is playing an ad"
        );
    }

    #[test]
    fn reauth_keeps_history_reassurance() {
        let line = describe_error(&ApiError::NeedsReauth, None);
        assert!(line.detail.unwrap().contains("History"));
    }
}
