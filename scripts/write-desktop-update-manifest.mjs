#!/usr/bin/env node

import { lstatSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { pathToFileURL } from "node:url";

const MAX_SIGNATURE_BYTES = 16 * 1024;
const VERSION =
  /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$/u;
const TAG =
  /^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-preview\.([1-9][0-9]*))?$/u;
const REPOSITORY = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u;

function fail(message) {
  throw new Error(`write-desktop-update-manifest: ${message}`);
}

function exactFile(path) {
  const metadata = lstatSync(path);
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    fail(`${basename(path)} must be a regular non-symlink file`);
  }
}

function signature(path) {
  exactFile(path);
  const value = readFileSync(path, "utf8").trim();
  if (
    value.length === 0 ||
    Buffer.byteLength(value) > MAX_SIGNATURE_BYTES ||
    value.includes("\0")
  ) {
    fail(`${basename(path)} is not a bounded updater signature`);
  }
  return value;
}

function releaseUrl(repository, tag, fileName) {
  return `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(fileName)}`;
}

export function buildDesktopUpdateManifest({
  channel,
  version,
  tag,
  repository,
  directory,
}) {
  if (!["stable", "developer_preview"].includes(channel)) {
    fail("channel must be stable or developer_preview");
  }
  if (!VERSION.test(version) || !TAG.test(tag) || version !== tag.slice(1)) {
    fail("version and tag must identify the same canonical release");
  }
  if (!REPOSITORY.test(repository)) {
    fail("repository must be an owner/name pair");
  }

  const preview = channel === "developer_preview";
  const macName = preview
    ? `Colossus-Desktop-DEVELOPER-PREVIEW-${tag}-aarch64-apple-darwin.app.tar.gz`
    : `Colossus-Desktop-${tag}-aarch64-apple-darwin.app.tar.gz`;
  exactFile(join(directory, macName));
  const platforms = {
    [`macos-aarch64-${channel}`]: {
      signature: signature(join(directory, `${macName}.sig`)),
      url: releaseUrl(repository, tag, macName),
    },
  };

  if (preview) {
    const windowsName = `Colossus-Desktop-UNSIGNED-DEVELOPER-PREVIEW-${tag}-x86_64-pc-windows-msvc-setup.exe`;
    exactFile(join(directory, windowsName));
    platforms[`windows-x86_64-${channel}`] = {
      signature: signature(join(directory, `${windowsName}.sig`)),
      url: releaseUrl(repository, tag, windowsName),
    };
  }

  return {
    schemaVersion: 1,
    channel,
    version,
    notes:
      channel === "stable"
        ? "Colossus Desktop stable update."
        : "Colossus Desktop Developer Preview update.",
    platforms,
  };
}

function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      fail("arguments must be --name value pairs");
    }
    values.set(name.slice(2), value);
  }
  for (const name of [
    "channel",
    "version",
    "tag",
    "repository",
    "directory",
    "output",
  ]) {
    if (!values.has(name)) {
      fail(`missing --${name}`);
    }
  }
  return Object.fromEntries(values);
}

function main() {
  const options = parseArguments(process.argv.slice(2));
  const manifest = buildDesktopUpdateManifest(options);
  writeFileSync(options.output, `${JSON.stringify(manifest, null, 2)}\n`, {
    encoding: "utf8",
    mode: 0o644,
    flag: "wx",
  });
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
