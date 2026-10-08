import { cx } from "./ui";

interface Props {
  value: string;
  onChange: (v: string) => void;
  hasProject: boolean;
}

// Centres a ~40rem (about 68 characters) column inside the full-width pane, so
// the whole pane is the click and scroll target but lines stay readable.
// Percent padding resolves against the pane, so textarea and footer line up.
const GUTTER = "px-[max(2rem,calc((100%_-_40rem)/2))]";

export function DraftPane({ value, onChange, hasProject }: Props) {
  const words = value.trim() ? value.trim().split(/\s+/).length : 0;
  return (
    <section className="flex min-h-0 flex-col bg-page" aria-label="Your draft">
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
            ? "Think out loud about a feature, a bug or a change. LiveBrainstorm reads along and replies when you pause."
            : "Open a project so LiveBrainstorm can check your ideas against the code, or just start thinking out loud."
        }
      />
      <footer className={cx("shrink-0 pt-1 pb-3 text-xs tabular-nums text-faint", GUTTER)}>{words} words</footer>
    </section>
  );
}
