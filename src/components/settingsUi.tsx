export function Section({
  title,
  children,
  icon,
  action,
}: {
  title: string;
  children: React.ReactNode;
  icon?: React.ReactNode;
  action?: React.ReactNode;
}) {
  return (
    <section className="set-section">
      <h2>
        {icon}
        {title}
        {action && <span className="set-section-action">{action}</span>}
      </h2>
      <div className="set-card">{children}</div>
    </section>
  );
}

export function Row({
  label,
  hint,
  warn,
  children,
}: {
  label: string;
  hint?: string;
  warn?: boolean;
  children?: React.ReactNode;
}) {
  return (
    <div className="set-row">
      <div className="set-label">
        <span>{label}</span>
        {hint && <span className={`set-hint ${warn ? "warn" : ""}`}>{hint}</span>}
      </div>
      <div className="set-control">{children}</div>
    </div>
  );
}

export type DotState = "off" | "online" | "error" | "warn" | "idle";

export function Dot({ state }: { state: DotState }) {
  return <span className={`live-dot ${state}`} />;
}
