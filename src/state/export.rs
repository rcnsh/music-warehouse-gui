//! Saving History as CSV through `GET /api/plays.csv`.
//!
//! The Worker builds the file from the same filters the list uses and
//! streams it a page at a time; this writes each chunk to disk as it
//! arrives, so a whole history never sits in memory. It goes to a `.part`
//! file beside the chosen path and is renamed into place only once complete,
//! so a failed or cancelled export never leaves a truncated file under the
//! name the user picked.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{Context, Subscription, Task};

use crate::api::{ApiClient, ApiError};
use crate::runtime;
use crate::state::history::ExportRequest;

/// How often the row count on screen catches up with the download.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportState {
    Idle,
    Running { path: PathBuf, rows: u64 },
    Done { path: PathBuf, rows: u64 },
    Failed { error: ApiError },
}

pub struct CsvExport {
    client: ApiClient,
    pub state: ExportState,
    /// Line breaks written so far, shared with the download.
    lines: Arc<AtomicU64>,
    _task: Option<Task<()>>,
    _progress: Option<Task<()>>,
    _quit: Subscription,
}

impl CsvExport {
    pub fn new(client: ApiClient, cx: &mut Context<Self>) -> Self {
        Self {
            client,
            state: ExportState::Idle,
            lines: Arc::default(),
            _task: None,
            _progress: None,
            // Quitting exits without dropping this entity or aborting the
            // download, which would leave its `.part` file behind.
            _quit: cx.on_app_quit(|this, cx| {
                this.cancel(cx);
                async {}
            }),
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, ExportState::Running { .. })
    }

    /// Downloads the plays `request` describes into `path`, replacing an
    /// export already running.
    pub fn start(&mut self, request: ExportRequest, path: PathBuf, cx: &mut Context<Self>) {
        self.cancel(cx);
        let url = self
            .client
            .plays_csv_url(request.before, request.after, &request.filter);
        let lines = Arc::new(AtomicU64::new(0));
        self.lines = lines.clone();
        self.state = ExportState::Running {
            path: path.clone(),
            rows: 0,
        };
        let client = self.client.clone();
        let target = path.clone();
        self._task = Some(cx.spawn(async move |this, cx| {
            // Cancellable: dropping this task (Cancel, or a new export)
            // stops the download rather than letting it run on unseen.
            let result = runtime::run(async move {
                let part = partial_path(&target);
                let outcome = download(&client, url, &part, &target, lines).await;
                if outcome.is_err() {
                    let _ = tokio::fs::remove_file(&part).await;
                }
                outcome
            })
            .await;
            this.update(cx, |this, cx| {
                this._progress = None;
                this.state = match result {
                    Ok(rows) => ExportState::Done { path, rows },
                    Err(error) => ExportState::Failed { error },
                };
                cx.notify();
            })
            .ok();
        }));
        self._progress = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PROGRESS_EVERY).await;
                let alive = this
                    .update(cx, |this, cx| {
                        let rows = rows_written(this.lines.load(Ordering::Relaxed));
                        if let ExportState::Running { rows: shown, .. } = &mut this.state
                            && *shown != rows
                        {
                            *shown = rows;
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// Stops an export in progress and removes its partial file.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self._task = None;
        self._progress = None;
        if self.is_running() {
            self.remove_partial();
            self.state = ExportState::Idle;
            cx.notify();
        }
    }

    /// Removes the partial file of an export still running.
    fn remove_partial(&self) {
        if let ExportState::Running { path, .. } = &self.state {
            let _ = std::fs::remove_file(partial_path(path));
        }
    }

    /// Forgets a finished or failed export, hiding its status.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.is_running() {
            self.state = ExportState::Idle;
            cx.notify();
        }
    }
}

/// Closing the window drops the export (and so aborts its download) without
/// a `cancel`; the partial file goes with it.
impl Drop for CsvExport {
    fn drop(&mut self) {
        self.remove_partial();
    }
}

async fn download(
    client: &ApiClient,
    url: url::Url,
    part: &Path,
    target: &Path,
    lines: Arc<AtomicU64>,
) -> Result<u64, ApiError> {
    let file_error = |e: std::io::Error| ApiError::File(e.to_string());
    let mut file = tokio::fs::File::create(part).await.map_err(file_error)?;
    client
        .download(url, &mut file, |chunk| {
            let breaks = chunk.iter().filter(|&&b| b == b'\n').count() as u64;
            lines.fetch_add(breaks, Ordering::Relaxed);
        })
        .await?;
    file.sync_all().await.map_err(file_error)?;
    drop(file);
    tokio::fs::rename(part, target).await.map_err(file_error)?;
    Ok(rows_written(lines.load(Ordering::Relaxed)))
}

/// Plays in a CSV with `lines` line breaks: every row ends in one, and the
/// first is the header. A track name holding a line break would count
/// twice, which no stored name does.
fn rows_written(lines: u64) -> u64 {
    lines.saturating_sub(1)
}

/// `plays.csv` downloads to `plays.csv.part` first.
fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

/// A file name for the save panel that says what was exported:
/// `plays.csv`, `plays-kendrick-lamar.csv`, `plays-kendrick-lamar-dna.csv`.
pub fn suggested_name(scope: Option<&str>, query: Option<&str>) -> String {
    let slug: Vec<String> = [scope, query]
        .into_iter()
        .flatten()
        .map(slugify)
        .filter(|s| !s.is_empty())
        .collect();
    if slug.is_empty() {
        "plays.csv".to_owned()
    } else {
        format!("plays-{}.csv", slug.join("-"))
    }
}

/// Lower-case letters and digits (any script) joined by single dashes, at
/// most 40 characters, so the name is safe on every file system.
fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.chars().count() >= 40 {
            break;
        }
    }
    slug.trim_end_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_leave_out_the_header() {
        assert_eq!(rows_written(0), 0);
        assert_eq!(rows_written(1), 0);
        assert_eq!(rows_written(101), 100);
    }

    #[test]
    fn partial_file_sits_beside_the_target() {
        assert_eq!(
            partial_path(Path::new("/tmp/out/plays.csv")),
            PathBuf::from("/tmp/out/plays.csv.part")
        );
    }

    #[test]
    fn suggested_names_describe_the_export() {
        assert_eq!(suggested_name(None, None), "plays.csv");
        assert_eq!(
            suggested_name(Some("Kendrick Lamar"), None),
            "plays-kendrick-lamar.csv"
        );
        assert_eq!(
            suggested_name(Some("Mr. Morale & The Big Steppers"), Some("  ")),
            "plays-mr-morale-the-big-steppers.csv"
        );
        assert_eq!(suggested_name(None, Some("DNA.")), "plays-dna.csv");
        assert_eq!(suggested_name(Some("岡部啓一"), None), "plays-岡部啓一.csv");
        assert_eq!(suggested_name(Some("/../"), None), "plays.csv");
        assert!(suggested_name(Some(&"a".repeat(100)), None).len() < 60);
    }
}
