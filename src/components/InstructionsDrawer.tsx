import { useState, type ReactNode } from "react";
import type { ProjectInstructions, Template } from "../lib/ipc";
import { applyTemplate, charCount, clearInstructions, counterText, firstLine, resetToDefault, saveAsDefault } from "../lib/instructions";
import { Button, Field, Select, cx, inputClass } from "./ui";

interface Props {
  instructions: ProjectInstructions;
  /** False until this project's instructions have loaded: nothing here can be edited before then. */
  loaded: boolean;
  templates: Template[];
  localModel: boolean;
  error: string | null;
  /** Tries loading the instructions again (the error says they couldn't load). */
  onRetry: () => void;
  onChange: (next: ProjectInstructions) => void;
  onClose: () => void;
  /** The templates section (TemplateList). */
  children?: ReactNode;
}

/** Same shell as BriefDrawer. App keys it by project root, so Undo never crosses projects. */
export function InstructionsDrawer(p: Props) {
  const [picked, setPicked] = useState("");
  const [undo, setUndo] = useState<ProjectInstructions | null>(null);
  const i = p.instructions;
  const pickedId = p.templates.some((t) => t.id === picked) ? picked : (p.templates[0]?.id ?? "");
  const counter = counterText(charCount(i.current));

  const change = (next: ProjectInstructions) => {
    setUndo(null);
    p.onChange(next);
  };
  const apply = () => {
    const t = p.templates.find((x) => x.id === pickedId);
    if (!t) return;
    const { next, undo } = applyTemplate(i, t);
    p.onChange(next);
    setUndo(undo);
  };

  return (
    <div
      className="absolute inset-y-0 right-0 flex w-[min(560px,100%)] flex-col border-l border-line bg-page shadow-[-12px_0_32px_rgb(0_0_0/0.08)]"
      role="dialog"
      aria-label="Session instructions"
    >
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-line px-5">
        <h2 className="flex-1 text-[15px] font-semibold">Session instructions</h2>
        <Button variant="ghost" onClick={p.onClose} aria-label="Close instructions">
          Close
        </Button>
      </header>

      <div className="grid min-h-0 flex-1 content-start gap-5 overflow-y-auto px-6 pt-4 pb-10">
        <p className="text-xs text-muted">These steer the notes for this project: what LiveBrainstorm looks for, not the four sections.</p>

        {/* Disabled until this project's instructions load: typing earlier would save over the stored ones. */}
        <fieldset disabled={!p.loaded} className="contents">
        {p.templates.length > 0 && (
          <div className="flex items-end gap-2">
            <Field label="Template">
              <Select
                label="Template"
                value={pickedId}
                options={p.templates.map((t) => ({ value: t.id, label: t.name }))}
                onChange={setPicked}
              />
            </Field>
            <Button className="shrink-0" onClick={apply}>
              Apply
            </Button>
          </div>
        )}
        {undo && (
          <p className="flex items-center gap-2 text-xs text-muted">
            Template applied.
            <Button
              variant="link"
              onClick={() => {
                p.onChange(undo);
                setUndo(null);
              }}
            >
              Undo
            </Button>
          </p>
        )}

        <Field label="Instructions">
          <textarea
            className={cx(inputClass, "min-h-48 resize-y leading-relaxed")}
            value={i.current}
            placeholder="For example: focus Faults on error handling and race conditions."
            onChange={(e) => change({ ...i, current: e.target.value })}
          />
          <small className={cx("text-xs tabular-nums", counter.over ? "text-fault" : "text-faint")}>{counter.text}</small>
          {p.localModel && <small className="text-xs text-muted">Local models read fewer characters, depending on the model.</small>}
        </Field>

        <div className="grid gap-2">
          <div className="flex flex-wrap gap-2">
            <Button onClick={() => change(clearInstructions(i))} disabled={!i.current}>
              Clear
            </Button>
            <Button onClick={() => change(resetToDefault(i))} disabled={i.current === i.default}>
              Reset to default
            </Button>
            <Button onClick={() => change(saveAsDefault(i))} disabled={i.current === i.default}>
              Save as default
            </Button>
          </div>
          <p className="truncate text-xs text-muted">{i.default.trim() ? `Default: ${firstLine(i.default)}` : "No default for this project"}</p>
        </div>
        </fieldset>

        {p.error && (
          <p className="flex items-center gap-2 text-[13px] text-fault" role="alert">
            {p.error}
            {!p.loaded && (
              <Button variant="link" onClick={p.onRetry}>
                Try again
              </Button>
            )}
          </p>
        )}
        {p.children}
      </div>
    </div>
  );
}
