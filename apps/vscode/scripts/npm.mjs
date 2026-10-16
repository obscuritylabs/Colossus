import { dirname, join } from "node:path";

// Windows cannot execute npm.cmd directly through spawnSync without a shell.
// Run npm's installed JavaScript entry point with the current Node runtime.
export function npmInvocation(args) {
  return process.platform === "win32"
    ? [
        process.execPath,
        [
          process.env.npm_execpath ??
            join(dirname(process.execPath), "node_modules/npm/bin/npm-cli.js"),
          ...args,
        ],
      ]
    : ["npm", args];
}
