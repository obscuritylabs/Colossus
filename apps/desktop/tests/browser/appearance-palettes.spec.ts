import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("light and dark colors preview, apply, persist, and reset independently", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Appearance", exact: true }).click();

  const colorTheme = page.getByRole("combobox", { name: /Color theme/u });
  const chooseTheme = async (theme: "Light" | "Dark") => {
    await colorTheme.click();
    await page
      .getByRole("listbox")
      .getByRole("option", { name: theme })
      .click();
  };
  const color = (name: string) =>
    page.locator(`input[type="color"][aria-label="${name}"]`);

  await chooseTheme("Light");
  await color("Light accent color").fill("#b94171");
  await color("Light background color").fill("#e8f0f2");
  await color("Light surface color").fill("#fff6e6");
  await color("Light icons color").fill("#a1366f");
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--blue")),
    )
    .toBe("#b94171");
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--main")),
    )
    .toBe("#e8f0f2");
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--icon-text")),
    )
    .toBe("#a1366f");
  await expect(page.locator(".sidebar-nav-item > svg").first()).toHaveCSS(
    "color",
    "rgb(161, 54, 111)",
  );

  await page.getByRole("button", { name: "Dark colors" }).click();
  await color("Dark accent color").fill("#26c6a6");
  await color("Dark background color").fill("#121827");
  await color("Dark surface color").fill("#20283a");
  await color("Dark icons color").fill("#50d9a8");
  await expect(page.getByLabel("dark theme preview")).toHaveCSS(
    "background-color",
    "rgb(18, 24, 39)",
  );
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--blue")),
    )
    .toBe("#b94171");

  await chooseTheme("Dark");
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--blue")),
    )
    .toBe("#26c6a6");
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--main")),
    )
    .toBe("#121827");
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--icon-text")),
    )
    .toBe("#50d9a8");
  const accessibility = await new AxeBuilder({ page })
    .include(".appearance-settings-card")
    .analyze();
  expect(
    accessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);

  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Appearance", exact: true }).click();
  await expect(color("Dark accent color")).toHaveValue("#26c6a6");
  await expect(color("Dark icons color")).toHaveValue("#50d9a8");
  await page.getByRole("button", { name: "Light colors" }).click();
  await expect(color("Light accent color")).toHaveValue("#b94171");
  await expect(color("Light icons color")).toHaveValue("#a1366f");
  await color("Light background color").fill("#000000");
  await expect(
    page.locator(".appearance-palette-editor").getByRole("status"),
  ).toContainText("Choose a lighter background color");
  await expect(color("Light background color")).toHaveValue("#e8f0f2");
  await color("Light icons color").fill("#ffffff");
  await expect(
    page.locator(".appearance-palette-editor").getByRole("status"),
  ).toContainText("Choose an icon color");
  await expect(color("Light icons color")).toHaveValue("#a1366f");

  await page.getByRole("button", { name: "Dark colors" }).click();
  await page.getByRole("button", { name: "Reset dark colors" }).click();
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--blue")),
    )
    .toBe("");
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--icon-text")),
    )
    .toBe("");
  await page.getByRole("button", { name: "Light colors" }).click();
  await expect(color("Light accent color")).toHaveValue("#b94171");

  await colorTheme.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "System" })
    .click();
  await page.emulateMedia({ colorScheme: "light" });
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--blue")),
    )
    .toBe("#b94171");
  await page.emulateMedia({ colorScheme: "dark" });
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) => root.style.getPropertyValue("--blue")),
    )
    .toBe("");
});
