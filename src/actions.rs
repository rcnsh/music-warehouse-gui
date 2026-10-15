use gpui::{Action, App, KeyBinding, Menu, MenuItem, SharedString, actions};

use crate::appearance::{self, Appearance};

actions!(
    mwgui,
    [
        ShowHistory,
        ShowOverview,
        ShowOnRepeat,
        ShowOnThisDay,
        ShowAlbums,
        ShowTopOverTime,
        ShowRhythm,
        ShowEras,
        ShowWrapped,
        ShowConcerts,
        GoBack,
        Refresh,
        FocusFilter,
        GoToDate,
        OpenCommandPalette,
        UseSystemAppearance,
        UseLightAppearance,
        UseDarkAppearance,
        OpenSettings,
        OpenInSpotify,
        CopyPlay,
        Quit
    ]
);

/// Opens the Artist page, from any view:
/// `window.dispatch_action(Box::new(OpenArtist::new(id)), cx)`, or hand it to
/// a widget that takes an action. The shell handles it and records the page
/// for Back.
///
/// `id` is a Spotify artist id or a `name:` placeholder, passed to the Worker
/// unchanged. Sources that only have a name (most stats) use
/// [`OpenArtist::by_name`].
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = mwgui, no_json)]
pub struct OpenArtist {
    pub id: SharedString,
}

impl OpenArtist {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self { id: id.into() }
    }

    pub fn by_name(name: &str) -> Self {
        Self::new(crate::models::artist_key(None, name))
    }
}

/// Opens the Track page for a Spotify track id. Any edition's id works: the
/// Worker folds the song's other editions in.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = mwgui, no_json)]
pub struct OpenTrack {
    pub id: SharedString,
}

impl OpenTrack {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self { id: id.into() }
    }
}

/// gpui-component's table binds arrows in this key context; j/k join them
/// there so they only act when a table has focus and never steal letters
/// from the filter or setup inputs.
const TABLE_CONTEXT: &str = "DataTable";

pub fn bind(cx: &mut App) {
    // Page shortcuts follow the sidebar's order (see `views::nav`).
    cx.bind_keys([
        KeyBinding::new("cmd-1", ShowHistory, None),
        KeyBinding::new("cmd-2", ShowOverview, None),
        KeyBinding::new("cmd-3", ShowOnRepeat, None),
        KeyBinding::new("cmd-4", ShowOnThisDay, None),
        KeyBinding::new("cmd-5", ShowAlbums, None),
        KeyBinding::new("cmd-6", ShowTopOverTime, None),
        KeyBinding::new("cmd-7", ShowRhythm, None),
        KeyBinding::new("cmd-8", ShowEras, None),
        KeyBinding::new("cmd-9", ShowWrapped, None),
        KeyBinding::new("cmd-0", ShowConcerts, None),
        KeyBinding::new("cmd-[", GoBack, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-f", FocusFilter, None),
        KeyBinding::new("cmd-g", GoToDate, None),
        KeyBinding::new("cmd-k", OpenCommandPalette, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("j", gpui_base::actions::SelectDown, Some(TABLE_CONTEXT)),
        KeyBinding::new("k", gpui_base::actions::SelectUp, Some(TABLE_CONTEXT)),
        KeyBinding::new("o", OpenInSpotify, Some(TABLE_CONTEXT)),
        KeyBinding::new("cmd-c", CopyPlay, Some(TABLE_CONTEXT)),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &UseSystemAppearance, cx| appearance::set(Appearance::System, cx));
    cx.on_action(|_: &UseLightAppearance, cx| appearance::set(Appearance::Light, cx));
    cx.on_action(|_: &UseDarkAppearance, cx| appearance::set(Appearance::Dark, cx));
    // The menu bar mirrors the shortcuts so they are discoverable. Go lists
    // every page, grouped as the sidebar groups them.
    cx.set_menus([
        Menu::new("Music Warehouse").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ]),
        Menu::new("View").items([
            MenuItem::action("Command Palette…", OpenCommandPalette),
            MenuItem::separator(),
            MenuItem::action("Find in History", FocusFilter),
            MenuItem::action("Go to Date…", GoToDate),
            MenuItem::action("Refresh", Refresh),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new("Appearance").items([
                MenuItem::action("Match System", UseSystemAppearance),
                MenuItem::action("Light", UseLightAppearance),
                MenuItem::action("Dark", UseDarkAppearance),
            ])),
        ]),
        Menu::new("Go").items([
            MenuItem::action("Back", GoBack),
            MenuItem::separator(),
            MenuItem::action("History", ShowHistory),
            MenuItem::action("Overview", ShowOverview),
            MenuItem::action("On Repeat", ShowOnRepeat),
            MenuItem::action("On This Day", ShowOnThisDay),
            MenuItem::separator(),
            MenuItem::action("Top Albums", ShowAlbums),
            MenuItem::action("Top Over Time", ShowTopOverTime),
            MenuItem::action("Rhythm", ShowRhythm),
            MenuItem::action("Eras", ShowEras),
            MenuItem::action("Wrapped", ShowWrapped),
            MenuItem::separator(),
            MenuItem::action("Concerts", ShowConcerts),
        ]),
    ]);
}
