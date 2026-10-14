//! The usage ledger: what each model call cost, in tokens and in list-price
//! dollars, kept per hour, project, kind and model in `<app_data>/usage.json`.
//! Prices are a pinned table: updating it is a deliberate one-line change.

use serde::{Deserialize, Serialize};

use crate::llm::Usage;

/// What a call was for. The UI breaks a day down by project and kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageKind {
    Notes,
    Brief,
}

/// Where a call's usage is filed. `root: None` is a tab with no project.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageTag {
    pub root: Option<String>,
    pub kind: UsageKind,
}

/// List price in dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

impl Price {
    /// Cache writes and reads are priced at the input rate. LiveBrainstorm sets
    /// no `cache_control`, so the API reports none today; if any appear, this errs high.
    pub fn cost(&self, u: &Usage) -> f64 {
        let input = (u.input + u.cache_write + u.cache_read) as f64;
        (input * self.input_per_mtok + u.output as f64 * self.output_per_mtok) / 1_000_000.0
    }
}

/// List prices, $ per million tokens: (model id, input, output). Source: the
/// Claude API reference model table, cached 2026-10-06. Haiku 5.5 costs more
/// above 100K prompt tokens, but analysis prompts stay near 30K, so the lower
/// rate is used. Updating this table is a deliberate one-line change.
pub const PRICES: &[(&str, f64, f64)] = &[
    ("claude-fable-5-1", 10.00, 50.00),
    ("claude-fable-5", 10.00, 50.00),
    ("claude-opus-5-5", 4.00, 20.00),
    ("claude-opus-5", 5.00, 25.00),
    ("claude-opus-4-8", 5.00, 25.00),
    ("claude-opus-4-7", 5.00, 25.00),
    ("claude-opus-4-6", 5.00, 25.00),
    ("claude-sonnet-5-5", 2.00, 10.00),
    ("claude-sonnet-5", 2.00, 10.00),
    ("claude-sonnet-4-6", 3.00, 15.00),
    ("claude-haiku-5-5", 0.10, 0.50),
    ("claude-haiku-4-5", 1.00, 5.00),
];

/// The Claude Code aliases, each the newest model of its family.
const ALIASES: [(&str, &str); 4] = [
    ("fable", "claude-fable-5-1"),
    ("opus", "claude-opus-5-5"),
    ("sonnet", "claude-sonnet-5-5"),
    ("haiku", "claude-haiku-5-5"),
];

/// Exact id first, then a CLI alias, then a local catalog model (free on this
/// machine). Anything else has no known price: its tokens still count.
pub fn price(model: &str) -> Option<Price> {
    let row = |id: &str| PRICES.iter().find(|(m, _, _)| *m == id);
    let alias = || ALIASES.iter().find(|(a, _)| *a == model).and_then(|&(_, id)| row(id));
    if let Some((_, input, output)) = row(model).or_else(alias) {
        return Some(Price { input_per_mtok: *input, output_per_mtok: *output });
    }
    crate::local::catalog::find(model).map(|_| Price { input_per_mtok: 0.0, output_per_mtok: 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn p(input_per_mtok: f64, output_per_mtok: f64) -> Price {
        Price { input_per_mtok, output_per_mtok }
    }

    #[test]
    fn every_model_in_the_spec_table_has_its_list_price() {
        let table = [
            ("claude-fable-5-1", p(10.0, 50.0)),
            ("claude-fable-5", p(10.0, 50.0)),
            ("claude-opus-5-5", p(4.0, 20.0)),
            ("claude-opus-5", p(5.0, 25.0)),
            ("claude-opus-4-8", p(5.0, 25.0)),
            ("claude-opus-4-7", p(5.0, 25.0)),
            ("claude-opus-4-6", p(5.0, 25.0)),
            ("claude-sonnet-5-5", p(2.0, 10.0)),
            ("claude-sonnet-5", p(2.0, 10.0)),
            ("claude-sonnet-4-6", p(3.0, 15.0)),
            ("claude-haiku-5-5", p(0.10, 0.50)),
            ("claude-haiku-4-5", p(1.0, 5.0)),
        ];
        for (id, want) in table {
            assert_eq!(price(id), Some(want), "{id}");
        }
        assert_eq!(PRICES.len(), table.len(), "no row the spec doesn't list");
    }

    #[test]
    fn the_cli_aliases_mean_the_newest_of_their_family() {
        assert_eq!(price("fable"), price("claude-fable-5-1"));
        assert_eq!(price("opus"), price("claude-opus-5-5"));
        assert_eq!(price("sonnet"), price("claude-sonnet-5-5"));
        assert_eq!(price("haiku"), price("claude-haiku-5-5"));
    }

    #[test]
    fn unknown_models_have_no_price() {
        assert_eq!(price("claude-sonnet-9"), None, "a newer family is unknown, not free");
        assert_eq!(price("Haiku"), None, "ids and aliases are exact");
        assert_eq!(price(""), None);
    }

    #[test]
    fn local_catalog_models_are_free() {
        assert_eq!(price("qwen3-4b-instruct"), Some(p(0.0, 0.0)));
        assert_eq!(price("llama-3.2-3b-instruct"), Some(p(0.0, 0.0)));
    }

    #[test]
    fn cache_tokens_are_priced_at_the_input_rate() {
        let u = Usage {
            model: "claude-haiku-5-5".into(),
            input: 1_000_000,
            cache_write: 1_000_000,
            cache_read: 1_000_000,
            output: 1_000_000,
            cost_usd: None,
        };
        assert!((p(0.10, 0.50).cost(&u) - 0.80).abs() < 1e-12);
        let small = Usage { input: 1200, cache_write: 0, cache_read: 0, output: 310, ..u };
        assert!((p(0.10, 0.50).cost(&small) - 0.000275).abs() < 1e-15);
    }
}
