import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const css = readFileSync(
  new URL("../styles/theme.css", import.meta.url),
  "utf8",
);
const hacker = css.match(
  /\[data-colossus-theme="dark"\]\[data-palette="hacker"\]\s*\{([^}]+)\}/u,
)?.[1];
const tokens = Object.fromEntries(
  [...hacker.matchAll(/(--[\w-]+):\s*(#[\da-f]{6});/gu)].map(
    ([, key, value]) => [key, value],
  ),
);

test("graphical Hacker semantic colors follow the authoritative TUI palette", () => {
  const rust = readFileSync(
    new URL(
      "../../../crates/colossus-presentation/src/palette.rs",
      import.meta.url,
    ),
    "utf8",
  );
  const palette = rust.match(/ThemeName::Hacker => Self \{([^}]+)\}/u)?.[1];
  for (const [role, token] of [
    ["indicator", "--blue"],
    ["assistant", "--text"],
    ["meta", "--muted"],
    ["activity", "--faint"],
    ["tool", "--cyan"],
    ["success", "--green"],
    ["warning", "--amber"],
    ["error", "--red"],
  ]) {
    const rgb = palette.match(
      new RegExp(`${role}:.*?RgbColor::new\\((\\d+), (\\d+), (\\d+)\\)`, "u"),
    );
    assert.ok(rgb, `TUI ${role} must exist`);
    const hex = `#${rgb
      .slice(1)
      .map((value) => Number(value).toString(16).padStart(2, "0"))
      .join("")}`;
    assert.equal(tokens[token], hex, `${token} must follow TUI ${role}`);
  }
});

function luminance(hex) {
  const channels = hex
    .slice(1)
    .match(/../gu)
    .map((value) => parseInt(value, 16) / 255);
  const [r, g, b] = channels.map((value) =>
    value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4,
  );
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
function contrast(a, b) {
  const values = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (values[0] + 0.05) / (values[1] + 0.05);
}
test("Hacker keeps messages, metadata, statuses and accent controls readable", () => {
  for (const background of [
    "--main",
    "--surface",
    "--surface-hover",
    "--surface-selected",
  ]) {
    for (const foreground of [
      "--text",
      "--muted",
      "--faint",
      "--cyan",
      "--green",
      "--amber",
      "--red",
    ]) {
      assert.ok(
        contrast(tokens[foreground], tokens[background]) >= 4.5,
        `${foreground} on ${background} must meet normal-text contrast`,
      );
    }
  }
  assert.ok(contrast(tokens["--text-on-accent"], tokens["--blue"]) >= 4.5);
});
