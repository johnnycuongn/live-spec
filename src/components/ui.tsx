// Shared building blocks, so components don't repeat long class lists.
import { useEffect, useRef, type ButtonHTMLAttributes, type ReactNode } from "react";

export const cx = (...parts: (string | false | null | undefined)[]) => parts.filter(Boolean).join(" ");

/** The accent rule's states: solid when notes are current, animated while thinking, dashed when the draft has moved on. */
export type RuleState = "idle" | "thinking" | "stale" | "fresh";

/** State classes for the accent rule, shared by the rule between the panes and each tab row's rule. */
export const RULE: Record<RuleState, string> = {
  idle: "bg-accent scale-y-0",
  fresh: "bg-accent",
  thinking: "bg-accent animate-fill motion-reduce:animate-none motion-reduce:opacity-50",
  // Dashed: the notes are still there, but your draft has moved on.
  stale: "bg-[repeating-linear-gradient(to_bottom,var(--color-accent)_0_6px,transparent_6px_12px)] opacity-70",
};

type Variant = "default" | "primary" | "ghost" | "link" | "mini";

const VARIANTS: Record<Variant, string> = {
  default: "rounded-md border border-line bg-page px-3 py-1.5 text-[13px] hover:border-faint",
  // text-page, not white: the dark-mode accent is light enough to need dark text.
  primary: "rounded-md border border-accent bg-accent px-3 py-1.5 text-[13px] font-medium text-page hover:opacity-90",
  ghost: "rounded-md px-2.5 py-1.5 text-[13px] text-muted hover:bg-subtle hover:text-ink",
  link: "text-[13px] text-accent underline underline-offset-2",
  // Compact, for sidebar rows and group headers.
  mini: "rounded px-1.5 py-0.5 text-xs text-muted hover:bg-subtle hover:text-ink",
};

export function Button({ variant = "default", className, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return (
    <button
      type="button"
      className={cx(
        "inline-flex cursor-pointer items-center gap-2 transition-colors disabled:cursor-default disabled:opacity-50",
        VARIANTS[variant],
        className,
      )}
      {...props}
    />
  );
}

export const inputClass =
  "w-full rounded-md border border-line bg-page px-2.5 py-1.5 text-[13px] text-ink placeholder:text-faint";

export function Field({ label, children, className }: { label: ReactNode; children: ReactNode; className?: string }) {
  return (
    <label className={cx("grid flex-1 gap-1.5 text-xs text-muted", className)}>
      <span>{label}</span>
      {children}
    </label>
  );
}

/** Small uppercase heading used for note sections and settings groups. */
export function Eyebrow({ children, dot }: { children: ReactNode; dot?: string }) {
  return (
    <span className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-muted">
      {dot && <span className={cx("size-1.5 rounded-full", dot)} aria-hidden />}
      {children}
    </span>
  );
}

/** An on/off switch. A real checkbox underneath, so keyboard and screen readers work. */
export function Toggle(props: { checked: boolean; onChange: (on: boolean) => void; label: string; hint?: string; disabled?: boolean }) {
  return (
    <label className={cx("flex items-start gap-3", props.disabled ? "opacity-50" : "cursor-pointer")}>
      <input
        type="checkbox"
        role="switch"
        className="peer sr-only"
        checked={props.checked}
        disabled={props.disabled}
        onChange={(e) => props.onChange(e.target.checked)}
      />
      <span
        aria-hidden
        className={cx(
          "relative mt-0.5 h-5 w-9 shrink-0 rounded-full transition-colors peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent",
          props.checked ? "bg-accent" : "bg-line",
        )}
      >
        <span className={cx("absolute top-0.5 left-0.5 size-4 rounded-full bg-page transition-transform", props.checked && "translate-x-4")} />
      </span>
      <span className="grid gap-0.5">
        <span className="text-[13px] text-ink">{props.label}</span>
        {props.hint && <span className="text-xs text-muted">{props.hint}</span>}
      </span>
    </label>
  );
}

export interface MenuItem {
  label: string;
  onSelect: () => void;
  disabled?: boolean;
}

/**
 * A small popup menu at a point: where you right-clicked, or under a ⋯ button.
 * Choosing an item, Esc, a click outside, or the window losing focus closes
 * it; arrow keys move between items. Its Esc never reaches the drawers' handler.
 */
export function Menu({ at, items, label, onClose }: { at: { x: number; y: number }; items: MenuItem[]; label: string; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;

  useEffect(() => {
    const node = ref.current;
    const buttons = () => [...(node?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    const opener = document.activeElement as HTMLElement | null;
    buttons()[0]?.focus();
    const outside = (e: MouseEvent) => {
      if (node?.contains(e.target as Node)) return;
      // The ⋯ button that opened this menu toggles it in its own click handler.
      if ((e.target as Element).closest?.('[aria-haspopup="menu"][aria-expanded="true"]')) return;
      close.current();
    };
    const keys = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close.current();
      } else if (e.key === "Tab") {
        close.current(); // focus moves on from where it is restored below
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const list = buttons();
        const i = list.indexOf(document.activeElement as HTMLButtonElement);
        const down = e.key === "ArrowDown";
        const next = i === -1 ? (down ? 0 : list.length - 1) : (i + (down ? 1 : -1) + list.length) % list.length;
        list[next]?.focus();
      }
    };
    const dismiss = () => close.current();
    window.addEventListener("mousedown", outside);
    window.addEventListener("keydown", keys, true);
    window.addEventListener("blur", dismiss);
    window.addEventListener("resize", dismiss);
    return () => {
      window.removeEventListener("mousedown", outside);
      window.removeEventListener("keydown", keys, true);
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("resize", dismiss);
      // Give focus back to what opened the menu, unless it moved on (Rename's input takes it).
      const now = document.activeElement;
      if ((now === null || now === document.body || node?.contains(now)) && opener?.isConnected) opener.focus();
    };
  }, []);

  // Keep it on screen near the edges.
  const style = {
    left: Math.max(4, Math.min(at.x, window.innerWidth - 224)),
    top: Math.max(4, Math.min(at.y, window.innerHeight - 12 - items.length * 32)),
  };
  return (
    <div
      ref={ref}
      role="menu"
      aria-label={label}
      style={style}
      className="fixed z-50 grid min-w-44 gap-0.5 rounded-md border border-line bg-page p-1 shadow-[0_8px_24px_rgb(0_0_0/0.12)]"
    >
      {items.map((item) => (
        <button
          key={item.label}
          type="button"
          role="menuitem"
          disabled={item.disabled}
          className="cursor-pointer rounded px-2.5 py-1.5 text-left text-[13px] text-ink hover:bg-subtle focus-visible:bg-subtle focus-visible:outline-none disabled:cursor-default disabled:opacity-50"
          onClick={() => {
            close.current();
            item.onSelect();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
