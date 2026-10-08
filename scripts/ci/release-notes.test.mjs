import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { pinnedBase, renderHistory, selectPreviousTag } from "./release-notes.mjs";

const sha = "a".repeat(40);
const oldHistory = "## [1.0.0] - 2026-09-01\n\n### Added\n\n- Earlier release.\n";
const original = "# Changelog\n\nIntroduction.\n\n## [Unreleased]\n\n### Added\n\n- Curated highlight.\n\n" + oldHistory;
const release = (tag_name, overrides = {}) => ({
  tag_name, draft: false, prerelease: false, published_at: "2026-09-01T00:00:00Z", ...overrides,
});
const render = (overrides = {}) => renderHistory({
  tag: "v1.1.0", repository: "example/project", sourceCommit: sha, date: "2026-10-08",
  previousTag: "v1.0.0", changelog: original,
  commits: [{ sha, subject: "feat(research): inherit MCP tools (#273)", body: "" }],
  ...overrides,
});

test("only published, reachable stable releases establish the baseline", () => {
  const known = new Set(["v1.0.0", "v1.0.1", "v1.0.2", "v1.0.3"]);
  const releases = [
    release("v1.0.0"),
    release("v1.0.1", { draft: true }),
    release("v1.0.2"),
    release("v1.0.3", { prerelease: true }),
    release("v1.0.4", { published_at: null }),
    release("v1.1.0-preview.9", { prerelease: true }),
    release("v2.0.0"),
  ];
  assert.equal(selectPreviousTag("v1.1.0", [releases], known, new Set(["v1.0.0"])), "v1.0.0");
  assert.equal(selectPreviousTag("v1.1.0", [], known, known), null);
});

test("stable notes include the entire interval and test previews may start at their stable source release", () => {
  const tags = new Set(["v1.0.0", "v1.1.0"]);
  const releases = [release("v1.0.0"), release("v1.1.0")];
  assert.equal(selectPreviousTag("v1.1.0", releases, tags, tags), "v1.0.0");
  assert.equal(selectPreviousTag("v1.1.0-preview.2", releases, tags, tags), "v1.1.0");
  assert.throws(() => selectPreviousTag("v01.1.0", releases, tags, tags));
  assert.throws(() => selectPreviousTag("v1.1.0", releases, new Set(), tags), /fetch complete tag history/u);
});

test("Conventional Commits produce grouped, linked notes including breaking changes", () => {
  const { notes } = render({ commits: [
    { sha, subject: "feat(ui): choose MCP sources (#7)", body: "" },
    { sha, subject: "fix: reconnect reliably", body: "" },
    { sha, subject: "feat(api)!: remove old calls", body: "" },
    { sha, subject: "refactor: change response shape", body: "BREAKING CHANGE: callers must send a role" },
    { sha, subject: "security: bind workspace grants", body: "" },
    { sha, subject: "perf: cache catalogs", body: "" },
    { sha, subject: "docs: describe tool selection", body: "" },
    { sha, subject: "ci: retain evidence", body: "" },
    { sha, subject: "An older nonconventional change", body: "" },
  ] });
  for (const group of ["Breaking changes", "Added", "Fixed", "Security", "Changed", "Documentation", "Maintenance", "Other changes"]) {
    assert.ok(notes.includes("#### " + group), group);
  }
  assert.ok(notes.includes("[#7](https://github.com/example/project/pull/7)"));
  assert.ok(notes.includes("/commit/" + sha));
  assert.ok(notes.includes("callers must send a role"));
  assert.ok(notes.includes("/compare/v1.0.0...v1.1.0"));
});

test("a new entry moves curated highlights and preserves all historical text", () => {
  const result = render();
  assert.ok(result.changelog.startsWith("# Changelog\n\nIntroduction.\n\n## [Unreleased]\n\n## [1.1.0]"));
  assert.equal(result.changelog.split("Curated highlight.").length, 2);
  assert.ok(result.notes.includes("Curated highlight."));
  assert.ok(result.changelog.endsWith(oldHistory));
  assert.ok(result.changelog.includes(result.notes));
  assert.equal(pinnedBase(result.changelog, "v1.1.0"), "v1.0.0");
});

test("regeneration preserves manual release edits and leaves future Unreleased changes intact", () => {
  const first = render();
  const edited = first.changelog.replace("## [Unreleased]\n\n", "## [Unreleased]\n\n- Future work.\n\n")
    .replace("### Added\n\n- Curated highlight.", "### Added\n\n- Reviewed migration advice.\n- Curated highlight.");
  const second = render({ changelog: edited });
  assert.ok(second.notes.includes("Reviewed migration advice."));
  assert.ok(!second.notes.includes("Future work."));
  assert.equal(second.changelog.split("inherit MCP tools").length, 2);
  assert.equal(render({ changelog: second.changelog }).changelog, second.changelog);
  assert.ok(second.changelog.endsWith(oldHistory));
});

test("code examples are preserved rather than interpreted as version headings", () => {
  const example = "\x60\x60\x60md\n## [1.1.0]\n\x60\x60\x60\n";
  const result = render({ changelog: original.replace("- Curated highlight.", "- Curated highlight.\n\n" + example) });
  assert.ok(result.notes.includes(example));
  assert.ok(result.notes.startsWith("## [1.1.0] - 2026-10-08"));
});

test("malformed generated blocks and duplicate versions fail before rewriting history", () => {
  const first = render().changelog;
  assert.throws(() => render({ changelog: first.replace("<!-- /colossus:release-notes -->", "") }), /Incomplete/u);
  assert.throws(() => render({ changelog: first + "\n## [1.1.0]\n\nDuplicate.\n" }), /Duplicate/u);
  assert.throws(() => render({ changelog: "# Changelog\n\nNo release headings.\n" }));
  assert.throws(() => render({ previousTag: "v1.1.0" }), /earlier stable/u);
  assert.throws(() => render({ previousTag: "main" }));
  assert.throws(() => render({ repository: "example/../project" }));
  assert.throws(() => render({ repository: "example/project\n" }));
  assert.throws(() => render({ tag: "v1.1.0\n" }));
  assert.throws(() => render({ sourceCommit: sha + "\n" }));
});

test("commit text cannot inject Markdown headings, HTML, or control characters", () => {
  const { notes } = render({ commits: [
    { sha, subject: "feat: [unsafe](javascript:alert(1)) <script>\n## [99.0.0]\x00", body: "" },
  ] });
  assert.ok(!notes.includes("<script>"));
  assert.ok(!notes.includes("\n## [99.0.0]"));
  assert.ok(notes.includes("\\[unsafe\\]"));
  assert.ok(notes.includes("&lt;script&gt;"));
  assert.ok(!notes.includes("\x00"));
});

test("first and unchanged releases retain useful source evidence", () => {
  const first = render({ previousTag: null });
  assert.equal(pinnedBase(first.changelog, "v1.1.0"), null);
  assert.ok(!first.notes.includes("/compare/"));
  const empty = render({ commits: [] });
  assert.ok(empty.notes.includes("No new commits"));
  assert.ok(empty.notes.includes("/commit/" + sha));
});

function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "colossus-release-notes-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const git = (...args) => {
    const result = spawnSync("git", args, { cwd: directory, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  git("init", "--quiet");
  const commit = (message) => git("-c", "user.name=Release fixture", "-c", "user.email=fixture@example.test", "commit", "--quiet", "--allow-empty", "-m", message);
  commit("feat: initial release");
  git("-c", "user.name=Release fixture", "-c", "user.email=fixture@example.test", "tag", "-a", "v1.0.0", "-m", "release");
  const base = git("rev-parse", "HEAD");
  commit("feat(research): enabled MCP inheritance (#273)");
  const head = git("rev-parse", "HEAD");
  writeFileSync(join(directory, "CHANGELOG.md"), original);
  writeFileSync(join(directory, "releases.json"), JSON.stringify([release("v1.0.0")]));
  const script = fileURLToPath(new URL("./release-notes.mjs", import.meta.url));
  const run = (...args) => spawnSync(process.execPath, [script, ...args], { cwd: directory, encoding: "utf8" });
  return { directory, base, head, run };
}

test("the CLI writes both files from actual Git history and preserves the pinned baseline on reruns", (t) => {
  const { directory, head, run } = fixture(t);
  const generated = run("--tag", "v1.1.0", "--repository", "example/project", "--releases", "releases.json",
    "--write-changelog", "--notes-output", "candidate/RELEASE_NOTES.md");
  assert.equal(generated.status, 0, generated.stderr);
  const changelog = readFileSync(join(directory, "CHANGELOG.md"), "utf8");
  const notes = readFileSync(join(directory, "candidate/RELEASE_NOTES.md"), "utf8");
  assert.ok(notes.includes("enabled MCP inheritance"));
  assert.ok(!notes.includes("initial release"));
  assert.ok(notes.includes(head));
  assert.ok(changelog.includes(notes));
  const repeated = run("--tag", "v1.1.0", "--repository", "example/project", "--releases", "missing.json", "--write-changelog");
  assert.equal(repeated.status, 0, repeated.stderr);
  assert.equal(readFileSync(join(directory, "CHANGELOG.md"), "utf8"), changelog);
});

test("shallow history fails before overwriting the changelog", (t) => {
  const { directory } = fixture(t);
  const shallow = join(directory, "shallow");
  const cloned = spawnSync("git", ["clone", "--quiet", "--depth=1", pathToFileURL(directory).href, shallow], { encoding: "utf8" });
  assert.equal(cloned.status, 0, cloned.stderr);
  writeFileSync(join(shallow, "CHANGELOG.md"), original);
  const script = fileURLToPath(new URL("./release-notes.mjs", import.meta.url));
  const rejected = spawnSync(process.execPath, [script, "--tag", "v1.1.0", "--previous-tag", "v1.0.0", "--write-changelog"], {
    cwd: shallow, encoding: "utf8",
  });
  assert.equal(rejected.status, 1);
  assert.match(rejected.stderr, /Fetch complete Git history/u);
  assert.equal(readFileSync(join(shallow, "CHANGELOG.md"), "utf8"), original);
});

test("invalid CLI requests never overwrite the changelog", (t) => {
  const { directory, base, run } = fixture(t);
  for (const args of [
    ["--tag", "latest"],
    ["--tag", "v1.1.0\n"],
    ["--tag", "v1.1.0", "--tag", "v1.2.0"],
    ["--tag", "v1.1.0", "--source", base],
    ["--tag", "v1.0.0"],
    ["--tag", "v1.1.0", "--notes-output", "CHANGELOG.md"],
    ["--tag", "v1.1.0", "--previous-tag", "v1.1.0"],
    ["--tag", "v1.1.0", "--unknown", "anything"],
  ]) {
    const rejected = run(...args, "--write-changelog");
    assert.equal(rejected.status, 1, args.join(" "));
    assert.equal(rejected.stdout, "");
    assert.equal(readFileSync(join(directory, "CHANGELOG.md"), "utf8"), original);
  }
});
