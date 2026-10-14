import { useEffect, useState } from "react";
import { api, type UsageBucket } from "../lib/ipc";
import { onUsageChanged } from "../lib/events";
import { footerText, sinceHourFor, todayTotals } from "../lib/usage";
import { Button } from "./ui";

/** "Today · 182k tokens · ~$0.41" in the sidebar footer. Refreshes after each
 *  recorded run (`usage-changed`, in every window) and once a minute, so the
 *  day turns over at local midnight. Opens the usage panel. */
export function UsageFooter({ onOpen }: { onOpen: () => void }) {
  const [view, setView] = useState<{ buckets: UsageBucket[]; at: number } | null>(null);

  useEffect(() => {
    let alive = true;
    const load = () => {
      const at = Date.now();
      // A failed read leaves the last line in place; the ledger never blocks the app.
      api.usageReport(sinceHourFor(at, 1)).then((buckets) => alive && setView({ buckets, at }), () => {});
    };
    load();
    const unlisten = onUsageChanged(load);
    const timer = window.setInterval(load, 60_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
      void unlisten.then((stop) => stop());
    };
  }, []);

  if (!view) return null;
  return (
    <Button variant="ghost" className="w-full justify-start tabular-nums" onClick={onOpen} title="Usage for the last 30 days">
      {footerText(todayTotals(view.buckets, view.at))}
    </Button>
  );
}
