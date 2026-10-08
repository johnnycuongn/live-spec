// Shared building blocks, so components don't repeat long class lists.
import type { ButtonHTMLAttributes, ReactNode } from "react";

export const cx = (...parts: (string | false | null | undefined)[]) => parts.filter(Boolean).join(" ");

type Variant = "default" | "primary" | "ghost" | "link";

const VARIANTS: Record<Variant, string> = {
  default: "rounded-md border border-line bg-page px-3 py-1.5 hover:border-faint",
  // text-page, not white: the dark-mode accent is light enough to need dark text.
  primary: "rounded-md border border-accent bg-accent px-3 py-1.5 font-medium text-page hover:opacity-90",
  ghost: "rounded-md px-2.5 py-1.5 text-muted hover:bg-subtle hover:text-ink",
  link: "text-accent underline underline-offset-2",
};

export function Button({ variant = "default", className, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return (
    <button
      type="button"
      className={cx(
        "inline-flex cursor-pointer items-center gap-2 text-[13px] transition-colors disabled:cursor-default disabled:opacity-50",
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
