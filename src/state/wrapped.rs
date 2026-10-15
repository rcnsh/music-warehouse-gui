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
                let ranges = this.ranges.data.as_ref();
                let choice = match &this.selected {
                    // The fresh entry, so its `computed` flag is current.
                    Some(selected) => Some(
                        ranges
                            .and_then(|r| r.ranges.iter().find(|p| same_range(p, selected)))
                            .unwrap_or(selected)
                            .clone(),
                    ),
                    None => ranges.and_then(default_range),
                };
                match choice {
                    // The same range keeps its summary on screen while it
                    // reloads, as `Remote` does everywhere else.
                    Some(preset) => this.select(preset, cx),
                    None => cx.notify(),
                }
            })
            .ok();
        }));
    }

    pub fn select(&mut self, preset: WrappedPreset, cx: &mut Context<Self>) {
        if !self
            .selected
            .as_ref()
            .is_some_and(|selected| same_range(selected, &preset))
        {
            self.wrapped.reset();
        }
        self.wrapped.start();
        let client = self.client.clone();
        let (from, to) = (preset.from.clone(), preset.to.clone());
        self.selected = Some(preset);
        self._wrapped_task = Some(state::fetch(
            cx,
            move || {
                let (client, from, to) = (client.clone(), from.clone(), to.clone());
                async move { client.wrapped(&from, &to).await }
            },
            |s| &mut s.wrapped,
        ));
    }
}

/// Whether two presets summarise the same days. Compared by dates alone, so
/// a range that has since been computed (or relabelled) still counts.
fn same_range(a: &WrappedPreset, b: &WrappedPreset) -> bool {
    (&a.from, &a.to) == (&b.from, &b.to)
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
    fn a_range_is_the_same_whatever_its_label_or_computed_flag() {
        let mut a = preset("2026 so far", false);
        a.from = "2026-01-01".into();
        a.to = "2026-10-14".into();
        let mut b = a.clone();
        b.computed = true;
        b.label = "This year".into();
        assert!(same_range(&a, &b));
        b.to = "2026-10-15".into();
        assert!(!same_range(&a, &b));
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
