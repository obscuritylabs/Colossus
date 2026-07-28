import {
  IconAlertTriangle,
  IconDownload,
  IconRefresh,
} from "@tabler/icons-react";
import { Component } from "react";
import type { ErrorInfo, ReactNode } from "react";

import { exportDiagnostics } from "../api";

interface AppErrorBoundaryProps {
  children: ReactNode;
}

interface AppErrorBoundaryState {
  failed: boolean;
  exportState: "idle" | "saving" | "saved" | "failed";
}

export class AppErrorBoundary extends Component<
  AppErrorBoundaryProps,
  AppErrorBoundaryState
> {
  state: AppErrorBoundaryState = {
    failed: false,
    exportState: "idle",
  };

  static getDerivedStateFromError(): Partial<AppErrorBoundaryState> {
    return { failed: true };
  }

  componentDidCatch(_error: Error, _errorInfo: ErrorInfo): void {
    // Error content can contain prompts or private runtime details. It is
    // deliberately neither persisted nor sent across the native boundary.
  }

  private exportLocalDiagnostics = async (): Promise<void> => {
    this.setState({ exportState: "saving" });
    try {
      const saved = await exportDiagnostics();
      this.setState({ exportState: saved ? "saved" : "idle" });
    } catch {
      this.setState({ exportState: "failed" });
    }
  };

  render(): ReactNode {
    if (!this.state.failed) {
      return this.props.children;
    }
    return (
      <main className="fatal-error-surface">
        <div className="fatal-error-card">
          <span className="fatal-error-icon" aria-hidden="true">
            <IconAlertTriangle size={26} stroke={1.7} />
          </span>
          <p className="eyebrow">Desktop renderer</p>
          <h1>Colossus needs to reload this view</h1>
          <p>
            The native runtime was not given this error text. Reload the
            interface, or export a local diagnostics file that excludes prompts,
            credentials, model output, and private paths.
          </p>
          <div className="fatal-error-actions">
            <button
              className="button primary"
              type="button"
              onClick={() => window.location.reload()}
            >
              <IconRefresh size={16} stroke={1.8} aria-hidden="true" />
              Reload
            </button>
            <button
              className="button secondary"
              type="button"
              disabled={this.state.exportState === "saving"}
              onClick={() => void this.exportLocalDiagnostics()}
            >
              <IconDownload size={16} stroke={1.8} aria-hidden="true" />
              {this.state.exportState === "saving"
                ? "Saving…"
                : "Export diagnostics"}
            </button>
          </div>
          {this.state.exportState === "saved" ? (
            <p className="success-message" role="status">
              Diagnostics exported.
            </p>
          ) : null}
          {this.state.exportState === "failed" ? (
            <p className="error-message" role="alert">
              Diagnostics could not be exported.
            </p>
          ) : null}
        </div>
      </main>
    );
  }
}
