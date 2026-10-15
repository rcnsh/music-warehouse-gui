//! The Concerts page (`/api/concerts`): upcoming shows for the most-played
//! artists in each region the Worker watches. Regions are whatever the
//! deployment configured (CONCERT_REGIONS), so every name shown comes from
//! the response, never from this app.

use chrono::{DateTime, NaiveDate, Utc};
use gpui::AppContext as _;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div, prelude::FluentBuilder,
    px,
};
use gpui_component::{
    ActiveTheme, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    tag::Tag,
    v_flex,
};
use gpui_kit_assets::IconName;

use crate::api::ApiClient;
use crate::models::{Concert, ConcertsResponse};
use crate::state::concerts::{ConcertsStore, NEW_DAYS};
use crate::views::widgets;

pub struct ConcertsView {
    store: Entity<ConcertsStore>,
    /// A region key from the response; `None` shows every region.
    region: Option<String>,
    focus: FocusHandle,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl ConcertsView {
    pub fn new(client: ApiClient, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| ConcertsStore::new(client, cx));
        let subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            region: None,
            focus: cx.focus_handle(),
            activated: false,
            _subscriptions: subscriptions,
        }
    }

    /// Called every time the page is shown, after the shell focuses it.
    /// Loads on the first call only, so a page never visited costs no request.
    pub fn activate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.activated {
            self.activated = true;
            self.refresh(cx);
        }
    }

    /// ⌘R while the page is showing.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| s.refresh(cx));
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let store = self.store.read(cx);
        let data = store.concerts.data.as_ref()?;
        let new_only = store.new_only;
        let region = self.selected_region(data);
        let regions: Vec<(String, String)> = data
            .regions
            .iter()
            .map(|r| (r.key.clone(), data.region_label(&r.key).to_owned()))
            .collect();
        if regions.is_empty() && !new_only {
            return None;
        }
        let region_button =
            |id: String, label: String, key: Option<String>, cx: &mut Context<Self>| {
                let selected = region.as_deref() == key.as_deref();
                Button::new(SharedString::from(id))
                    .small()
                    .label(label)
                    .selected(selected)
                    .when(!selected, |b| b.ghost())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.region = key.clone();
                        cx.notify();
                    }))
            };
        let mut row = h_flex().gap_1();
        // One region needs no filter.
        if regions.len() > 1 {
            row = row.child(region_button("region-all".into(), "All".into(), None, cx));
            for (key, label) in regions {
                row = row.child(region_button(format!("region-{key}"), label, Some(key), cx));
            }
            row = row.child(div().w(px(8.)));
        }
        row = row.child(
            Button::new("concerts-new-only")
                .small()
                .label(format!("New in last {NEW_DAYS} days"))
                .selected(new_only)
                .when(!new_only, |b| b.ghost())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.store.update(cx, |s, cx| s.set_new_only(!new_only, cx));
                })),
        );
        Some(row.into_any_element())
    }

    /// The chosen region if the response still has it.
    fn selected_region(&self, data: &ConcertsResponse) -> Option<String> {
        self.region
            .clone()
            .filter(|key| data.regions.iter().any(|r| &r.key == key))
    }
}

impl Focusable for ConcertsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ConcertsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controls = self.render_controls(cx);
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let store = self.store.read(cx);
        let new_only = store.new_only;
        let region = store
            .concerts
            .data
            .as_ref()
            .and_then(|data| self.selected_region(data));
        let now = Utc::now();
        let body = widgets::page_body(
            "concerts-retry",
            &store.concerts,
            "Loading concerts…",
            retry,
            cx,
            |data| render_concerts(data, region.as_deref(), new_only, now, cx),
        );
        widgets::page("concerts")
            .track_focus(&self.focus)
            .child(widgets::page_header(
                "Concerts",
                Some("Upcoming shows for the artists you've played most over the last year".into()),
                controls,
                cx,
            ))
            .child(body)
    }
}

fn render_concerts(
    data: &ConcertsResponse,
    region: Option<&str>,
    new_only: bool,
    now: DateTime<Utc>,
    cx: &App,
) -> AnyElement {
    if data.regions.is_empty() && data.concerts.is_empty() {
        return empty(
            IconName::MapPin,
            "No concert regions are set up",
            "Set CONCERT_REGIONS on the Worker to the places it should watch. Its daily \
             check then looks up shows for your top artists there.",
            cx,
        );
    }
    let groups = group_by_region(data, region);
    if new_only && groups.iter().all(|(_, shows)| shows.is_empty()) {
        return empty(
            IconName::Ticket,
            "Nothing new this week",
            "No shows were added in the last week. Turn off the filter to see everything \
             still coming up.",
            cx,
        );
    }
    v_flex()
        .gap_5()
        .children(groups.into_iter().map(|(key, shows)| {
            let mut detail = vec![match shows.len() {
                1 => "1 show".to_owned(),
                n => format!("{} shows", widgets::count(n as u64)),
            }];
            match data.checked.get(key).and_then(|iso| checked_ago(iso, now)) {
                Some(ago) => detail.push(format!("checked {ago}")),
                None => detail.push("not checked yet".to_owned()),
            }
            v_flex()
                .gap_2()
                .child(widgets::section_heading(
                    data.region_label(key).to_owned(),
                    Some(detail.join(" · ").into()),
                    cx,
                ))
                .when(shows.is_empty(), |this| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if data.checked.contains_key(key) {
                                "None of your top artists have shows listed here."
                            } else {
                                "The Worker checks once a day; shows appear after its next run."
                            }),
                    )
                })
                .children(
                    shows
                        .into_iter()
                        .enumerate()
                        .map(|(i, show)| render_show(key, i, show, cx)),
                )
                .into_any_element()
        }))
        .into_any_element()
}

fn empty(icon: IconName, title: &str, detail: &'static str, cx: &App) -> AnyElement {
    widgets::page_state(Some(icon), title.to_owned(), Some(detail.into()), cx)
}

fn render_show(region: &str, index: usize, show: &Concert, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let id = format!("concert-{region}-{index}");
    let artists =
        widgets::credit_links(format!("{id}-artist"), &show.credits(), cx).font_semibold();
    let place = [show.venue.as_deref(), show.city.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
    let when = show
        .date
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    let detail = [
        when.map(|d| d.format("%A").to_string()),
        show.time.clone(),
        (!place.is_empty()).then_some(place),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    // The event name often just repeats the bill.
    let event_name = (show.name != show.artists).then(|| show.name.clone());

    h_flex()
        .gap_3()
        .p_2()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .child(date_block(when, cx))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    h_flex()
                        .gap_2()
                        .child(artists)
                        .when_some(status_label(show.status.as_deref()), |this, label| {
                            this.child(Tag::warning().xsmall().child(label))
                        }),
                )
                .when_some(event_name, |this, name| {
                    this.child(div().text_sm().truncate().child(name))
                })
                .when(!detail.is_empty(), |this| {
                    this.child(div().text_xs().text_color(muted).truncate().child(detail))
                }),
        )
        .when_some(show.url.clone(), |this, url| {
            this.child(
                Button::new(SharedString::from(format!("{id}-tickets")))
                    .small()
                    .outline()
                    .icon(IconName::ExternalLink)
                    .label("Open tickets")
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            )
        })
        .into_any_element()
}

/// A calendar-leaf date: month over day over year, or "TBA" when the event
/// has no date yet.
fn date_block(date: Option<NaiveDate>, cx: &App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let block = v_flex()
        .w(px(52.))
        .flex_shrink_0()
        .items_center()
        .py_1()
        .rounded_md()
        .bg(cx.theme().muted.opacity(0.6));
    match date {
        Some(date) => block
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(date.format("%b").to_string().to_uppercase()),
            )
            .child(
                div()
                    .text_xl()
                    .font_semibold()
                    .child(date.format("%-d").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(date.format("%Y").to_string()),
            ),
        None => block.justify_center().child(
            div()
                .text_sm()
                .font_semibold()
                .text_color(muted)
                .child("TBA"),
        ),
    }
}

/// Shows grouped by region, each group soonest first as the Worker sent
/// them. Configured regions come first in the configured order, even when
/// empty, so each one reads as checked; keys only an older Worker sends
/// (no `regions` list) follow in order of appearance.
fn group_by_region<'a>(
    data: &'a ConcertsResponse,
    only: Option<&str>,
) -> Vec<(&'a str, Vec<&'a Concert>)> {
    let mut keys: Vec<&str> = data.regions.iter().map(|r| r.key.as_str()).collect();
    for show in &data.concerts {
        if !keys.contains(&show.region.as_str()) {
            keys.push(&show.region);
        }
    }
    keys.into_iter()
        .filter(|key| only.is_none_or(|only| only == *key))
        .map(|key| {
            let shows = data.concerts.iter().filter(|c| c.region == key).collect();
            (key, shows)
        })
        .collect()
}

/// A tag for anything other than a normal on-sale event.
fn status_label(status: Option<&str>) -> Option<String> {
    let status = status?.trim().to_ascii_lowercase();
    Some(
        match status.as_str() {
            "" | "onsale" => return None,
            "offsale" => "Off sale",
            "cancelled" | "canceled" => "Cancelled",
            "postponed" => "Postponed",
            "rescheduled" => "Rescheduled",
            "tba" | "tbd" => "Date to be announced",
            other => return Some(other.to_owned()),
        }
        .to_owned(),
    )
}

/// `just now`, `25 min ago`, `3 h ago`, `2 days ago` for a fetch time.
fn checked_ago(iso: &str, now: DateTime<Utc>) -> Option<String> {
    let at = DateTime::parse_from_rfc3339(iso).ok()?.with_timezone(&Utc);
    let minutes = (now - at).num_minutes().max(0);
    Some(match minutes {
        0 => "just now".to_owned(),
        1..=59 => format!("{minutes} min ago"),
        60..=2879 => format!("{} h ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ConcertsResponse {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/concerts.json"
        )))
        .unwrap()
    }

    #[test]
    fn groups_follow_configured_regions() {
        let data = fixture();
        let groups = group_by_region(&data, None);
        let keys: Vec<_> = groups.iter().map(|(k, s)| (*k, s.len())).collect();
        assert_eq!(keys, [("berlin", 1), ("pt", 2)]);
        // Soonest first within a group, undated last, as sent.
        assert_eq!(groups[1].1[0].date.as_deref(), Some("2027-02-12"));
        assert_eq!(groups[1].1[1].date, None);

        let only = group_by_region(&data, Some("pt"));
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].0, "pt");
    }

    #[test]
    fn empty_regions_still_show_and_unknown_keys_follow() {
        let mut data = fixture();
        data.concerts.retain(|c| c.region == "pt");
        data.concerts[0].region = "old".into();
        let keys: Vec<_> = group_by_region(&data, None)
            .iter()
            .map(|(k, s)| (*k, s.len()))
            .collect();
        assert_eq!(keys, [("berlin", 0), ("pt", 1), ("old", 1)]);
    }

    #[test]
    fn only_unusual_statuses_get_a_tag() {
        assert_eq!(status_label(Some("onsale")), None);
        assert_eq!(status_label(None), None);
        assert_eq!(status_label(Some("offsale")).as_deref(), Some("Off sale"));
        assert_eq!(status_label(Some("canceled")).as_deref(), Some("Cancelled"));
        assert_eq!(status_label(Some("weird")).as_deref(), Some("weird"));
    }

    #[test]
    fn checked_times_read_as_ago() {
        let now = DateTime::parse_from_rfc3339("2026-10-15T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ago = |iso| checked_ago(iso, now);
        assert_eq!(ago("2026-10-15T09:59:40.000Z").as_deref(), Some("just now"));
        assert_eq!(ago("2026-10-15T09:35:00Z").as_deref(), Some("25 min ago"));
        assert_eq!(ago("2026-10-15T07:00:00Z").as_deref(), Some("3 h ago"));
        assert_eq!(ago("2026-10-12T09:00:00Z").as_deref(), Some("3 days ago"));
        assert_eq!(ago("2026-10-15T11:00:00Z").as_deref(), Some("just now"));
        assert_eq!(ago("garbage"), None);
    }
}
