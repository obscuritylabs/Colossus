import type { ReactNode } from "react";
export function WorkspaceSurfaceHeader({
  eyebrow,
  title,
  description,
  actions,
}: {
  eyebrow: string;
  title: string;
  description: string;
  actions?: ReactNode;
}) {
  return (
    <header className="surface-header overview-header shared-overview-header">
      <div className="surface-title-copy">
        <p className="surface-breadcrumb">{eyebrow}</p>
        <h2>{title}</h2>
        <span>{description}</span>
      </div>
      {actions ? <div className="surface-header-actions">{actions}</div> : null}
    </header>
  );
}
