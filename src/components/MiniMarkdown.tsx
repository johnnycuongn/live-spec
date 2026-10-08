// Renders the small Markdown subset models produce here: headings, bullets,
// numbered items, `code` and **bold**. React elements only, no innerHTML,
// so model output can never inject markup.
import type { ReactNode } from "react";
import { cx } from "./ui";

function inline(text: string): ReactNode[] {
  return text.split(/(`[^`]+`|\*\*[^*]+\*\*)/g).map((part, i) => {
    if (part.startsWith("`") && part.endsWith("`") && part.length > 1)
      return (
        <code key={i} className="rounded bg-subtle px-1 py-px font-mono text-[0.85em]">
          {part.slice(1, -1)}
        </code>
      );
    if (part.startsWith("**") && part.endsWith("**") && part.length > 3)
      return (
        <strong key={i} className="font-semibold">
          {part.slice(2, -2)}
        </strong>
      );
    return part;
  });
}

/** `className` replaces the default text size (e.g. the brief drawer reads larger). */
export function MiniMarkdown({ text, className }: { text: string; className?: string }) {
  const blocks: ReactNode[] = [];
  let items: ReactNode[] = [];
  const flush = () => {
    if (items.length)
      blocks.push(
        <ul key={`ul${blocks.length}`} className="mb-1.5 list-disc space-y-1 pl-4 marker:text-faint">
          {items}
        </ul>,
      );
    items = [];
  };

  text.split("\n").forEach((raw, i) => {
    const line = raw.trimEnd();
    const bullet = /^\s*(?:[-*•]|\d+[.)])\s+(.*)$/.exec(line);
    const heading = /^#{1,4}\s+(.*)$/.exec(line);
    if (bullet) items.push(<li key={i}>{inline(bullet[1])}</li>);
    else {
      flush();
      if (heading)
        blocks.push(
          <h3 key={i} className="mt-4 mb-1 font-semibold first:mt-0">
            {inline(heading[1])}
          </h3>,
        );
      else if (line.trim())
        blocks.push(
          <p key={i} className="mb-1.5">
            {inline(line)}
          </p>,
        );
    }
  });
  flush();
  return <div className={cx("leading-relaxed", className ?? "text-[13.5px]")}>{blocks}</div>;
}
