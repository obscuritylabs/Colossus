import { useState } from "react";
import { pluginIconSource } from "../plugins";
import "./plugin-icon.css";

/** Decorative identity next to a visible name; failed or absent images use a monogram. */
export function PluginIcon({
  name,
  icon,
  size = "medium",
}: {
  name: string;
  icon?: string | null | undefined;
  size?: "small" | "medium" | "large";
}) {
  const source = pluginIconSource(icon);
  const [failed, setFailed] = useState<string | null>(null);
  return (
    <span className={`plugin-icon plugin-icon--${size}`} aria-hidden="true">
      {source && failed !== source ? (
        <img
          src={source}
          alt=""
          draggable={false}
          onError={() => setFailed(source)}
        />
      ) : (
        <span>{name.slice(0, 2).toLocaleUpperCase()}</span>
      )}
    </span>
  );
}
