import assert from "node:assert/strict";

// VS Code viewsExtensionPoint rejects dotted container IDs, even though dots are
// valid in view IDs. Missing containers send their views into Explorer instead.
export function validateViewContributions(manifest) {
  const contributed = manifest.contributes ?? {};
  const containers = new Set();
  for (const [location, entries] of Object.entries(
    contributed.viewsContainers ?? {},
  )) {
    assert.ok(
      ["activitybar", "secondarySidebar", "panel"].includes(location),
      `Unsupported view container location: ${location}`,
    );
    assert.ok(
      Array.isArray(entries),
      `View containers at ${location} must be an array`,
    );
    for (const container of entries) {
      assert.ok(
        typeof container.id === "string" &&
          /^[a-zA-Z0-9_-]+$/.test(container.id),
        `Invalid view container ID: ${container.id}. VS Code permits only letters, digits, underscores, and hyphens.`,
      );
      assert.ok(
        typeof container.title === "string" && container.title.trim(),
        "View container title is required",
      );
      assert.ok(
        typeof container.icon === "string" && container.icon.trim(),
        "View container icon is required",
      );
      assert.ok(
        !containers.has(container.id),
        `Duplicate view container: ${container.id}`,
      );
      containers.add(container.id);
    }
  }
  const standard = new Set(["explorer", "debug", "scm", "remote"]);
  const views = new Set();
  for (const [container, entries] of Object.entries(contributed.views ?? {})) {
    assert.ok(
      containers.has(container) || standard.has(container),
      `View targets an undeclared container: ${container}`,
    );
    assert.ok(
      Array.isArray(entries),
      `Views for ${container} must be an array`,
    );
    for (const view of entries) {
      assert.ok(
        typeof view.id === "string" && view.id.trim(),
        "View ID is required",
      );
      assert.ok(!views.has(view.id), `Duplicate view ID: ${view.id}`);
      views.add(view.id);
    }
  }
}
