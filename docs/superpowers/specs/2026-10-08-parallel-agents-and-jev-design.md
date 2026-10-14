# Parallel agents and Jev: design

Date: 2026-10-08 · Status: approved in conversation, awaiting written-spec review

## Goal

Make LiveBrainstorm faster at both of its slow jobs: building the project brief, and writing the live notes. Ground the notes in the real code without adding Claude usage. Do this with:

1. **Parallel agents.** The brief is mapped by several explorer agents at once, and the notes are written by several section agents at once. The user caps how many run in parallel.
2. **Jev**, TypeSafe's System One model: fast, typed, calibrated judgments. Jev is used for three decisions on each pause. It never generates text.

## Decisions (from the brainstorm)

| Topic | Decision | Source |
|---|---|---|
| "Search" to speed up | Both: the brief, and code lookup on every pause | User |
| Lookup UX | Notes first, then a "Checked in code" section a few seconds later | User |
| Jev uses | Gate re-analysis, rerank code snippets, fact-check the notes | User |
| Architecture | One orchestrator in the Rust core (approach A) | User |
| What max agents limits | Only LLM agents; Jev requests have their own fixed limit | User approved |
| Default max agents | 4 | User approved |
| Key source | Project `.env.local` for local development, built first. A Keychain key set in Settings takes precedence when present (lookup order below). | User |
| Parallel notes | The four sections can be split across agents, but **off by default** (`agents.parallelNotes: false`) | Originally assumed on; turned off after the phase 3 measurement showed 4 lanes slower than 1 with Claude Code |
| Live code lookup | Done by plain code plus Jev checks, *not* by LLM agents grepping files | Follows from the Jev choice |

## Non-goals

- Using Jev to write notes or briefs. It is not a generative model.
- LLM "lookup agents" that grep the repo on every pause (replaced by the Jev fact-check).
- A different model per agent; Ollama or LM Studio providers; a persistent `claude` session.
- Parallelism for the local model. One llama-server would just queue the requests.

## Prerequisite: harden the Claude Code invocation

Added after a security review of the initial commit, which found two problems in `llm/claude_cli.rs`. Pending the user's plan review.

1. **Untrusted workspace config.** `claude -p` runs with the opened project as its working directory, and print mode skips Claude Code's workspace-trust prompt. A cloned repo's `.claude/settings.json` hooks or `.mcp.json` servers could therefore run commands.
2. **Incomplete denylist.** Tools are blocked by a fixed `--disallowedTools` list that misses newer tools (`Agent`, `PowerShell`, MCP tools). If the user's own settings pre-approve such tools, prompt injection from repo files could reach them.

**Fix**, using flags verified in Claude Code 2.1.293:
- Every run passes `--restricted`, which ignores user, project and local settings files, confines file tools to the working directory and refuses bypassPermissions.
- Every run passes `--strict-mcp-config` with no `--mcp-config`, so no MCP servers start.
- Tools come from an allowlist via `--tools`: `Read,Glob,Grep` for brief explorers and `""` for live analysis.
- The denylist is removed.
- An older CLI that rejects these flags gets the message "Update it with `claude update`" instead of a raw error.

This lands before the parallel explorers of phase 4 multiply the number of `claude` runs.

## Success criteria

- With max agents = 1 and no TypeSafe key, behaviour is identical to today. Covered by tests.
- First note appears at most 2 s later than today in the worst case (the gate and rerank time budget), and typically only one Jev round trip later. Measured.
- With 4 agents, complete notes arrive sooner than with 1, on this repo. Measured.
- With 4 explorers, the brief builds faster than with 1, on this repo. Measured.
- "Checked in code" fills in within about 2 s of the Faults section finishing. Measured.
- Typing during a fan-out leaves no orphaned `claude` processes. Tested by counting processes.

## Architecture

```
src/ (React)                 src-tauri/ (shell)                crates/livebrainstorm-core/
useLiveAnalysis ── analyze ──► commands::analyze ──────────────► analysis::run
  lanes + checks ◄── UiEvent ── maps (lane, Chunk) → Channel         ├─ jev::gate     ┐ concurrently,
NotesPane: sections                                                  ├─ jev::rerank   ┘ 2 s budget
  + "Checked in code"                                                ├─ agents::run_limited(section lanes)
SettingsDialog:                                                      │     └─ Faults lane → jev::check
  Agents + Jev groups       secrets: Keychain / env / .env.local     └─ prompt (per-section)
BriefDrawer ── build_brief ─► commands::build_brief ─────────────► explore::build
  agent-tagged activity                                              ├─ explore::slices (plain code)
                                                                     ├─ agents::run_limited(explorers)
                                                                     └─ merge call
```

### New core modules

| Module | Responsibility | Model |
|---|---|---|
| `agents` | `run_limited(limit, jobs)`: runs futures concurrently, at most `limit` at once, and returns results in job order. Runs inside one task, with no spawning, so dropping the parent future cancels every job and `kill_on_drop` kills each `claude` child. Also holds the lane-tagged sink type. | none |
| `jev` (`mod.rs`) | Typed HTTP client for `POST https://api.typesafe.ai/v1/systemone`: Noul and Choice questions and answers, retry, timeout. | Jev |
| `jev::gate` | Whether an edit changes the idea. | Jev Noul |
| `jev::rerank` | Picks the best snippets from a keyword shortlist. | Jev Noul per snippet |
| `jev::check` | Extracts claims from Faults, finds evidence in the index, and gets a verdict for each. | Jev Choice per claim |
| `analysis` | The per-pause pipeline. | composes the above |
| `project::explore` (new `project/explore.rs`) | Builds the brief: slicing, explorers, merge. | provider |

The existing `project/brief.rs` (the cache) is unchanged. The build logic lives in `explore.rs`, so the cache and the build stay separate.

### Changed core code

- `prompt.rs`: `analysis_request` takes the list of sections the agent must write, and the system prompt is assembled from per-section descriptions. For all four sections, the assembled prompt says what today's `ANALYSIS_SYSTEM` says. New explorer and merge prompts are added for the brief. `SECTIONS` stays the single source of section names.
- `project/retrieve.rs`: a `shortlist(index, query, n, per_snippet_chars)` that returns up to `n` snippets in TF-IDF order, reusing the existing scoring and window logic. `retrieve` keeps its signature.
- `project/digest.rs`: `build_slice(index, prefixes, budget)`, the same digest restricted to the given path prefixes.
- `settings.rs`: new fields (see Settings).
- `llm`: unchanged. Agents are just concurrent calls to `Provider::stream`.

## Live analysis pipeline (each pause)

Inputs from the UI: `draft`, plus `previous_draft`.
- **What `previous_draft` is:** the draft of the last analysis that actually ran and reached `done` for the current context (project root and brief timestamp).
- **Skipped pauses never update it,** so small edits accumulate against the last analysed draft until together they amount to a real change.
- **It is null** after a context change and for a manual "Try again".

1. **Pre-phase.** The gate and the rerank run concurrently, sharing one 2,000 ms deadline (`PRE_BUDGET`, to be tuned after measuring).
   - **Gate** runs only if the Jev gate is on, a key is present, and `previous_draft` is set. If the gate answers "no real change", emit `Skipped` and stop. Any error or timeout counts as "changed".
   - **Rerank** runs only if Jev rerank is on, a key is present, and a project is open. Otherwise use `retrieve` exactly as today.
   - Both start at once. If the gate skips, the rerank result is discarded and any rerank requests still running are cancelled. The rerank costs at most one wasted round trip, never added latency.
2. Emit `Started`. The UI clears the old notes only now, so a skipped pause never blanks them.
3. **Section agents.** `lanes = section_groups(n)`, where `n = effective_note_agents()`. Each lane gets the same context (brief, snippets, draft) and a prompt for its sections only. Lanes run through `agents::run_limited(n, …)`, and every chunk is emitted as `Text { lane, text }`.
4. **Check.** When the lane containing Faults finishes, its future continues straight into `jev::check`, while the other lanes keep streaming.
   - It emits `Checking { count, total }`, where `total` is the number of Faults bullets and `count` is how many cite something checkable. Then one `Check` per claim as each verdict arrives.
   - If the check is off, no key is found, or the Faults lane failed, there are no check events and the UI shows no "Checked in code" section.
5. Emit `Done { text }`, where `text` is the lanes' final texts joined in lane order. If some lanes failed, emit a `Notice` for each failed lane. Only if all lanes fail does the pipeline finish with `Error`.

### Section groups

Sections in fixed order: Intent, Faults, Directions, Questions.

| n | Lanes |
|---|---|
| 1 | [Intent, Faults, Directions, Questions] (today) |
| 2 | [Intent, Faults], [Directions, Questions] |
| 3 | [Intent, Faults], [Directions], [Questions] |
| 4 | [Intent], [Faults], [Directions], [Questions] |

`effective_note_agents()` returns `min(max_agents, 4)` if parallel notes is on and the provider is not local; otherwise 1.

### Jev questions

All questions keep each `state` small and on topic, as Jev's "large irrelevant state" guidance requires. Drafts are shaped with the existing head-plus-tail rule.

**Gate** (Noul)
- state: `{"previous_draft": <shaped, ≤12k chars>, "current_draft": <shaped, ≤12k chars>}`
- instructions: "Compared with `previous_draft`, does `current_draft` add, remove, or change an idea, goal, constraint, or question?"
- criteria.true: "At least one idea, goal, constraint, or question was added, removed, or changed in meaning."
- criteria.false: "Only wording, spelling, punctuation, or formatting changed; the ideas are the same."
- Decision: skip iff `noul < 0.3` (`GATE_SKIP_BELOW`).

**Rerank** (one Noul per shortlisted snippet, all in parallel)
- Shortlist: `shortlist(index, draft, 20, 1_500)`.
- state: `{"draft": <shaped, ≤4k chars>, "code": {"path", "start_line", "text"}}`
- instructions: "Is `code` relevant to what `draft` is discussing? Relevant means it implements, defines, configures, or directly affects something the draft talks about."
- criteria.true: "The code implements, defines, configures, or directly affects a feature, component, or behaviour the draft discusses."
- criteria.false: "The code is unrelated, or only shares generic words with the draft."
- Decision: sort by `noul` descending (ties keep TF-IDF order) and keep `max_files` (3 or 5, by provider budget, as today). Snippets are then clipped to today's per-file budget.

**Check** (one Choice per claim; adapted from TypeSafe's citation-check cookbook)
- Claims: the bullets of the Faults section, skipping "Nothing so far.", at most 6.
- Evidence (plain code, no model):
  1. Take the claim's backticked spans. If a claim has none, it isn't checked; the section header reports "k of m faults checked".
  2. A span containing `/` or a file extension matches files whose path ends with it. Other spans match files whose content contains the span (case-sensitive).
  3. Among the matching files, pick the best using the existing TF-IDF scoring on the claim text. Take the window around the line with the most span hits, using the existing window function, up to 1,500 chars.
  4. No matching file means the verdict is `notFound` and Jev isn't called.
- state: `{"claim": <bullet text>, "code": {"path", "start_line", "text"}}`
- instructions: "How does `code` relate to `claim`?"
- criteria:
  - `supports`: "The code shows the claim is true, or directly implies it."
  - `contradicts`: "The code shows the claim is false, or directly implies the opposite."
  - `says_nothing`: "The code does not address the claim either way."
- Verdict mapping: `supports` with confidence ≥ 0.8 → `confirmed`; `contradicts` with confidence ≥ 0.8 → `contradicted`; `says_nothing` → `notShown`; anything else → `unclear` (`CHECK_MIN_CONFIDENCE = 0.8`).
- Option order is fixed. Jev 1.13 can lean towards the first option, which is a risk to evaluate, not a reason to shuffle by default.

### Jev client

- Request body: `{"state", "model", "questions": {id: {"type": "noul" | "choice", "instructions", "criteria"}}}`. Response body: `{"model", "answers": {id: {"type", …}}, "usage"}`, as in the API reference.
- Model: `settings.jev.model`, default `"jev-latest"`. The answering model ID from each response is kept for logging.
- At most 8 Jev requests in flight for fact-checks (`JEV_CONCURRENCY`), independent of max agents. The rerank sends its whole 20-snippet shortlist at once (`RERANK_CONCURRENCY`), so the pre-phase costs about one round trip, as the success criteria require (changed after the phase 2 review: 3 waves of 8 put normal pauses at 1.2–1.5 s against a 1.5 s budget).
- Timeouts: each request is bounded by the remaining pre-phase budget (`PRE_BUDGET`, 1,500 ms: about one round trip of 0.3 s for the gate and the whole rerank shortlist at once, plus room for a slow answer or one 503 retry), or 5 s during the check phase.
- Retries: on 429, 503 or 529 (503 `model_unavailable` was observed intermittently on the first call in phase 1 measurements), up to 2 retries with 250 ms then 750 ms backoff (or `retry-after`, if shorter than the remaining budget).
- Errors:
  - 401 → `Error::Precondition("TypeSafe key rejected. Check .env.local or Settings → Jev.")`.
  - Anything else → `Error::Provider("Jev: …")`.
  - Jev errors never fail a pause: they become fallbacks plus at most one `Notice` per pause.

## Brief build (parallel explorers)

1. **Slices** (`explore::slices(index, n)`, plain code):
   - Group indexed files by their first path segment. Root-level key files (README, manifests, agent docs) go to the merge step via the overview; other root files are explored as single-file entries (changed after the phase 4 review: excluding them meant root source code, e.g. a Go service's `*.go`, was read by nobody).
   - If one group holds more than 50% of the bytes and has subfolders, split it one level deeper.
   - Assign groups largest-first to the lightest slice.
   - Slice count is `min(n, groups)`. If fewer than 40 files are indexed, use 1 slice.
2. **Explorers**, `effective_brief_agents()` of them: `max_agents` if parallel brief is on and the provider is not local, otherwise 1. They run via `agents::run_limited`.
   - **Claude CLI:** `claude -p` in the project root, with the same read-only `--allowedTools Read Glob Grep` and all mutating tools disallowed, and `--max-turns 20`. The system prompt says: "You are agent k of n mapping this codebase in parallel. Explore only: <prefixes>. Others cover the rest. Write ≤400 words of Markdown notes: ## Modules (`path`: responsibility), ## Flows, ## Conventions, ## Features and gaps. Cite paths. Output only the notes."
   - **API provider:** no tools, so each explorer gets `digest::build_slice(index, prefixes, budget)` and the same notes format.
   - Activity is emitted as `Activity { lane: k, label }`.
3. **Merge:** one Deep-tier call with no tools. Input: each explorer's notes (`<notes agent="k" paths="…">`), the file tree and key-file sections of the digest (exposed as `digest::overview(index, budget)`), and an `<unmapped>` list of the prefixes of failed explorers. It outputs the brief in today's `BRIEF_FORMAT` and word limits, streamed as `Text { lane: 0 }`. Unmapped prefixes are named under "Gaps and TODOs".
4. **Failure:** if some explorers fail, merge the rest. If all fail, return the first error. With n = 1 the path is today's single explorer, with no merge step.
5. **Stored brief:** `StoredBrief.source` gains the agent count, e.g. "Claude Code (sonnet) · 4 agents". Fingerprint and staleness rules are unchanged.

## Settings

Rust (`settings.rs`). Every struct uses `#[serde(rename_all = "camelCase", default)]`, so old files keep loading.

```rust
pub struct Settings {
    pub provider: ProviderSettings,
    pub debounce_ms: u64,
    pub agents: AgentSettings,
    pub jev: JevSettings,
}
pub struct AgentSettings {
    pub max_agents: u8,       // clamped to 1..=8, default 4
    pub parallel_brief: bool, // default false (measured slower; see Measurements)
    pub parallel_notes: bool, // default false (measured slower; see Measurements)
}
pub struct JevSettings {
    pub gate: bool,    // default true
    pub rerank: bool,  // default true
    pub check: bool,   // default true
    pub model: String, // default "jev-latest"
}
```

These are mirrored in `src/lib/ipc.ts` and in `DEFAULTS`-style defaults in `SettingsDialog.tsx`. `jev.model` is not shown in the UI. It exists so a tuned threshold can be pinned to a versioned model ID such as `jev-1.13.0`.

## TypeSafe key

- **Lookup order** (`secrets::jev_key() -> Option<(String, KeySource)>`):
  1. Keychain: service `app.livebrainstorm.desktop`, account `typesafe-api-key`.
  2. `TYPESAFE_API_KEY` in the process environment.
  3. **Debug builds only:** `TYPESAFE_API_KEY` read from `<repo>/.env.local`, at the path `concat!(env!("CARGO_MANIFEST_DIR"), "/../.env.local")`.
- **Reading the file:** `dotenvy::from_path_iter` reads only that key. Nothing is written to the process environment, and release builds compile this branch out.
- **New commands:** `set_jev_key(key)` (empty removes it) and `jev_status() -> { source: "keychain" | "environment" | "envFile" | null }`. The key itself never crosses IPC.
- **Repo files:** `.gitignore` gets `.env*.local`, and `.env.example` is added with `TYPESAFE_API_KEY=`.
- **Vite safety:** Vite also reads `.env.local`, but it only exposes `VITE_*` variables to the bundle. The variable must never be renamed with a `VITE_` prefix. This goes into AGENTS.md.

## IPC changes

`UiEvent` (tag `type`, camelCase):

| Variant | Change | Used by |
|---|---|---|
| `text { text, lane }` | `lane: u8` added | analysis lanes; brief merge (lane 0) |
| `activity { label, lane }` | `lane: u8` added | brief explorers (lane = agent number) |
| `started` | new | analysis passed the gate |
| `skipped` | new | gate found no real change |
| `checking { count, total }` | new | k of m faults being checked |
| `check { claim, verdict, confidence?, path?, line? }` | new | one verdict |
| `notice { message }` | new | non-fatal problems (Jev unavailable, a lane failed) |
| `done`, `error`, `progress` | unchanged | |

- `analyze(draft, previousDraft: string | null, onEvent)` gains `previousDraft`.
- New commands: `set_jev_key`, `jev_status`.

## Frontend

- `useLiveAnalysis`:
  - **Lanes:** keeps per-lane buffers and builds the markdown by joining lanes in order. This is a pure helper in `src/lib/lanes.ts`, unit-tested.
  - **Gate events:** on `started` it clears the old notes. On `skipped` it keeps the current notes and shows them as current: the displayed-for draft becomes the current draft, so the rule isn't dashed.
  - **Previous draft:** a separate `lastAnalyzed` value holds the draft of the last analysis that reached `done`, and is sent as `previousDraft`. It is not updated by `skipped`, is reset on a context change, and is sent as null on rerun.
  - **Checks:** collects `check` events.
- `NotesPane`:
  - Adds a "Checked in code" section after Questions, with the header "Checking k of m faults…" and then the rows.
  - Each row shows ✓ confirmed, ✗ contradicted, or an "unclear", "not shown in this code" or "not found in indexed code" label, plus the claim (one line, truncated) and `path:line`.
  - Notices appear as one quiet line under the status line.
- `BriefDrawer`: activity lines are prefixed "Agent k ·".
- `SettingsDialog`, two new groups in the existing style:
  - **Agents:** a "Parallel agents" slider (1–8), the "Map the project with parallel agents" and "Write notes with parallel agents" switches, and per-provider hints. Local: "Local models run one agent at a time." API: "Each agent is billed for the shared project context."
  - **Jev:** a key field that saves to the Keychain, a source line (e.g. "Using TYPESAFE_API_KEY from .env.local (development)"), and three switches. The switches are disabled with a hint when no key is found.

## Cost (per pause, all Jev features on)

Gate about 3–6k tokens, rerank 20 × about 1.5k, check up to 6 × about 1k: about 40k input tokens, or about $0.0017 at $0.042 per million input tokens. Claude usage per pause grows up to the number of note agents (default 4), offset by gate skips.

## Testing

Core unit tests, with no network and no Tauri:
- `agents::run_limited`: at most `limit` run concurrently, results come back in job order, and dropping the future cancels the pending jobs.
- `section_groups(1..=8)` matches the table. `effective_*_agents` handles local and the parallel switches.
- Prompts: each lane's system prompt contains exactly its own `## Heading`s. For all four sections, it contains every rule from today's prompt.
- Jev serialization: the request JSON equals the API reference examples' shape, and responses parse from the reference examples.
- Gate, rerank and check decision logic with canned answers: thresholds, tie order, falling back on error, and the verdict mapping.
- `check` claim extraction and evidence location against the existing scan fixture (`createInvoice`, `CartStore`).
- `explore::slices`: balance, root files excluded, the dominant-folder split, and the small-repo case.
- `digest::build_slice` stays within budget and only includes the given prefixes.
- Settings: old JSON without the new fields loads with defaults, and `max_agents` is clamped.

Frontend (vitest): `lanes.ts` merging and out-of-order lane chunks; `parseSections` still passes.

Opt-in network test (`#[ignore]`): one real Jev Noul and one Choice using `TYPESAFE_API_KEY` from the environment or `.env.local`; it skips without a key. It prints latency for each request type so `PRE_BUDGET` can be tuned.

Manual: `tauri dev` with the user's key on this repo. Check the timings in the success criteria, and confirm no orphaned `claude` processes appear after typing mid-fan-out. Screenshots come from the mocked-IPC harness (scratchpad) for the new UI states.

## Implementation order

Each phase ends with passing tests and a working app.

1. **Hardening, then Jev locally:** the Claude Code hardening above, then `.env.local` key loading (debug builds), the `jev` client, the opt-in latency test, and `jev::check` with the "Checked in code" UI. This gives the first real Jev measurement.
2. **Gate and rerank:** the pre-phase, the `started`, `skipped` and `notice` events, and `previousDraft`.
3. **Parallel notes:** `agents::run_limited`, section groups, per-section prompts, lane events, and lane merging in the UI.
4. **Parallel brief:** `explore::slices`, explorers, merge, and agent-tagged activity.
5. **Settings:** the Agents and Jev groups, the Keychain key and `jev_status`, then the docs.

Until phase 5, the new settings use their defaults (max agents 4, everything on), and the key comes from `.env.local` or the environment.

## Risks

- **Jev latency is unpublished.** The pre-phase budget and the fallbacks protect time-to-first-note. Measure before tuning.
- **Jev reads literally and struggles with multi-hop reasoning.** Claims about behaviour spanning files may come back `notShown` or `unclear`. The UI wording presents verdicts as signals, not proof.
- **Choice option-order bias** in Jev 1.13. To be evaluated on real claims.
- **Running 4 `claude` processes per pause** may hit subscription rate limits. Failed lanes degrade to notices, and the user can lower max agents.
- **API provider:** input cost multiplies with the number of lanes.

## Documentation updates

AGENTS.md (the agents module, Jev, key lookup and the `VITE_` warning, new events, and new cross-language contracts: section groups and the `UiEvent` additions), README (features, settings, `.env.local`), `.env.example`, `.gitignore`.

## Measurements (phase 1, 2026-10-08)

`cargo test -p livebrainstorm-core --test jev_live -- --ignored --nocapture`, model `jev-1.13.0`, from a home network in 3 runs of 3 rounds each:

| Request | Observed latency |
|---|---|
| Gate Noul (two short drafts) | 219–402 ms (typically about 300 ms) |
| Check Choice (claim + 3-line snippet) | 212–286 ms (typically about 255 ms) |

- No cold-start penalty was visible: round 1 was no slower than rounds 2–3.
- **503 `model_unavailable`** came back on the first call of two early attempts (the key authenticated fine) and cleared within about 20 s. The client now retries 503 like 429/529. Without that, a fact-check would have shown a notice on a transient blip.
- The single check verdict (`createInvoice` returns `total: 0`, claim "ignores the cart's items") was `supports` at confidence 1.00, as it should be. The gate answered 0.98 for an edit that adds a new idea.
- Phase 2 sizes `PRE_BUDGET` from these numbers: gate and rerank run concurrently at about 0.3 s each, so the planned 2,000 ms budget leaves wide margin and can be tightened to about 1,000 ms.

## Measurements (phase 2, 2026-10-08)

`cargo test -p livebrainstorm-core --test jev_live pre_phase_on_this_repo -- --ignored --nocapture` on this repo (75 files), `PRE_BUDGET` = 1,500 ms:

| Draft vs previous | Rounds (elapsed, outcome) |
|---|---|
| Adds an idea | 1,472 ms, analysed (reranked); 1,212 ms, analysed (reranked); 798 ms, analysed with keyword fallback (a rerank request got TypeSafe 503 `model_unavailable` even after 2 retries, so one notice) |
| Rewords only | 346 ms, SKIP; 313 ms, SKIP; 633 ms, SKIP |

- Reranked picks: `llm/claude_cli.rs`, `AGENTS.md`, `src-tauri/src/provider.rs`, `src-tauri/src/commands.rs`, `llm/mod.rs`. Keyword order without Jev: the phase 2 plan, the phase 1 plan, `AGENTS.md`, `tests/jev_live.rs`, the spec. The reranked set is clearly more on-topic for a draft about `ClaudeCli::stream`; keyword order favours the Markdown docs that quote those identifiers.
- `PRE_BUDGET`: kept at 1,500 ms by the plan's rule (no round timed out). But the slowest analysed round finished within 30 ms of the budget, because the 20 rerank requests run in 3 waves of 8. If timeouts show up in use, the cheaper fix is a shorter shortlist or higher rerank concurrency, not a longer budget.
- One failed request out of 20 discards the whole ranking (by design). With TypeSafe's intermittent 503s that happened in 1 of 3 rounds; the fallback worked as intended.
- **After the review fix** (rerank shortlist sent at once instead of in waves of 8), same test: adds an idea 979 / 960 / 359 ms, analysed with reranked code and no notices; rewords only 308 / 379 / 386 ms, SKIP. Normal analysed pauses now finish well inside `PRE_BUDGET`.

## Measurements (phase 3, 2026-10-14)

`cargo test -p livebrainstorm-core --test notes_live -- --ignored --nocapture`, Claude Code `haiku` via the subscription, this repo, keyword snippets. Two runs of the test, each alternating 1, 4, 1, 4 lanes:

| Lanes | First text (ms) | Complete notes (ms) | Missing sections |
|---|---|---|---|
| 1 | 3,309; 3,048; 9,766; 3,146 | 7,408; 7,085; 12,567; 6,983 | none |
| 4 | 3,880; 4,103; 4,868; 5,259 | 11,089; 10,757; 8,853; 13,946 | none |

Per lane, second run: one section (≈ 370–680 chars) took 6.6–14 s; the single four-section call (≈ 1,500–1,600 chars) took 7–12.6 s.

- **The success criterion "with 4 agents, complete notes arrive sooner than with 1" is not met with the Claude subscription.** The median is ≈ 11 s for 4 lanes against ≈ 7.2 s for 1, and 4 lanes cost 4 `claude` processes per pause.
- Why: each call's time is dominated by fixed per-call cost (spawning `claude -p`, reading the ≈ 30k-char project context), not by output length. Splitting the output buys almost nothing, and 4 concurrent calls finish when the slowest of 4 variable latencies does.
- Not measured: the Anthropic API provider (no process spawn) and local models (always 1 lane).
- Implication for phase 4: the brief's explorers do far more work per agent (tool turns over the repo), so parallelism there is more likely to pay off. Measure before assuming.

## Measurements (phase 4, 2026-10-14)

`cargo test -p livebrainstorm-core --test brief_live -- --ignored --nocapture`, Claude Code `sonnet` via the subscription, this repo (81 indexed files). Two runs:

| Explorers | Total (run 1; run 2) | Words | Missing sections | Notes |
|---|---|---|---|---|
| 1 | 47,665 ms; 44,132 ms | 1,473; 1,349 | none | today's single explorer |
| 4 | 55,356 ms; 50,962 ms | 1,562; 1,257 | none | explorers 21–26 s each in parallel (docs/, crates/, src/, src-tauri/), then merge 29.3 s; 25.6 s; unmapped none |

- Combined: 4 explorers 106.3 s vs 1 explorer 91.8 s, so **4 is ≈16% slower**. Run 1 alone was within the 20% band, so the pair was measured twice as the rule requires.
- Why: parallel exploration halves the mapping time (≈25 s vs ≈45 s), but the extra merge call (≈25–29 s) costs more than that saves. The brief quality was comparable (all eight sections, similar length).
- One explorer spent its slice on `docs/` (plans and specs), not code. A future slicer could deprioritise documentation folders.
- **Default: turned off** (`agents.parallelBrief: false`), by the plan's rule: 4 explorers were not ≥ 20% faster. It stays available as an opt-in. Together with phase 3, neither parallel notes nor parallel briefs beat a single agent with Claude Code on this repo. Per-call overhead (spawning `claude -p`, re-reading context, an extra merge round) dominates.
