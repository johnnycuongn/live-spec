import { useState } from "react";
import type { Template } from "../lib/ipc";
import { removeTemplate, upsertTemplate } from "../lib/instructions";
import { Button, Eyebrow, Field, cx, inputClass } from "./ui";

interface Props {
  templates: Template[];
  /** The current instructions: a new template starts from them. */
  current: string;
  onSave: (next: Template[]) => void;
  onRestore: () => void;
}

/** Add, edit, delete and restore templates. Confirmations are inline, never browser dialogs. */
export function TemplateList({ templates, current, onSave, onRestore }: Props) {
  const [editing, setEditing] = useState<Template | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const isNew = editing !== null && !templates.some((t) => t.id === editing.id);

  const commit = () => {
    if (!editing?.name.trim()) return;
    onSave(upsertTemplate(templates, { ...editing, name: editing.name.trim() }));
    setEditing(null);
  };
  const editor = editing && <TemplateEditor value={editing} onChange={setEditing} onSave={commit} onCancel={() => setEditing(null)} />;

  return (
    <section className="grid gap-3 border-t border-line pt-5" aria-label="Templates">
      <div className="flex items-center justify-between">
        <h3>
          <Eyebrow>Templates</Eyebrow>
        </h3>
        <Button variant="ghost" onClick={() => setEditing({ id: crypto.randomUUID(), name: "", text: current })} disabled={editing !== null}>
          + New template
        </Button>
      </div>
      {isNew && editor}
      <ul className="grid gap-1">
        {templates.map((t) =>
          editing?.id === t.id ? (
            <li key={t.id}>{editor}</li>
          ) : deleting === t.id ? (
            <li key={t.id} className="flex items-center gap-2 text-[13px]">
              <span className="min-w-0 flex-1 truncate">Delete {t.name}?</span>
              <Button
                onClick={() => {
                  onSave(removeTemplate(templates, t.id));
                  setDeleting(null);
                }}
              >
                Yes
              </Button>
              <Button variant="ghost" onClick={() => setDeleting(null)}>
                No
              </Button>
            </li>
          ) : (
            <li key={t.id} className="flex items-center gap-2 text-[13px]">
              <span className="min-w-0 flex-1 truncate">{t.name}</span>
              <Button variant="ghost" onClick={() => setEditing(t)} disabled={editing !== null}>
                Edit
              </Button>
              <Button variant="ghost" onClick={() => setDeleting(t.id)}>
                Delete
              </Button>
            </li>
          ),
        )}
      </ul>
      {templates.length === 0 && <p className="text-xs text-muted">No templates.</p>}
      <div>
        <Button variant="link" onClick={onRestore}>
          Restore starter templates
        </Button>
      </div>
    </section>
  );
}

function TemplateEditor(props: { value: Template; onChange: (t: Template) => void; onSave: () => void; onCancel: () => void }) {
  const v = props.value;
  return (
    <div className="grid gap-2 rounded-lg border border-line p-3">
      <Field label="Name">
        <input className={inputClass} value={v.name} onChange={(e) => props.onChange({ ...v, name: e.target.value })} autoFocus />
      </Field>
      <Field label="Instructions">
        <textarea
          className={cx(inputClass, "min-h-32 resize-y leading-relaxed")}
          value={v.text}
          onChange={(e) => props.onChange({ ...v, text: e.target.value })}
        />
      </Field>
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={props.onCancel}>
          Cancel
        </Button>
        <Button variant="primary" onClick={props.onSave} disabled={!v.name.trim()}>
          Save template
        </Button>
      </div>
    </div>
  );
}
