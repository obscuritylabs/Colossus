import type { GitStatus } from "../../git";

// Presentation snapshots only: never persist these or use them to authorize reads.
// Bound memory while retaining recently refreshed workspaces for instant switching.
export class GitStatusCache {
  private readonly entries = new Map<string, GitStatus>();

  get(key: string | null): GitStatus | null {
    return key === null ? null : (this.entries.get(key) ?? null);
  }

  set(key: string, status: GitStatus): void {
    this.entries.delete(key);
    this.entries.set(key, status);
    if (this.entries.size > 20) {
      this.entries.delete(this.entries.keys().next().value!);
    }
  }
}
