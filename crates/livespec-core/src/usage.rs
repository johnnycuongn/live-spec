//! The usage ledger: what each model call cost, in tokens and in list-price
//! dollars, kept per hour, project, kind and model in `<app_data>/usage.json`.
//! Prices are a pinned table: updating it is a deliberate one-line change.

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::llm::{ChatRequest, Chunk, OnChunk, Provider, Usage};
use crate::store::{self, Loaded};
use crate::Result;

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
    /// Cache writes and reads are priced at the input rate. Live Spec sets
    /// no `cache_control`, so the API reports none today; if any appear, this errs high.
    pub fn cost(&self, u: &Usage) -> f64 {
        let input = u.input as f64 + u.cache_write as f64 + u.cache_read as f64;
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

// ---------- the ledger ----------

/// Buckets older than this are dropped when the ledger loads.
pub const KEEP_DAYS: u64 = 90;

/// One hour (UTC, `unix_secs / 3600`) of one project's usage, for one kind and
/// one model. `cost_usd` is the total, estimates included; `estimated_cost_usd`
/// is the part that came from estimates. Unpriced runs add tokens, not cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub hour: u64,
    pub root: Option<String>,
    pub kind: UsageKind,
    pub model: String,
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub cost_usd: f64,
    pub runs: u64,
    pub estimated_runs: u64,
    pub estimated_cost_usd: f64,
    pub unpriced_runs: u64,
}

impl UsageBucket {
    fn empty(hour: u64, tag: &UsageTag, model: &str) -> Self {
        Self {
            hour,
            root: tag.root.clone(),
            kind: tag.kind,
            model: model.to_owned(),
            input: 0,
            cache_write: 0,
            cache_read: 0,
            output: 0,
            cost_usd: 0.0,
            runs: 0,
            estimated_runs: 0,
            estimated_cost_usd: 0.0,
            unpriced_runs: 0,
        }
    }

    fn key(&self) -> Key {
        (self.hour, self.root.clone(), self.kind, self.model.clone())
    }

    /// `new_run` is false for the second and later records of one call (a CLI call that used two models):
    /// they add tokens and cost but are not another run. `new_unpriced` says whether this record is the
    /// first unpriced one of its call.
    fn add(&mut self, u: &Usage, estimated: bool, new_run: bool, new_unpriced: bool) {
        // Counts come from a CLI or API payload: a bogus one saturates, never panics or wraps.
        self.input = self.input.saturating_add(u.input);
        self.cache_write = self.cache_write.saturating_add(u.cache_write);
        self.cache_read = self.cache_read.saturating_add(u.cache_read);
        self.output = self.output.saturating_add(u.output);
        if new_run {
            self.runs = self.runs.saturating_add(1);
        }
        if estimated && new_run {
            self.estimated_runs = self.estimated_runs.saturating_add(1);
        }
        match u.cost_usd {
            // NaN or infinity would serialize as null and make usage.json unreadable: count it as unpriced.
            Some(cost) if cost.is_finite() => {
                self.cost_usd += cost;
                if estimated {
                    self.estimated_cost_usd += cost;
                }
            }
            _ if new_unpriced => self.unpriced_runs = self.unpriced_runs.saturating_add(1),
            _ => {}
        }
    }

    /// Adds another bucket's counts (same key on load; any key for totals in tests).
    fn merge(&mut self, o: &UsageBucket) {
        self.input = self.input.saturating_add(o.input);
        self.cache_write = self.cache_write.saturating_add(o.cache_write);
        self.cache_read = self.cache_read.saturating_add(o.cache_read);
        self.output = self.output.saturating_add(o.output);
        self.cost_usd += o.cost_usd;
        self.runs = self.runs.saturating_add(o.runs);
        self.estimated_runs = self.estimated_runs.saturating_add(o.estimated_runs);
        self.estimated_cost_usd += o.estimated_cost_usd;
        self.unpriced_runs = self.unpriced_runs.saturating_add(o.unpriced_runs);
    }
}

/// (hour, root, kind, model): one bucket per key.
type Key = (u64, Option<String>, UsageKind, String);

type OnRecord = Box<dyn Fn() + Send + Sync>;

#[derive(Serialize, Deserialize)]
struct LedgerFile {
    version: u32,
    buckets: Vec<UsageBucket>,
}

struct Inner {
    buckets: BTreeMap<Key, UsageBucket>,
    /// Changed since the last write.
    dirty: bool,
    /// False when usage.json exists but couldn't be read: it's never overwritten.
    persist: bool,
}

/// Every model call's usage, in memory, saved to `usage.json` by the shell
/// (`flush_if_dirty` every 5 s, `flush` on exit). `std` locks only, held for
/// synchronous work: `Meter::drop` records through it from aborted tasks.
pub struct UsageLedger {
    path: PathBuf,
    inner: Mutex<Inner>,
    /// Serializes writers, so an older snapshot never lands after a newer one.
    writing: Mutex<()>,
    on_record: Mutex<Option<OnRecord>>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A panic elsewhere never stops the ledger: it holds plain numbers.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the ledger starts with, and whether it may write `usage.json`, given
/// what loading found. A file that exists but couldn't be read, or couldn't be
/// set aside as `.bad`, is never written over.
fn start_from(loaded: Result<Loaded<LedgerFile>>, path: &Path) -> (Vec<UsageBucket>, bool) {
    match loaded {
        Ok(Loaded::Parsed(file)) => (file.buckets, true),
        Ok(Loaded::Missing) => (Vec::new(), true),
        Ok(Loaded::Corrupt { kept }) => {
            let name = kept.as_ref().map_or_else(|| "nothing".to_owned(), |p| p.display().to_string());
            eprintln!("usage.json couldn't be parsed; kept it as {name} and started a new ledger");
            (Vec::new(), kept.is_some())
        }
        Err(e) => {
            eprintln!("Couldn't read {}: {e}. Usage is counted for this session but not saved.", path.display());
            (Vec::new(), false)
        }
    }
}

impl UsageLedger {
    pub fn new(path: PathBuf) -> Self {
        Self::load_at(path, now_secs())
    }

    /// Never fails: a missing or corrupt file starts an empty ledger (a corrupt
    /// one is kept as `.bad`), and one that can't be read is left alone and not
    /// saved over. Buckets older than `KEEP_DAYS` are dropped.
    fn load_at(path: PathBuf, now: u64) -> Self {
        let (loaded, persist) = start_from(store::load_json::<LedgerFile>(&path), &path);
        let oldest = (now / 3600).saturating_sub(KEEP_DAYS * 24);
        let mut buckets = BTreeMap::new();
        for b in loaded {
            if b.hour < oldest {
                continue;
            }
            match buckets.entry(b.key()) {
                Entry::Occupied(mut have) => UsageBucket::merge(have.get_mut(), &b),
                Entry::Vacant(slot) => {
                    slot.insert(b);
                }
            }
        }
        Self {
            path,
            inner: Mutex::new(Inner { buckets, dirty: false, persist }),
            writing: Mutex::new(()),
            on_record: Mutex::new(None),
        }
    }

    /// The shell's hook, called after every record (outside the ledger's lock).
    pub fn set_on_record(&self, f: OnRecord) {
        *lock(&self.on_record) = Some(f);
    }

    /// Adds one call to its hour's bucket. Synchronous and never awaits, so
    /// `Meter::drop` can call it from a task that is being aborted.
    pub fn record(&self, tag: &UsageTag, usage: &Usage, estimated: bool) {
        self.record_part(tag, usage, estimated, true, true)
    }

    /// One record of a call. A call that reports several models records each; `Meter` passes
    /// `new_run` / `new_unpriced` so the call still counts as one run (and one unpriced run) in total.
    fn record_part(&self, tag: &UsageTag, usage: &Usage, estimated: bool, new_run: bool, new_unpriced: bool) {
        self.record_part_at(now_secs(), tag, usage, estimated, new_run, new_unpriced)
    }

    #[cfg(test)]
    fn record_at(&self, now: u64, tag: &UsageTag, usage: &Usage, estimated: bool) {
        self.record_part_at(now, tag, usage, estimated, true, true)
    }

    fn record_part_at(&self, now: u64, tag: &UsageTag, usage: &Usage, estimated: bool, new_run: bool, new_unpriced: bool) {
        {
            let mut inner = lock(&self.inner);
            let hour = now / 3600;
            inner
                .buckets
                .entry((hour, tag.root.clone(), tag.kind, usage.model.clone()))
                .or_insert_with(|| UsageBucket::empty(hour, tag, &usage.model))
                .add(usage, estimated, new_run, new_unpriced);
            inner.dirty = true;
        }
        if let Some(on_record) = lock(&self.on_record).as_ref() {
            on_record();
        }
    }

    /// Buckets from `since_hour` on, sorted by (hour, root, kind, model).
    pub fn report(&self, since_hour: u64) -> Vec<UsageBucket> {
        lock(&self.inner).buckets.values().filter(|b| b.hour >= since_hour).cloned().collect()
    }

    /// Writes the ledger if anything changed since the last write.
    pub fn flush_if_dirty(&self) -> Result<()> {
        self.write(false)
    }

    /// Writes the ledger now (on exit), which also saves the 90-day prune.
    pub fn flush(&self) -> Result<()> {
        self.write(true)
    }

    fn write(&self, always: bool) -> Result<()> {
        let _writing = lock(&self.writing);
        let bytes = {
            let mut inner = lock(&self.inner);
            if !inner.persist || !(always || inner.dirty) {
                return Ok(());
            }
            let file = LedgerFile { version: 1, buckets: inner.buckets.values().cloned().collect() };
            inner.dirty = false;
            serde_json::to_vec(&file)?
        };
        if let Err(e) = store::write_atomic(&self.path, &bytes) {
            lock(&self.inner).dirty = true; // try again at the next flush
            return Err(e);
        }
        Ok(())
    }
}

// ---------- metering one call ----------

/// Wraps one model call. A `Chunk::Usage` is recorded exactly; a call that
/// succeeds without one, or is dropped mid-stream (a keystroke aborted it), is
/// estimated from its characters; a call that fails without one records nothing.
pub struct Meter {
    ledger: Arc<UsageLedger>,
    tag: UsageTag,
    model: String,
    sent_chars: usize,
    received_chars: usize,
    /// A `Chunk::Usage` arrived and was recorded.
    reported: bool,
    /// A record of this call already counted as unpriced.
    unpriced_counted: bool,
    /// `finish` ran, so `Drop` adds nothing.
    settled: bool,
}

impl Meter {
    pub fn new(ledger: Arc<UsageLedger>, tag: UsageTag, model: String, sent_chars: usize) -> Self {
        Self { ledger, tag, model, sent_chars, received_chars: 0, reported: false, unpriced_counted: false, settled: false }
    }

    pub fn observe(&mut self, chunk: &Chunk) {
        match chunk {
            Chunk::Text(t) => self.received_chars += t.chars().count(),
            Chunk::Activity(_) => {}
            Chunk::Usage(u) => {
                // One call is one run, however many models it reports: only the first record counts it.
                let unpriced = !u.cost_usd.is_some_and(f64::is_finite);
                let new_unpriced = unpriced && !self.unpriced_counted;
                self.ledger.record_part(&self.tag, u, false, !self.reported, new_unpriced);
                self.unpriced_counted |= unpriced;
                self.reported = true;
            }
        }
    }

    /// Ends the call. A call that failed without reporting usage (bad key, CLI
    /// missing) is assumed unbilled.
    pub fn finish(mut self, ok: bool) {
        if ok && !self.reported {
            self.record_estimate();
        }
        self.settled = true;
    }

    /// `ceil(chars / 4)` tokens each way, all input at the input rate, priced by
    /// the model the provider was asked for.
    fn record_estimate(&self) {
        let tokens = |chars: usize| chars.div_ceil(4) as u64;
        let mut u = Usage {
            model: self.model.clone(),
            input: tokens(self.sent_chars),
            cache_write: 0,
            cache_read: 0,
            output: tokens(self.received_chars),
            cost_usd: None,
        };
        u.cost_usd = price(&u.model).map(|p| p.cost(&u));
        self.ledger.record(&self.tag, &u, true);
    }
}

/// Dropped before `finish`: the call was aborted mid-stream, after the request
/// went out. Records through the ledger's `std` mutex; never awaits.
impl Drop for Meter {
    fn drop(&mut self) {
        if !self.settled && !self.reported {
            self.record_estimate();
        }
    }
}

/// Characters sent: `system` + `user`, as Unicode scalars (not bytes).
pub fn sent_chars(req: &ChatRequest) -> usize {
    req.system.chars().count() + req.user.chars().count()
}

/// One model call, metered. The meter is created here, immediately before
/// `provider.stream`, so a run cancelled earlier (during the Jev pre-phase)
/// records nothing; if this future is dropped mid-stream, the meter's `Drop`
/// records an estimate. `Text` and `Activity` go on to `on_chunk`; `Usage` is
/// recorded and not forwarded.
pub async fn metered(
    provider: &Provider,
    req: &ChatRequest,
    ledger: Arc<UsageLedger>,
    tag: UsageTag,
    on_chunk: OnChunk<'_>,
) -> Result<String> {
    let mut meter = Meter::new(ledger, tag, provider.model().to_owned(), sent_chars(req));
    let result = provider
        .stream(req, &mut |chunk: Chunk| {
            meter.observe(&chunk);
            if !matches!(chunk, Chunk::Usage(_)) {
                on_chunk(chunk);
            }
        })
        .await;
    meter.finish(result.is_ok());
    result
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

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// 2025-10-14T00:00:00Z: the spec's example hour, 489000.
    const T: u64 = 1_760_400_000;
    const DAY: u64 = 86_400;

    fn tag(root: &str, kind: UsageKind) -> UsageTag {
        UsageTag { root: Some(root.into()), kind }
    }

    fn used(model: &str, input: u64, output: u64, cost_usd: Option<f64>) -> Usage {
        Usage { model: model.into(), input, cache_write: 0, cache_read: 0, output, cost_usd }
    }

    fn ledger() -> (tempfile::TempDir, UsageLedger) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::load_at(dir.path().join("usage.json"), T);
        (dir, ledger)
    }

    #[test]
    fn runs_add_up_in_hourly_buckets_per_project_kind_and_model() {
        let (_dir, l) = ledger();
        let notes = tag("/a", UsageKind::Notes);
        l.record_at(T + 10, &notes, &used("claude-haiku-5-5", 100, 10, Some(0.001)), false);
        l.record_at(T + 3599, &notes, &used("claude-haiku-5-5", 200, 20, Some(0.002)), false);
        l.record_at(T + 3600, &notes, &used("claude-haiku-5-5", 1, 1, Some(0.0)), false);
        l.record_at(T + 20, &tag("/b", UsageKind::Notes), &used("claude-haiku-5-5", 1, 1, Some(0.0)), false);
        l.record_at(T + 30, &tag("/a", UsageKind::Brief), &used("claude-sonnet-5-5", 1, 1, Some(0.0)), false);
        l.record_at(T + 40, &notes, &used("haiku", 1, 1, Some(0.0)), false);
        let all = l.report(0);
        assert_eq!(all.len(), 5, "{all:?}");
        let first = all
            .iter()
            .find(|b| b.hour == 489_000 && b.root.as_deref() == Some("/a") && b.kind == UsageKind::Notes && b.model == "claude-haiku-5-5")
            .unwrap();
        assert_eq!((first.input, first.output, first.runs), (300, 30, 2));
        assert!((first.cost_usd - 0.003).abs() < 1e-12);
        let later = l.report(489_001);
        assert_eq!(later.len(), 1, "report starts at the given hour");
        assert_eq!(later[0].hour, 489_001);
    }

    #[test]
    fn estimates_and_unpriced_runs_are_kept_apart() {
        let (_dir, l) = ledger();
        let t = tag("/a", UsageKind::Notes);
        l.record_at(T, &t, &used("m", 10, 0, Some(0.5)), false);
        l.record_at(T, &t, &used("m", 10, 0, Some(0.25)), true);
        l.record_at(T, &t, &used("m", 10, 0, None), false);
        let b = &l.report(0)[0];
        assert_eq!((b.runs, b.estimated_runs, b.unpriced_runs), (3, 1, 1));
        assert_eq!(b.cost_usd, 0.75, "the total includes estimates");
        assert_eq!(b.estimated_cost_usd, 0.25);
        assert_eq!(b.input, 30, "unpriced runs still count their tokens");
    }

    #[test]
    fn the_file_has_the_spec_shape() {
        let (dir, l) = ledger();
        let u = Usage { model: "claude-haiku-5-5".into(), input: 1200, cache_write: 22000, cache_read: 90000, output: 3100, cost_usd: Some(0.0123) };
        l.record_at(T, &tag("/Users/me/code/shop-api", UsageKind::Notes), &u, false);
        l.flush().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("usage.json")).unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "version": 1, "buckets": [{
                "hour": 489000, "root": "/Users/me/code/shop-api", "kind": "notes", "model": "claude-haiku-5-5",
                "input": 1200, "cacheWrite": 22000, "cacheRead": 90000, "output": 3100,
                "costUsd": 0.0123, "runs": 1, "estimatedRuns": 0, "estimatedCostUsd": 0.0, "unpricedRuns": 0 }] })
        );
    }

    #[test]
    fn the_ledger_survives_a_restart_and_forgets_after_90_days() {
        let (dir, l) = ledger();
        let t = tag("/a", UsageKind::Notes);
        l.record_at(T - 91 * DAY, &t, &used("m", 1, 1, Some(0.0)), false);
        l.record_at(T - 89 * DAY, &t, &used("m", 2, 2, Some(0.0)), false);
        l.record_at(T, &tag("/b", UsageKind::Brief), &used("m", 3, 3, None), true);
        l.flush().unwrap();
        let again = UsageLedger::load_at(dir.path().join("usage.json"), T);
        let kept: Vec<(u64, u64)> = again.report(0).iter().map(|b| (b.hour, b.input)).collect();
        assert_eq!(kept, [((T - 89 * DAY) / 3600, 2), (T / 3600, 3)]);
        assert_eq!(again.report(0)[1].unpriced_runs, 1, "every field survives the round trip");
    }

    #[test]
    fn a_corrupt_ledger_is_kept_as_bad_and_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.json");
        std::fs::write(&path, "{ not json").unwrap();
        let l = UsageLedger::load_at(path.clone(), T);
        assert!(l.report(0).is_empty());
        let bad = dir.path().join("usage.json.bad");
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ not json");
        l.record_at(T, &tag("/a", UsageKind::Notes), &used("m", 1, 1, Some(0.0)), false);
        l.flush().unwrap();
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ not json", "later writes never touch the .bad copy");
        assert_eq!(UsageLedger::load_at(path, T).report(0).len(), 1);
    }

    #[test]
    fn an_unreadable_ledger_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.json");
        std::fs::create_dir(&path).unwrap(); // in the way: a read error, not a parse error
        let l = UsageLedger::load_at(path.clone(), T);
        l.record_at(T, &tag("/a", UsageKind::Notes), &used("m", 1, 1, Some(0.0)), false);
        assert_eq!(l.report(0).len(), 1, "still counted for this session");
        l.flush().unwrap();
        l.flush_if_dirty().unwrap();
        assert!(path.is_dir(), "left in place");
        assert!(!dir.path().join("usage.json.bad").exists());
    }

    #[test]
    fn a_corrupt_file_that_could_not_be_set_aside_is_never_overwritten() {
        let (_, persist) = start_from(Ok(Loaded::Corrupt { kept: None }), Path::new("usage.json"));
        assert!(!persist, "the unparseable file we couldn't keep must not be written over");
        let (_, persist) = start_from(Ok(Loaded::Corrupt { kept: Some(PathBuf::from("usage.json.bad")) }), Path::new("usage.json"));
        assert!(persist, "once it is safely set aside the ledger may write a fresh file");
    }

    #[test]
    fn huge_counts_saturate_instead_of_overflowing() {
        let (_dir, l) = ledger();
        let t = tag("/a", UsageKind::Notes);
        let huge = Usage {
            model: "claude-haiku-5-5".into(),
            input: u64::MAX,
            cache_write: u64::MAX,
            cache_read: u64::MAX,
            output: u64::MAX,
            cost_usd: Some(1.0),
        };
        l.record_at(T, &t, &huge, false);
        l.record_at(T, &t, &huge, false);
        let b = &l.report(0)[0];
        assert_eq!((b.input, b.cache_write, b.cache_read, b.output), (u64::MAX, u64::MAX, u64::MAX, u64::MAX));
        assert_eq!(b.runs, 2);
        let mut sum = b.clone();
        sum.merge(b);
        assert_eq!((sum.input, sum.output, sum.runs), (u64::MAX, u64::MAX, 4));
        let c = p(0.10, 0.50).cost(&huge);
        assert!(c.is_finite() && c > 0.0, "the three input counts are summed as f64, not u64: {c}");
    }

    #[test]
    fn a_non_finite_cost_counts_as_unpriced() {
        let (_dir, l) = ledger();
        let t = tag("/a", UsageKind::Notes);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            l.record_at(T, &t, &used("m", 5, 0, Some(bad)), true);
        }
        l.record_at(T, &t, &used("m", 5, 0, Some(0.5)), false);
        let b = &l.report(0)[0];
        assert_eq!((b.runs, b.unpriced_runs, b.estimated_runs, b.input), (4, 3, 3, 20));
        assert_eq!((b.cost_usd, b.estimated_cost_usd), (0.5, 0.0));
        l.flush().unwrap(); // and the file still parses: a NaN would have serialized as null
    }

    #[test]
    fn flush_if_dirty_writes_only_after_a_change() {
        let (dir, l) = ledger();
        let path = dir.path().join("usage.json");
        l.flush_if_dirty().unwrap();
        assert!(!path.exists(), "nothing recorded, nothing written");
        l.record_at(T, &tag("/a", UsageKind::Notes), &used("m", 1, 1, Some(0.0)), false);
        l.flush_if_dirty().unwrap();
        assert!(path.exists());
        std::fs::remove_file(&path).unwrap();
        l.flush_if_dirty().unwrap();
        assert!(!path.exists(), "clean after a write");
    }

    #[test]
    fn on_record_runs_after_every_record_and_may_read_the_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let l = Arc::new(UsageLedger::load_at(dir.path().join("usage.json"), T));
        let calls = Arc::new(AtomicUsize::new(0));
        let (seen, weak) = (calls.clone(), Arc::downgrade(&l));
        l.set_on_record(Box::new(move || {
            // Called outside the ledger's lock: reading it here must not deadlock.
            let runs: u64 = weak.upgrade().map_or(0, |l| l.report(0).iter().map(|b| b.runs).sum());
            seen.fetch_add(runs as usize, Ordering::SeqCst);
        }));
        l.record_at(T, &tag("/a", UsageKind::Notes), &used("m", 1, 1, Some(0.0)), false);
        l.record_at(T, &tag("/a", UsageKind::Notes), &used("m", 1, 1, Some(0.0)), false);
        assert_eq!(calls.load(Ordering::SeqCst), 1 + 2);
    }

    use std::time::Duration;

    use crate::jev::test_server::{serve, Canned};
    use crate::llm::OpenAiCompat;

    fn shared() -> (tempfile::TempDir, Arc<UsageLedger>) {
        let (dir, l) = ledger();
        (dir, Arc::new(l))
    }

    /// Every bucket summed, so a test that straddles an hour boundary still passes.
    fn total(l: &UsageLedger) -> UsageBucket {
        let all = l.report(0);
        let mut sum = all.first().cloned().expect("something was recorded");
        for b in &all[1..] {
            sum.merge(b);
        }
        sum
    }

    #[test]
    fn a_meter_dropped_mid_call_records_one_estimate() {
        let (_dir, l) = shared();
        let mut m = Meter::new(l.clone(), tag("/a", UsageKind::Notes), "claude-haiku-5-5".into(), 4001);
        m.observe(&Chunk::Text("éééé".into())); // 4 characters, 8 bytes: one token
        m.observe(&Chunk::Activity("Read src/main.rs".into()));
        drop(m);
        let b = total(&l);
        assert_eq!((b.input, b.output, b.runs, b.estimated_runs), (1001, 1, 1, 1), "ceil(4001 / 4) and ceil(4 / 4)");
        let expected = (1001.0 * 0.10 + 1.0 * 0.50) / 1e6;
        assert!((b.cost_usd - expected).abs() < 1e-15);
        assert!((b.estimated_cost_usd - expected).abs() < 1e-15);
    }

    #[test]
    fn finishing_without_usage_estimates_only_when_the_call_succeeded() {
        let (_dir, l) = shared();
        Meter::new(l.clone(), tag("/a", UsageKind::Notes), "haiku".into(), 40).finish(true);
        Meter::new(l.clone(), tag("/b", UsageKind::Notes), "haiku".into(), 40).finish(false);
        let b = total(&l);
        assert_eq!((b.runs, b.estimated_runs, b.input), (1, 1, 10), "a failed call without usage is assumed unbilled");
        assert_eq!(l.report(0)[0].root.as_deref(), Some("/a"));
    }

    #[test]
    fn reported_usage_is_recorded_exactly_once() {
        let (_dir, l) = shared();
        let exact = Usage { model: "claude-haiku-5-5".into(), input: 2, cache_write: 2198, cache_read: 535, output: 45, cost_usd: Some(0.00046765) };
        let meter = || Meter::new(l.clone(), tag("/a", UsageKind::Notes), "haiku".into(), 4000);
        let mut m = meter();
        m.observe(&Chunk::Usage(exact.clone()));
        m.finish(true);
        let mut m = meter();
        m.observe(&Chunk::Usage(exact.clone()));
        drop(m);
        let mut m = meter();
        m.observe(&Chunk::Usage(exact));
        m.finish(false); // a failed run whose result still carried usage
        let b = total(&l);
        assert_eq!((b.runs, b.estimated_runs, b.input, b.cache_write), (3, 0, 6, 3 * 2198));
        assert!((b.cost_usd - 3.0 * 0.00046765).abs() < 1e-15);
    }

    #[test]
    fn a_call_that_used_two_models_is_one_run_with_both_models_tokens() {
        let (_dir, l) = shared();
        let mut m = Meter::new(l.clone(), tag("/a", UsageKind::Brief), "sonnet".into(), 4000);
        m.observe(&Chunk::Usage(used("claude-sonnet-5-5", 100, 10, Some(0.001))));
        m.observe(&Chunk::Usage(used("claude-haiku-5-5", 40, 4, Some(0.0002))));
        m.finish(true);
        let report = l.report(0);
        assert_eq!(report.len(), 2, "one bucket per model");
        let by = |model: &str| report.iter().find(|b| b.model == model).unwrap().clone();
        let (sonnet, haiku) = (by("claude-sonnet-5-5"), by("claude-haiku-5-5"));
        assert_eq!((sonnet.input, sonnet.output, sonnet.runs), (100, 10, 1));
        assert_eq!((haiku.input, haiku.output, haiku.runs), (40, 4, 0), "tokens and cost, but not a second run");
        assert!((haiku.cost_usd - 0.0002).abs() < 1e-15);
        assert_eq!(total(&l).runs, 1);
    }

    #[test]
    fn a_second_unpriced_model_marks_the_call_unpriced_once() {
        let (_dir, l) = shared();
        let mut m = Meter::new(l.clone(), tag("/a", UsageKind::Brief), "sonnet".into(), 4000);
        m.observe(&Chunk::Usage(used("claude-sonnet-5-5", 100, 10, Some(0.001))));
        m.observe(&Chunk::Usage(used("claude-sonnet-9", 40, 4, None)));
        m.observe(&Chunk::Usage(used("claude-sonnet-10", 8, 1, None)));
        m.finish(true);
        let b = total(&l);
        assert_eq!((b.runs, b.unpriced_runs, b.estimated_runs, b.input), (1, 1, 0, 148));
        let unpriced: u64 = l.report(0).iter().map(|b| b.unpriced_runs).sum();
        assert_eq!(unpriced, 1, "two unpriced records, one unpriced call");

        let (_dir, l) = shared();
        let mut m = Meter::new(l.clone(), tag("/a", UsageKind::Brief), "sonnet".into(), 4000);
        m.observe(&Chunk::Usage(used("claude-sonnet-9", 40, 4, None)));
        m.observe(&Chunk::Usage(used("claude-sonnet-10", 8, 1, None)));
        m.finish(true);
        assert_eq!((total(&l).runs, total(&l).unpriced_runs), (1, 1), "unpriced first: not counted again");
    }

    #[test]
    fn an_estimate_for_an_unpriced_model_counts_tokens_but_no_cost() {
        let (_dir, l) = shared();
        drop(Meter::new(l.clone(), tag("/a", UsageKind::Notes), "claude-sonnet-9".into(), 400));
        let b = total(&l);
        assert_eq!((b.input, b.runs, b.unpriced_runs, b.cost_usd), (100, 1, 1, 0.0));
    }

    #[test]
    fn sent_chars_counts_characters_not_bytes() {
        let req = ChatRequest { system: "ééé".into(), user: "ab".into(), max_tokens: 1, cwd: None, allow_repo_tools: false, max_turns: None };
        assert_eq!(sent_chars(&req), 5);
    }

    fn local(base_url: String, model: &str) -> Provider {
        Provider::OpenAiCompat(OpenAiCompat { base_url, model: model.into(), context_tokens: 4096, http: reqwest::Client::new() })
    }

    /// 400 characters: an estimate of 100 input tokens.
    fn request() -> ChatRequest {
        ChatRequest { system: "s".repeat(40), user: "u".repeat(360), max_tokens: 100, cwd: None, allow_repo_tools: false, max_turns: None }
    }

    const WITH_USAGE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n\
        data: {\"choices\":[{\"finish_reason\":\"stop\",\"delta\":{}}],\"usage\":{\"prompt_tokens\":31,\"completion_tokens\":2}}\n\n\
        data: [DONE]\n\n";
    const WITHOUT_USAGE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n\n";

    #[tokio::test]
    async fn a_reported_call_is_recorded_exactly_and_never_forwarded() {
        let (url, _) = serve(vec![Canned::new(200, WITH_USAGE)]).await;
        let (_dir, l) = shared();
        let mut seen = Vec::new();
        let provider = local(url, "qwen3-4b-instruct");
        let text = metered(&provider, &request(), l.clone(), tag("/a", UsageKind::Notes), &mut |c: Chunk| seen.push(c)).await.unwrap();
        assert_eq!(text, "Hi");
        assert_eq!(seen, vec![Chunk::Text("Hi".into())], "usage is recorded, never shown");
        let b = total(&l);
        assert_eq!((b.input, b.output, b.runs, b.estimated_runs, b.cost_usd), (31, 2, 1, 0, 0.0));
    }

    #[tokio::test]
    async fn a_call_without_usage_is_estimated_from_its_characters() {
        let (url, _) = serve(vec![Canned::new(200, WITHOUT_USAGE)]).await;
        let (_dir, l) = shared();
        let provider = local(url, "claude-haiku-5-5"); // priced, to check the arithmetic
        metered(&provider, &request(), l.clone(), tag("/a", UsageKind::Notes), &mut |_: Chunk| {}).await.unwrap();
        let b = total(&l);
        assert_eq!((b.input, b.output, b.estimated_runs), (100, 1, 1), "400 characters sent, 2 received");
        assert!((b.estimated_cost_usd - (100.0 * 0.10 + 1.0 * 0.50) / 1e6).abs() < 1e-15);
    }

    #[tokio::test]
    async fn a_failed_call_without_usage_records_nothing() {
        let (url, _) = serve(vec![Canned::new(500, "{}")]).await;
        let (_dir, l) = shared();
        let provider = local(url, "qwen3-4b-instruct");
        let result = metered(&provider, &request(), l.clone(), tag("/a", UsageKind::Notes), &mut |_: Chunk| {}).await;
        assert!(result.is_err());
        assert!(l.report(0).is_empty());
    }

    #[tokio::test]
    async fn a_call_dropped_mid_stream_records_one_estimate() {
        let (url, _) = serve(vec![Canned::new(200, WITH_USAGE).delayed(5_000)]).await;
        let (_dir, l) = shared();
        let (provider, req) = (local(url, "qwen3-4b-instruct"), request());
        let mut sink = |_: Chunk| {};
        let call = metered(&provider, &req, l.clone(), tag("/a", UsageKind::Notes), &mut sink);
        assert!(tokio::time::timeout(Duration::from_millis(200), call).await.is_err(), "the keystroke came first");
        let b = total(&l);
        assert_eq!((b.runs, b.estimated_runs, b.input, b.output, b.cost_usd), (1, 1, 100, 0, 0.0), "local models are free");
    }

    #[tokio::test]
    async fn a_run_cancelled_before_the_request_records_nothing() {
        let (url, hits) = serve(vec![Canned::new(200, WITH_USAGE)]).await;
        let (_dir, l) = shared();
        let provider = local(url, "qwen3-4b-instruct");
        let run = async {
            tokio::time::sleep(Duration::from_secs(5)).await; // the Jev pre-phase
            metered(&provider, &request(), l.clone(), tag("/a", UsageKind::Notes), &mut |_: Chunk| {}).await
        };
        assert!(tokio::time::timeout(Duration::from_millis(100), run).await.is_err());
        assert!(l.report(0).is_empty(), "no meter existed yet");
        assert_eq!(hits.load(Ordering::SeqCst), 0, "and no request was sent");
    }
}
