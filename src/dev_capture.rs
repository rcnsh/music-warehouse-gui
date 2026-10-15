//! Development-only window capture, compiled in with `--features dev-capture`.
//!
//! macOS screen capture needs a Screen Recording grant that build machines
//! and agents often lack, so this renders the window's own scene to a PNG
//! instead. Set `MWGUI_CAPTURE_DIR`, then write commands to `<dir>/request`,
//! one per line:
//!
//!   capture <name>             save the window as `<dir>/<name>.png`
//!   action <namespace::Name>   dispatch a registered action, e.g. `mwgui::ShowOverview`
//!   artist <id>                dispatch `OpenArtist` (an id or a `name:` placeholder)
//!   track <id>                 dispatch `OpenTrack`
//!   scroll <dy> [<x> <y>]      scroll by `dy` pixels (positive is down) at a point
//!                              in the window, by default its centre
//!   click <x> <y>              left-click at a point in the window, in points
//!                              from its top-left (a capture's pixels divided
//!                              by the display's scale)
//!
//! The file is deleted once handled and `<dir>/done` is touched, so a script
//! can wait for each step. Answers that arrive later (a fetch, album art)
//! need the script to wait before capturing.
//!
//! Each command first brings the window forward and draws a fresh frame, and
//! a capture first keeps drawing for a moment so animations finish.
//! `scroll` and `click` are delivered to the window directly, so they work
//! whether or not the window can take real mouse input. Without that, a window behind others keeps its last frame: captures come
//! out stale, and actions go to whatever had focus in that frame rather than
//! to the page now showing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{
    Action, AnyWindowHandle, App, AsyncApp, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px,
};

use crate::actions::{OpenArtist, OpenTrack};

pub fn install(cx: &mut App) {
    let Some(dir) = std::env::var_os("MWGUI_CAPTURE_DIR").map(PathBuf::from) else {
        return;
    };
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let request = dir.join("request");
            let Ok(text) = std::fs::read_to_string(&request) else {
                continue;
            };
            let _ = std::fs::remove_file(&request);
            for line in text.lines() {
                if line.trim_start().starts_with("capture") {
                    settle(cx).await;
                }
                let result = cx.update(|cx| handle(line.trim(), &dir, cx));
                if let Err(message) = result {
                    eprintln!("dev-capture: {line}: {message}");
                }
            }
            let _ = std::fs::write(dir.join("done"), text);
        }
    })
    .detach();
}

/// How long a capture lets animations run first. Charts grow their bars in
/// over a few hundred milliseconds, and a window macOS is not presenting
/// only advances them when this module draws it.
const SETTLE: Duration = Duration::from_millis(800);
const SETTLE_FRAME: Duration = Duration::from_millis(50);

async fn settle(cx: &mut AsyncApp) {
    for _ in 0..SETTLE.as_millis() / SETTLE_FRAME.as_millis() {
        cx.update(|cx| {
            if let Some(&window) = cx.windows().first() {
                let _ = bring_forward(window, cx);
            }
        });
        cx.background_executor().timer(SETTLE_FRAME).await;
    }
}

fn handle(line: &str, dir: &Path, cx: &mut App) -> Result<(), String> {
    let window = *cx.windows().first().ok_or("no window")?;
    let (command, arg) = line.split_once(' ').unwrap_or((line, ""));
    if command.is_empty() {
        return Ok(());
    }
    bring_forward(window, cx)?;
    match command {
        "capture" => {
            let image = window
                .update(cx, |_, window, _| window.render_to_image())
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            image
                .save(dir.join(format!("{arg}.png")))
                .map_err(|e| e.to_string())
        }
        "action" => {
            let action = cx.build_action(arg, None).map_err(|e| e.to_string())?;
            dispatch(window, action, cx)
        }
        // These carry an id, so they cannot be built by name.
        "artist" => dispatch(window, Box::new(OpenArtist::new(arg.to_owned())), cx),
        "track" => dispatch(window, Box::new(OpenTrack::new(arg.to_owned())), cx),
        "scroll" => scroll(window, arg, cx),
        "click" => click(window, arg, cx),
        other => Err(format!("unknown command {other}")),
    }
}

/// Activates the app and window, then draws a frame so the scene, hitboxes
/// and focus path match the current state. The window is marked dirty again
/// afterwards so the screen catches up too: this frame is never presented.
fn bring_forward(window: AnyWindowHandle, cx: &mut App) -> Result<(), String> {
    cx.activate(true);
    window
        .update(cx, |_, window, cx| {
            window.activate_window();
            window.refresh();
            window.draw(cx).clear(cx);
            window.refresh();
        })
        .map_err(|e| e.to_string())
}

fn dispatch(window: AnyWindowHandle, action: Box<dyn Action>, cx: &mut App) -> Result<(), String> {
    window
        .update(cx, |_, window, cx| window.dispatch_action(action, cx))
        .map_err(|e| e.to_string())
}

fn scroll(window: AnyWindowHandle, arg: &str, cx: &mut App) -> Result<(), String> {
    let numbers = arg
        .split_whitespace()
        .map(|n| n.parse::<f32>().map_err(|_| format!("not a number: {n}")))
        .collect::<Result<Vec<_>, _>>()?;
    window
        .update(cx, |_, window, cx| {
            let size = window.viewport_size();
            let (dy, x, y) = match numbers[..] {
                [dy] => (dy, size.width / 2., size.height / 2.),
                [dy, x, y] => (dy, px(x), px(y)),
                _ => return Err("expected `scroll <dy> [<x> <y>]`".to_owned()),
            };
            let event = ScrollWheelEvent {
                position: point(x, y),
                // Wheel deltas point the way the content moves.
                delta: ScrollDelta::Pixels(point(px(0.), px(-dy))),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            };
            window.dispatch_event(PlatformInput::ScrollWheel(event), cx);
            Ok(())
        })
        .map_err(|e| e.to_string())?
}

fn click(window: AnyWindowHandle, arg: &str, cx: &mut App) -> Result<(), String> {
    let numbers = arg
        .split_whitespace()
        .map(|n| n.parse::<f32>().map_err(|_| format!("not a number: {n}")))
        .collect::<Result<Vec<_>, _>>()?;
    let [x, y] = numbers[..] else {
        return Err("expected `click <x> <y>`".to_owned());
    };
    let position = point(px(x), px(y));
    window
        .update(cx, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }),
                cx,
            );
        })
        .map_err(|e| e.to_string())
}
