import { expect, test } from "@playwright/test";

for (const path of ["/", "/explorer", "/inspector", "/settings"]) {
  test(`Hacker reaches ${path} and respects host light and high contrast themes`, async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (
        window as unknown as { acquireVsCodeApi: () => unknown }
      ).acquireVsCodeApi = () => ({
        postMessage() {},
        getState() {},
        setState() {},
      });
    });
    await page.goto(path);
    const token = (name: string) =>
      page
        .locator("html")
        .evaluate(
          (root, name) => getComputedStyle(root).getPropertyValue(name).trim(),
          name,
        );
    await page.evaluate(() =>
      window.postMessage({ type: "palette", palette: "hacker" }, "*"),
    );
    await expect(page.locator("html")).toHaveAttribute(
      "data-palette",
      "hacker",
    );
    await expect.poll(() => token("--blue")).toBe("#00ff66");
    await expect.poll(() => token("--main")).toBe("#080d0a");
    await page.evaluate(() => {
      document.body.className = "vscode-light";
    });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect.poll(() => token("--blue")).toBe("#2563d9");
    for (const theme of [
      "vscode-high-contrast",
      "vscode-high-contrast-light",
    ]) {
      await page.evaluate((theme) => {
        document.body.className = theme;
        document.documentElement.style.setProperty(
          "--vscode-focusBorder",
          "#ff00ff",
        );
        document.documentElement.style.setProperty(
          "--vscode-editor-background",
          "#000000",
        );
      }, theme);
      await expect(page.locator("html")).toHaveAttribute(
        "data-theme",
        "high-contrast",
      );
      await expect.poll(() => token("--blue-strong")).toBe("#ff00ff");
      await expect.poll(() => token("--main")).toBe("#000000");
    }
    await page.evaluate(() => {
      document.body.className = "vscode-dark";
    });
    await expect.poll(() => token("--blue")).toBe("#00ff66");
    await page.evaluate(() =>
      window.postMessage({ type: "palette", palette: "arbitrary-css" }, "*"),
    );
    await expect(page.locator("html")).toHaveAttribute(
      "data-palette",
      "hacker",
    );
    await expect.poll(() => token("--blue")).toBe("#00ff66");
  });
}
