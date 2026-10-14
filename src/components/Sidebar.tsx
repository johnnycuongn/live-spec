import { useCallback, useEffect, useState, useSyncExternalStore, type ReactNode } from "react";
import type { TabSummary } from "../lib/ipc";
import { groupKey, groupTabs, unreadableNotice, type TabGroup } from "../lib/tabList";
import { sessionFor, tabs, useWorkspace, type WorkspaceState } from "../lib/tabs";
import { anyThinking } from "../lib/tabStatus";
import { ClosedTabs } from "./ClosedTabs";
import { TabRow } from "./TabRow";
import { Button, Menu, cx } from "./ui";

interface Props {
  /** Collapsed with ⌘\: kept mounted, so folded groups and dismissed notices survive. */
  hidden: boolean;
  opening: boolean;
  providerLabel: string;
  /** Rendered above the provider label. Phase 4 passes the usage line. */
  footer?: ReactNode;
  onOpenProject: () => void;
  onShowBrief: (root: string) => void;
  onBuildBrief: (root: string) => void;
  onOpenSettings: () => void;
}

/** This window's tabs, grouped by project in order of first appearance, then Recently closed. */
export function Sidebar(p: Props) {
  const ws = useWorkspace();
  const [folded, setFolded] = useState<string[]>([]);
  const [dismissed, setDismissed] = useState<string[]>([]);
  const groups = groupTabs(ws.view?.tabs ?? []);
  const notices = (ws.view?.unreadable ?? []).filter((path) => !dismissed.includes(path));

  // Showing a tab in a folded group (⌃Tab, reopening it) unfolds the group.
  const activeRoot = ws.view?.tabs.find((t) => t.id === ws.activeId)?.root;
  useEffect(() => {
    if (activeRoot === undefined) return;
    const k = groupKey(activeRoot);
    setFolded((f) => (f.includes(k) ? f.filter((x) => x !== k) : f));
  }, [ws.activeId, activeRoot]);

  const toggle = (k: string) => setFolded((f) => (f.includes(k) ? f.filter((x) => x !== k) : [...f, k]));

  return (
    <nav aria-label="Tabs" className={cx(p.hidden ? "hidden" : "flex", "min-h-0 flex-col border-r border-line bg-panel")}>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-3">
        {ws.view && groups.length === 0 && <p className="px-2 pb-2 text-xs text-faint">No tabs in this window.</p>}
        {groups.map((g) => (
          <Group
            key={groupKey(g.root)}
            group={g}
            ws={ws}
            folded={folded.includes(groupKey(g.root))}
            onFold={() => toggle(groupKey(g.root))}
            onShowBrief={p.onShowBrief}
            onBuildBrief={p.onBuildBrief}
          />
        ))}
        <Button variant="ghost" className="mt-1 w-full" onClick={p.onOpenProject} disabled={p.opening}>
          {p.opening ? "Reading project…" : "Open a project…"}
        </Button>
        <ClosedTabs closed={ws.view?.closed ?? []} />
      </div>

      <footer className="grid shrink-0 gap-1.5 border-t border-line px-2 py-2">
        {notices.map((path) => (
          <p key={path} className="flex items-start gap-2 px-2 text-xs text-muted" role="status">
            <span className="min-w-0 flex-1 break-words" title={path}>
              {unreadableNotice(path)}
            </span>
            <Button variant="mini" onClick={() => setDismissed((d) => [...d, path])}>
              OK
            </Button>
          </p>
        ))}
        {ws.saveError && (
          <p className="px-2 text-xs text-fault" role="alert">
            {ws.saveError}
          </p>
        )}
        {ws.error && (
          <p className="px-2 text-xs text-fault" role="alert">
            {ws.error}
          </p>
        )}
        {p.footer}
        <Button variant="ghost" className="w-full min-w-0" onClick={p.onOpenSettings} title="Model and settings">
          <span className="truncate">{p.providerLabel}</span>
        </Button>
      </footer>
    </nav>
  );
}

interface GroupProps {
  group: TabGroup<TabSummary>;
  ws: WorkspaceState;
  folded: boolean;
  onFold: () => void;
  onShowBrief: (root: string) => void;
  onBuildBrief: (root: string) => void;
}

/** A project's header (name, ⋯ menu, +) and its tabs. "No project" has no ⋯ menu. */
function Group({ group, ws, folded, onFold, onShowBrief, onBuildBrief }: GroupProps) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const root = group.root;
  return (
    <section className="mb-3" aria-label={group.name}>
      <div className="flex items-center gap-0.5">
        <button
          type="button"
          aria-expanded={!folded}
          onClick={onFold}
          title={root ?? undefined}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 rounded-md px-2 py-1 text-left text-xs font-semibold text-muted hover:text-ink"
        >
          <span aria-hidden className={cx("text-faint transition-transform motion-reduce:transition-none", !folded && "rotate-90")}>
            ▸
          </span>
          <span className="truncate">{group.name}</span>
          {folded && <FoldedDot ids={group.tabs.map((t) => t.id)} ws={ws} />}
        </button>
        {root !== null && (
          <Button
            variant="mini"
            aria-label={`${group.name}: project actions`}
            aria-haspopup="menu"
            aria-expanded={menu !== null}
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              setMenu(menu ? null : { x: r.left, y: r.bottom + 4 });
            }}
          >
            ⋯
          </Button>
        )}
        <Button variant="mini" aria-label={`New tab in ${group.name}`} title="New tab" onClick={() => void tabs.newTab(root)}>
          +
        </Button>
      </div>
      {!folded && (
        <ul className="mt-0.5 grid gap-0.5">
          {group.tabs.map((t) => (
            <TabRow key={t.id} tab={t} active={t.id === ws.activeId} />
          ))}
        </ul>
      )}
      {menu && root !== null && (
        <Menu
          at={menu}
          label={`${group.name}: project actions`}
          onClose={() => setMenu(null)}
          items={[
            { label: "Project brief…", onSelect: () => onShowBrief(root) },
            { label: "Rebuild brief", onSelect: () => onBuildBrief(root) },
          ]}
        />
      )}
    </section>
  );
}

/** A folded group pulses while any of its tabs is analysing; the hidden text says so to screen readers. */
function FoldedDot({ ids, ws }: { ids: string[]; ws: WorkspaceState }) {
  const key = ids.join(" ");
  // Re-subscribes when the workspace changes, which is when sessions are created.
  const subscribe = useCallback(
    (fn: () => void) => {
      const offs = key.split(" ").map((id) => sessionFor(id)?.subscribe(fn));
      return () => offs.forEach((off) => off?.());
    },
    [key, ws],
  );
  const thinking = useSyncExternalStore(subscribe, () => anyThinking(key.split(" ").map((id) => sessionFor(id)?.getState())));
  if (!thinking) return null;
  return (
    <span className="ml-auto size-1.5 shrink-0 rounded-full bg-accent animate-pulse-dot motion-reduce:animate-none" title="Analysing…">
      <span className="sr-only">Analysing…</span>
    </span>
  );
}
