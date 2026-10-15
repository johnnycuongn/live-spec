# Setting up Live Spec

This guide covers installing the app, choosing the model that reviews your spec, and building Live Spec from source. For how the app works inside, see [ARCHITECTURE.md](ARCHITECTURE.md).

- [Install the app](#install-the-app)
- [Choose a model](#choose-a-model)
- [Claude Code and restricted mode](#claude-code-and-restricted-mode)
- [Optional: fact-check with Jev](#optional-fact-check-with-jev)
- [Your first spec](#your-first-spec)
- [Build from source](#build-from-source)
- [Run the tests](#run-the-tests)
- [Where your data lives](#where-your-data-lives)
- [Upgrading from LiveBrainstorm](#upgrading-from-livebrainstorm)
- [Troubleshooting](#troubleshooting)

## Install the app

### macOS (Apple Silicon)

1. Download the latest `Live-Spec_<version>_aarch64.dmg` from [Releases](https://github.com/johnnycuongn/live-spec/releases/latest).
2. Open the DMG and drag **Live Spec** into **Applications**.
3. Open Live Spec from Applications.

The release is ad-hoc signed but not notarized by Apple, so the first launch is blocked with "Apple could not verify 'Live Spec' is free of malware". To allow it:

- Open **System Settings → Privacy & Security**, scroll to the message about Live Spec, and click **Open Anyway**. Then open the app again and confirm.
- Or, in Terminal: `xattr -dr com.apple.quarantine "/Applications/Live Spec.app"`

You only need to do this once per version.

### Intel Macs, Windows and Linux

There are no prebuilt installers yet. [Build from source](#build-from-source): `npm run tauri build` produces the installer for the machine you build on.

## Choose a model

Open **Settings** (top right, or ⌘,) and pick a provider. Live Spec uses two model slots: **While you type** (fast, for the live notes) and **Project brief** (thorough, run once per project).

| Provider | Setup | Good for |
|---|---|---|
| **Claude subscription** (default) | Install [Claude Code](https://docs.claude.com/en/docs/claude-code/overview) and run `claude` once in a terminal to log in | Your own machine, using your own Claude plan |
| **Anthropic API key** | Paste a key from the [Anthropic Console](https://console.anthropic.com/). It's saved in the OS keychain | Anyone without Claude Code, or sharing Live Spec with others |
| **Local model** | Click **Install engine**, then **Download** a model (Qwen3 4B, Llama 3.2 3B or Qwen2.5 Coder 7B) | Private or offline work. Notes are weaker, and nothing leaves your computer |

With Claude Code, the model fields take an alias (`haiku`, `sonnet`, `opus`, `fable`) or any full model id through **Custom…**. Effort **Default** keeps the model's own level, and a lower effort usually means faster notes. If Live Spec can't find `claude`, put its full path in **Claude Code location** (run `which claude` in a terminal to get it).

## Claude Code and restricted mode

Live Spec runs Claude Code with `--restricted`, so a project you open can't run its own hooks, settings or MCP servers. That mode also ignores *your* Claude Code settings files, including `~/.claude/settings.json`. In practice:

- **Normal login works.** If `claude` works in your terminal after `claude` / `/login`, it works in Live Spec.
- **`apiKeyHelper` doesn't.** Choose **Sign in with: My Anthropic API key** in Settings instead. Live Spec passes the key from your keychain to Claude Code as `ANTHROPIC_API_KEY`, and usage is billed to your API account rather than your subscription.
- **Settings `env` blocks don't apply.** For Bedrock, Vertex or a proxy base URL, export those variables in your shell environment instead.
- **Your environment wins.** An `ANTHROPIC_API_KEY` exported in your shell takes precedence over the Claude Code login. Unset it if you want your subscription used. `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK` and `CLAUDE_CODE_USE_VERTEX` can override the keychain key the same way.
- **Needs a recent Claude Code.** If yours doesn't support `--restricted`, run `claude update`.

## Optional: fact-check with Jev

With a [TypeSafe](https://typesafe.ai) API key, Live Spec checks every Conflict in its notes against your code and marks it ✓ confirmed, ✗ contradicted, or unclear. It can also skip re-analysis when an edit doesn't change the idea, and choose which code snippets the notes model sees.

1. Open **Settings → Jev (TypeSafe)** and paste the key. It's saved in your system keychain.
2. Turn on the features you want. Settings shows which key source is in use.

These calls send your spec, each Conflict and short code excerpts to TypeSafe. They never run with the Local provider.

For development you can instead put `TYPESAFE_API_KEY=your-key` in `.env.local` at the repo root (git ignores it, and only debug builds read it), or export it in your environment. Never name it `VITE_TYPESAFE_API_KEY`: Vite would ship it to the webview.

## Your first spec

1. Click **Open a project…** (⌘O) and choose the folder of the codebase the spec is for. You can also write without a project; the notes then can't check against code.
2. Build the project **brief**: click **Build it now** above the notes (or **⋯ → Rebuild brief** beside the project in the sidebar). It's a one-time read of the project that every tab on it shares, and with Claude Code it takes a minute or two. Notes work without it, but they're sharper with it.
3. Write the spec on the left. When you pause, the notes stream in on the right.
4. Optional: click **Sections ▾** to choose which sections this tab gets, or **Instructions** to apply a template (Feature spec, Bug fix spec, Refactor spec, Hand-off pass, Devil's advocate).
5. Revise until Conflicts, Gaps and Ambiguities have nothing left you care about, then click **Copy spec** and paste it to your coding agent.

## Build from source

### Prerequisites

| Tool | Version | Notes |
|---|---|---|
| Rust | stable | [rustup.rs](https://rustup.rs) |
| Node.js | **20.19+ or 22.12+** | Vite 7 requires it. On Node 20.11 the app fails to start with `crypto.hash is not a function`, while typecheck and tests still pass |
| Tauri v2 system dependencies | | macOS: Xcode Command Line Tools (`xcode-select --install`). Linux and Windows: follow [Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/) |

### Run in development

```bash
git clone https://github.com/johnnycuongn/live-spec.git
cd live-spec
npm install
npm run tauri dev
```

This starts Vite on port 1420 and the Rust shell, with hot reload for the UI. `npm run dev` starts Vite alone, but every backend call fails outside the Tauri window, so use `npm run tauri dev` for anything beyond styling.

### Build an installer

```bash
npm run tauri build                      # every bundle type for this OS
npm run tauri build -- --bundles dmg     # just the macOS DMG
```

The output lands in `target/release/bundle/` (for example `target/release/bundle/dmg/Live Spec_0.1.0_aarch64.dmg`). To ad-hoc sign the macOS app, `src-tauri/tauri.conf.json` sets `bundle.macOS.signingIdentity` to `"-"`. Set `CI=true` when building without a desktop session, so the DMG script skips arranging the Finder window.

## Run the tests

```bash
npm run typecheck                               # tsc --noEmit
npx vitest run --dir src                        # frontend tests
cargo test --workspace                          # Rust unit tests
cargo clippy --workspace --all-targets -- -D warnings
```

`npx vitest run --dir src` keeps vitest out of any `.claude/worktrees` copies; plain `npm test` collects those too.

Live tests are ignored by default because they need the network, a key or your Claude subscription:

```bash
cargo test -p livespec-core -- --ignored                                  # downloads and runs the real llama.cpp build
cargo test -p livespec-core --test jev_live -- --ignored --nocapture      # TypeSafe API (needs the key)
cargo test -p livespec-core --test notes_live -- --ignored --nocapture    # Claude Code, 1 vs 4 note lanes
cargo test -p livespec-core --test brief_live -- --ignored --nocapture    # Claude Code, 1 vs 4 brief explorers (several minutes)
```

To see which files retrieval would pick for a draft:

```bash
cargo run -p livespec-core --example probe -- <project-dir> "<draft text>"
```

## Where your data lives

| OS | Folder |
|---|---|
| macOS | `~/Library/Application Support/app.livespec.desktop/` |
| Linux | `~/.config/app.livespec.desktop/` (settings, instructions) and `~/.local/share/app.livespec.desktop/` (tabs, briefs, usage, models) |
| Windows | `%APPDATA%\app.livespec.desktop\` |

API keys are kept in the OS keychain (service `app.livespec.desktop`), never in these folders. To start fresh, quit Live Spec and move the folder aside.

## Upgrading from LiveBrainstorm

Live Spec used to be called LiveBrainstorm. On first launch it moves the old app folder (tabs, briefs, settings, usage, local models) to the new location and copies your saved keys to the new keychain entry. macOS may ask permission to read the old keychain item. Nothing is deleted. Window positions may reset once, and the sidebar's shown or hidden state starts over.

If you build an older LiveBrainstorm version from source afterwards, it opens empty, because its data has moved. Renaming the folder back restores it.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `crypto.hash is not a function` when starting dev | Upgrade Node to 20.19+ or 22.12+ (`nvm install 22`) |
| `Port 1420 is already in use` | Another `npm run tauri dev` is running. Stop it, or use that one |
| "Couldn't run Claude Code at '…'" although it is installed | Set **Claude Code location** in Settings to the output of `which claude` |
| Claude Code fails to authenticate | See [restricted mode](#claude-code-and-restricted-mode). Check `echo $ANTHROPIC_API_KEY` |
| "unknown option --restricted" | `claude update` |
| macOS says the app "is damaged" or can't be verified | See [Install the app](#install-the-app) |
| A local model download fails | Model URLs point at Hugging Face. Check that it's reachable from your network, then retry; downloads resume |
| Notes never appear | The draft needs at least 24 characters. Check the status line under the tab in the sidebar for an error |
