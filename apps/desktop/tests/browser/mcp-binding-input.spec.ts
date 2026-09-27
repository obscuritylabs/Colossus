import { expect, test } from "@playwright/test";

for (const transport of ["http", "stdio"] as const) {
  test(`MCP ${transport} credential bindings keep focus while typing and retain the correct row after deletion`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto("/?fixture=operations-studio");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("button", { name: "Global", exact: true }).click();
    await page.getByRole("button", { name: "MCP", exact: true }).click();
    await page
      .getByRole("button", { name: "Add MCP server", exact: true })
      .click();
    const editor = page.locator(".mcp-server-editor");
    const name = `focus-test-${transport}`;
    await editor.getByLabel("Server name", { exact: false }).fill(name);
    if (transport === "http") {
      await editor
        .getByRole("combobox", { name: "Transport", exact: true })
        .click();
      await page
        .getByRole("option", { name: "Remote endpoint (HTTP)", exact: true })
        .click();
      await editor
        .getByLabel("Server URL", { exact: true })
        .fill("https://mcp.example.test/rpc");
    } else {
      await editor
        .getByLabel("Executable", { exact: true })
        .fill("example-mcp-server");
    }
    const groupName =
      transport === "http"
        ? "Credential headers"
        : "Environment credential bindings";
    const fieldName = transport === "http" ? "Header" : "Variable";
    const bindingName = transport === "http" ? "Authorization" : "MCP_TOKEN";
    const bindings = editor.getByRole("group", {
      name: groupName,
      exact: true,
    });
    await bindings.getByRole("button", { name: "Add binding" }).click();
    const firstName = bindings
      .getByRole("textbox", { name: fieldName, exact: true })
      .first();
    await firstName.focus();
    await page.keyboard.type(bindingName);
    await expect(firstName).toHaveValue(bindingName);
    await expect(firstName).toBeFocused();
    // Editing in the middle must retain the caret, not merely restore focus at the end.
    await page.keyboard.press(
      process.platform === "darwin" ? "Meta+ArrowLeft" : "Home",
    );
    await page.keyboard.press("ArrowRight");
    expect(
      await firstName.evaluate(
        (input: HTMLInputElement) => input.selectionStart,
      ),
    ).toBe(1);
    await page.keyboard.type("x");
    await expect(firstName).toHaveValue(
      `${bindingName[0]}x${bindingName.slice(1)}`,
    );
    await page.keyboard.press("Backspace");
    await expect(firstName).toHaveValue(bindingName);
    await expect(firstName).toBeFocused();

    await bindings.getByRole("button", { name: "Add binding" }).click();
    const secondName = bindings
      .getByRole("textbox", { name: fieldName, exact: true })
      .nth(1);
    const retainedName = transport === "http" ? "X-Api-Key" : "SERVICE_API_KEY";
    await secondName.focus();
    await page.keyboard.type(retainedName);
    await expect(secondName).toHaveValue(retainedName);
    await expect(secondName).toBeFocused();
    if (transport === "http") {
      const scheme = bindings
        .getByRole("textbox", { name: "Scheme", exact: true })
        .nth(1);
      await scheme.focus();
      await page.keyboard.type("Bearer");
      await expect(scheme).toHaveValue("Bearer");
      await expect(scheme).toBeFocused();
    }
    await bindings
      .getByRole("combobox", { name: "Credential", exact: true })
      .nth(1)
      .click();
    await page
      .getByRole("option", { name: "Documentation API key", exact: true })
      .click();
    const retainedInput = await secondName.elementHandle();
    await bindings
      .getByRole("button", {
        name: `Remove ${fieldName.toLowerCase()} binding 1`,
        exact: true,
      })
      .click();
    expect(await retainedInput!.evaluate((input) => input.isConnected)).toBe(
      true,
    );
    await expect(firstName).toHaveValue(retainedName);
    await expect(
      bindings.getByRole("combobox", { name: "Credential", exact: true }),
    ).toContainText("Documentation API key");
    await editor
      .getByRole("button", { name: "Add server", exact: true })
      .click();
    await expect(editor).toHaveCount(0);
    await page
      .getByRole("button", { name: `Edit ${name}`, exact: true })
      .click();
    await expect(firstName).toHaveValue(retainedName);
    await expect(
      bindings.getByRole("combobox", { name: "Credential", exact: true }),
    ).toContainText("Documentation API key");
    if (transport === "http")
      await expect(
        bindings.getByRole("textbox", { name: "Scheme", exact: true }),
      ).toHaveValue("Bearer");
    await firstName.focus();
    await page.keyboard.press(
      process.platform === "darwin" ? "Meta+ArrowRight" : "End",
    );
    await page.keyboard.type("_EDITED");
    await expect(firstName).toHaveValue(`${retainedName}_EDITED`);
    await expect(firstName).toBeFocused();
  });
}
