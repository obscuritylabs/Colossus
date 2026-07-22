#!/usr/bin/env node

import {
  closeSync,
  lstatSync,
  openSync,
  readSync,
  readdirSync,
  realpathSync,
} from "node:fs";
import { basename, isAbsolute, join } from "node:path";

const MAX_ARCHIVE_BYTES = 2 * 1024 * 1024 * 1024;
const MAX_EXTRACTED_BYTES = 2 * 1024 * 1024 * 1024;
const MAX_CENTRAL_DIRECTORY_BYTES = 64 * 1024 * 1024;
const MAX_ENTRIES = 200_000;
const MAX_NAME_BYTES = 1024;
const EOCD_SIGNATURE = 0x06054b50;
const CENTRAL_SIGNATURE = 0x02014b50;
const LOCAL_SIGNATURE = 0x04034b50;
const UNIX_CREATOR = 3;
const UNIX_FILE_TYPE = 0xf000;
const UNIX_DIRECTORY = 0x4000;
const UNIX_REGULAR = 0x8000;
const UNIX_SYMLINK = 0xa000;
const ROOT = "Colossus Desktop.app";
const FORBIDDEN_EXTRA_FIELDS = new Set([0x0001, 0x7075]);

function fail(message) {
  process.stderr.write(`verify-desktop-unsigned-archive: ${message}\n`);
  process.exit(1);
}

function parseArguments(argv) {
  if (
    (argv.length !== 2 && argv.length !== 4) ||
    argv[0] !== "--archive" ||
    (argv.length === 4 && argv[2] !== "--extracted-root")
  ) {
    fail(
      "usage: scripts/verify-desktop-unsigned-archive.mjs --archive ABSOLUTE_ZIP [--extracted-root ABSOLUTE_DIRECTORY]",
    );
  }
  return { archive: argv[1], extractedRoot: argv[3] };
}

function readExact(descriptor, length, position) {
  const result = Buffer.alloc(length);
  let offset = 0;
  while (offset < length) {
    const count = readSync(
      descriptor,
      result,
      offset,
      length - offset,
      position + offset,
    );
    if (count === 0) {
      fail("archive ended before a complete ZIP structure was read");
    }
    offset += count;
  }
  return result;
}

function validatedName(bytes) {
  if (bytes.length === 0 || bytes.length > MAX_NAME_BYTES) {
    fail("ZIP entry name is empty or exceeds its size limit");
  }
  let name;
  try {
    name = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    fail("ZIP entry name is not strict UTF-8");
  }
  if (
    name.includes("\\") ||
    name.startsWith("/") ||
    /^[A-Za-z]:/u.test(name) ||
    /\p{Cc}/u.test(name)
  ) {
    fail("ZIP entry name is absolute, ambiguous, or contains control bytes");
  }
  const directory = name.endsWith("/");
  const components = name.split("/");
  if (directory) {
    components.pop();
  }
  if (
    components.length === 0 ||
    components.some(
      (component) =>
        component.length === 0 || component === "." || component === "..",
    ) ||
    components[0] !== ROOT
  ) {
    fail("ZIP entry escapes the one expected application root");
  }
  return { directory, name };
}

function validateExtraFields(bytes) {
  let cursor = 0;
  while (cursor < bytes.length) {
    if (cursor + 4 > bytes.length) {
      fail("ZIP extra-field header is truncated");
    }
    const identifier = bytes.readUInt16LE(cursor);
    const size = bytes.readUInt16LE(cursor + 2);
    cursor += 4;
    if (cursor + size > bytes.length) {
      fail("ZIP extra-field payload is truncated");
    }
    if (FORBIDDEN_EXTRA_FIELDS.has(identifier)) {
      fail("ZIP64 and Unicode path override extra fields are not accepted");
    }
    cursor += size;
  }
}

function validateLocalHeader(descriptor, archiveSize, centralOffset, entry) {
  if (entry.localOffset + 30 > centralOffset) {
    fail("ZIP local header overlaps the central directory");
  }
  const local = readExact(descriptor, 30, entry.localOffset);
  if (local.readUInt32LE(0) !== LOCAL_SIGNATURE) {
    fail("ZIP local header signature is invalid");
  }
  const localFlags = local.readUInt16LE(6);
  const localMethod = local.readUInt16LE(8);
  const nameLength = local.readUInt16LE(26);
  const extraLength = local.readUInt16LE(28);
  const dataOffset = entry.localOffset + 30 + nameLength + extraLength;
  if (
    localFlags !== entry.flags ||
    localMethod !== entry.method ||
    nameLength !== entry.nameBytes.length ||
    dataOffset + entry.compressedSize > centralOffset ||
    dataOffset + entry.compressedSize > archiveSize
  ) {
    fail("ZIP local and central entry metadata do not agree");
  }
  const localName = readExact(descriptor, nameLength, entry.localOffset + 30);
  if (!localName.equals(entry.nameBytes)) {
    fail("ZIP local and central entry names do not agree");
  }
  validateExtraFields(
    readExact(descriptor, extraLength, entry.localOffset + 30 + nameLength),
  );
}

function validateExtractedRoot(root) {
  if (!isAbsolute(root)) {
    fail("extracted root must be absolute");
  }
  const rootMetadata = lstatSync(root);
  if (
    !rootMetadata.isDirectory() ||
    rootMetadata.isSymbolicLink() ||
    realpathSync(root) !== root
  ) {
    fail("extracted root must be a canonical non-symlink directory");
  }
  const topLevel = readdirSync(root);
  if (topLevel.length !== 1 || topLevel[0] !== ROOT) {
    fail("extraction did not produce exactly one expected top-level app");
  }
  const app = join(root, ROOT);
  const appMetadata = lstatSync(app);
  if (
    !appMetadata.isDirectory() ||
    appMetadata.isSymbolicLink() ||
    realpathSync(app) !== app
  ) {
    fail("extracted application root is not a canonical directory");
  }

  const pending = [app];
  let count = 0;
  let extractedBytes = 0;
  while (pending.length > 0) {
    const directory = pending.pop();
    for (const name of readdirSync(directory)) {
      count += 1;
      if (count > MAX_ENTRIES || /\p{Cc}/u.test(name)) {
        fail(
          "extracted application is oversized or has a control-containing name",
        );
      }
      const path = join(directory, name);
      const entry = lstatSync(path);
      if (entry.isSymbolicLink()) {
        fail("extracted application contains a symlink");
      }
      if (entry.isDirectory()) {
        pending.push(path);
      } else if (!entry.isFile()) {
        fail("extracted application contains a special filesystem entry");
      } else {
        if (entry.nlink !== 1) {
          fail("extracted application contains a hard-linked file");
        }
        extractedBytes += entry.size;
        if (extractedBytes > MAX_EXTRACTED_BYTES) {
          fail("extracted application exceeds its byte budget");
        }
      }
    }
  }
}

const { archive, extractedRoot } = parseArguments(process.argv.slice(2));
if (
  !isAbsolute(archive) ||
  basename(archive) !== "Colossus-Desktop-unsigned-aarch64-apple-darwin.zip"
) {
  fail("archive must use its exact absolute release-workflow path");
}
const metadata = lstatSync(archive);
if (
  !metadata.isFile() ||
  metadata.isSymbolicLink() ||
  metadata.size < 22 ||
  metadata.size > MAX_ARCHIVE_BYTES ||
  realpathSync(archive) !== archive
) {
  fail("archive must be a bounded canonical regular file");
}

const descriptor = openSync(archive, "r");
try {
  const tailLength = Math.min(metadata.size, 65_557);
  const tailOffset = metadata.size - tailLength;
  const tail = readExact(descriptor, tailLength, tailOffset);
  const signature = Buffer.from([0x50, 0x4b, 0x05, 0x06]);
  const eocdIndex = tail.lastIndexOf(signature);
  if (eocdIndex < 0 || eocdIndex + 22 > tail.length) {
    fail("ZIP end-of-central-directory record is missing");
  }
  const eocd = tail.subarray(eocdIndex);
  const commentLength = eocd.readUInt16LE(20);
  if (eocdIndex + 22 + commentLength !== tail.length) {
    fail("ZIP has trailing or malformed end-of-directory data");
  }
  if (
    eocd.readUInt32LE(0) !== EOCD_SIGNATURE ||
    eocd.readUInt16LE(4) !== 0 ||
    eocd.readUInt16LE(6) !== 0
  ) {
    fail("multi-disk ZIP archives are not accepted");
  }
  const diskEntries = eocd.readUInt16LE(8);
  const entryCount = eocd.readUInt16LE(10);
  const centralSize = eocd.readUInt32LE(12);
  const centralOffset = eocd.readUInt32LE(16);
  if (
    diskEntries !== entryCount ||
    entryCount === 0 ||
    entryCount === 0xffff ||
    entryCount > MAX_ENTRIES ||
    centralSize === 0xffffffff ||
    centralSize > MAX_CENTRAL_DIRECTORY_BYTES ||
    centralOffset === 0xffffffff ||
    centralOffset + centralSize !== tailOffset + eocdIndex
  ) {
    fail("ZIP central directory is empty, ZIP64, oversized, or out of bounds");
  }

  const central = readExact(descriptor, centralSize, centralOffset);
  const seen = new Set();
  const entries = [];
  let totalUncompressedSize = 0;
  let cursor = 0;
  for (let index = 0; index < entryCount; index += 1) {
    if (
      cursor + 46 > central.length ||
      central.readUInt32LE(cursor) !== CENTRAL_SIGNATURE
    ) {
      fail("ZIP central directory entry is truncated or invalid");
    }
    const versionMadeBy = central.readUInt16LE(cursor + 4);
    const flags = central.readUInt16LE(cursor + 8);
    const method = central.readUInt16LE(cursor + 10);
    const compressedSize = central.readUInt32LE(cursor + 20);
    const uncompressedSize = central.readUInt32LE(cursor + 24);
    const nameLength = central.readUInt16LE(cursor + 28);
    const extraLength = central.readUInt16LE(cursor + 30);
    const entryCommentLength = central.readUInt16LE(cursor + 32);
    const diskStart = central.readUInt16LE(cursor + 34);
    const externalAttributes = central.readUInt32LE(cursor + 38);
    const localOffset = central.readUInt32LE(cursor + 42);
    const entryLength = 46 + nameLength + extraLength + entryCommentLength;
    if (
      cursor + entryLength > central.length ||
      diskStart !== 0 ||
      (flags & 1) !== 0 ||
      compressedSize === 0xffffffff ||
      uncompressedSize === 0xffffffff ||
      localOffset === 0xffffffff
    ) {
      fail("ZIP entry is encrypted, ZIP64, multi-disk, or out of bounds");
    }
    if (method !== 0 && method !== 8) {
      fail("ZIP entry uses an unsupported compression method");
    }
    const nameBytes = Buffer.from(
      central.subarray(cursor + 46, cursor + 46 + nameLength),
    );
    validateExtraFields(
      central.subarray(
        cursor + 46 + nameLength,
        cursor + 46 + nameLength + extraLength,
      ),
    );
    const { directory, name } = validatedName(nameBytes);
    if (seen.has(name)) {
      fail("ZIP contains duplicate entry names");
    }
    seen.add(name);
    totalUncompressedSize += uncompressedSize;
    if (totalUncompressedSize > MAX_EXTRACTED_BYTES) {
      fail("ZIP entries exceed the bounded extracted-byte budget");
    }

    const creator = versionMadeBy >>> 8;
    const fileType = (externalAttributes >>> 16) & UNIX_FILE_TYPE;
    if (
      creator !== UNIX_CREATOR ||
      fileType === UNIX_SYMLINK ||
      (directory && fileType !== UNIX_DIRECTORY) ||
      (!directory && fileType !== UNIX_REGULAR)
    ) {
      fail("ZIP entry is not a regular file or directory with Unix metadata");
    }
    entries.push({
      compressedSize,
      flags,
      localOffset,
      method,
      nameBytes,
    });
    cursor += entryLength;
  }
  if (cursor !== central.length || !seen.has(`${ROOT}/`)) {
    fail("ZIP central directory has extra data or lacks its application root");
  }
  for (const entry of entries) {
    validateLocalHeader(descriptor, metadata.size, centralOffset, entry);
  }
} finally {
  closeSync(descriptor);
}

if (extractedRoot !== undefined) {
  validateExtractedRoot(extractedRoot);
}

process.stdout.write("Verified isolated unsigned Desktop ZIP structure.\n");
