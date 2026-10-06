//! Development-only window capture, compiled in with `--features dev-capture`.
//!
//! macOS screen capture needs a Screen Recording grant that build machines
//! and agents often lack, so this renders the window's own scene to a PNG
//! instead. Set `MWGUI_CAPTURE_DIR`, then write commands to `<dir>/request`:
//!
//!   capture <name>         save the window as `<dir>/<name>.png`
//!   action <namespace::Name>   dispatch a registered action, e.g. `mwgui::ShowOverview`
//!
//! The file is deleted once handled and `<dir>/done` is touched, so a script
//! can wait for each step.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AsyncApp};

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

fn handle(line: &str, dir: &std::path::Path, cx: &mut App) -> Result<(), String> {
    let window = *cx.windows().first().ok_or("no window")?;
    let (command, arg) = line.split_once(' ').unwrap_or((line, ""));
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
            window
                .update(cx, |_, window, cx| window.dispatch_action(action, cx))
                .map_err(|e| e.to_string())
        }
        "" => Ok(()),
        other => Err(format!("unknown command {other}")),
    }
}
