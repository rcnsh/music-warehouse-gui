//! Entities that own fetched data. Views observe these and render; they never
//! talk to the API directly.

pub mod history;
pub mod now_playing;
pub mod overview;

use crate::api::ApiError;

/// A remote value that keeps its last good data while reloading or after a
/// failed refresh, so a transient error does not blank a populated view.
#[derive(Debug, Clone)]
pub struct Remote<T> {
    pub data: Option<T>,
    pub loading: bool,
    pub error: Option<ApiError>,
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self {
            data: None,
            loading: false,
            error: None,
        }
    }
}

impl<T> Remote<T> {
    pub fn start(&mut self) {
        self.loading = true;
    }

    pub fn finish(&mut self, result: Result<T, ApiError>) {
        self.loading = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Clears data that belongs to a different query, such as a new range.
    pub fn reset(&mut self) {
        self.data = None;
        self.error = None;
    }
}
