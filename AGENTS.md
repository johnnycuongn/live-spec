# AGENTS.md

Guidance for AI coding agents working in this repository.

## What this is

LiveBrainstorm is a Tauri v2 desktop app for brainstorming against a codebase. The user types a free-form draft on the left; when they pause, an LLM that has read the selected project streams notes into the right-hand panel under four fixed headings: **Intent, Faults, Directions, Questions**. The model comes from one of three providers: the user's installed Claude Code CLI (default), the Anthropic API (key in the OS keychain), or a local GGUF model served by a llama.cpp runtime the app downloads on demand.

## Commands

Prerequisites: Rust stable, the Tauri v2 system prerequisites, and **Node 20.19+ or 22.12+**. Vite 7 requires that: on Node 20.11, `vite` crashes with `crypto.hash is not a function`. Typecheck and vitest still pass on older Node, which hides the problem until you start the app.

```bash
npm install
npm run tauri dev        # the app: Vite on :1420 (strictPort) + the Rust shell
npm run tauri build      # platform installer
npm run typecheck        # tsc --noEmit
npm test                 # vitest
npx vitest run src/lib/sections.test.ts -t "partial streams"   # one frontend test

cargo test -p livebrainstorm-core                    # Rust unit tests
cargo test -p livebrainstorm-core retrieve::tests    # filter by module/test path
cargo test -p livebrainstorm-core -- --ignored       # network: downloads and runs the real llama.cpp build
cargo clippy --workspace --all-targets               # currently warning-free
cargo check -p livebrainstorm                        # Tauri shell (package `livebrainstorm`, lib `livebrainstorm_lib`)

# What retrieval would pick for a draft; use it when tuning terms/retrieve
cargo run -p livebrainstorm-core --example probe -- <project-dir> "<draft text>"
```

- `npm run dev` starts Vite alone. Every `invoke` fails outside the Tauri webview, so UI work needs `npm run tauri dev`.
- `src-tauri` has no tests. Logic you want covered belongs in `livebrainstorm-core`.
- There is no rustfmt config. The Rust is hand-wrapped at about 120 columns, and `cargo fmt` rewrites about 80 hunks, so don't run it across the repo; match the surrounding style instead. There is no ESLint or Prettier either.

## Architecture

| Layer | Path | Role |
|---|---|---|
| UI | `src/` (React 19 + Vite, no state library) | Draft, notes, brief drawer, settings; state lives in `App.tsx` |
| Shell | `src-tauri/` | IPC commands, `AppState`, settings → provider resolution, keychain. Deliberately thin |
| Core | `crates/livebrainstorm-core/` | Scan/index/retrieve, brief cache, prompts, providers, local runtime. **Must not depend on Tauri** |

### UI and styling

- **Tailwind v4** through `@tailwindcss/vite`. There's no `tailwind.config`; theme and tokens live in `src/styles.css`.
- **Colours are semantic tokens** (`bg-page`, `bg-panel`, `bg-subtle`, `border-line`, `text-ink`, `text-muted`, `text-faint`, `text-accent`, `text-fault`, `text-question`). They map to `--lb-*` variables that switch under `prefers-color-scheme: dark`. Use these rather than raw palette classes or `dark:` variants. The note colours carry meaning: faults red, directions blue, questions amber.
- **Fonts are bundled from Fontsource, never fetched**, because the app must work offline and the CSP only allows `'self'`. The user's draft is set in Literata (`font-serif`), the AI's notes and the UI in Inter (`font-sans`), and code in JetBrains Mono (`font-mono`).
- **Shared primitives** live in `src/components/ui.tsx` (`Button`, `Field`, `inputClass`, `Eyebrow`, and the `cx` class joiner). Use them instead of repeating class lists.
- **Layout:** both panes are flex columns with `min-h-0` and their own `overflow-y-auto`. The draft `<textarea>` fills its pane, and percentage padding centres a 40rem text column, so the textarea and the word-count footer line up.
- **Rule animation:** the accent rule between the panes (`Rule` in `NotesPane.tsx`) shows state: solid when notes are current, animated while thinking, dashed when the draft has moved on.
- **Settings** is a native `<dialog>` opened with `showModal()`, which gives focus trapping, Esc and `::backdrop` for free. Esc for the brief drawer is handled in `App.tsx`.

### Live analysis flow

1. `src/lib/useLiveAnalysis.ts` waits for a typing pause (`debounceMs`, default 1200). It needs at least 24 characters after whitespace normalization and skips the call if the normalized draft hasn't changed. When the project or brief changes (`contextKey`), it re-sends the same draft.
2. `analyze` (`src-tauri/src/commands.rs`) returns immediately after spawning a task into the `AppState.analysis` slot. `replace_task` aborts the previous task, and dropping that future kills the `claude` child process (`kill_on_drop`).
3. The task resolves a provider at `Tier::Fast` (`src-tauri/src/provider.rs`), then runs `analysis::pre_phase`. The Jev gate ("did the idea change since `previousDraft`?") and the Jev rerank (scores the 20-snippet keyword shortlist and keeps the best 3 or 5) run concurrently under `PRE_BUDGET` (1.5 s). A confident "unchanged" emits `skipped` and stops. Otherwise it emits `started`, then at most one pre-phase `notice`, builds the prompt with `prompt::analysis_request` and streams. Without Jev, or for the Local provider, or when Jev is slow or failing, this is plain keyword retrieval (`project::retrieve`).
4. After `started`, the sections are written by `Settings::effective_note_agents()` lanes at once (1 unless `agents.parallelNotes` is on, which is off by default: 4 lanes measured slower than 1 with the Claude CLI) (`prompt::section_groups`: 1 → all four, 2 → Intent+Faults | Directions+Questions, 3 → Intent+Faults | Directions | Questions, 4 → one each; always 1 for the Local provider). Lanes run through `agents::run_limited`, so dropping the task cancels every `claude` child. Every chunk is `text { text, lane }`; `done { text, partial }` carries the lanes joined in lane order (`analysis::merge_lanes` keeps only each lane's own sections), and any missing sections become one `notice` with `partial: true` (the UI offers "Try again", and partial notes never become the gate's comparison point). A failed fact-check is `checksStopped`, distinct from other notices. The fact-check starts when the Faults lane finishes (a oneshot from that lane), concurrently with the other lanes; `done` never waits for it. Events: `text`, `activity`, `started`, `skipped`, `notice`, `checking`, `check`, `done`, `error`, `progress`.
5. `src/lib/sections.ts` splits the streamed Markdown into the four sections, and `MiniMarkdown` renders them.

Brief building (`build_brief`, `Tier::Deep`) and downloads (`install_runtime`, `download_model`) follow the same pattern: spawn, keep the handle in an `AppState` slot, stream over a Channel, then send a final `done` or `error`.

### Project understanding: two layers

- **Brief (once per project).** If `Provider::can_explore_repo()` (Claude CLI only), Claude Code explores the repo itself with the project as cwd: `--allowedTools Read Glob Grep`, all mutating tools disallowed, `--max-turns 40`. Other providers get `project::digest`, a budgeted dump of the tree, key files (README, CLAUDE.md, AGENTS.md, manifests), then the heads of "central" files. Briefs are cached at `<app_data>/briefs/<fnv(root)>.json` and marked stale when the project fingerprint changes.
- **Retrieval (every pause).** TF-IDF over identifier terms, with no embeddings. `project/terms.rs` splits camelCase and snake_case, folds plurals, and drops stopwords. The same tokenizer runs at index time and query time, so a change there affects both. The last 400 characters of the draft are weighted 1.6×, and path matches get a bonus.

`project::scan` honours `.gitignore` even outside git (`require_git(false)`), skips hidden files and `SKIP_DIRS`, and indexes at most 6,000 text files of 256 KB or less. The fingerprint hashes (path, size, mtime) of every non-ignored file. Persisted hashes use the in-house FNV-1a (`project::Fnv`), never `DefaultHasher`, whose output isn't stable across Rust versions.

### Providers

`llm::Provider` is a closed enum (`ClaudeCli | AnthropicApi | OpenAiCompat`) with `match` dispatch, chosen on purpose over `dyn Trait` + `async_trait`. All three take the same `ChatRequest`; providers ignore fields they can't use (`cwd`, `allow_repo_tools`).

- **`ClaudeCli`** runs the user's unmodified `claude -p --output-format stream-json --verbose --include-partial-messages` and sends the prompt on stdin to avoid argv length limits. Every run passes `--restricted` (ignores settings files, so an opened repo's hooks never run; confines file tools to the project) and `--strict-mcp-config` (no MCP servers). Tools come from an allowlist via `--tools`: none (`""`) for live analysis, which also *replaces* the system prompt (`--system-prompt`, `--max-turns 1`); `Read,Glob,Grep` for the brief, which *appends* (`--append-system-prompt`) so Claude Code keeps its own repo-exploration prompt. Never reintroduce a `--disallowedTools` denylist: new tools would slip past it. Known cost: `--restricted` also ignores the user's own settings files, so auth via `apiKeyHelper` or settings `env` no longer works (documented in the README). The NDJSON parser ignores unknown event types on purpose, so CLI upgrades don't break it. The final answer comes from the `result` event, which excludes the narration streamed during exploration.
- **`AnthropicApi` / `OpenAiCompat`** share `llm/sse.rs`. Its buffer is `Vec<u8>`, not `String`, because a network chunk can split a UTF-8 character.
- **Local:** `provider::ensure_local` starts `llama-server` in its *own* spawned task. Analysis tasks are aborted on every keystroke, and an abort must not kill a model that's halfway through loading. `AppState.local` is a `tokio::sync::Mutex` because it's held across that slow startup. Every other lock in `AppState` is `std` and must never be held across `.await`. On app exit and when leaving the local provider, the server is stopped to free RAM.
- **Budgets:** `Provider::context_budget_chars()` drives all prompt sizing: 120k characters for Claude, `context_tokens × 1.5` for local models. The brief is clipped to 3/5 of the budget and snippets get 1/4. Drafts over 12k characters keep the first 2k plus the tail.

### Jev (TypeSafe)

`livebrainstorm-core::jev` is a typed client for TypeSafe's System One API (`POST https://api.typesafe.ai/v1/systemone`): one `state`, any number of Noul/Choice questions, typed answers with probabilities. It judges; it never writes text. Choice options serialize in the order given (`jev::Options`) because Jev 1.13 can lean towards the first option.

- **Fact-check ("Checked in code"):** after `done`, `commands::analyze` takes the Faults section (`prompt::section_body`) and runs `jev::check::run`. Plain code finds the evidence: backticked names and paths in each bullet, matched against the in-memory index (names found in more than a quarter of the files are too generic). Jev answers supports / contradicts / says_nothing per claim; ✓/✗ need confidence ≥ 0.8. Check events arrive after `done`, and the next keystroke aborts them with the rest of the task.
- **Key lookup** (`secrets::jev_key`): Keychain account `typesafe-api-key`, then `TYPESAFE_API_KEY` in the environment, then, in debug builds only, `TYPESAFE_API_KEY` in the repo's `.env.local` (read by `env_file::read_value`, never exported). The key never crosses IPC. **Never prefix it `VITE_`:** Vite reads `.env.local` too and ships `VITE_*` variables to the webview bundle.
- Jev only runs for the fact-check when `Settings::jev_check_enabled()` is true: never with the Local provider, whatever the switch says (it would send code off the machine).
- Evidence matching is by whole identifiers and whole path segments; a name found in more than `min(8, files/4)` files (at least 2) is too generic to check.
- **Gate and rerank** (`jev::gate`, `jev::rerank`, `analysis::pre_phase`): the UI sends the draft of the last *completed* analysis as `previousDraft` (skips never move it; "Try again" sends null). Gate errors mean "analyse"; rerank errors mean keyword order. Neither ever adds more than `PRE_BUDGET` before the model call. Futures awaited inside the shell's spawned task must avoid closures over borrowed arguments (`Option::filter(|_| …)`, `.map(|(i, &x)| …)`): they trip rustc's Send check ("FnOnce is not general enough").
- `Settings::jev_allowed()` is false for the Local provider; `jev_gate_enabled`, `jev_rerank_enabled` and `jev_check_enabled` all respect it.
- Parallel-notes live test (runs Claude Code, uses the subscription; prints 1-lane vs 4-lane timings per lane): `cargo test -p livebrainstorm-core --test notes_live -- --ignored --nocapture`. Phase 3 measured 4 lanes *slower* than 1 with the Claude CLI (per-call overhead dominates); see the spec's measurements.
- Jev failures never fail an analysis; they surface as one `notice` event. The client retries 429/503/529 twice.
- Live test against the real API (needs the key; prints latency): `cargo test -p livebrainstorm-core --test jev_live -- --ignored --nocapture`.

### Errors

`livebrainstorm_core::Error` serializes as a plain string across IPC. `Precondition` means the user must act (open a project, install Claude Code, download a model). The UI shows its message verbatim, so write it as an instruction to the user. `Provider` means the model, CLI, or network failed.

### Persistence

- Settings: `<app_config>/settings.json`, written atomically (tmp file + rename). A missing or corrupt file falls back to defaults instead of blocking startup.
- API key: OS keychain (service `app.livebrainstorm.desktop`, account `anthropic-api-key`). Never written to settings.json, never sent to the webview.
- Local runtime and models: `<app_data>/local/{runtime,models,downloads}`, server log at `local/llama-server.log`. Downloads resume through `.part` files and HTTP Range requests. Runtime installs extract into a staging dir, then swap it in.
- Frontend: drafts are stored per project in `localStorage` (`livebrainstorm.draft.<root>`), plus `livebrainstorm.lastProject`. Download progress lives in a module-level store (`src/lib/downloads.ts`, `useSyncExternalStore`) so it survives closing Settings.

## Cross-language contracts (no codegen; keep in sync by hand)

- **IPC types:** `src/lib/ipc.ts` mirrors the serde types in `src-tauri/src/commands.rs` and `livebrainstorm-core` (`settings.rs`, `project/brief.rs`, `local/`). Serde uses `rename_all = "camelCase"`. Enums are internally tagged (`UiEvent` on `type`, `ProviderSettings` on `kind`), and `LocalStatus` / `ModelState` use `#[serde(flatten)]`.
- **Section headings:** `SECTIONS` in `crates/livebrainstorm-core/src/prompt.rs` ↔ the heading regex and titles in `src/lib/sections.ts`. The parser deliberately tolerates `**Faults:**` and `### Faults` because small local models drift from the format.
- **Settings defaults:** `Settings::default()` in `settings.rs` ↔ `DEFAULTS` in `src/components/SettingsDialog.tsx`.
- **New IPC command:** `#[tauri::command]` in `commands.rs`, then register it in `generate_handler!` in `src-tauri/src/lib.rs`, then add a wrapper to `api` in `ipc.ts`. App commands need no capability entry. Plugin permissions do: `src-tauri/capabilities/default.json` grants only `core:default` and `dialog:allow-open`.
- **New provider** touches: `ProviderSettings` (`settings.rs`), a module in `llm/` plus an arm in each `match` on `Provider`, `provider::resolve`, `ipc.ts`, `SettingsDialog.tsx`, and `providerLabel` in `App.tsx`.

- **Section parsing:** `prompt::section_body` (Rust) mirrors `parseSections` in `src/lib/sections.ts`; change both together.
- **Check events:** `CheckResult` / `Verdict` in `jev/check.rs` ↔ `CheckResult` / `Verdict` in `src/lib/ipc.ts`.

- **Lanes:** `prompt::section_groups` (Rust) decides which sections each `lane` carries; `src/lib/lanes.ts` joins lane buffers in lane order. `analysis_system(&SECTIONS)` must stay byte-identical to the single-agent prompt (`all_four_sections_is_todays_prompt`).

## Deliberate decisions to preserve

- **Never handle Claude subscription credentials.** Anthropic only permits subscription OAuth inside Claude Code and claude.ai, so LiveBrainstorm only runs the user's own `claude` binary. Don't add a "Sign in with Claude" flow or read Claude Code's token files. The API-key provider is the path for anyone else using LiveBrainstorm.
- Live analysis is stateless: each request spawns a fresh `claude -p`. A persistent stream-json session would start faster but would accumulate conversation history.
- Model output is rendered only through `MiniMarkdown` (React elements, never `innerHTML`), behind the strict CSP in `tauri.conf.json`.
- The llama.cpp build is pinned (`LLAMA_CPP_BUILD` in `local/runtime.rs`); upgrading is a deliberate one-line change. CPU builds everywhere except macOS, where Metal is built in.
- Apps launched from a GUI don't inherit the shell `PATH`. `shell_env::login_shell_path` asks the login shell once, with a 4 s timeout. The result is used both to find `claude` and as the child's `PATH`, because the npm build of Claude Code needs `node`.
- reqwest uses `rustls-tls-native-roots` (the OS certificate store) so corporate TLS-inspecting proxies work.

## Extension points

- Prompts and the section rules: `crates/livebrainstorm-core/src/prompt.rs` holds everything the model sees.
- Another local model: add a `ModelSpec` to `crates/livebrainstorm-core/src/local/catalog.rs`.
- Ollama / LM Studio: both speak the OpenAI API, so build `OpenAiCompat` with their base URL behind a new provider setting.
- Fuzzier retrieval ("payments" finding `billing/`): replace the body of `project::retrieve` with embeddings and keep its signature.
- GPU builds on Windows/Linux: change the asset suffix in `local/runtime.rs::asset_name`.
