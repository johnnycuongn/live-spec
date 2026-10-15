// Shared building blocks, so components don't repeat long class lists.
import { useEffect, useId, useLayoutEffect, useRef, useState, type ButtonHTMLAttributes, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { moveActive, typeahead } from "../lib/listbox";

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

export interface SelectOption<T extends string> {
  value: T;
  label: string;
}

/** Where the open list sits: under the trigger, or above it when there's more room there. */
interface Placement {
  left: number;
  width: number;
  top?: number;
  bottom?: number;
  maxHeight: number;
}

const LIST_MAX = 288;

function place(trigger: HTMLElement): Placement {
  const r = trigger.getBoundingClientRect();
  const below = window.innerHeight - r.bottom - 8;
  const above = r.top - 8;
  const up = below < 160 && above > below;
  const left = Math.max(4, Math.min(r.left, window.innerWidth - r.width - 4));
  return up
    ? { left, width: r.width, bottom: window.innerHeight - r.top + 4, maxHeight: Math.min(LIST_MAX, above) }
    : { left, width: r.width, top: r.bottom + 4, maxHeight: Math.min(LIST_MAX, below) };
}

/**
 * A dropdown that looks like the rest of the app instead of the system's.
 * A select-only combobox (WAI-ARIA APG): focus stays on the trigger, arrows,
 * Home/End and type-ahead move through the list, Enter or Space picks, Tab
 * picks and moves on, Esc closes without reaching the drawers or the
 * Settings dialog. The list is portalled into the enclosing <dialog> (else
 * <body>) so a modal dialog doesn't make it inert and a wrapping <label>
 * doesn't swallow its clicks.
 */
export function Select<T extends string>(props: {
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  /** Names the list for screen readers; the trigger takes its name from the surrounding Field. */
  label: string;
  disabled?: boolean;
  className?: string;
}) {
  const { value, options, onChange } = props;
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [at, setAt] = useState<{ place: Placement; host: Element } | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const typed = useRef({ text: "", at: 0 });
  const id = useId();
  const selected = Math.max(0, options.findIndex((o) => o.value === value));
  const current = options.find((o) => o.value === value);

  const show = (index = selected) => {
    if (!trigger.current) return;
    setAt({ place: place(trigger.current), host: trigger.current.closest("dialog") ?? document.body });
    setActive(index);
    setOpen(true);
  };
  const choose = (index: number) => {
    setOpen(false);
    const o = options[index];
    if (o && o.value !== value) onChange(o.value);
  };

  // Close on a press elsewhere, the window losing focus, a resize, or a scroll that would leave the list behind.
  useEffect(() => {
    if (!open) return;
    const outside = (e: MouseEvent) => {
      const t = e.target as Node;
      // The trigger and its Field label toggle the list in the trigger's own click handler.
      const owner = trigger.current?.closest("label") ?? trigger.current;
      if (!owner?.contains(t) && !list.current?.contains(t)) setOpen(false);
    };
    const scrolled = (e: Event) => {
      if (!list.current?.contains(e.target as Node)) setOpen(false);
    };
    const dismiss = () => setOpen(false);
    window.addEventListener("mousedown", outside);
    window.addEventListener("scroll", scrolled, true);
    window.addEventListener("blur", dismiss);
    window.addEventListener("resize", dismiss);
    return () => {
      window.removeEventListener("mousedown", outside);
      window.removeEventListener("scroll", scrolled, true);
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("resize", dismiss);
    };
  }, [open]);

  // Keep the active option in view. Not scrollIntoView, which can also scroll the dialog behind the list.
  useLayoutEffect(() => {
    const box = list.current;
    const el = box?.querySelector<HTMLElement>(`[data-index="${active}"]`);
    if (!open || !box || !el) return;
    if (el.offsetTop < box.scrollTop) box.scrollTop = el.offsetTop - 4;
    else if (el.offsetTop + el.offsetHeight > box.scrollTop + box.clientHeight)
      box.scrollTop = el.offsetTop + el.offsetHeight - box.clientHeight + 4;
  }, [open, active]);

  const onKeyDown = (e: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const now = Date.now();
    const typing = typed.current.text !== "" && now - typed.current.at < 600;
    // Type-ahead; a space continues a label ("Extra high") only mid-word.
    if (e.key.length === 1 && (e.key !== " " || typing)) {
      e.preventDefault();
      const text = (typing ? typed.current.text : "") + e.key;
      typed.current = { text, at: now };
      const hit = typeahead(
        options.map((o) => o.label),
        text,
        open ? active : selected,
      );
      if (hit !== null) open ? setActive(hit) : show(hit);
      return;
    }
    typed.current.text = "";
    if (!open) {
      if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
        e.preventDefault();
        show();
      } else if (e.key === "Home" || e.key === "End") {
        e.preventDefault();
        show(e.key === "Home" ? 0 : options.length - 1);
      }
    } else if (e.key === "Escape") {
      // preventDefault keeps the Settings dialog open; stopPropagation keeps App's drawer handler out of it.
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      choose(active);
    } else if (e.key === "Tab") {
      choose(active);
    } else {
      const next = moveActive(e.key, active, options.length);
      if (next !== null) {
        e.preventDefault();
        setActive(next);
      }
    }
  };

  return (
    <>
      <button
        ref={trigger}
        type="button"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? `${id}-list` : undefined}
        aria-activedescendant={open ? `${id}-${active}` : undefined}
        disabled={props.disabled}
        onClick={() => (open ? setOpen(false) : show())}
        onKeyDown={onKeyDown}
        className={cx(
          "group flex w-full cursor-pointer items-center gap-2 rounded-md border bg-page py-1.5 pr-2 pl-2.5 text-left text-[13px] text-ink transition-colors",
          "hover:border-faint disabled:cursor-default disabled:opacity-50 disabled:hover:border-line",
          open ? "border-accent" : "border-line",
          props.className,
        )}
      >
        <span className="min-w-0 flex-1 truncate">{current?.label ?? ""}</span>
        <svg
          viewBox="0 0 16 16"
          className={cx("size-3.5 shrink-0 transition-colors", open ? "text-accent" : "text-faint group-hover:text-muted")}
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <path d="M5 6.5 8 3.5l3 3M5 9.5l3 3 3-3" />
        </svg>
      </button>
      {open &&
        at &&
        createPortal(
          <div
            ref={list}
            id={`${id}-list`}
            role="listbox"
            aria-label={props.label}
            style={at.place}
            // Keep focus on the trigger, which owns the keyboard.
            onMouseDown={(e) => e.preventDefault()}
            className={cx(
              "fixed z-50 grid content-start gap-0.5 overflow-y-auto overscroll-contain rounded-md border border-line bg-page p-1 text-ink shadow-[0_8px_24px_rgb(0_0_0/0.12)]",
              "animate-pop motion-reduce:animate-none",
              at.place.bottom !== undefined && "origin-bottom",
            )}
          >
            {options.map((o, i) => (
              <div
                key={o.value}
                id={`${id}-${i}`}
                data-index={i}
                role="option"
                aria-selected={i === selected}
                onMouseMove={() => i !== active && setActive(i)}
                onClick={() => choose(i)}
                className={cx(
                  "flex cursor-pointer items-center gap-2 rounded px-2.5 py-1.5 text-[13px]",
                  i === active && "bg-subtle",
                  i === selected && "font-medium",
                )}
              >
                <span className="min-w-0 flex-1 truncate">{o.label}</span>
                <svg
                  viewBox="0 0 16 16"
                  className={cx("size-3.5 shrink-0 text-accent", i !== selected && "invisible")}
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.75"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden
                >
                  <path d="m3.5 8.5 3 3 6-7" />
                </svg>
              </div>
            ))}
          </div>,
          at.host,
        )}
    </>
  );
}
