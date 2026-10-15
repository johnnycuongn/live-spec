import { useEffect, useState } from "react";
import { copyText } from "../lib/clipboard";
import { Button, cx } from "./ui";

interface Props {
  value: string;
  onChange: (v: string) => void;
  hasProject: boolean;
}

// Centres a ~40rem (about 68 characters) column inside the full-width pane, so
// the whole pane is the click and scroll target but lines stay readable.
// Percent padding resolves against the pane, so textarea and footer line up.
const GUTTER = "px-[max(2rem,calc((100%_-_40rem)/2))]";

const COPY_FAILED = "Couldn't copy. Select the text and copy it instead.";

export function DraftPane({ value, onChange, hasProject }: Props) {
  const words = value.trim() ? value.trim().split(/\s+/).length : 0;
  const [copy, setCopy] = useState<"idle" | "copied" | "failed">("idle");

  // "Copied" and the failure note clear themselves.
  useEffect(() => {
    if (copy === "idle") return;
    const t = setTimeout(() => setCopy("idle"), 2000);
    return () => clearTimeout(t);
  }, [copy]);

  const copySpec = () => {
    void copyText(value).then((ok) => setCopy(ok ? "copied" : "failed"));
  };

  return (
    <section className="flex min-h-0 flex-col bg-page" aria-label="Your spec">
      <textarea
        className={cx(
          "min-h-0 w-full flex-1 resize-none overflow-y-auto bg-transparent pt-10 pb-6 font-serif text-[17px] leading-[1.75] outline-none focus-visible:outline-none placeholder:text-faint",
          GUTTER,
        )}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        spellCheck
        autoFocus
        placeholder={
          hasProject
            ? "Write the spec here. Live Spec reads along and replies when you pause."
            : "Open the project the spec is for, so Live Spec can check it against the code, or just start writing."
        }
      />
      <footer className={cx("flex shrink-0 items-center gap-3 pt-1 pb-3 text-xs tabular-nums text-faint", GUTTER)}>
        <span>{words} words</span>
        <Button variant="link" onClick={copySpec} disabled={!value.trim()} title="Copy the spec to the clipboard">
          Copy spec
        </Button>
        {copy === "copied" && <span aria-live="polite">Copied</span>}
        {copy === "failed" && (
          <span className="text-fault" role="alert">
            {COPY_FAILED}
          </span>
        )}
      </footer>
    </section>
  );
}
