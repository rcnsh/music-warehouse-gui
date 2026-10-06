use gpui::{App, KeyBinding, Menu, MenuItem, actions};

actions!(
    mwgui,
    [
        ShowHistory,
        ShowOverview,
        Refresh,
        FocusFilter,
        GoToDate,
        OpenSettings,
        OpenInSpotify,
        CopyPlay,
        Quit
    ]
);

/// gpui-component's table binds arrows in this key context; j/k join them
/// there so they only act when a table has focus and never steal letters
/// from the filter or setup inputs.
const TABLE_CONTEXT: &str = "DataTable";

pub fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-1", ShowHistory, None),
        KeyBinding::new("cmd-2", ShowOverview, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-f", FocusFilter, None),
        KeyBinding::new("cmd-g", GoToDate, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("j", gpui_base::actions::SelectDown, Some(TABLE_CONTEXT)),
        KeyBinding::new("k", gpui_base::actions::SelectUp, Some(TABLE_CONTEXT)),
        KeyBinding::new("o", OpenInSpotify, Some(TABLE_CONTEXT)),
        KeyBinding::new("cmd-c", CopyPlay, Some(TABLE_CONTEXT)),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    // The menu bar mirrors the shortcuts so they are discoverable.
    cx.set_menus([
        Menu::new("Music Warehouse").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ]),
        Menu::new("View").items([
            MenuItem::action("History", ShowHistory),
            MenuItem::action("Overview", ShowOverview),
            MenuItem::separator(),
            MenuItem::action("Find in History", FocusFilter),
            MenuItem::action("Go to Date…", GoToDate),
            MenuItem::action("Refresh", Refresh),
        ]),
    ]);
}
