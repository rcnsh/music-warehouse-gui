mod actions;
mod api;
mod config;
mod dates;
#[cfg(feature = "dev-capture")]
mod dev_capture;
mod models;
mod runtime;
mod state;
mod views;

use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, Theme};

use crate::views::root::AppRoot;

fn main() {
    // The full Lucide catalog costs about a megabyte of binary; worth it so any
    // icon name compiles without curating a list.
    let app = gpui_platform::application().with_assets(gpui_kit_assets::AllAssets);

    app.run(|cx: &mut App| {
        // GPUI's img() fetches remote artwork through the App's HTTP client;
        // without one, every image falls back to its placeholder.
        match reqwest_client::ReqwestClient::user_agent(concat!(
            "music-warehouse-gui/",
            env!("CARGO_PKG_VERSION")
        )) {
            Ok(client) => cx.set_http_client(std::sync::Arc::new(client)),
            Err(e) => eprintln!("album art disabled: {e}"),
        }
        gpui_component::init(cx);
        actions::bind(cx);
        cx.activate(true);

        let bounds = Bounds::centered(None, size(px(1180.), px(820.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Music Warehouse".into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(860.), px(560.))),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            let view = cx.new(|cx| AppRoot::new(window, cx));
            // Root hosts gpui-component's overlay layers (tooltips, popovers),
            // which render nothing without it.
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("open the main window");

        // A single-window app has nothing to show once its window closes.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        #[cfg(feature = "dev-capture")]
        dev_capture::install(cx);
    });
}
