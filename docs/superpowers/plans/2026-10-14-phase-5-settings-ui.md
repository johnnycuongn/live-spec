# Phase 5: Settings UI for Agents and Jev — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Configure everything from Settings instead of hand-editing settings.json:
- an **Agents** group: agents at once, parallel notes and parallel brief, with honest hints that both measured slower;
- a **Jev** group: a TypeSafe key saved to the Keychain, a line showing where the key comes from, and the three Jev switches, disabled without a key or for the Local model.

**Architecture:**
- **Core:** `livebrainstorm-core::jev::key` holds the pure key-lookup rule (`pick_key`: Keychain → environment → `.env.local`), `KeySource`, and `KeyStatus`, a status type that cannot carry the key.
- **Shell:** the Tauri shell uses it in `secrets` and adds two commands, `set_jev_key` and `jev_status`.
- **UI:** the React Settings dialog gains two groups, built on a new `Toggle` primitive and pure, tested helpers in `src/lib/settingsHints.ts`.

**Tech Stack:** Rust 2021 (serde, keyring), Tauri v2, React 19 + Tailwind v4, vitest.

**Spec:** `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md` (phase 5: "Settings: the Agents and Jev groups, the Keychain key and `jev_status`, then the docs"; sections "Settings", "TypeSafe key", "Frontend" → SettingsDialog; and the phase 3/4 measurements that made both parallel features opt-in).

## Global Constraints

- **Key lookup order:**
  1. Keychain (service `app.livebrainstorm.desktop`, account `typesafe-api-key`).
  2. `TYPESAFE_API_KEY` in the environment.
  3. Debug builds only: `TYPESAFE_API_KEY` in `<repo>/.env.local`.

  A blank or whitespace-only value at one level never shadows a real key further down.
- **New commands:**
  - `set_jev_key(key)`: an empty or blank key removes the Keychain entry.
  - `jev_status() -> { source: "keychain" | "environment" | "envFile" | null }`.
  - The key itself never crosses IPC.
- **Source line copy:**

  | Source | Line |
  |---|---|
  | `keychain` | "Saved in your Keychain." |
  | `environment` | "Using TYPESAFE_API_KEY from the environment." |
  | `envFile` | "Using TYPESAFE_API_KEY from .env.local (development)." |
  | `null` | "No TypeSafe key found." |

- **Jev switches** ("Skip re-analysis when an edit doesn't change the idea", "Let Jev pick the code the notes see", "Check notes against the code") are disabled when no key is found or the provider is Local. The hint explains why.
- **Agents group:**
  - "Agents at once" slider from 1 to 8; values are clamped to 1..=8 before saving.
  - The "Write notes with parallel agents" and "Map the project with parallel agents" switches are off by default.
  - Hints:
    - parallel notes: "Off by default: with Claude Code, four agents measured slower than one."
    - parallel brief: "Off by default: on a typical repo, parallel explorers plus the merge took about 16% longer."
  - All three controls are disabled for the Local provider, with the hint "Local models run one agent at a time."
- **Per-provider agent hint:**
  - Claude: "Each agent is a separate claude process on your subscription."
  - API: "Each agent is a separate API call, billed for the shared project context."
- **Privacy hint (Jev group):** "Jev sends your draft, faults and short code excerpts to TypeSafe." For Local: "Jev never runs with the Local model: nothing leaves this computer."
- **Process:**
  - Branch `feat/settings-ui`.
  - Commit trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  - Never run `cargo fmt` across the repo; `cargo clippy --workspace --all-targets` stays warning-free.
  - `src-tauri` has no tests.
  - UI uses the semantic Tailwind tokens and `ui.tsx` primitives.
  - Node 20.19+ for `npm run build` (prefix `PATH="$HOME/.nvm/versions/node/v24.15.0/bin:$PATH"`).

## Review Focus

1. **The key never reaches the webview:** `jev_status` returns only where the key comes from. Pinned in Task 1 (`status_serializes_only_the_source`).
2. **A blank `TYPESAFE_API_KEY=` or empty Keychain entry never hides a real key further down.** Pinned in Task 1 (`blank_values_never_shadow_a_real_key`).
3. **Removing the Keychain key falls back** to the environment or `.env.local`, and Settings shows that new source. Pinned in Task 1 (`lookup_order_is_keychain_then_environment_then_env_file`); the UI refreshes the status after every save or remove (Task 4 manual check).
4. **The Local model:** the agent and Jev controls are disabled with a clear reason, so nobody believes Jev or parallel agents are running. Pinned in Task 3 (`local disables agents and jev with a reason`).
5. **An odd slider or stored value** (0, 12, NaN) is clamped to 1..=8 before saving, so settings.json never holds a value that would reset all settings on load. Pinned in Task 3 (`clampAgents keeps 1..8`).

---

### Task 1: Core — where the key comes from

**Files:**
- Create: `crates/livebrainstorm-core/src/jev/key.rs`
- Modify: `crates/livebrainstorm-core/src/jev/mod.rs` (`pub mod key;`)

**Interfaces:**
- Produces:
  - `jev::key::KeySource { Keychain, Environment, EnvFile }`, serialized camelCase as `"keychain" | "environment" | "envFile"`
  - `jev::key::pick_key(keychain: Option<String>, environment: Option<String>, env_file: impl FnOnce() -> Option<String>) -> Option<(String, KeySource)>`
  - `jev::key::KeyStatus { pub source: Option<KeySource> }` (Serialize)

- [ ] **Step 0: Branch**

```bash
git switch -c feat/settings-ui
```

- [ ] **Step 1: Write the failing tests**

Add `pub mod key;` to `jev/mod.rs` after `pub mod gate;`. Create `crates/livebrainstorm-core/src/jev/key.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn some(s: &str) -> Option<String> {
        Some(s.to_owned())
    }

    #[test]
    fn lookup_order_is_keychain_then_environment_then_env_file() {
        assert_eq!(pick_key(some("kc"), some("env"), || some("file")), Some(("kc".into(), KeySource::Keychain)));
        assert_eq!(pick_key(None, some("env"), || some("file")), Some(("env".into(), KeySource::Environment)));
        assert_eq!(pick_key(None, None, || some("file")), Some(("file".into(), KeySource::EnvFile)));
        assert_eq!(pick_key(None, None, || None), None);
    }

    #[test]
    fn blank_values_never_shadow_a_real_key() {
        assert_eq!(pick_key(some("  "), some(""), || some(" file-key ")), Some(("file-key".into(), KeySource::EnvFile)));
    }

    #[test]
    fn the_env_file_is_only_read_when_needed() {
        let read = Cell::new(false);
        let found = pick_key(some("kc"), None, || {
            read.set(true);
            some("file")
        });
        assert_eq!(found.map(|(_, s)| s), Some(KeySource::Keychain));
        assert!(!read.get());
    }

    #[test]
    fn status_serializes_only_the_source() {
        let status = KeyStatus { source: Some(KeySource::EnvFile) };
        assert_eq!(serde_json::to_value(&status).unwrap(), serde_json::json!({"source": "envFile"}));
        assert_eq!(serde_json::to_value(KeyStatus { source: None }).unwrap(), serde_json::json!({"source": null}));
        assert_eq!(serde_json::to_value(KeySource::Keychain).unwrap(), "keychain");
        assert_eq!(serde_json::to_value(KeySource::Environment).unwrap(), "environment");
    }
}
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core jev::key`
Expected: compile errors, because `pick_key`, `KeySource` and `KeyStatus` are not found.

- [ ] **Step 3: Implement** (insert at the top of `key.rs`)

```rust
//! Where the TypeSafe key comes from. The shell supplies the three sources
//! (Keychain, environment, `.env.local` in debug builds); this decides which
//! one wins. `KeyStatus` is what the UI may see: the source, never the key.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    Keychain,
    Environment,
    EnvFile,
}

/// What `jev_status` sends to the webview. Deliberately has no key field.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyStatus {
    pub source: Option<KeySource>,
}

/// The first non-blank key: Keychain, then the environment, then the env
/// file. `env_file` only runs when the first two have nothing.
pub fn pick_key(
    keychain: Option<String>,
    environment: Option<String>,
    env_file: impl FnOnce() -> Option<String>,
) -> Option<(String, KeySource)> {
    let present = |k: Option<String>| k.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty());
    if let Some(k) = present(keychain) {
        return Some((k, KeySource::Keychain));
    }
    if let Some(k) = present(environment) {
        return Some((k, KeySource::Environment));
    }
    present(env_file()).map(|k| (k, KeySource::EnvFile))
}

```

- [ ] **Step 4: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core --lib && cargo clippy --workspace --all-targets`
Expected: all tests pass, including the 4 new `jev::key` tests; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/jev
git commit -m "feat(jev): key lookup order and a status that carries only the source" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Shell — `set_jev_key` and `jev_status`

**Files:**
- Modify: `src-tauri/src/secrets.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs` (register both commands)

**Interfaces:**
- Consumes: `jev::key::{pick_key, KeySource, KeyStatus}` (Task 1).
- Produces the IPC contract Tasks 3–4 mirror:
  - `set_jev_key({ key })`: an empty key removes the Keychain entry; errors come back as a string.
  - `jev_status() -> { source: "keychain" | "environment" | "envFile" | null }`.

- [ ] **Step 1: Rework `secrets.rs`**

Replace `set_api_key` and `jev_key` with the following, keeping `entry`, `keychain_get`, `api_key` and `dev_env_file_key` as they are:

```rust
pub fn set_api_key(key: &str) -> Result<()> {
    set_secret(ACCOUNT, key, "API key")
}

/// Saves the TypeSafe key to the Keychain; an empty key removes it.
pub fn set_jev_key(key: &str) -> Result<()> {
    set_secret(JEV_ACCOUNT, key, "TypeSafe key")
}

fn set_secret(account: &str, key: &str, what: &str) -> Result<()> {
    let entry = entry(account)?;
    let result = if key.trim().is_empty() {
        match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        }
    } else {
        entry.set_password(key.trim())
    };
    result.map_err(|e| Error::Provider(format!("Couldn't save the {what} to the keychain: {e}")))
}

/// The TypeSafe key: Keychain, then `TYPESAFE_API_KEY`, then (debug builds
/// only) the repo's `.env.local`. It never crosses IPC.
pub fn jev_key() -> Option<String> {
    jev_key_with_source().map(|(key, _)| key)
}

pub fn jev_key_with_source() -> Option<(String, KeySource)> {
    pick_key(keychain_get(JEV_ACCOUNT).ok().flatten(), std::env::var(JEV_ENV).ok(), dev_env_file_key)
}
```

Add to the imports in `secrets.rs`:

```rust
use livebrainstorm_core::jev::key::{pick_key, KeySource};
```

- [ ] **Step 2: Add the commands** (`commands.rs`, after `has_api_key`)

```rust
#[tauri::command]
pub fn set_jev_key(key: String) -> Result<()> {
    secrets::set_jev_key(&key)
}

/// Where the TypeSafe key comes from, never the key itself.
#[tauri::command]
pub fn jev_status() -> KeyStatus {
    KeyStatus { source: secrets::jev_key_with_source().map(|(_, source)| source) }
}
```

Add `use livebrainstorm_core::jev::key::KeyStatus;` to the imports. In `src-tauri/src/lib.rs`, add `commands::set_jev_key,` and `commands::jev_status,` to `generate_handler![…]` after `commands::has_api_key,`.

- [ ] **Step 3: Build and lint**

Run: `cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core`
Expected: no warnings; all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri
git commit -m "feat: set_jev_key and jev_status commands (the key never crosses IPC)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Frontend — IPC and settings hints

**Files:**
- Modify: `src/lib/ipc.ts`
- Create: `src/lib/settingsHints.ts`, `src/lib/settingsHints.test.ts`

**Interfaces:**
- Consumes: the IPC contract from Task 2.
- Produces:
  - In `ipc.ts`: `type KeySource`, `interface JevStatus`, `api.setJevKey(key)`, `api.jevStatus()`.
  - In `settingsHints.ts`:
    - `PARALLEL_NOTES_HINT`, `PARALLEL_BRIEF_HINT`
    - `clampAgents(n: number): number`
    - `agentHint(kind: ProviderSettings["kind"]): string`
    - `keySourceLabel(source: KeySource | null): string`
    - `jevUsable(source: KeySource | null, kind: ProviderSettings["kind"]): boolean`
    - `jevHint(source: KeySource | null, kind: ProviderSettings["kind"]): string`

- [ ] **Step 1: Mirror the IPC contract** (`src/lib/ipc.ts`)

Add after the `Settings` interface:

```ts
export type KeySource = "keychain" | "environment" | "envFile";

/** Where the TypeSafe key comes from; the key itself never reaches the webview. */
export interface JevStatus {
  source: KeySource | null;
}
```

Add to `api`, after `hasApiKey`:

```ts
  setJevKey: (key: string) => invoke<void>("set_jev_key", { key }),
  jevStatus: () => invoke<JevStatus>("jev_status"),
```

- [ ] **Step 2: Write the failing tests** (`src/lib/settingsHints.test.ts`)

```ts
import { describe, expect, it } from "vitest";
import { agentHint, clampAgents, jevHint, jevUsable, keySourceLabel, PARALLEL_BRIEF_HINT, PARALLEL_NOTES_HINT } from "./settingsHints";

describe("agents", () => {
  it("clampAgents keeps 1..8", () => {
    expect([0, 1, 4, 8, 12, -3, 2.6, Number.NaN].map(clampAgents)).toEqual([1, 1, 4, 8, 8, 1, 3, 1]);
  });

  it("explains what an agent costs per provider", () => {
    expect(agentHint("claudeCli")).toBe("Each agent is a separate claude process on your subscription.");
    expect(agentHint("anthropicApi")).toBe("Each agent is a separate API call, billed for the shared project context.");
    expect(agentHint("local")).toBe("Local models run one agent at a time.");
  });

  it("says plainly that parallel was measured slower", () => {
    expect(PARALLEL_NOTES_HINT).toBe("Off by default: with Claude Code, four agents measured slower than one.");
    expect(PARALLEL_BRIEF_HINT).toBe("Off by default: on a typical repo, parallel explorers plus the merge took about 16% longer.");
  });
});

describe("jev", () => {
  it("names where the key comes from", () => {
    expect(keySourceLabel("keychain")).toBe("Saved in your Keychain.");
    expect(keySourceLabel("environment")).toBe("Using TYPESAFE_API_KEY from the environment.");
    expect(keySourceLabel("envFile")).toBe("Using TYPESAFE_API_KEY from .env.local (development).");
    expect(keySourceLabel(null)).toBe("No TypeSafe key found.");
  });

  it("local disables agents and jev with a reason", () => {
    expect(jevUsable("keychain", "local")).toBe(false);
    expect(jevHint("keychain", "local")).toBe("Jev never runs with the Local model: nothing leaves this computer.");
  });

  it("needs a key before the switches mean anything", () => {
    expect(jevUsable(null, "claudeCli")).toBe(false);
    expect(jevHint(null, "claudeCli")).toBe("Add a key to turn these on. Jev sends your draft, faults and short code excerpts to TypeSafe.");
    expect(jevUsable("envFile", "anthropicApi")).toBe(true);
    expect(jevHint("envFile", "anthropicApi")).toBe("Jev sends your draft, faults and short code excerpts to TypeSafe.");
  });
});
```

- [ ] **Step 3: Run and confirm the failure**

Run: `npm test`
Expected: FAIL, because `./settingsHints` cannot be resolved.

- [ ] **Step 4: Implement** (`src/lib/settingsHints.ts`)

```ts
// Copy and rules for the Agents and Jev settings groups, kept pure so they're
// testable. The Rust side enforces the same limits (Settings::max_agents
// clamps to 1..=8; Jev never runs for the Local provider).
import type { KeySource, ProviderSettings } from "./ipc";

type Kind = ProviderSettings["kind"];

export const PARALLEL_NOTES_HINT = "Off by default: with Claude Code, four agents measured slower than one.";
export const PARALLEL_BRIEF_HINT = "Off by default: on a typical repo, parallel explorers plus the merge took about 16% longer.";

export const clampAgents = (n: number) => (Number.isFinite(n) ? Math.min(8, Math.max(1, Math.round(n))) : 1);

const AGENT_HINT: Record<Kind, string> = {
  claudeCli: "Each agent is a separate claude process on your subscription.",
  anthropicApi: "Each agent is a separate API call, billed for the shared project context.",
  local: "Local models run one agent at a time.",
};

export const agentHint = (kind: Kind) => AGENT_HINT[kind];

const SOURCE_LABEL: Record<KeySource, string> = {
  keychain: "Saved in your Keychain.",
  environment: "Using TYPESAFE_API_KEY from the environment.",
  envFile: "Using TYPESAFE_API_KEY from .env.local (development).",
};

export const keySourceLabel = (source: KeySource | null) => (source ? SOURCE_LABEL[source] : "No TypeSafe key found.");

export const jevUsable = (source: KeySource | null, kind: Kind) => source !== null && kind !== "local";

const SENDS = "Jev sends your draft, faults and short code excerpts to TypeSafe.";

export function jevHint(source: KeySource | null, kind: Kind): string {
  if (kind === "local") return "Jev never runs with the Local model: nothing leaves this computer.";
  return source ? SENDS : `Add a key to turn these on. ${SENDS}`;
}
```

- [ ] **Step 5: Run and confirm the pass**

Run: `npm test && npm run typecheck`
Expected: 37 tests pass (31 existing and 6 new); no type errors.

- [ ] **Step 6: Commit**

```bash
git add src/lib
git commit -m "feat(ui): settings hints and the jev key commands in the IPC layer" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Frontend — the Agents and Jev groups

**Files:**
- Modify: `src/components/ui.tsx` (add `Toggle`)
- Modify: `src/components/SettingsDialog.tsx` (two groups and two field components)

**Interfaces:**
- Consumes: `api.setJevKey`, `api.jevStatus`, `JevStatus`, `AgentSettings`, `JevSettings` (`ipc.ts`); all of `settingsHints.ts` (Task 3).
- Produces: `Toggle({ checked, onChange, label, hint?, disabled? })` in `ui.tsx`.

- [ ] **Step 1: Add the `Toggle` primitive** (append to `src/components/ui.tsx`)

```tsx
/** An on/off switch. A real checkbox underneath, so keyboard and screen readers work. */
export function Toggle(props: { checked: boolean; onChange: (on: boolean) => void; label: string; hint?: string; disabled?: boolean }) {
  return (
    <label className={cx("flex items-start gap-3", props.disabled ? "opacity-50" : "cursor-pointer")}>
      <input
        type="checkbox"
        role="switch"
        className="peer sr-only"
        checked={props.checked}
        disabled={props.disabled}
        onChange={(e) => props.onChange(e.target.checked)}
      />
      <span
        aria-hidden
        className={cx(
          "relative mt-0.5 h-5 w-9 shrink-0 rounded-full transition-colors peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent",
          props.checked ? "bg-accent" : "bg-line",
        )}
      >
        <span className={cx("absolute top-0.5 left-0.5 size-4 rounded-full bg-page transition-transform", props.checked && "translate-x-4")} />
      </span>
      <span className="grid gap-0.5">
        <span className="text-[13px] text-ink">{props.label}</span>
        {props.hint && <span className="text-xs text-muted">{props.hint}</span>}
      </span>
    </label>
  );
}
```

- [ ] **Step 2: Add the field components** (append to `src/components/SettingsDialog.tsx`)

```tsx
function AgentFields({ settings, update }: { settings: Settings; update: (a: Partial<AgentSettings>) => void }) {
  const a = settings.agents;
  const kind = settings.provider.kind;
  const local = kind === "local";
  return (
    <div className="grid gap-3">
      <Field label={`Agents at once: ${local ? 1 : a.maxAgents}`}>
        <input
          type="range"
          className="w-full accent-accent"
          min={1}
          max={8}
          step={1}
          value={a.maxAgents}
          disabled={local}
          onChange={(e) => update({ maxAgents: clampAgents(Number(e.target.value)) })}
        />
      </Field>
      <Toggle
        label="Write notes with parallel agents"
        hint={PARALLEL_NOTES_HINT}
        checked={a.parallelNotes && !local}
        disabled={local}
        onChange={(parallelNotes) => update({ parallelNotes })}
      />
      <Toggle
        label="Map the project with parallel agents"
        hint={PARALLEL_BRIEF_HINT}
        checked={a.parallelBrief && !local}
        disabled={local}
        onChange={(parallelBrief) => update({ parallelBrief })}
      />
      <p className="text-xs text-muted">{agentHint(kind)}</p>
    </div>
  );
}

function JevFields({ settings, update }: { settings: Settings; update: (j: Partial<JevSettings>) => void }) {
  const [status, setStatus] = useState<JevStatus | null>(null);
  const [key, setKey] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const refresh = () => api.jevStatus().then(setStatus, (e) => setNote(errorMessage(e)));
  useEffect(() => void refresh(), []);

  const kind = settings.provider.kind;
  const source = status?.source ?? null;
  const inKeychain = source === "keychain";
  const usable = jevUsable(source, kind);
  const j = settings.jev;

  const saveKey = async () => {
    try {
      await api.setJevKey(key);
      setNote(key.trim() ? "Key saved to your Keychain." : "Key removed from your Keychain.");
      setKey("");
      await refresh();
    } catch (e) {
      setNote(errorMessage(e));
    }
  };

  return (
    <div className="grid gap-3">
      <Field label="TypeSafe API key">
        <div className="flex gap-2">
          <input
            type="password"
            className={inputClass}
            value={key}
            placeholder={inKeychain ? "Paste a new key to replace it" : "Paste a key from console.typesafe.ai/keys"}
            onChange={(e) => setKey(e.target.value)}
          />
          <Button onClick={saveKey} disabled={!key.trim() && !inKeychain}>
            {key.trim() || !inKeychain ? "Save key" : "Remove key"}
          </Button>
        </div>
        <small className="text-xs text-muted">{status ? keySourceLabel(source) : "Checking for a key…"}</small>
        {note && <small className="text-xs text-muted">{note}</small>}
      </Field>
      <Toggle label="Skip re-analysis when an edit doesn't change the idea" checked={j.gate && usable} disabled={!usable} onChange={(gate) => update({ gate })} />
      <Toggle label="Let Jev pick the code the notes see" checked={j.rerank && usable} disabled={!usable} onChange={(rerank) => update({ rerank })} />
      <Toggle label="Check notes against the code" checked={j.check && usable} disabled={!usable} onChange={(check) => update({ check })} />
      <p className="text-xs text-muted">{jevHint(source, kind)}</p>
    </div>
  );
}
```

- [ ] **Step 3: Wire them into the dialog**

In `SettingsDialog.tsx`:
1. Extend the imports:
   - from `../lib/ipc`: add `type AgentSettings, type JevSettings, type JevStatus`;
   - from `./ui`: add `Toggle`;
   - add `import { agentHint, clampAgents, jevHint, jevUsable, keySourceLabel, PARALLEL_BRIEF_HINT, PARALLEL_NOTES_HINT } from "../lib/settingsHints";`.
2. Inside `SettingsDialog`, after `const patch = …`, add:

```tsx
  const updateAgents = (a: Partial<AgentSettings>) => setDraft((d) => ({ ...d, agents: { ...d.agents, ...a } }));
  const updateJev = (j: Partial<JevSettings>) => setDraft((d) => ({ ...d, jev: { ...d.jev, ...j } }));
```

3. Insert between the closing `</Group>` of "Where LiveBrainstorm thinks" and `<Group title="Timing">`:

```tsx
        <Group title="Agents">
          <AgentFields settings={draft} update={updateAgents} />
        </Group>

        <Group title="Jev (TypeSafe)">
          <JevFields settings={draft} update={updateJev} />
        </Group>
```

These switches show "off" whenever they can't apply (Local, or no key) but keep the stored value. Saving doesn't wipe a user's choice; it only takes effect when it can.

- [ ] **Step 4: Verify**

Run: `npm test && npm run typecheck && PATH="$HOME/.nvm/versions/node/v24.15.0/bin:$PATH" npm run build`
Expected: 37 tests pass; no type errors; the build prints `✓ built in`.

- [ ] **Step 5: Look at it**

Using the mocked-IPC preview harness from earlier in this session (`preview.mjs`, `mock-ipc.js`, `shots.sh` in the session scratchpad), first update `mock-ipc.js`:
- `get_settings` returns `agents: { maxAgents: 4, parallelBrief: false, parallelNotes: false }` and `jev: { gate: true, rerank: true, check: true, model: "jev-latest" }`;
- `jev_status` returns `{ source: "envFile" }`.

Then screenshot `#settings` in light and dark mode. Check:
- both groups render under "Where LiveBrainstorm thinks";
- the switches are aligned;
- the hints are legible in both themes.

If the harness is gone, skip this step and ledger that the visual check is left to the user (Task 5, Step 3).

- [ ] **Step 6: Commit**

```bash
git add src/components
git commit -m "feat(ui): Agents and Jev groups in Settings" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Docs and end-to-end

**Files:**
- Modify: `README.md`, `AGENTS.md`, `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md`

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Update the docs**

1. In `README.md`:
   - Under "### Optional: fact-check notes with Jev", replace the paragraph starting "For local development, put the key in `.env.local`" and the code block and paragraph after it with:

     ```markdown
     Add the key in **Settings → Jev (TypeSafe)**; it's saved to your system Keychain. For local development you can instead put `TYPESAFE_API_KEY=your-key` in `.env.local` at the repo root (git ignores it; development builds only) or in your environment. Settings shows which one is in use, and has switches for each Jev feature.
     ```

   - In the "**Live analysis:**" paragraph and in item 1 of "Understanding the project happens in two layers", replace "set `agents.parallelNotes: true` in settings" with "turn on **Settings → Agents → Write notes with parallel agents**", and replace "(`agents.parallelBrief: true`, capped by `agents.maxAgents`)" with "(**Settings → Agents → Map the project with parallel agents**)".
2. In `AGENTS.md`, under "### Jev (TypeSafe)", append to the **Key lookup** bullet:

   ```markdown
    The rule itself is `jev::key::pick_key` (blank values never shadow a later source); `jev_status` returns a `KeyStatus { source }` (never the key) and `set_jev_key` saves to or removes from the Keychain. Settings → Jev shows the source and greys out the switches without a key or for Local.
   ```

3. In the spec's "## Settings" Rust block, change the comments `// default true` on `parallel_brief` and `parallel_notes` to `// default false (measured slower; see Measurements)`.

- [ ] **Step 2: Final verification and commit**

Run: `cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core && npm test && npm run typecheck`
Expected: no warnings; all tests pass.

```bash
git add README.md AGENTS.md docs/superpowers/specs
git commit -m "docs: Settings now configures agents and Jev" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 3: End-to-end check (the user, in the running app)**

Ask the user to restart `npm run tauri dev` and open **Settings**:
1. **Jev group, initial state:** it says "Using TYPESAFE_API_KEY from .env.local (development)." and the three switches are enabled.
2. **Save a key:** paste a key and click **Save key**. The line changes to "Saved in your Keychain."
3. **Remove it:** clear the field and click **Remove key**. The line falls back to the `.env.local` source.
4. **Local model:** switch the provider to **Local model**. The agent and Jev controls grey out, with the Local hints.
5. **Persistence:** turn on **Write notes with parallel agents**, set **Agents at once** to 2, and save. Reopen Settings: both are kept. Pause on a draft and run `pgrep -fl "claude -p" | wc -l`: it shows 2 while the notes stream.
