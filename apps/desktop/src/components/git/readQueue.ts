// Shared by status, history and file diffs, including across view remounts.
let tail: Promise<unknown> = Promise.resolve();
export function queueGitRead<T>(action: () => Promise<T>): Promise<T> {
  const result = tail.then(action, action);
  tail = result.catch(() => undefined);
  return result;
}
