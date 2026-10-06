"use client";
import * as React from "react";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";

type ButtonVariant = "default" | "outline" | "ghost" | "destructive" | "secondary";
const variants: Record<ButtonVariant, string> = {
  default: "bg-primary text-primary-foreground hover:bg-primary/90",
  outline: "border border-border bg-transparent hover:bg-muted",
  ghost: "hover:bg-muted",
  destructive: "bg-destructive text-white hover:bg-destructive/90",
  secondary: "bg-muted text-foreground hover:bg-muted/80",
};

export function Button({
  className, variant = "default", size = "md", ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; size?: "sm" | "md" | "icon" }) {
  return (
    <button
      className={cn(
        "inline-flex items-center justify-center gap-1.5 rounded-md text-sm font-medium transition-colors focus-visible:outline-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-50 cursor-pointer",
        size === "sm" ? "h-8 px-2.5 text-xs" : size === "icon" ? "h-9 w-9" : "h-9 px-3.5",
        variants[variant],
        className,
      )}
      {...props}
    />
  );
}

export function Card({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("rounded-lg border border-border bg-card text-card-foreground shadow-sm", className)} {...props} />;
}
export function CardHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("flex items-center justify-between gap-2 border-b border-border px-4 py-3", className)} {...props} />;
}
export function CardTitle({ className, ...props }: React.HTMLAttributes<HTMLHeadingElement>) {
  return <h2 className={cn("text-sm font-semibold", className)} {...props} />;
}
export function CardContent({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("p-4", className)} {...props} />;
}

type Tone = "default" | "success" | "warning" | "danger" | "info" | "muted";
const tones: Record<Tone, string> = {
  default: "bg-muted text-foreground",
  success: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400",
  warning: "bg-amber-500/15 text-amber-700 dark:text-amber-400",
  danger: "bg-red-500/15 text-red-700 dark:text-red-400",
  info: "bg-sky-500/15 text-sky-700 dark:text-sky-400",
  muted: "bg-muted text-muted-foreground",
};
export function Badge({ tone = "default", className, ...props }: React.HTMLAttributes<HTMLSpanElement> & { tone?: Tone }) {
  return <span className={cn("inline-flex items-center rounded px-1.5 py-0.5 text-xs font-medium whitespace-nowrap", tones[tone], className)} {...props} />;
}

export const Input = React.forwardRef<HTMLInputElement, React.InputHTMLAttributes<HTMLInputElement>>(function Input({ className, ...props }, ref) {
  return (
    <input
      ref={ref}
      className={cn("h-9 w-full rounded-md border border-border bg-background px-3 text-sm outline-none focus:ring-2 focus:ring-ring/50 disabled:opacity-60", className)}
      {...props}
    />
  );
});

export function Select({ className, ...props }: React.SelectHTMLAttributes<HTMLSelectElement>) {
  return <select className={cn("h-9 rounded-md border border-border bg-background px-2 text-sm outline-none focus:ring-2 focus:ring-ring/50", className)} {...props} />;
}

export function Label({ className, ...props }: React.LabelHTMLAttributes<HTMLLabelElement>) {
  return <label className={cn("grid gap-1 text-xs font-medium text-muted-foreground", className)} {...props} />;
}

export function FieldError({ msg }: { msg?: string }) {
  return msg ? <p className="text-xs text-destructive" role="alert">{msg}</p> : null;
}

function useEscape(open: boolean, onClose: () => void) {
  React.useEffect(() => {
    if (!open) return;
    const h = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [open, onClose]);
}

export function Dialog({ open, onClose, title, children, footer, wide }: {
  open: boolean; onClose: () => void; title: string; children: React.ReactNode; footer?: React.ReactNode; wide?: boolean;
}) {
  useEscape(open, onClose);
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-4 pt-[10vh]" onMouseDown={onClose}>
      <div role="dialog" aria-modal="true" aria-label={title} className={cn("w-full rounded-lg border border-border bg-card shadow-xl", wide ? "max-w-2xl" : "max-w-md")} onMouseDown={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between border-b border-border px-4 py-3">
          <h2 className="text-sm font-semibold">{title}</h2>
          <Button variant="ghost" size="icon" onClick={onClose} aria-label="close"><X className="h-4 w-4" /></Button>
        </div>
        <div className="max-h-[65vh] overflow-y-auto p-4">{children}</div>
        {footer && <div className="flex justify-end gap-2 border-t border-border px-4 py-3">{footer}</div>}
      </div>
    </div>
  );
}

export function Sheet({ open, onClose, title, children }: { open: boolean; onClose: () => void; title: string; children: React.ReactNode }) {
  useEscape(open, onClose);
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-40 bg-black/40" onMouseDown={onClose}>
      <aside role="dialog" aria-modal="true" aria-label={title} className="absolute right-0 top-0 flex h-full w-full max-w-xl flex-col border-l border-border bg-card shadow-2xl" onMouseDown={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between border-b border-border px-4 py-3">
          <h2 className="text-sm font-semibold">{title}</h2>
          <Button variant="ghost" size="icon" onClick={onClose} aria-label="close"><X className="h-4 w-4" /></Button>
        </div>
        <div className="flex-1 overflow-y-auto p-4">{children}</div>
      </aside>
    </div>
  );
}

export function Tabs<T extends string>({ value, onChange, items }: { value: T; onChange: (v: T) => void; items: { value: T; label: React.ReactNode }[] }) {
  return (
    // Scrolls sideways on phones instead of wrapping the tab labels.
    <div role="tablist" className="inline-flex max-w-full overflow-x-auto rounded-md border border-border bg-muted/50 p-0.5">
      {items.map((it) => (
        <button
          key={it.value}
          role="tab"
          aria-selected={value === it.value}
          onClick={() => onChange(it.value)}
          className={cn("whitespace-nowrap rounded px-3 py-1 text-sm cursor-pointer", value === it.value ? "bg-card shadow-sm font-medium" : "text-muted-foreground hover:text-foreground")}
        >
          {it.label}
        </button>
      ))}
    </div>
  );
}

export function Stat({ label, value, sub, tone }: { label: string; value: React.ReactNode; sub?: React.ReactNode; tone?: "up" | "down" }) {
  return (
    <Card className="p-4">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className={cn("mt-1 text-2xl font-semibold tabular-nums", tone === "up" && "text-emerald-600 dark:text-emerald-400", tone === "down" && "text-red-600 dark:text-red-400")}>{value}</div>
      {sub && <div className="mt-1 text-xs text-muted-foreground">{sub}</div>}
    </Card>
  );
}

export function PageHeader({ title, children }: { title: string; children?: React.ReactNode }) {
  return (
    <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
      <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
      <div className="flex flex-wrap items-center gap-2">{children}</div>
    </div>
  );
}

export function Pnl({ value, children }: { value: number; children: React.ReactNode }) {
  return <span className={cn("tabular-nums", value > 0 && "text-emerald-600 dark:text-emerald-400", value < 0 && "text-red-600 dark:text-red-400")}>{children}</span>;
}
