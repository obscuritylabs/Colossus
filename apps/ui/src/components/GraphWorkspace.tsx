import type { ReactNode, Ref } from "react";
import { Button } from "./ui/button.js";
import "../../styles/graph-workspace.css";

/** Transport-free graph chrome; hosts own graph data, selection and persistence. */
export function GraphWorkspace({
  children,
  inspector,
  inspectorLabel,
  canvasRef,
  label,
  hint,
  onZoomIn,
  onZoomOut,
  onFit,
  fitLabel = "Fit graph",
  zoomInIcon = "+",
  zoomOutIcon = "−",
  fitIcon,
}: {
  children: ReactNode;
  inspector: ReactNode;
  inspectorLabel: string;
  canvasRef?: Ref<HTMLDivElement>;
  label: string;
  hint?: ReactNode;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onFit: () => void;
  fitLabel?: string;
  zoomInIcon?: ReactNode;
  zoomOutIcon?: ReactNode;
  fitIcon?: ReactNode;
}) {
  return (
    <div className="graph-workspace">
      <div className="graph-workspace__canvas" ref={canvasRef}>
        <div
          className="graph-workspace__toolbar"
          role="group"
          aria-label={label}
        >
          <Button
            variant="ghost"
            size="icon"
            aria-label="Zoom in"
            onClick={onZoomIn}
          >
            {zoomInIcon}
          </Button>
          <Button
            variant="ghost"
            size="icon"
            aria-label="Zoom out"
            onClick={onZoomOut}
          >
            {zoomOutIcon}
          </Button>
          <Button variant="ghost" size="sm" onClick={onFit}>
            {fitIcon}
            {fitLabel}
          </Button>
        </div>
        <div className="graph-workspace__view">{children}</div>
        {hint ? <span className="graph-workspace__hint">{hint}</span> : null}
      </div>
      <aside className="graph-workspace__inspector" aria-label={inspectorLabel}>
        {inspector}
      </aside>
    </div>
  );
}
