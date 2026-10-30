import type { ButtonHTMLAttributes, ReactNode } from "react";

export function PageHeader({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <header className="pb-6">
      <h1 className="font-display text-[44px] leading-none font-extrabold uppercase tracking-tight">{title}</h1>
      <p className="mt-2 text-[15px] text-muted">{subtitle}</p>
    </header>
  );
}

export function Card({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <section className={`rounded-card border border-stroke bg-card ${className}`}>{children}</section>;
}

export function SectionLabel({ children, trailing }: { children: ReactNode; trailing?: ReactNode }) {
  return (
    <div className="flex items-center justify-between px-1 pb-2">
      <span className="text-meta font-semibold uppercase tracking-[0.12em] text-faint">{children}</span>
      {trailing && <span className="text-meta text-faint">{trailing}</span>}
    </div>
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "primary" | "glass" | "link" };

export function Button({ variant = "glass", className = "", ...props }: ButtonProps) {
  const look = {
    primary: "bg-accent text-ink hover:brightness-105 active:brightness-95 rounded-full px-5 py-2.5 font-semibold",
    glass:
      "bg-glass hover:bg-glass-hover active:bg-glass-pressed border border-stroke rounded-full px-4 py-2 font-semibold",
    link: "text-accent hover:underline font-semibold px-1",
  }[variant];
  return (
    <button
      className={`text-body transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${look} ${className}`}
      {...props}
    />
  );
}

export function Pill({ tone = "neutral", children }: { tone?: "neutral" | "good" | "warn" | "bad"; children: ReactNode }) {
  const look = {
    neutral: "bg-glass text-muted",
    good: "bg-accent/15 text-accent",
    warn: "bg-warning/15 text-warning",
    bad: "bg-danger/15 text-danger",
  }[tone];
  return <span className={`rounded-full px-2.5 py-0.5 text-meta font-semibold ${look}`}>{children}</span>;
}

export function Progress({ value }: { value: number }) {
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-track">
      <div className="h-full rounded-full bg-accent transition-[width]" style={{ width: `${Math.round(value * 100)}%` }} />
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  note,
  action,
}: {
  icon: ReactNode;
  title: string;
  note: string;
  action?: { label: string; onClick: () => void };
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 rounded-card bg-well p-10 text-center">
      <div className="text-faint">{icon}</div>
      <div className="text-row font-semibold">{title}</div>
      <p className="max-w-sm text-note text-muted">{note}</p>
      {action && (
        <Button className="mt-1" onClick={action.onClick}>
          {action.label}
        </Button>
      )}
    </div>
  );
}

/** A warning or an error with what to do about it. */
export function Notice({ tone = "warn", title, children }: { tone?: "warn" | "bad"; title: string; children?: ReactNode }) {
  const look = tone === "bad" ? "border-danger/40 bg-danger/10" : "border-warning/40 bg-warning/10";
  return (
    <div className={`rounded-field border px-4 py-3 ${look}`}>
      <div className={`text-body font-semibold ${tone === "bad" ? "text-danger" : "text-warning"}`}>{title}</div>
      {children && <div className="mt-1 text-note text-muted">{children}</div>}
    </div>
  );
}

/** A bar for work whose share done is not known. */
export function Busy() {
  return (
    <div className="relative h-1.5 w-full overflow-hidden rounded-full bg-track">
      <div className="absolute inset-y-0 w-1/3 animate-[busy_1.4s_ease-in-out_infinite] rounded-full bg-accent" />
    </div>
  );
}
