# AGENTS.md

Guidance for AI coding agents working in this repository.

| Read | For |
|---|---|
| **This file** | Commands, invariants, cross-language contracts and decisions every change must respect |
| [ARCHITECTURE.md](ARCHITECTURE.md) | How the app is built: layers, the live-analysis flow, sections, project understanding, providers, Jev, tabs and windows, persistence, security model, where to change things. Read the relevant section before changing a flow |
| [SETUP.md](SETUP.md) | Installing, providers, building, tests, troubleshooting |

## What this is

Live Spec is a Tauri v2 desktop app for writing a spec against a codebase, the first step of spec-driven development. The user types the spec on the left; when they pause, an LLM that has read the selected project streams review notes into the right-hand panel under the sections the tab has chosen from a five-entry catalogue: **Intent, Conflicts (with code), Gaps, Ambiguities, Questions**. The goal is a spec an AI coding agent can implement without further discovery or guessing. The model comes from one of three providers: the user's installed Claude Code CLI (default), the Anthropic API (key in the OS keychain), or a local GGUF model served by a llama.cpp runtime the app downloads on demand.

## Commands

Prerequisites: Rust stable, the Tauri v2 system prerequisites, and **Node 20.19+ or 22.12+**. Vite 7 requires that: on Node 20.11, `vite` crashes with `crypto.hash is not a function`. Typecheck and vitest still pass on older Node, which hides the problem until you start the app.

```bash
npm install
npm run tauri dev        # the app: Vite on :1420 (strictPort) + the Rust shell
npm run tauri build      # platform installer (target/release/bundle/)
npm run typecheck        # tsc --noEmit
npx vitest run --dir src # frontend tests (plain `npm test` also collects .claude/worktrees copies)
npx vitest run src/lib/sections.test.ts -t "partial streams"   # one frontend test

cargo test -p livespec-core                    # Rust unit tests
cargo test -p livespec-core retrieve::tests    # filter by module/test path
cargo test -p livespec-core -- --ignored       # network: downloads and runs the real llama.cpp build
cargo clippy --workspace --all-targets -- -D warnings   # currently warning-free
cargo check -p livespec                        # Tauri shell (package `livespec`, lib `livespec_lib`)

# What retrieval would pick for a draft; use it when tuning terms/retrieve
cargo run -p livespec-core --example probe -- <project-dir> "<draft text>"
```

Live tests (ignored by default; they print timings):

- `cargo test -p livespec-core --test jev_live -- --ignored --nocapture`: the real TypeSafe API (needs the key).
- `cargo test -p livespec-core --test notes_live -- --ignored --nocapture`: Claude Code, 1 vs 4 note lanes (uses the subscription).
- `cargo test -p livespec-core --test brief_live -- --ignored --nocapture`: Claude Code sonnet, 1 explorer vs 4 plus merge (several minutes, uses the subscription).

Notes:

- `npm run dev` starts Vite alone. Every `invoke` fails outside the Tauri webview, so UI work needs `npm run tauri dev`.
- `src-tauri` has no tests. Logic you want covered belongs in `livespec-core`.
- There is no rustfmt config. The Rust is hand-wrapped at about 120 columns, and `cargo fmt` rewrites about 80 hunks, so don't run it across the repo; match the surrounding style instead. There is no ESLint or Prettier either.

## Layers

| Layer | Path | Role |
|---|---|---|
| UI | `src/` (React 19 + Vite, no state library) | Sidebar of tabs, draft, notes, drawers, settings. Tabs in `src/lib/tabs.ts`, projects in `src/lib/projects.ts` (module stores read with `useSyncExternalStore`), each tab's live analysis a `TabSession` (`src/lib/tabSession.ts`) outside React. `App.tsx` holds only settings and which drawer is open |
| Shell | `src-tauri/` | IPC commands, `AppState`, settings → provider resolution, keychain, windows, menu. Deliberately thin |
| Core | `crates/livespec-core/` | Scan/index/retrieve, brief cache, prompts, sections, providers, local runtime, sessions, usage ledger. **Must not depend on Tauri** |

See [ARCHITECTURE.md](ARCHITECTURE.md) for how these fit together.

## Invariants

Break one of these and something fails at runtime, leaks, or silently changes behaviour.

### Rust

- **Locks.** Every lock in `AppState` is `std` and must never be held across `.await`. The one exception is `AppState.local`, a `tokio::sync::Mutex` held across the slow `llama-server` start-up. The usage ledger's locks are `std` too, because `Meter::drop` records through them.
- **Local server start-up** runs in its own spawned task (`provider::ensure_local`): analysis tasks are aborted on every keystroke, and an abort must not kill a model halfway through loading.
- **Send in spawned tasks.** Futures awaited inside the shell's spawned task must avoid closures over borrowed arguments (`Option::filter(|_| …)`, `.map(|(i, &x)| …)`): they trip rustc's Send check ("FnOnce is not general enough").
- **Cancellation.** Long work is a spawned task kept in an `AppState` slot. Dropping it must kill its `claude` children (`kill_on_drop`), and lanes and explorers run through `agents::run_limited` so that holds for every child.
- **Usage.** Every provider call goes through `usage::metered`, which creates the `Meter` right before `provider.stream`. Sinks that `match` on `Chunk` need a `Usage` arm; `metered` records it and never forwards it.
- **Hashes.** Persisted hashes use the in-house FNV-1a (`project::Fnv`), never `DefaultHasher`, whose output isn't stable across Rust versions.
- **Tokenizer.** `project/terms.rs` runs at index time and at query time; a change there affects both.
- **Files.** Stores write through `store::write_atomic`. A file that doesn't parse is kept as `.bad`, `.bad.1`, … (`store::load_json`, never replacing an earlier copy). A file that can't be read is reported and never overwritten. Settings fall back to defaults instead of blocking start-up.
- **Errors.** `Error::Precondition` messages are shown verbatim, so write them as an instruction to the user. `Provider` is for model, CLI or network failures.
- **Jev never fails an analysis.** Gate errors mean "analyse", rerank errors mean keyword order, check failures become one notice. Neither the gate nor the rerank may add more than `PRE_BUDGET` before the model call.
- **Jev option order.** Choice options serialize in the order given (`jev::Options`), because Jev 1.13 can lean towards the first option; don't sort or reorder them.
- **Saving never fails an analysis.** A failed history save becomes one notice.

### Claude Code

- Every run passes `--restricted` and `--strict-mcp-config`. Tools come from an allowlist via `--tools`: `""` for live analysis (which also *replaces* the system prompt with `--system-prompt`, `--max-turns 1`), `Read,Glob,Grep` for the brief (which *appends* with `--append-system-prompt`). **Never reintroduce a `--disallowedTools` denylist**: new tools would slip past it.
- The prompt goes on stdin, never argv. An API key reaches the child only as `ANTHROPIC_API_KEY` in its environment, never argv, which any process can list.
- The NDJSON parser ignores unknown event types on purpose, so CLI upgrades don't break it. The final answer comes from the `result` event.
- `--effort` is passed only when a level is set.
- To see the NDJSON `ClaudeCli::stream` parses: `echo "say hi" | claude -p --output-format stream-json --verbose --include-partial-messages --model haiku --max-turns 1`.

### Secrets

- API keys live only in the OS keychain (service `app.livespec.desktop`; accounts `anthropic-api-key`, `typesafe-api-key`). Never in settings.json, never sent to the webview. In Rust a key travels as `llm::Secret`, whose `Debug` prints `Secret(***)`.
- The TypeSafe key never crosses IPC: `jev_status` returns a `KeyStatus { source }` only. **Never prefix it `VITE_`:** Vite reads `.env.local` too and ships `VITE_*` variables to the webview bundle. `env_file::read_value` reads `.env.local` in debug builds only and never exports it.
- Jev never runs with the Local provider (`Settings::jev_allowed()`), whatever the switches say: it would send code off the machine.

### Frontend

- **Model output** renders only through `MiniMarkdown` (React elements, never `innerHTML`), behind the strict CSP in `tauri.conf.json`.
- **Ids.** Every tab or window id from the webview passes `sessions::valid_id` (`^[tw]-[0-9a-f-]{1,40}$`) before it's used, in the command and again in `SessionStore`, because a tab id becomes a file name. Anything else is `Precondition("Unknown tab.")`.
- **Context keys.** `src/lib/projects.ts` hands a project's `analysisContextKey` to its tabs only once both its summary and its instructions have loaded; a key computed earlier (`root|0|`) starts a run at launch. A tab's key adds `|<ids>` only when its sections differ from the default (`tabContextKey`), so old tabs keep their key. A project that can't be read hands its tabs only the instructions (`setContext(null, …)`).
- **Gate input.** The UI sends the draft of the last *completed* analysis as `previousDraft` (skips never move it; "Try again" sends null). Partial notes never become the comparison point.
- **Drafts** save through one `createSaver` keyed by tab id (500 ms), flushed on a switch, before closing a tab and on `pagehide`. Instructions save the same way and flush before switching projects.
- **Styling.** Use the semantic colour tokens (`bg-page`, `bg-panel`, `bg-subtle`, `border-line`, `text-ink`, `text-muted`, `text-faint`, `text-accent`, `text-fault`, `text-question`), never raw palette classes or `dark:` variants. Note colours carry meaning: conflicts red, gaps blue, ambiguities and questions amber. There's no `tailwind.config`; the theme lives in `src/styles.css`.
- **Fonts are bundled from Fontsource, never fetched**: the app must work offline and the CSP only allows `'self'`. Literata (`font-serif`) for the draft, Inter (`font-sans`) for notes and UI, JetBrains Mono (`font-mono`) for code.
- **Primitives.** Use `src/components/ui.tsx` (`Button` with its `mini` variant, `Field`, `inputClass`, `Eyebrow`, `Menu`, `Select`, `RULE` / `RuleState`, `cx`) instead of repeating class lists. Use `Select`, never a native `<select>`. A `Menu` gets the check-mark column only when an item has `checked`.
- **Dialogs.** Settings is a native `<dialog>` opened with `showModal()`. Only one drawer is open at a time, with Esc handled in `App.tsx`. Destructive actions ask inline ("Delete for good?"), never with `confirm()`.
- **Shortcuts** go through `useActions` (`src/lib/actions.ts`): a key runs only the handlers `App.tsx` passes, calls `preventDefault()` only then, and does nothing while a modal `<dialog>` is open. On macOS Ctrl+letter keeps its text-editing meaning.
- **Reduced motion.** Animated rules must have a static `motion-reduce:` fallback, and the status line must carry the state in words.
- `localStorage` access is wrapped in `try`/`catch`.

## Cross-language contracts (no codegen; keep in sync by hand)

- **IPC types:** `src/lib/ipc.ts` mirrors the serde types in `src-tauri/src/commands.rs` and `livespec-core` (`settings.rs`, `project/brief.rs`, `local/`). Serde uses `rename_all = "camelCase"`. Enums are internally tagged (`UiEvent` on `type`, `ProviderSettings` on `kind`), and `LocalStatus` / `ModelState` use `#[serde(flatten)]`.
- **Sections:** `sections::CATALOGUE` / `DEFAULT_SECTIONS` / `heading_id` in `crates/livespec-core/src/sections.rs` ↔ `SECTIONS` / `DEFAULT_SECTIONS` / the heading regex and `keyFor` in `src/lib/sections.ts`. Ids, order, the default set and the alias table (exact words: conflict(s) and fault(s) → conflicts, gap(s), ambiguity/ambiguities, question(s), intent) must match; both test suites pin them. The parser tolerates `**Conflicts:**`, `### Conflicts with code` and the old build's `## Faults`; the TypeScript side also renders the old `## Directions` (display-only).
- **Section parsing:** `prompt::section_body` (Rust) mirrors `parseSections` in `src/lib/sections.ts`; change both together.
- **Lanes:** `prompt::section_groups` (Rust) decides which sections each `lane` carries; `src/lib/lanes.ts` joins lane buffers in lane order. `analysis_system(&resolve(&[]), "")` (no instructions) must stay byte-identical to the single-lane prompt (`all_five_sections_is_the_single_lane_prompt`).
- **Settings defaults:** `Settings::default()` in `settings.rs` ↔ `DEFAULTS` in `src/components/SettingsDialog.tsx`, including `fastEffort` / `deepEffort` (`null`) and `auth` (`"login"`). `Effort` serializes lowercase, `ClaudeAuth` camelCase (↔ `Effort` / `ClaudeAuth` in `ipc.ts`). Both are read through `settings::lenient`, so an unknown value falls back to its default instead of resetting the whole file.
- **Check events:** `CheckResult` / `Verdict` in `jev/check.rs` ↔ `CheckResult` / `Verdict` in `src/lib/ipc.ts`.
- **Saved checks:** `Snapshot.checksTotal` is the `count` of the last `checking` event (the claims sent to Jev) and `checksFaults` its `total` (all Conflicts; the field keeps its old name), as `Snapshot::apply_check` records them. `snapshotAnalysis` in `src/lib/history.ts` reads fewer checks than that, without `checksStopped`, as a run an edit cut off, and shows "Checks stopped when you edited." Change both together.
- **Session instructions:** `Template` (with `sections`: empty leaves the tab's sections alone) / `ProjectInstructions` / `InstructionsView` in `instructions.rs` ↔ `src/lib/ipc.ts`; `MAX_INSTRUCTION_CHARS` in `prompt.rs` ↔ `src/lib/instructions.ts`.
- **Sessions:** `Snapshot` / `Tab` (with `sections`) / `TabSummary` / `WindowView` / `LegacyDraft` in `sessions/mod.rs` ↔ `src/lib/ipc.ts`. `sessions::first_line` ↔ `firstLine` in `src/lib/tabTitle.ts` (the first non-empty line, trimmed of Unicode White_Space, at most 48 Unicode scalars; TS uses `\p{White_Space}`, not `trim()`, which differs on U+FEFF and U+0085); both test suites pin the same strings.
- **Brief and workspace events:** `brief-status` (`events::BriefStatus { root, state, message }`, `state` lowercase) and `workspace-changed` (`events::WorkspaceChanged { windows }`) in `src-tauri/src/events.rs` ↔ `onBriefStatus` / `onWorkspaceChanged` in `src/lib/events.ts`. Every session command emits `workspace-changed`, listing the windows whose tab list changed; `save_draft` emits it only when the draft's first line changed.
- **Usage:** `UsageBucket` / `UsageKind` in `usage.rs` ↔ `src/lib/ipc.ts`; the `usage-changed` event (`events::USAGE_CHANGED`, no payload, at most once a second, to every window) ↔ `onUsageChanged` in `src/lib/events.ts`. The panel groups the hourly UTC buckets into local days in `src/lib/usage.ts` (`groupByLocalDay`), which always returns the last 30 days.
- **Menu actions:** the ids in `livespec-core::menu` are the frontend's `ActionId`s in `src/lib/actions.ts`.
- **New IPC command:** `#[tauri::command]` in `commands.rs`, then register it in `generate_handler!` in `src-tauri/src/lib.rs`, then add a wrapper to `api` in `ipc.ts`. App commands need no capability entry. Plugin and window permissions do: `src-tauri/capabilities/default.json` grants the `w-*` windows `core:default`, `dialog:allow-open`, `core:window:allow-set-title` and `core:window:allow-close`.
- **New provider** touches: `ProviderSettings` (`settings.rs`), a module in `llm/` plus an arm in each `match` on `Provider`, `provider::resolve`, `ipc.ts`, `SettingsDialog.tsx`, and `providerLabel` in `src/lib/providerOptions.ts`.

## Deliberate decisions to preserve

- **Never handle Claude subscription credentials.** Anthropic only permits subscription OAuth inside Claude Code and claude.ai, so Live Spec only runs the user's own `claude` binary. Don't add a "Sign in with Claude" flow or read Claude Code's token files. The API-key provider is the path for anyone else using Live Spec.
- **Live analysis is stateless:** each request spawns a fresh `claude -p`. A persistent stream-json session would start faster but would accumulate conversation history. A tab's saved analyses are for the user to browse; they're never sent to the model.
- **Providers are a closed enum** with `match` dispatch, chosen on purpose over `dyn Trait` + `async_trait`.
- **Parallel notes and parallel brief stay off by default.** Measured with the Claude CLI, 4 note lanes were slower than 1 and 4 explorers plus merge about 16% slower than 1 explorer.
- The old build's section names (`Faults`, `Directions`) stay in both parsers so saved notes render; never remove them.
- The identifier is `app.livespec.desktop` (was `app.livebrainstorm.desktop`). `migrate::move_app_dirs` and `secrets::migrate_from_old_service` (once per install, marker `.keychain-migrated`) never delete anything or fail the launch. The legacy `localStorage` keys `livebrainstorm.draft.*` and `livebrainstorm.lastProject` are read by `import_legacy_drafts`; keep those names.
- The llama.cpp build is pinned (`LLAMA_CPP_BUILD` in `local/runtime.rs`); upgrading is a deliberate one-line change. CPU builds everywhere except macOS, where Metal is built in.
- Prices are pinned in `usage::PRICES` (list $ per million tokens, from the Claude API model table cached 2026-10-06). Updating them is a deliberate one-line change. The CLI aliases map to the newest model of their family, local catalog models cost $0, and any other model is counted with no price ("—", "price unknown for N runs"), never as $0.
- Apps launched from a GUI don't inherit the shell `PATH`. `shell_env::login_shell_path` asks the login shell once, with a 4 s timeout. The result is used both to find `claude` and as the child's `PATH`, because the npm build of Claude Code needs `node`.
- reqwest uses `rustls-tls-native-roots` (the OS certificate store) so corporate TLS-inspecting proxies work.
- Release builds are ad-hoc signed (`bundle.macOS.signingIdentity: "-"` in `tauri.conf.json`); without a signature macOS calls a downloaded app damaged.

## Extension points

See [Where to change things](ARCHITECTURE.md#where-to-change-things). The common ones:

- Prompts and the section rules: `crates/livespec-core/src/prompt.rs` holds everything the model sees.
- Another section: add a `SectionSpec` to `sections::CATALOGUE` and a matching entry to `SECTIONS` in `src/lib/sections.ts`; the prompt, lanes, picker and parser follow.
- Starter templates: `STARTERS` in `crates/livespec-core/src/instructions.rs` (text plus a section set). Existing users get new ones through *Restore starter templates*.
- Another local model: add a `ModelSpec` to `crates/livespec-core/src/local/catalog.rs`.
