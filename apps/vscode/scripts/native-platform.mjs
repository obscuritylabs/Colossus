/** Verify the executable format and CPU before assigning a VSIX target label. */
export function assertNativeTarget(header, target) {
  const [platform, architecture] = target.split("-");
  if (!["x64", "arm64"].includes(architecture))
    throw new Error("Unsupported VSIX CPU.");
  let valid = false;
  if (
    platform === "win32" &&
    header.length >= 64 &&
    header.readUInt16LE(0) === 0x5a4d
  ) {
    const offset = header.readUInt32LE(60);
    valid =
      offset + 6 <= header.length &&
      header.readUInt32LE(offset) === 0x4550 &&
      header.readUInt16LE(offset + 4) ===
        (architecture === "x64" ? 0x8664 : 0xaa64);
  } else if (platform === "linux" && header.length >= 20) {
    valid =
      header.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46])) &&
      header[4] === 2 &&
      header[5] === 1 &&
      header.readUInt16LE(18) === (architecture === "x64" ? 62 : 183);
  } else if (platform === "darwin" && header.length >= 8) {
    valid =
      header.readUInt32LE(0) === 0xfeedfacf &&
      header.readUInt32LE(4) ===
        (architecture === "x64" ? 0x01000007 : 0x0100000c);
  }
  if (!valid)
    throw new Error(
      "Native executable does not match the VSIX platform and CPU.",
    );
}
