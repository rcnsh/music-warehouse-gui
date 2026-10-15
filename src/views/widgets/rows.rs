//! Rows and links: stat tiles, ranked rows with share bars, and credits.

use super::*;

/// A headline figure: a big value over a muted label, with an optional
/// detail line. Lay several out in an `h_flex().gap_3()`; each takes an equal
/// share of the row.
pub fn stat_tile(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    detail: Option<SharedString>,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .truncate()
                .child(label.into()),
        )
        .child(
            div()
                .text_2xl()
                .font_semibold()
                .truncate()
                .child(value.into()),
        )
        .when_some(detail, |this, detail| {
            this.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(detail),
            )
        })
}

/// One line of a ranked list: rank, optional art, title over a muted
/// subtitle, a bar showing `share` (0–1, relative to the list's top entry or
/// to a total, as the page decides), and the count. With an action the whole
/// row is clickable and dispatches it, e.g. [`crate::actions::OpenArtist`].
#[derive(IntoElement)]
pub struct RankedRow {
    id: ElementId,
    rank: Option<usize>,
    art: Option<AnyElement>,
    title: SharedString,
    subtitle: Option<SharedString>,
    value: Option<SharedString>,
    share: Option<f32>,
    value_width: Pixels,
    action: Option<Box<dyn Action>>,
}

/// The value column's default width, enough for `12 h 34 min` or `12,345`.
/// Fixed rather than fitted, so the bars of rows with longer and shorter
/// values still line up.
pub const RANKED_VALUE_WIDTH: Pixels = px(84.);

impl RankedRow {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            rank: None,
            art: None,
            title: title.into(),
            subtitle: None,
            value: None,
            share: None,
            value_width: RANKED_VALUE_WIDTH,
            action: None,
        }
    }

    /// 1-based position shown on the left.
    pub fn rank(mut self, rank: usize) -> Self {
        self.rank = Some(rank);
        self
    }

    /// Usually [`album_art`] or [`artwork`] at 32px.
    pub fn art(mut self, art: AnyElement) -> Self {
        self.art = Some(art);
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// The count on the right, already formatted ([`count`], [`minutes`]).
    pub fn value(mut self, value: impl Into<SharedString>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Width of the value column, for lists whose values are much shorter
    /// (`7%`) or longer than [`RANKED_VALUE_WIDTH`] allows. Use the same width
    /// for every row of a list.
    pub fn value_width(mut self, width: Pixels) -> Self {
        self.value_width = width;
        self
    }

    /// Bar length, 0–1. Values outside are clamped.
    pub fn share(mut self, share: f32) -> Self {
        self.share = Some(share.clamp(0., 1.));
        self
    }

    pub fn on_click_action(mut self, action: impl Action) -> Self {
        self.action = Some(Box::new(action));
        self
    }
}

impl RenderOnce for RankedRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let hover = cx.theme().muted.opacity(0.5);
        let clickable = self.action.is_some();
        h_flex()
            .id(self.id)
            .gap_2()
            .px_1()
            .py_0p5()
            .rounded_md()
            .items_center()
            .text_sm()
            .when_some(self.rank, |this, rank| {
                this.child(
                    div()
                        .w(px(24.))
                        .flex_shrink_0()
                        .text_right()
                        .text_color(muted)
                        .child(rank.to_string()),
                )
            })
            .when_some(self.art, |this, art| this.child(art))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(self.title))
                    .when_some(self.subtitle, |this, subtitle| {
                        this.child(div().truncate().text_xs().text_color(muted).child(subtitle))
                    }),
            )
            .when_some(self.share, |this, share| {
                this.child(
                    div()
                        .w(px(120.))
                        .flex_shrink_0()
                        .child(share_bar(share, cx)),
                )
            })
            .when_some(self.value, |this, value| {
                this.child(
                    div()
                        .w(self.value_width)
                        .flex_shrink_0()
                        .text_right()
                        .truncate()
                        .child(value),
                )
            })
            .when_some(self.action, |this, action| {
                this.cursor_pointer()
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            })
            .when(clickable, |this| this.hover(move |style| style.bg(hover)))
    }
}

/// A thin rounded bar filled to `share` (0–1) in the accent colour.
pub fn share_bar(share: f32, cx: &App) -> impl IntoElement {
    div()
        .w_full()
        .h(px(6.))
        .rounded_full()
        .bg(cx.theme().muted)
        .child(
            div()
                .h_full()
                .rounded_full()
                .w(relative(share.clamp(0., 1.)))
                .bg(cx.theme().primary),
        )
}

/// Text that dispatches `action` when clicked, for names inside other text
/// (an artist under a song title, say).
pub fn link(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: impl Action,
    cx: &App,
) -> impl IntoElement {
    let action: Box<dyn Action> = Box::new(action);
    let hover = cx.theme().primary;
    div()
        .id(id)
        .cursor_pointer()
        .hover(move |style| style.text_color(hover))
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        .child(label.into())
}

/// Each credited artist a link to their page, comma-separated. A credit with
/// an id opens that artist; one without (an older Worker's joined names, an
/// artist on a concert bill the warehouse does not know) opens by name.
pub fn credit_links(id: impl Into<SharedString>, credits: &[Credit], cx: &App) -> gpui::Div {
    let id = id.into();
    let mut row = h_flex().flex_wrap();
    for (ix, credit) in credits.iter().enumerate() {
        if ix > 0 {
            row = row.child(div().pr_1().child(","));
        }
        row = row.child(link(
            ElementId::named_usize(id.clone(), ix),
            credit.name.clone(),
            OpenArtist::by_id_or_name(credit.id.as_deref(), &credit.name),
            cx,
        ));
    }
    row
}
