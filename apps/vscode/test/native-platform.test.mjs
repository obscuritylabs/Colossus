import assert from "node:assert/strict";
import { test } from "node:test";
import { assertNativeTarget } from "../scripts/native-platform.mjs";

test("native staging rejects cross-platform, cross-CPU and truncated binaries", () => {
  const pe = Buffer.alloc(256);
  pe.writeUInt16LE(0x5a4d);
  pe.writeUInt32LE(128, 60);
  pe.writeUInt32LE(0x4550, 128);
  pe.writeUInt16LE(0x8664, 132);
  assert.doesNotThrow(() => assertNativeTarget(pe, "win32-x64"));
  assert.throws(() => assertNativeTarget(pe, "win32-arm64"));
  assert.throws(() => assertNativeTarget(pe, "linux-x64"));
  pe.writeUInt32LE(0xffffffff, 60);
  assert.throws(() => assertNativeTarget(pe, "win32-x64"));
  const elf = Buffer.alloc(64);
  elf.set([0x7f, 0x45, 0x4c, 0x46, 2, 1]);
  elf.writeUInt16LE(183, 18);
  assert.doesNotThrow(() => assertNativeTarget(elf, "linux-arm64"));
  assert.throws(() => assertNativeTarget(elf, "linux-x64"));
  const macho = Buffer.alloc(32);
  macho.writeUInt32LE(0xfeedfacf);
  macho.writeUInt32LE(0x01000007, 4);
  assert.doesNotThrow(() => assertNativeTarget(macho, "darwin-x64"));
  assert.throws(() => assertNativeTarget(macho, "darwin-arm64"));
  assert.throws(() => assertNativeTarget(Buffer.alloc(0), "win32-x64"));
});
