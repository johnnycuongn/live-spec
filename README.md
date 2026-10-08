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

1. **Brief (once):** an onboarding doc for the AI. With Claude Code, Claude explores the repo itself using read-only tools (`Read`, `Glob`, `Grep`; edits, shell and web are disallowed). Other providers get a budgeted digest (tree, manifests, docs, heads of central files). Cached per project and marked stale when files change.
2. **Retrieval (every pause):** TF-IDF over code identifiers picks the 3 to 5 files your draft is about, weighting what you typed most recently. Runs in microseconds, offline.

**Live analysis:** the UI waits for a pause (configurable), sends the draft, and streams the reply into four sections. A new request aborts the previous one in Rust (which kills the `claude` process too), and stale replies are dropped by request id. If you keep typing, the last notes stay visible and the rule beside them turns dashed.

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
