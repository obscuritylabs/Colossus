import { chmod, lstat, readdir, realpath, rm } from "node:fs/promises";
import { basename, dirname, isAbsolute, join, relative, sep } from "node:path";
import { homedir } from "node:os";

/** Remove only a stopped test's freshly-created home, including immutable plugins. */
export async function removeManagedFixture(root: string) {
  const metadata = await lstat(root);
  const canonical = await realpath(root);
  if (
    metadata.isSymbolicLink() ||
    !metadata.isDirectory() ||
    !basename(canonical).startsWith(".colossus-vscode-managed-test-") ||
    dirname(canonical) !== (await realpath(homedir()))
  )
    throw new Error("Refusing to remove an unrelated fixture directory.");
  const writableDirectories = async (path: string) => {
    const child = relative(canonical, path);
    if (isAbsolute(child) || child === ".." || child.startsWith(`..${sep}`))
      throw new Error("Fixture cleanup escaped its owned home.");
    const info = await lstat(path);
    if (info.isSymbolicLink() || !info.isDirectory()) return;
    // This runs after both native owners stopped. Production plugin stores remain
    // immutable; only directories beneath this exact throwaway home are changed.
    await chmod(path, 0o700);
    for (const entry of await readdir(path, { withFileTypes: true }))
      if (entry.isDirectory() && !entry.isSymbolicLink())
        await writableDirectories(join(path, entry.name));
  };
  if (process.platform !== "win32") await writableDirectories(canonical);
  await rm(canonical, { recursive: true, force: true });
}
