//! Data for the Concerts page: `GET /api/concerts`, every configured region
//! at once.

use gpui::{Context, Task};

use crate::api::ApiClient;
use crate::dates;
use crate::models::ConcertsResponse;
use crate::state::{self, Remote};

pub struct ConcertsStore {
    client: ApiClient,
    pub concerts: Remote<ConcertsResponse>,
    _task: Option<Task<()>>,
}

impl ConcertsStore {
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            concerts: Remote::default(),
            _task: None,
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.concerts.start();
        let client = self.client.clone();
        let tz = dates::system_timezone();
        self._task = Some(state::fetch(
            cx,
            async move { client.concerts(None, None, &tz).await },
            |s| &mut s.concerts,
        ));
    }
}
