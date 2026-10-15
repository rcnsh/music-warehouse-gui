//! Data for the Wrapped page: the ranges a read token may summarise
//! (`GET /api/wrapped/ranges`) and the chosen one (`GET /api/wrapped`).

use gpui::{Context, Task};

use crate::api::ApiClient;
use crate::models::{Wrapped, WrappedPreset, WrappedRanges};
use crate::runtime;
use crate::state::{self, Remote};

pub struct WrappedStore {
    client: ApiClient,
    pub ranges: Remote<WrappedRanges>,
    pub selected: Option<WrappedPreset>,
    pub wrapped: Remote<Wrapped>,
    _ranges_task: Option<Task<()>>,
    _wrapped_task: Option<Task<()>>,
}

impl WrappedStore {
    pub fn new(client: ApiClient, _cx: &mut Context<Self>) -> Self {
        Self {
            client,
            ranges: Remote::default(),
            selected: None,
            wrapped: Remote::default(),
            _ranges_task: None,
            _wrapped_task: None,
        }
    }

    /// Reloads the ranges, then the selected summary. The first load picks
    /// the newest range the Worker has already computed, so opening the page
    /// never waits on a cold compute.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.ranges.start();
        cx.notify();
        let client = self.client.clone();
        self._ranges_task = Some(cx.spawn(async move |this, cx| {
            let result = runtime::run(async move { client.wrapped_ranges().await }).await;
            this.update(cx, |this, cx| {
                this.ranges.finish(result);
                let choice = match &this.selected {
                    Some(selected) => Some(selected.clone()),
                    None => this.ranges.data.as_ref().and_then(default_range),
                };
                match choice {
                    Some(preset) => {
                        this.selected = None;
                        this.select(preset, cx);
                    }
                    None => cx.notify(),
                }
            })
            .ok();
        }));
    }

    pub fn select(&mut self, preset: WrappedPreset, cx: &mut Context<Self>) {
        if self.selected.as_ref() != Some(&preset) {
            self.wrapped.reset();
        }
        self.wrapped.start();
        let client = self.client.clone();
        let (from, to) = (preset.from.clone(), preset.to.clone());
        self.selected = Some(preset);
        self._wrapped_task = Some(state::fetch(
            cx,
            async move { client.wrapped(&from, &to).await },
            |s| &mut s.wrapped,
        ));
    }
}

fn default_range(ranges: &WrappedRanges) -> Option<WrappedPreset> {
    ranges
        .ranges
        .iter()
        .find(|r| r.computed)
        .or_else(|| ranges.ranges.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(label: &str, computed: bool) -> WrappedPreset {
        WrappedPreset {
            label: label.into(),
            computed,
            ..Default::default()
        }
    }

    #[test]
    fn opens_on_the_newest_computed_range() {
        let ranges = WrappedRanges {
            ranges: vec![preset("2026 so far", false), preset("2025", true)],
        };
        assert_eq!(default_range(&ranges).unwrap().label, "2025");
        let cold = WrappedRanges {
            ranges: vec![preset("2026 so far", false)],
        };
        assert_eq!(default_range(&cold).unwrap().label, "2026 so far");
        assert_eq!(default_range(&WrappedRanges::default()), None);
    }
}
