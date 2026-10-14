# LiveBrainstorm

Brainstorm in a draft on the left; an AI that has read your project thinks along in the notes panel on the right. When you pause typing it tells you what it thinks you're after, where the idea conflicts with how the project actually works, which directions are worth trying, and the questions that would most change the design.

## Run it

Prerequisites: Rust (stable), Node 20.19+ or 22.12+ (required by Vite 7), and the [Tauri v2 system prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run tauri dev      # develop
npm run tauri build    # installer for your platform
```

Then pick a model in Settings (top right):

| Provider | Setup | When to use |
|---|---|---|
| **Claude subscription** (default) | Install [Claude Code](https://docs.claude.com/en/docs/claude-code/overview), run `claude` once to log in | Your own machine, your own subscription |
| **Anthropic API key** | Paste a key; it's stored in the OS keychain | Sharing LiveBrainstorm with anyone else |
| **Local model** | Click *Install engine*, then *Download* a model | Private/offline; weaker critique |

### Optional: fact-check notes with Jev

With a [TypeSafe](https://typesafe.ai) API key, LiveBrainstorm checks every Fault in its notes against your code and marks it ✓ confirmed, ✗ contradicted, or unclear. Add the key in **Settings → Jev (TypeSafe)**; it's saved to your system Keychain. For local development you can instead put `TYPESAFE_API_KEY=your-key` in `.env.local` at the repo root (git ignores it; development builds only) or in your environment. Settings shows which one is in use, and has switches for each Jev feature.

With a key, Jev also skips re-analysis when an edit doesn't change the idea, and picks which code snippets the notes model sees. These calls send your draft, each Fault, and short code excerpts to TypeSafe. They never run with the Local model option, which promises nothing leaves your computer.

### Requirements for the Claude subscription option

LiveBrainstorm runs Claude Code in restricted mode (`--restricted`, so a project you open can't run its own hooks or servers). That mode ignores every Claude Code settings file, including your own `~/.claude/settings.json`, so authentication must come from your normal Claude Code login or from environment variables. Setups that depend on `apiKeyHelper` or an `env` block in settings (Bedrock, Vertex, a proxy base URL) need those exported in the environment instead. It also needs a Claude Code version that supports `--restricted`; if yours is older, update it with `claude update`.

## How it works

```
React (Vite, Tailwind)               Rust
┌──────────────┐  invoke + Channel  ┌───────────────────────────┐
│ DraftPane    │ ─── analyze ─────► │ src-tauri  (thin shell)   │
│ NotesPane  ◄─┼──── text deltas ── │   commands, state, keyring│
│ BriefDrawer  │                    ├───────────────────────────┤
│ Settings     │                    │ crates/livebrainstorm-core│
└──────────────┘                    │   project: scan → index → │
                                    │            retrieve, brief│
                                    │   prompt:  all prompts    │
                                    │   llm:     ClaudeCli |    │
                                    │            AnthropicApi | │
                                    │            OpenAiCompat   │
                                    │   local:   llama.cpp      │
                                    └───────────────────────────┘
```

**Understanding the project happens in two layers**, the way a new teammate ramps up:

1. **Brief (once):** an onboarding doc for the AI. With Claude Code, Claude explores the repo itself using read-only tools (`Read`, `Glob`, `Grep`; edits, shell and web are disallowed). Other providers get a budgeted digest (tree, manifests, docs, heads of central files). Cached per project and marked stale when files change. Optionally (**Settings → Agents → Map the project with parallel agents**), larger repos are split into slices mapped by parallel explorers and one call merges their notes. It's off by default: measured on this repo with Claude Code, 4 explorers plus the merge took about 16% longer than one explorer.
2. **Retrieval (every pause):** TF-IDF over code identifiers picks the 3 to 5 files your draft is about, weighting what you typed most recently. Runs in microseconds, offline.

**Live analysis:** the UI waits for a pause (configurable), sends the draft, and streams the reply into four sections. A new request aborts the previous one in Rust (which kills the `claude` process too), and stale replies are dropped by request id. If you keep typing, the last notes stay visible and the rule beside them turns dashed.

Optionally, the four sections can be written by up to four agents at once: turn on **Settings → Agents → Write notes with parallel agents** (capped by **Agents at once**). It's off by default because, measured with Claude Code, 4 agents finished *slower* than 1 (each call's start-up dominates) while using 4× the subscription. Each agent is a separate model call.

## Design decisions

- **No custom Claude login.** Anthropic only permits subscription OAuth inside Claude Code and claude.ai. LiveBrainstorm never handles those credentials; it runs your unmodified `claude` binary in print mode, which uses your existing login. Fine for personal use; anyone you share LiveBrainstorm with should use the API-key provider.
- **Core crate without Tauri.** Indexing, retrieval, prompts, providers and the local runtime are plain Rust with unit tests. The Tauri layer only wires commands, state and the keychain.
- **Enum dispatch for providers.** Three known providers, so `match` beats `dyn Trait` + `async_trait` boxing.
- **llama.cpp downloaded at runtime, version pinned.** `llama-server` ships with a folder of shared libraries, which a Tauri sidecar (single binary) can't carry. On-demand download keeps the installer small. The build is pinned (`LLAMA_CPP_BUILD`) rather than "latest" for reproducibility and to avoid GitHub API rate limits.
- **Model downloads resume.** `.part` files plus HTTP range requests; pausing or quitting mid-download loses nothing.
- **GUI PATH fix.** Apps launched from Finder don't see your shell `PATH`, so LiveBrainstorm asks your login shell once (with a timeout) to find `claude` and `node`.
- **OS certificate store** for HTTPS, so corporate TLS-inspecting proxies work.

## Tests

```bash
cargo test -p livebrainstorm-core                  # 17 unit tests
cargo test -p livebrainstorm-core -- --ignored     # downloads + runs the real llama.cpp runtime
npm test                                   # section parser
```

## Extending

- **Another local model:** add a `ModelSpec` to `crates/livebrainstorm-core/src/local/catalog.rs`.
- **GPU on Windows/Linux:** change the asset suffix in `local/runtime.rs` to a Vulkan or CUDA build.
- **Ollama / LM Studio:** both speak the OpenAI API; add a provider setting that builds `OpenAiCompat` with their base URL.
- **Fuzzier retrieval** ("payments" finding `billing/`): swap `project::retrieve` for embeddings behind the same signature.
- **Prompt tuning:** everything the model sees is in `crates/livebrainstorm-core/src/prompt.rs`.

## Known limits

- Each Claude Code analysis spawns a fresh `claude -p` process, adding a second or two of startup. A persistent `--input-format stream-json` session would be faster but accumulates conversation history, which this stateless design avoids.
- Indexes up to 6,000 text files of at most 256 KB each; larger repos are partially indexed (the brief still covers them when using Claude Code).
- Model download URLs point at Hugging Face; verify they still resolve if a download fails.
