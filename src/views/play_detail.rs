//! Side panel for the selected History row. Everything shown comes from the
//! stored row itself; nothing is fetched or derived across rows.

use gpui::{
    AnyElement, App, ClickEvent, IntoElement, ParentElement, SharedString, Styled, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit_assets::IconName;

use crate::dates;
use crate::models::{Play, Rendition, album_rendition};
use crate::views::widgets;

/// Web link rather than a `spotify:` URI: it opens the desktop app when one
/// is installed and still works in a browser when it is not.
pub fn track_url(track_id: &str) -> String {
    format!("https://open.spotify.com/track/{track_id}")
}

/// Plain text for the clipboard, in the order people paste it into chats.
pub fn copy_text(play: &Play) -> String {
    let track = play.track_name.as_deref().unwrap_or("Unknown track");
    match play.artists.as_deref().filter(|a| !a.is_empty()) {
        Some(artists) => format!("{track} — {artists}\n{}", track_url(&play.track_id)),
        None => format!("{track}\n{}", track_url(&play.track_id)),
    }
}

/// Where the play started, plus a link to it when Spotify has a page for it.
/// The warehouse stores the context URI but not its name, so playlists are
/// described by kind only.
pub fn context_line(play: &Play) -> (String, Option<String>) {
    if play.source.as_deref() == Some("export") {
        return (
            "Imported from Spotify's streaming history export".into(),
            None,
        );
    }
    let link = play.context_uri.as_deref().and_then(context_url);
    let label = match play.context_type.as_deref() {
        Some("album") => "Played from the album",
        Some("playlist") => "Played from a playlist",
        Some("artist") => "Played from the artist's page",
        Some("collection") => "Played from Liked Songs",
        Some("show") => "Played from a show",
        Some(_) => "Played from Spotify",
        // Roughly a third of live plays: search results, the queue or radio.
        None => "Played from search, the queue or radio",
    };
    (label.into(), link)
}

/// `spotify:playlist:abc` → `https://open.spotify.com/playlist/abc`.
fn context_url(uri: &str) -> Option<String> {
    let mut parts = uri.split(':');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("spotify"), Some(kind), Some(id), None)
            if matches!(kind, "album" | "playlist" | "artist" | "show") && !id.is_empty() =>
        {
            Some(format!("https://open.spotify.com/{kind}/{id}"))
        }
        _ => None,
    }
}

pub fn format_duration(ms: i64) -> String {
    let seconds = (ms.max(0) + 500) / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub struct DetailActions {
    pub on_open: ClickHandler,
    pub on_copy: ClickHandler,
    pub on_close: ClickHandler,
}

pub fn render(play: &Play, actions: DetailActions, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let art = play
        .image_url
        .as_deref()
        .map(|url| SharedString::from(album_rendition(url, Rendition::Px300)));
    let (context, context_link) = context_line(play);
    let played_at = dates::format_played_at(play.played_at_ms, &chrono::Local);

    let fact = |label: &'static str, value: String| {
        v_flex()
            .gap_0p5()
            .child(div().text_xs().text_color(muted).child(label))
            .child(div().text_sm().child(value))
    };

    v_flex()
        .w(px(300.))
        .h_full()
        .flex_shrink_0()
        .gap_4()
        .p_4()
        .border_l_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(
            h_flex()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(muted)
                        .child("PLAY"),
                )
                .child(
                    Button::new("detail-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("Close (Esc)")
                        .on_click(actions.on_close),
                ),
        )
        .child(widgets::artwork(art, px(268.), false, cx))
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_lg()
                        .font_semibold()
                        .child(play.track_name.clone().unwrap_or("Unknown track".into())),
                )
                .when_some(play.artists.clone(), |this, artists| {
                    this.child(div().text_sm().child(artists))
                })
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(play.album_name.clone().unwrap_or("No album info".into())),
                ),
        )
        .child(fact("Played", played_at))
        .when_some(play.duration_ms, |this, ms| {
            this.child(fact("Length", format_duration(ms)))
        })
        .child(fact("Source", context))
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("detail-open")
                        .small()
                        .primary()
                        .icon(IconName::ExternalLink)
                        .label("Open in Spotify")
                        .tooltip("o")
                        .on_click(actions.on_open),
                )
                .child(
                    Button::new("detail-copy")
                        .small()
                        .icon(IconName::Copy)
                        .label("Copy")
                        .tooltip("⌘C")
                        .on_click(actions.on_copy),
                ),
        )
        .when_some(context_link, |this, link| {
            this.child(
                Button::new("detail-context")
                    .xsmall()
                    .ghost()
                    .label("Open where it was played from")
                    .on_click(move |_, _, cx| cx.open_url(&link)),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play() -> Play {
        Play {
            played_at_ms: 0,
            track_id: "67XC51nlZncNpHmZ8rOU9a".into(),
            context_uri: Some("spotify:album:79ONNoS4M9tfIA1mYLBYVX".into()),
            context_type: Some("album".into()),
            source: Some("api".into()),
            track_name: Some("We Cry Together".into()),
            duration_ms: Some(341_307),
            album_name: None,
            image_url: None,
            artists: Some("Kendrick Lamar, Taylour Paige".into()),
        }
    }

    #[test]
    fn copy_text_has_credits_and_link() {
        assert_eq!(
            copy_text(&play()),
            "We Cry Together — Kendrick Lamar, Taylour Paige\n\
             https://open.spotify.com/track/67XC51nlZncNpHmZ8rOU9a"
        );
    }

    #[test]
    fn context_links_only_to_pages_spotify_has() {
        let (label, link) = context_line(&play());
        assert_eq!(label, "Played from the album");
        assert_eq!(
            link.as_deref(),
            Some("https://open.spotify.com/album/79ONNoS4M9tfIA1mYLBYVX")
        );
        // Liked Songs has a URI but no public page.
        assert_eq!(context_url("spotify:user:me:collection"), None);
        assert_eq!(context_url("not a uri"), None);
    }

    #[test]
    fn export_rows_and_missing_context_are_described() {
        let mut p = play();
        p.context_type = None;
        p.context_uri = None;
        assert_eq!(context_line(&p).0, "Played from search, the queue or radio");
        p.source = Some("export".into());
        assert!(context_line(&p).0.contains("export"));
    }

    #[test]
    fn durations_round_to_the_second() {
        assert_eq!(format_duration(341_307), "5:41");
        assert_eq!(format_duration(59_600), "1:00");
        assert_eq!(format_duration(0), "0:00");
    }
}
