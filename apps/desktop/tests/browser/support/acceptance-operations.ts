import { EventEmitter, once } from "node:events";

/** Wait for the submitted native request, then let the test assert its rendered result. */
export class AcceptanceOperations {
  private readonly completions = new EventEmitter();

  complete(operation: string): void {
    this.completions.emit(operation);
  }

  async submit(
    operation: string,
    trigger: () => Promise<unknown>,
    timeoutMs = 60_000,
  ): Promise<void> {
    const controller = new AbortController();
    const timeout = setTimeout(
      () =>
        controller.abort(
          new Error(`Native ${operation} operation did not complete`),
        ),
      timeoutMs,
    );
    try {
      // Subscribe before clicking. IPC has a 45-second command bound; a generic
      // five-second DOM assertion must not terminate a still-pending native call.
      await Promise.all([
        once(this.completions, operation, { signal: controller.signal }),
        Promise.resolve().then(trigger),
      ]);
    } catch (error) {
      if (controller.signal.aborted) throw controller.signal.reason;
      throw error;
    } finally {
      clearTimeout(timeout);
      controller.abort();
    }
  }
}
