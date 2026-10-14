import { useRef, useState, type MouseEvent } from "react";
import type { TabSummary } from "../lib/ipc";
import { renameResult, rowTitle } from "../lib/tabList";
import { sessionFor, tabs } from "../lib/tabs";
import { ruleState, statusLine } from "../lib/tabStatus";
import { useTabSession } from "../lib/useTabSession";
import { Menu, RULE, cx, inputClass } from "./ui";

/**
 * One tab: a 2 × 28 px rule in the same states as the rule between the panes,
 * the title, and a status line. Double-click or F2 renames; right-click (or
 * the context-menu key) opens Rename and Close.
 */
export function TabRow({ tab, active }: { tab: TabSummary; active: boolean }) {
  // Null until the tab is first shown: the rule is faint and there's no status line.
  const view = useTabSession(sessionFor(tab.id) ?? null);
  const [editing, setEditing] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const cancelled = useRef(false);
  const live = view?.draft ?? null;
  const title = rowTitle(tab, live);
  const auto = rowTitle({ ...tab, title: null }, live);
  const status = statusLine(view);

  const startRename = () => {
    cancelled.current = false;
    setEditing(title);
  };
  const finishRename = () => {
    const input = editing;
    setEditing(null);
    if (cancelled.current || input === null) return;
    const next = renameResult(input, tab.title, auto);
    if (next !== undefined) void tabs.rename(tab.id, next);
  };
  const openMenu = (e: MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    // The context-menu key reports 0,0: open under the row instead.
    const r = e.currentTarget.getBoundingClientRect();
    setMenu(e.clientX === 0 && e.clientY === 0 ? { x: r.left + 12, y: r.bottom } : { x: e.clientX, y: e.clientY });
  };

  const rule = (
    <span className="relative h-7 w-0.5 shrink-0 bg-line" aria-hidden>
      <span className={cx("absolute inset-0 origin-top", RULE[ruleState(view)])} />
    </span>
  );

  return (
    <li>
      {editing !== null ? (
        <div className="flex items-center gap-2.5 rounded-md bg-subtle px-2 py-1">
          {rule}
          <input
            autoFocus
            aria-label="Tab name"
            className={inputClass}
            value={editing}
            onChange={(e) => setEditing(e.target.value)}
            onFocus={(e) => e.currentTarget.select()}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              else if (e.key === "Escape") {
                e.stopPropagation(); // Esc cancels the rename; it doesn't close a drawer
                cancelled.current = true;
                e.currentTarget.blur();
              }
            }}
            onBlur={finishRename}
          />
        </div>
      ) : (
        <button
          type="button"
          aria-current={active ? "true" : undefined}
          className={cx("flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-1 text-left", active ? "bg-subtle" : "hover:bg-subtle")}
          onClick={() => void tabs.switchTo(tab.id)}
          onDoubleClick={startRename}
          onKeyDown={(e) => {
            if (e.key === "F2") startRename();
          }}
          onContextMenu={openMenu}
        >
          {rule}
          <span className="grid min-w-0 flex-1">
            <span className="truncate text-[13px] text-ink">{title}</span>
            {status && <span className={cx("truncate text-xs", status.fault ? "text-fault" : "text-muted")}>{status.text}</span>}
          </span>
        </button>
      )}
      {menu && (
        <Menu
          at={menu}
          label={`${title}: tab actions`}
          onClose={() => setMenu(null)}
          items={[
            { label: "Rename", onSelect: startRename },
            { label: "Close", onSelect: () => void tabs.close(tab.id) },
          ]}
        />
      )}
    </li>
  );
}
