import assert from "node:assert/strict";
import {
  chmodSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync, spawnSync } from "node:child_process";
import test from "node:test";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const verifier = join(
  repository,
  "scripts/verify-desktop-unsigned-archive.mjs",
);
const archiveName = "Colossus-Desktop-unsigned-aarch64-apple-darwin.zip";
const rootName = "Colossus Desktop.app/";

function zipArchive(path, entries, options = {}) {
  const localRecords = [];
  const centralRecords = [];
  let localOffset = 0;
  for (const entry of entries) {
    const localName = Buffer.from(entry.localName ?? entry.name, "utf8");
    const centralName = Buffer.from(entry.name, "utf8");
    const localExtra = entry.localExtra ?? Buffer.alloc(0);
    const centralExtra = entry.centralExtra ?? localExtra;
    const local = Buffer.alloc(30 + localName.length + localExtra.length);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt32LE(entry.uncompressedSize ?? 0, 22);
    local.writeUInt16LE(localName.length, 26);
    local.writeUInt16LE(localExtra.length, 28);
    localName.copy(local, 30);
    localExtra.copy(local, 30 + localName.length);
    localRecords.push(local);

    const central = Buffer.alloc(46 + centralName.length + centralExtra.length);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE((3 << 8) | 20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt32LE(entry.uncompressedSize ?? 0, 24);
    central.writeUInt16LE(centralName.length, 28);
    central.writeUInt16LE(centralExtra.length, 30);
    central.writeUInt32LE((entry.mode << 16) >>> 0, 38);
    central.writeUInt32LE(localOffset, 42);
    centralName.copy(central, 46);
    centralExtra.copy(central, 46 + centralName.length);
    centralRecords.push(central);
    localOffset += local.length;
  }
  const central = Buffer.concat(centralRecords);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(central.length, 12);
  eocd.writeUInt32LE(localOffset, 16);
  writeFileSync(
    path,
    Buffer.concat([
      ...localRecords,
      central,
      options.centralGap ?? Buffer.alloc(0),
      eocd,
    ]),
    { mode: 0o600 },
  );
}

function verify(archive, extractedRoot) {
  const arguments_ = [verifier, "--archive", archive];
  if (extractedRoot !== undefined) {
    arguments_.push("--extracted-root", extractedRoot);
  }
  return spawnSync(process.execPath, arguments_, { encoding: "utf8" });
}

test(
  "accepts one ditto app and its symlink-free extraction",
  { skip: process.platform !== "darwin" },
  () => {
    const temporary = realpathSync(
      mkdtempSync(join(tmpdir(), "colossus-unsigned-archive-")),
    );
    try {
      const source = join(temporary, "source");
      const app = join(source, "Colossus Desktop.app");
      const macos = join(app, "Contents", "MacOS");
      mkdirSync(macos, { recursive: true, mode: 0o755 });
      const executable = join(macos, "Colossus Desktop");
      writeFileSync(executable, "desktop", { mode: 0o755 });
      chmodSync(executable, 0o755);
      const archive = join(temporary, archiveName);
      execFileSync("/usr/bin/ditto", [
        "-c",
        "-k",
        "--keepParent",
        "--norsrc",
        "--noextattr",
        app,
        archive,
      ]);
      const archiveResult = verify(archive);
      assert.equal(archiveResult.status, 0, archiveResult.stderr);

      const extracted = join(temporary, "extracted");
      mkdirSync(extracted, { mode: 0o700 });
      execFileSync("/usr/bin/ditto", ["-x", "-k", archive, extracted]);
      const extractionResult = verify(archive, extracted);
      assert.equal(extractionResult.status, 0, extractionResult.stderr);

      const extractedExecutable = join(
        extracted,
        "Colossus Desktop.app",
        "Contents",
        "MacOS",
        "Colossus Desktop",
      );
      const hardLink = join(extracted, "Colossus Desktop.app", "hard-link");
      linkSync(extractedExecutable, hardLink);
      assert.notEqual(verify(archive, extracted).status, 0);
      unlinkSync(hardLink);

      symlinkSync("Contents", join(extracted, "Colossus Desktop.app", "link"));
      assert.notEqual(verify(archive, extracted).status, 0);
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  },
);

test("rejects traversal, absolute, control, duplicate, link, and split names", () => {
  const cases = [
    [
      { name: rootName, mode: 0o040755 },
      { name: `${rootName}../escape`, mode: 0o100644 },
    ],
    [
      { name: rootName, mode: 0o040755 },
      { name: "/absolute", mode: 0o100644 },
    ],
    [
      { name: rootName, mode: 0o040755 },
      { name: `${rootName}bad\nname`, mode: 0o100644 },
    ],
    [
      { name: rootName, mode: 0o040755 },
      { name: rootName, mode: 0o040755 },
    ],
    [
      { name: rootName, mode: 0o040755 },
      { name: `${rootName}link`, mode: 0o120777 },
    ],
    [
      { name: rootName, mode: 0o040755 },
      {
        name: `${rootName}aa`,
        localName: `${rootName}bb`,
        mode: 0o100644,
      },
    ],
  ];
  for (const entries of cases) {
    const temporary = realpathSync(
      mkdtempSync(join(tmpdir(), "colossus-unsigned-archive-invalid-")),
    );
    try {
      const archive = join(temporary, archiveName);
      zipArchive(archive, entries);
      assert.notEqual(verify(archive).status, 0);
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  }
});

test("rejects oversized expansion, path override extras, and central gaps", () => {
  const forbiddenExtraFields = [
    Buffer.from([0x01, 0x00, 0x00, 0x00]),
    Buffer.from([0x75, 0x70, 0x00, 0x00]),
  ];
  const archives = [
    {
      entries: [
        { name: rootName, mode: 0o040755 },
        {
          name: `${rootName}huge`,
          mode: 0o100644,
          uncompressedSize: 0x80000001,
        },
      ],
    },
    ...forbiddenExtraFields.map((centralExtra) => ({
      entries: [
        { name: rootName, mode: 0o040755 },
        {
          centralExtra,
          name: `${rootName}extra`,
          mode: 0o100644,
        },
      ],
    })),
    {
      centralGap: Buffer.from("gap"),
      entries: [{ name: rootName, mode: 0o040755 }],
    },
  ];
  for (const fixture of archives) {
    const temporary = realpathSync(
      mkdtempSync(join(tmpdir(), "colossus-unsigned-archive-bounded-")),
    );
    try {
      const archive = join(temporary, archiveName);
      zipArchive(archive, fixture.entries, {
        centralGap: fixture.centralGap,
      });
      assert.notEqual(verify(archive).status, 0);
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  }
});
