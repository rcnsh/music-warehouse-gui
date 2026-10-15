//! The pages the shell can show, how the sidebar groups them, and what each
//! is called everywhere it is named (sidebar, palette, menus, saved state).

use gpui::{Action, SharedString};
use gpui_kit_assets::IconName;

use crate::actions;
use crate::ui_state::SavedPage;

/// A page with a fixed place in the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    History,
    Overview,
    OnRepeat,
    OnThisDay,
    Albums,
    TopOverTime,
    Rhythm,
    Eras,
    Wrapped,
    Concerts,
}

/// Anything the shell can show. Artist and Track pages are reached from
/// other pages, so they carry the id they show and have no sidebar entry
/// until opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Section(Section),
    Artist(SharedString),
    Track(SharedString),
}

impl From<Section> for Page {
    fn from(section: Section) -> Self {
        Self::Section(section)
    }
}

pub struct Group {
    pub label: &'static str,
    pub sections: &'static [Section],
}

/// Sidebar order, which is also the ⌘1…⌘0 order.
pub const GROUPS: [Group; 3] = [
    Group {
        label: "Listening",
        sections: &[
            Section::History,
            Section::Overview,
            Section::OnRepeat,
            Section::OnThisDay,
        ],
    },
    Group {
        label: "Stats",
        sections: &[
            Section::Albums,
            Section::TopOverTime,
            Section::Rhythm,
            Section::Eras,
            Section::Wrapped,
        ],
    },
    Group {
        label: "Live",
        sections: &[Section::Concerts],
    },
];

impl Section {
    pub const ALL: [Section; 10] = [
        Self::History,
        Self::Overview,
        Self::OnRepeat,
        Self::OnThisDay,
        Self::Albums,
        Self::TopOverTime,
        Self::Rhythm,
        Self::Eras,
        Self::Wrapped,
        Self::Concerts,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::History => "History",
            Self::Overview => "Overview",
            Self::OnRepeat => "On Repeat",
            Self::OnThisDay => "On This Day",
            Self::Albums => "Top Albums",
            Self::TopOverTime => "Top Over Time",
            Self::Rhythm => "Rhythm",
            Self::Eras => "Eras",
            Self::Wrapped => "Wrapped",
            Self::Concerts => "Concerts",
        }
    }

    /// Extra words the palette matches, beyond the label.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::History => &["plays", "list", "recent"],
            Self::Overview => &["chart", "daily", "artists", "spotify top"],
            Self::OnRepeat => &["repeat", "loop", "obsessed", "recent"],
            Self::OnThisDay => &["anniversary", "years ago", "memories"],
            Self::Albums => &["albums", "records"],
            Self::TopOverTime => &["quarters", "years", "leaders", "history"],
            Self::Rhythm => &["streaks", "hours", "time of day", "habits"],
            Self::Eras => &["move", "drift", "change", "before after"],
            Self::Wrapped => &["year in review", "recap", "summary"],
            Self::Concerts => &["gigs", "tickets", "events", "tour", "live"],
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::History => IconName::Inbox,
            Self::Overview => IconName::ChartColumn,
            Self::OnRepeat => IconName::Repeat,
            Self::OnThisDay => IconName::CalendarDays,
            Self::Albums => IconName::Disc3,
            Self::TopOverTime => IconName::TrendingUp,
            Self::Rhythm => IconName::Clock,
            Self::Eras => IconName::MapPin,
            Self::Wrapped => IconName::Sparkles,
            Self::Concerts => IconName::Ticket,
        }
    }

    /// The digit after ⌘ that shows this page.
    pub fn shortcut(self) -> char {
        match self {
            Self::History => '1',
            Self::Overview => '2',
            Self::OnRepeat => '3',
            Self::OnThisDay => '4',
            Self::Albums => '5',
            Self::TopOverTime => '6',
            Self::Rhythm => '7',
            Self::Eras => '8',
            Self::Wrapped => '9',
            Self::Concerts => '0',
        }
    }

    pub fn action(self) -> Box<dyn Action> {
        match self {
            Self::History => Box::new(actions::ShowHistory),
            Self::Overview => Box::new(actions::ShowOverview),
            Self::OnRepeat => Box::new(actions::ShowOnRepeat),
            Self::OnThisDay => Box::new(actions::ShowOnThisDay),
            Self::Albums => Box::new(actions::ShowAlbums),
            Self::TopOverTime => Box::new(actions::ShowTopOverTime),
            Self::Rhythm => Box::new(actions::ShowRhythm),
            Self::Eras => Box::new(actions::ShowEras),
            Self::Wrapped => Box::new(actions::ShowWrapped),
            Self::Concerts => Box::new(actions::ShowConcerts),
        }
    }

    pub fn saved(self) -> SavedPage {
        match self {
            Self::History => SavedPage::History,
            Self::Overview => SavedPage::Overview,
            Self::OnRepeat => SavedPage::OnRepeat,
            Self::OnThisDay => SavedPage::OnThisDay,
            Self::Albums => SavedPage::Albums,
            Self::TopOverTime => SavedPage::TopOverTime,
            Self::Rhythm => SavedPage::Rhythm,
            Self::Eras => SavedPage::Eras,
            Self::Wrapped => SavedPage::Wrapped,
            Self::Concerts => SavedPage::Concerts,
        }
    }

    pub fn from_saved(saved: SavedPage) -> Self {
        Self::ALL
            .into_iter()
            .find(|s| s.saved() == saved)
            .unwrap_or(Self::History)
    }
}

/// Where Back goes. Capped so a long session cannot grow it without bound.
#[derive(Debug, Default)]
pub struct BackStack {
    pages: Vec<Page>,
}

impl BackStack {
    const LIMIT: usize = 50;

    pub fn push(&mut self, page: Page) {
        // Going A → B → A should not leave A twice in a row.
        if self.pages.last() == Some(&page) {
            return;
        }
        if self.pages.len() == Self::LIMIT {
            self.pages.remove(0);
        }
        self.pages.push(page);
    }

    pub fn pop(&mut self) -> Option<Page> {
        self.pages.pop()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_is_in_exactly_one_group_in_shortcut_order() {
        let grouped: Vec<Section> = GROUPS
            .iter()
            .flat_map(|g| g.sections.iter().copied())
            .collect();
        assert_eq!(grouped, Section::ALL);
        let keys: String = Section::ALL.iter().map(|s| s.shortcut()).collect();
        assert_eq!(keys, "1234567890");
    }

    #[test]
    fn saved_pages_round_trip() {
        for section in Section::ALL {
            assert_eq!(Section::from_saved(section.saved()), section);
        }
    }

    #[test]
    fn back_stack_skips_repeats_and_is_bounded() {
        let mut stack = BackStack::default();
        stack.push(Section::History.into());
        stack.push(Section::History.into());
        stack.push(Page::Artist("a".into()));
        assert_eq!(stack.pop(), Some(Page::Artist("a".into())));
        assert_eq!(stack.pop(), Some(Page::Section(Section::History)));
        assert!(stack.is_empty());
        for i in 0..60 {
            stack.push(Page::Track(i.to_string().into()));
        }
        assert_eq!(stack.pages.len(), BackStack::LIMIT);
        assert_eq!(stack.pages[0], Page::Track("10".into()));
    }
}
