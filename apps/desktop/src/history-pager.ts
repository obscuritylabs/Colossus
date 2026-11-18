/** Foreground history discovery only. It never owns or cancels an agent run. */
export interface HistoryRequest {
  readonly scope: number;
  readonly cursor: string;
  readonly replace: boolean;
}

const MAX_EMPTY_PAGES = 8;
const MAX_AUTOMATIC_PAGES = 16;

export class HistoryPager {
  private scope = 0;
  private request: HistoryRequest | null = null;
  private cursors = new Set<string>();
  private automaticPages = 0;
  private emptyPages = 0;
  private blocked = false;
  pauseReason: "budget" | "no-progress" | "cycle" | "error" | null = null;
  private next = "";
  private started = false;

  reset(): void {
    this.scope += 1;
    this.request = null;
    this.cursors.clear();
    this.automaticPages = 0;
    this.emptyPages = 0;
    this.blocked = false;
    this.pauseReason = null;
    this.next = "";
    this.started = false;
  }

  /** A deliberate user action permits another finite automatic burst. */
  resume(): void {
    this.automaticPages = 0;
    this.emptyPages = 0;
    this.blocked = false;
    this.pauseReason = null;
  }

  begin(
    cursor: string,
    replace: boolean,
    automatic = false,
  ): HistoryRequest | null {
    if (this.request !== null || (automatic && this.blocked)) return null;
    if (!replace && (!this.started || cursor === "" || cursor !== this.next))
      return null;
    if (automatic && this.automaticPages >= MAX_AUTOMATIC_PAGES) {
      this.blocked = true;
      this.pauseReason = "budget";
      return null;
    }
    if (replace) this.reset();
    if (automatic) this.automaticPages += 1;
    const request = Object.freeze({ scope: this.scope, cursor, replace });
    this.request = request;
    return request;
  }

  isCurrent(request: HistoryRequest): boolean {
    return this.request === request && request.scope === this.scope;
  }

  accept(
    request: HistoryRequest,
    next: string,
    visibleProgress: boolean,
  ): string {
    if (!this.isCurrent(request)) return "";
    this.started = true;
    this.emptyPages = visibleProgress ? 0 : this.emptyPages + 1;
    const repeated =
      next !== "" && (next === request.cursor || this.cursors.has(next));
    if (request.cursor !== "") this.cursors.add(request.cursor);
    if (this.cursors.size > 64)
      this.cursors.delete(this.cursors.values().next().value!);
    this.next = next;
    if (repeated) {
      this.blocked = true;
      this.pauseReason = "cycle";
      // Retrying a cyclic cursor cannot make progress. A refresh starts a new chain.
      return "History returned a repeated page. Refresh threads to continue.";
    }
    if (
      this.emptyPages >= MAX_EMPTY_PAGES ||
      this.automaticPages >= MAX_AUTOMATIC_PAGES
    ) {
      this.blocked = true;
      this.pauseReason =
        this.emptyPages >= MAX_EMPTY_PAGES ? "no-progress" : "budget";
      return next === ""
        ? ""
        : "Automatic loading paused. Load older threads to continue.";
    }
    return "";
  }

  fail(request: HistoryRequest): void {
    if (this.isCurrent(request)) {
      this.blocked = true;
      this.pauseReason = "error";
    }
  }

  finish(request: HistoryRequest): boolean {
    if (!this.isCurrent(request)) return false;
    this.request = null;
    return true;
  }
}
