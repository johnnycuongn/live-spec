import { useState } from "react";
import type { TabSummary } from "../lib/ipc";
import { groupKey, groupTabs } from "../lib/tabList";
import { tabs } from "../lib/tabs";
import { tabTitle } from "../lib/tabTitle";
import { Button, cx } from "./ui";

/**
 * "Recently closed (n)": shared by every window, newest first, grouped by
 * project. Delete asks inline ("Delete for good?"), never with confirm().
 */
export function ClosedTabs({ closed }: { closed: TabSummary[] }) {
  const [open, setOpen] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  if (closed.length === 0) return null;

  return (
    <section className="mt-4" aria-label="Recently closed">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex w-full cursor-pointer items-center gap-1.5 rounded-md px-2 py-1 text-left text-xs text-muted hover:text-ink"
      >
        <span aria-hidden className={cx("text-faint transition-transform motion-reduce:transition-none", open && "rotate-90")}>
          ▸
        </span>
        Recently closed ({closed.length})
      </button>
      {open &&
        groupTabs(closed).map((g) => (
          <div key={groupKey(g.root)} className="mt-2 grid gap-0.5">
            <p className="truncate px-2 text-xs font-medium text-faint" title={g.root ?? undefined}>
              {g.name}
            </p>
            <ul className="grid gap-0.5">
              {g.tabs.map((t) => (
                <li key={t.id} className="flex items-center gap-1 px-2">
                  {deleting === t.id ? (
                    <>
                      <span className="min-w-0 flex-1 truncate text-xs text-ink">Delete for good?</span>
                      <Button
                        variant="mini"
                        onClick={() => {
                          setDeleting(null);
                          void tabs.remove(t.id);
                        }}
                      >
                        Yes
                      </Button>
                      <Button variant="mini" onClick={() => setDeleting(null)}>
                        No
                      </Button>
                    </>
                  ) : (
                    <>
                      <span className="min-w-0 flex-1 truncate text-xs text-muted" title={tabTitle(t)}>
                        {tabTitle(t)}
                      </span>
                      <Button variant="mini" onClick={() => void tabs.reopen(t.id)}>
                        Reopen
                      </Button>
                      <Button variant="mini" onClick={() => setDeleting(t.id)}>
                        Delete
                      </Button>
                    </>
                  )}
                </li>
              ))}
            </ul>
          </div>
        ))}
    </section>
  );
}
