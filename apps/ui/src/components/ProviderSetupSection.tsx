import { useId, type ReactNode } from "react";
import "../../styles/provider-setup.css";

export function ProviderSetupSection({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  const heading = useId();
  return (
    <section className="provider-editor-section" aria-labelledby={heading}>
      <div className="provider-editor-section-heading">
        <h5 id={heading}>{title}</h5>
        <p>{description}</p>
      </div>
      {children}
    </section>
  );
}
