import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  api,
  errorMessage,
  type AgentSettings,
  type Effort,
  type JevSettings,
  type JevStatus,
  type LocalStatus,
  type ProviderSettings,
  type Settings,
} from "../lib/ipc";
import { formatBytes, pauseDownload, startDownload, useDownloads } from "../lib/downloads";
import { agentHint, clampAgents, clampedAgents, jevHint, jevUsable, keySourceLabel, PARALLEL_BRIEF_HINT, PARALLEL_NOTES_HINT } from "../lib/settingsHints";
import { CLAUDE_ALIASES, EFFORT_OPTIONS, checkModels, isCustomModel } from "../lib/providerOptions";
import { Button, Eyebrow, Field, Select, Toggle, cx, inputClass } from "./ui";

interface Props {
  settings: Settings;
  onSave: (s: Settings) => Promise<void>;
  onClose: () => void;
}

const DEFAULTS: Record<ProviderSettings["kind"], ProviderSettings> = {
  claudeCli: { kind: "claudeCli", binaryPath: null, fastModel: "haiku", deepModel: "sonnet", fastEffort: null, deepEffort: null, auth: "login" },
  anthropicApi: { kind: "anthropicApi", fastModel: "claude-haiku-5-5", deepModel: "claude-sonnet-5-5", fastEffort: null, deepEffort: null },
  local: { kind: "local", modelId: "qwen3-4b-instruct" },
};

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="grid gap-3">
      <h3>
        <Eyebrow>{title}</Eyebrow>
      </h3>
      {children}
    </section>
  );
}

export function SettingsDialog({ settings, onSave, onClose }: Props) {
  const [draft, setDraft] = useState<Settings>(settings);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const p = draft.provider;

  // Native modal: focus trap, Esc (fires `close`) and ::backdrop for free.
  // StrictMode runs this twice, and older WebKit throws on a second showModal().
  useEffect(() => {
    if (dialog.current && !dialog.current.open) dialog.current.showModal();
  }, []);

  const choose = (kind: ProviderSettings["kind"]) =>
    setDraft((d) => ({ ...d, provider: d.provider.kind === kind ? d.provider : DEFAULTS[kind] }));
  const patch = (fields: Partial<ProviderSettings>) =>
    setDraft((d) => ({ ...d, provider: { ...d.provider, ...fields } as ProviderSettings }));
  const updateAgents = (a: Partial<AgentSettings>) => setDraft((d) => ({ ...d, agents: { ...d.agents, ...a } }));
  const updateJev = (j: Partial<JevSettings>) => setDraft((d) => ({ ...d, jev: { ...d.jev, ...j } }));

  const save = async () => {
    const { provider, error: modelError } = checkModels(draft.provider);
    if (modelError) return setError(modelError);
    setSaving(true);
    setError(null);
    try {
      await onSave({ ...draft, provider, agents: clampedAgents(draft.agents) });
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <dialog
      ref={dialog}
      onClose={onClose}
      // A press on the backdrop targets the <dialog> itself; the content div covers the rest.
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
      aria-labelledby="settings-title"
      className="m-auto max-h-[calc(100%-2.5rem)] w-[min(620px,calc(100%-2.5rem))] overflow-y-auto rounded-xl border border-line bg-page text-ink shadow-2xl backdrop:bg-black/40"
    >
      <div className="grid gap-6 p-6">
        <h2 id="settings-title" className="text-lg font-semibold tracking-tight">
          Settings
        </h2>

        <Group title="Where LiveBrainstorm thinks">
          <fieldset className="grid grid-cols-3 gap-2">
            <legend className="sr-only">Model provider</legend>
            <ProviderOption checked={p.kind === "claudeCli"} onSelect={() => choose("claudeCli")} title="Claude subscription">
              Runs your installed Claude Code and its existing login. Meant for your own use on this machine.
            </ProviderOption>
            <ProviderOption checked={p.kind === "anthropicApi"} onSelect={() => choose("anthropicApi")} title="Anthropic API key">
              Pay per use with your own key. The right choice if you share LiveBrainstorm with others.
            </ProviderOption>
            <ProviderOption checked={p.kind === "local"} onSelect={() => choose("local")} title="Local model">
              Runs fully on this computer. Private and offline, but less sharp.
            </ProviderOption>
          </fieldset>

          {p.kind === "claudeCli" && <ClaudeCliFields p={p} patch={patch} />}
          {p.kind === "anthropicApi" && <ApiFields p={p} patch={patch} />}
          {p.kind === "local" && <LocalFields modelId={p.modelId} onPick={(modelId) => patch({ modelId })} />}
        </Group>

        <Group title="Agents">
          <AgentFields settings={draft} update={updateAgents} />
        </Group>

        <Group title="Jev (TypeSafe)">
          <JevFields settings={draft} update={updateJev} />
        </Group>

        <Group title="Timing">
          <Field label={`Pause before LiveBrainstorm replies: ${(draft.debounceMs / 1000).toFixed(1)} s`}>
            <input
              type="range"
              className="w-full accent-accent"
              min={500}
              max={3000}
              step={100}
              value={draft.debounceMs}
              onChange={(e) => setDraft((d) => ({ ...d, debounceMs: Number(e.target.value) }))}
            />
          </Field>
        </Group>

        {error && (
          <p className="text-[13px] text-fault" role="alert">
            {error}
          </p>
        )}
        <footer className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={save} disabled={saving}>
            {saving ? "Saving…" : "Save settings"}
          </Button>
        </footer>
      </div>
    </dialog>
  );
}

function ProviderOption(props: { checked: boolean; onSelect: () => void; title: string; children: string }) {
  return (
    <label
      className={cx(
        // Only border-color animates: transition-colors would fade the focus ring in from black.
        "grid cursor-pointer content-start gap-1 rounded-lg border p-3 transition-[border-color] has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-accent",
        props.checked ? "border-accent ring-1 ring-accent" : "border-line hover:border-faint",
      )}
    >
      <input type="radio" name="provider" className="sr-only" checked={props.checked} onChange={props.onSelect} />
      <span className="text-[13px] font-semibold">{props.title}</span>
      <span className="text-xs leading-snug text-muted">{props.children}</span>
    </label>
  );
}

function ClaudeCliFields({ p, patch }: { p: Extract<ProviderSettings, { kind: "claudeCli" }>; patch: (f: Partial<ProviderSettings>) => void }) {
  const [check, setCheck] = useState<string | null>(null);
  const run = async () => {
    setCheck("Checking…");
    try {
      setCheck(`Found Claude Code ${await api.checkClaude(p.binaryPath)}`);
    } catch (e) {
      setCheck(errorMessage(e));
    }
  };
  return (
    <div className="grid gap-3">
      <Field label="Claude Code location (leave empty to find it automatically)">
        <div className="flex gap-2">
          <input className={inputClass} value={p.binaryPath ?? ""} placeholder="claude" onChange={(e) => patch({ binaryPath: e.target.value || null })} />
          <Button className="shrink-0" onClick={run}>Check</Button>
        </div>
        {check && <small className="text-xs text-muted">{check}</small>}
      </Field>
      <fieldset className="grid gap-1.5">
        <legend className="mb-1.5 text-xs text-muted">Sign in with</legend>
        <div className="flex flex-wrap gap-x-5 gap-y-1.5 text-[13px]">
          <label className="flex cursor-pointer items-center gap-2">
            <input type="radio" name="claude-auth" className="accent-accent" checked={p.auth === "login"} onChange={() => patch({ auth: "login" })} />
            My Claude Code login
          </label>
          <label className="flex cursor-pointer items-center gap-2">
            <input type="radio" name="claude-auth" className="accent-accent" checked={p.auth === "apiKey"} onChange={() => patch({ auth: "apiKey" })} />
            My Anthropic API key
          </label>
        </div>
      </fieldset>
      {p.auth === "apiKey" && <ApiKeyField note="Usage is billed to your API account, not your Claude subscription." />}
      <div className="grid grid-cols-2 gap-3">
        <div className="grid content-start gap-2">
          <ModelPicker label="While you type" value={p.fastModel} onChange={(fastModel) => patch({ fastModel })} />
          <EffortSelect label="Effort while you type" value={p.fastEffort} onChange={(fastEffort) => patch({ fastEffort })} />
        </div>
        <div className="grid content-start gap-2">
          <ModelPicker label="Project brief" value={p.deepModel} onChange={(deepModel) => patch({ deepModel })} />
          <EffortSelect label="Effort for the brief" value={p.deepEffort} onChange={(deepEffort) => patch({ deepEffort })} />
        </div>
      </div>
    </div>
  );
}

const MODEL_OPTIONS = [
  ...CLAUDE_ALIASES.map((o) => ({ value: o, label: o[0].toUpperCase() + o.slice(1) })),
  { value: "custom", label: "Custom…" },
];
const EFFORT_SELECT = EFFORT_OPTIONS.map((o) => ({ value: o.value ?? "", label: o.label }));

/** An alias, or "Custom…" with a text field for a full model id. */
function ModelPicker(props: { label: string; value: string; onChange: (v: string) => void }) {
  const [custom, setCustom] = useState(isCustomModel(props.value));
  return (
    <Field label={props.label}>
      <Select
        label={props.label}
        value={custom ? "custom" : props.value}
        options={MODEL_OPTIONS}
        onChange={(v) => {
          setCustom(v === "custom");
          props.onChange(v === "custom" ? "" : v);
        }}
      />
      {custom && (
        <input
          className={inputClass}
          value={props.value}
          placeholder="claude-opus-5-5"
          aria-label={`${props.label}: model id`}
          onChange={(e) => props.onChange(e.target.value)}
        />
      )}
    </Field>
  );
}

function EffortSelect(props: { label: string; value: Effort | null; onChange: (v: Effort | null) => void }) {
  return (
    <Field label={props.label}>
      <Select
        label={props.label}
        value={props.value ?? ""}
        options={EFFORT_SELECT}
        onChange={(v) => props.onChange((v || null) as Effort | null)}
      />
    </Field>
  );
}

/** The Anthropic API key in the system keychain; shared by both Claude providers. */
function ApiKeyField({ note: extra }: { note?: string }) {
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [key, setKey] = useState("");
  const [note, setNote] = useState<string | null>(null);
  useEffect(() => void api.hasApiKey().then(setHasKey), []);

  const saveKey = async () => {
    try {
      await api.setApiKey(key);
      setHasKey(!!key.trim());
      setKey("");
      setNote(key.trim() ? "Key saved to your system keychain." : "Key removed.");
    } catch (e) {
      setNote(errorMessage(e));
    }
  };

  return (
    <Field label={`Anthropic API key${hasKey ? " (saved in your keychain)" : ""}`}>
      <div className="flex gap-2">
        <input
          type="password"
          className={inputClass}
          value={key}
          placeholder={hasKey ? "Paste a new key to replace it" : "sk-ant-…"}
          onChange={(e) => setKey(e.target.value)}
        />
        <Button className="shrink-0" onClick={saveKey} disabled={!key.trim() && !hasKey}>
          {key.trim() || !hasKey ? "Save key" : "Remove key"}
        </Button>
      </div>
      {note ? <small className="text-xs text-muted">{note}</small> : hasKey === false && <small className="text-xs text-muted">No key saved yet.</small>}
      {extra && <small className="text-xs text-muted">{extra}</small>}
    </Field>
  );
}

function ApiFields({ p, patch }: { p: Extract<ProviderSettings, { kind: "anthropicApi" }>; patch: (f: Partial<ProviderSettings>) => void }) {
  return (
    <div className="grid gap-3">
      <ApiKeyField />
      <div className="grid grid-cols-2 gap-3">
        <div className="grid content-start gap-2">
          <Field label="While you type">
            <input className={inputClass} value={p.fastModel} onChange={(e) => patch({ fastModel: e.target.value })} />
          </Field>
          <EffortSelect label="Effort while you type" value={p.fastEffort} onChange={(fastEffort) => patch({ fastEffort })} />
        </div>
        <div className="grid content-start gap-2">
          <Field label="Project brief">
            <input className={inputClass} value={p.deepModel} onChange={(e) => patch({ deepModel: e.target.value })} />
          </Field>
          <EffortSelect label="Effort for the brief" value={p.deepEffort} onChange={(deepEffort) => patch({ deepEffort })} />
        </div>
      </div>
    </div>
  );
}

function LocalFields({ modelId, onPick }: { modelId: string; onPick: (id: string) => void }) {
  const [status, setStatus] = useState<LocalStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const downloads = useDownloads();
  const refresh = () => api.localStatus().then(setStatus, (e) => setError(errorMessage(e)));
  useEffect(() => void refresh(), []);

  if (error) return <p className="text-[13px] text-fault">{error}</p>;
  if (!status) return <p className="text-xs text-muted">Checking local models…</p>;
  if (!status.platformSupported) return <p className="text-[13px] text-fault">Local models aren't available on this platform yet.</p>;

  return (
    <div className="grid gap-2">
      <div className="rounded-lg border border-line px-3 py-2.5">
        <DownloadRow
          id="runtime"
          title="Local engine (llama.cpp)"
          detail={`Build ${status.runtimeBuild}, about 20 to 60 MB`}
          installed={status.runtimeInstalled}
          busy={status.downloading.includes("runtime")}
          state={downloads.runtime}
          actionLabel="Install engine"
          onDone={refresh}
        />
      </div>
      {status.models.map((m) => (
        <div key={m.id} className={cx("grid gap-2 rounded-lg border px-3 py-2.5", modelId === m.id ? "border-accent" : "border-line")}>
          <label className="flex cursor-pointer items-start gap-2.5">
            <input type="radio" name="local-model" className="mt-0.5 accent-accent" checked={modelId === m.id} onChange={() => onPick(m.id)} />
            <span className="grid gap-0.5">
              <span className="text-[13px]">
                <strong className="font-semibold">{m.name}</strong>
                {status.runningModel === m.id && <em className="ml-1.5 text-xs not-italic text-accent">running</em>}
              </span>
              <small className="text-xs text-muted">{m.note}</small>
            </span>
          </label>
          <DownloadRow
            id={m.id}
            detail={`About ${m.approxSizeGb} GB`}
            installed={m.downloaded}
            busy={status.downloading.includes(m.id)}
            state={downloads[m.id]}
            actionLabel="Download"
            onDone={refresh}
          />
        </div>
      ))}
      <p className="text-xs text-muted">LiveBrainstorm starts the selected model automatically when you save. Downloads resume if interrupted.</p>
    </div>
  );
}

function DownloadRow(props: {
  id: string;
  title?: string;
  detail: string;
  installed: boolean;
  busy: boolean;
  state?: ReturnType<typeof useDownloads>[string];
  actionLabel: string;
  onDone: () => void;
}) {
  const s = props.state;
  const active = (s && !s.finished) || (props.busy && !s);
  const pct = s?.total ? Math.floor((s.downloaded / s.total) * 100) : null;
  return (
    <div className="flex flex-wrap items-center gap-2.5 text-xs">
      {props.title && <strong className="text-[13px] font-semibold">{props.title}</strong>}
      <small className="text-muted">{props.detail}</small>
      {props.installed && !active ? (
        <span className="ml-auto text-accent">Ready</span>
      ) : active ? (
        <>
          <progress className="min-w-30 flex-1 accent-accent" max={100} value={pct ?? undefined} aria-label={`${props.id} download`} />
          <small className="text-muted tabular-nums">
            {s ? `${formatBytes(s.downloaded)}${s.total ? ` of ${formatBytes(s.total)}` : ""}` : "Downloading…"}
          </small>
          <Button variant="ghost" onClick={() => pauseDownload(props.id)}>
            Pause
          </Button>
        </>
      ) : (
        <Button className="ml-auto" onClick={() => startDownload(props.id, props.onDone)}>
          {s?.downloaded ? "Resume" : props.actionLabel}
        </Button>
      )}
      {s?.error && <small className="basis-full text-fault">{s.error}</small>}
    </div>
  );
}

function AgentFields({ settings, update }: { settings: Settings; update: (a: Partial<AgentSettings>) => void }) {
  const a = settings.agents;
  const kind = settings.provider.kind;
  const local = kind === "local";
  const shown = local ? 1 : clampAgents(a.maxAgents);
  return (
    <div className="grid gap-3">
      <Field label={`Agents at once: ${shown}`}>
        <input
          type="range"
          className="w-full accent-accent"
          min={1}
          max={8}
          step={1}
          value={shown}
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
          <Button className="shrink-0" onClick={saveKey} disabled={!key.trim() && !inKeychain}>
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
