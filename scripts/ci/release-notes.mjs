import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const tagPattern = /^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-preview\.([1-9][0-9]*))?$/u;
const startPattern = /<!-- colossus:release-notes base=(none|v[0-9.]+(?:-preview\.[0-9]+)?) -->/u;
const endMarker = "<!-- /colossus:release-notes -->";
const groups = ["Breaking changes", "Added", "Fixed", "Security", "Changed", "Documentation", "Maintenance", "Other changes"];
const categories = { feat: "Added", fix: "Fixed", security: "Security", perf: "Changed", refactor: "Changed", revert: "Changed", docs: "Documentation" };

function fullMatch(value, pattern, message) {
  assert.equal(typeof value, "string", message);
  const match = value.match(pattern);
  assert.ok(match && match[0] === value, message);
  return match;
}

function parseTag(tag) {
  const match = fullMatch(tag, tagPattern, "Expected vX.Y.Z or vX.Y.Z-preview.N");
  return match.slice(1, 4).map(BigInt);
}

function compareVersion(left, right) {
  for (let i = 0; i < 3; i++) {
    if (left[i] !== right[i]) return left[i] > right[i] ? 1 : -1;
  }
  return 0;
}

function earlierStable(tag, candidate) {
  const comparison = compareVersion(parseTag(candidate), parseTag(tag));
  return !candidate.includes("-preview.") && (comparison < 0 || (comparison === 0 && tag.includes("-preview.")));
}

export function selectPreviousTag(tag, releases, knownTags, reachableTags) {
  parseTag(tag);
  assert.ok(Array.isArray(releases), "Expected GitHub release metadata");
  const candidates = releases.flat().filter((release) =>
    release && release.draft === false && release.prerelease === false && release.published_at &&
    typeof release.tag_name === "string" && release.tag_name.match(tagPattern)?.[0] === release.tag_name &&
    earlierStable(tag, release.tag_name));
  candidates.sort((a, b) => compareVersion(parseTag(b.tag_name), parseTag(a.tag_name)));
  for (const release of candidates) {
    assert.ok(knownTags.has(release.tag_name), "Missing release tag " + release.tag_name + "; fetch complete tag history");
    if (reachableTags.has(release.tag_name)) return release.tag_name;
  }
  return null;
}

function markdown(value) {
  return value.replace(/[\r\n\x00-\x1f\x7f]/gu, " ")
    .replace(/&/gu, "&amp;").replace(/</gu, "&lt;").replace(/>/gu, "&gt;")
    .replace(/([\\\x60*_[\]{}])/gu, "\\$1");
}

function sections(changelog) {
  const found = [];
  let offset = 0;
  let fence = null;
  for (const line of changelog.split(/(?<=\n)/u)) {
    const marker = line.match(/^ {0,3}(\x60{3,}|~{3,})/u)?.[1];
    if (marker) {
      if (!fence) fence = marker;
      else if (marker[0] === fence[0] && marker.length >= fence.length &&
        line.slice(line.indexOf(marker) + marker.length).trim() === "") fence = null;
    } else if (!fence) {
      const heading = line.match(/^## \[([^\]\r\n]+)\][^\r\n]*/u);
      if (heading) found.push({ version: heading[1], start: offset, body: offset + line.length, heading: heading[0] });
    }
    offset += line.length;
  }
  return found.map((section, i) => ({ ...section, end: found[i + 1]?.start ?? changelog.length }));
}

function generatedBase(body) {
  const matches = [...body.matchAll(new RegExp(startPattern.source, "gu"))];
  assert.ok(matches.length <= 1, "Duplicate generated release-note blocks");
  if (!matches.length) {
    assert.ok(!body.includes(endMarker), "Incomplete generated release-note block");
    return undefined;
  }
  assert.ok(body.indexOf(endMarker, matches[0].index) >= 0, "Incomplete generated release-note block");
  assert.equal(body.split(endMarker).length, 2, "Duplicate generated release-note blocks");
  return matches[0][1] === "none" ? null : matches[0][1];
}

export function pinnedBase(changelog, tag) {
  parseTag(tag);
  const matching = sections(changelog).filter((section) => section.version === tag.slice(1));
  assert.ok(matching.length <= 1, "Duplicate changelog version");
  if (!matching.length) return undefined;
  return generatedBase(changelog.slice(matching[0].body, matching[0].end));
}

function curated(body) {
  const base = generatedBase(body);
  if (base === undefined) return body.trim();
  const start = body.search(startPattern);
  const end = body.indexOf(endMarker, start) + endMarker.length;
  return (body.slice(0, start) + body.slice(end)).trim();
}

export function renderHistory({ tag, repository, sourceCommit, date, previousTag, commits, changelog }) {
  parseTag(tag);
  fullMatch(repository, /^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/u, "Expected GitHub owner/repository");
  assert.ok(![".", ".."].includes(repository.split("/")[1]), "Invalid repository");
  fullMatch(sourceCommit, /^[a-f0-9]{40}$/u, "Expected an immutable source commit");
  fullMatch(date, /^\d{4}-\d{2}-\d{2}$/u, "Expected an ISO release date");
  if (previousTag !== null) {
    assert.ok(earlierStable(tag, previousTag), "The baseline must be an earlier stable release");
  }
  const url = "https://github.com/" + repository;
  const entries = new Map(groups.map((group) => [group, []]));
  for (const commit of commits) {
    fullMatch(commit.sha, /^[a-f0-9]{40}$/u, "Invalid commit identifier");
    const match = commit.subject.match(/^([a-z]+)(?:\(([^)\r\n]+)\))?(!)?: (.+)$/u);
    const breaking = commit.body.match(/^BREAKING[ -]CHANGE:\s*(.+)$/mu)?.[1];
    const category = match?.[3] || breaking ? "Breaking changes" :
      match ? categories[match[1]] ?? "Maintenance" : "Other changes";
    const subject = match?.[4] ?? commit.subject;
    const pull = subject.match(/\s+\(#([1-9][0-9]*)\)$/u);
    const description = pull ? subject.slice(0, pull.index) : subject;
    const scope = match?.[2] ? "**" + markdown(match[2]) + ":** " : "";
    const detail = breaking ? " — " + markdown(breaking) : "";
    const link = pull ? " ([#" + pull[1] + "](" + url + "/pull/" + pull[1] + "))" :
      " ([" + commit.sha.slice(0, 7) + "](" + url + "/commit/" + commit.sha + "))";
    entries.get(category).push("- " + scope + markdown(description) + detail + link);
  }
  const generated = [ "<!-- colossus:release-notes base=" + (previousTag ?? "none") + " -->", "### Merged changes" ];
  for (const group of groups) {
    if (entries.get(group).length) generated.push("#### " + group, entries.get(group).join("\n"));
  }
  if (!commits.length) generated.push("No new commits since the previous release.");
  generated.push("**Source:** [" + sourceCommit.slice(0, 7) + "](" + url + "/commit/" + sourceCommit + ")");
  if (previousTag) generated.push("**Full changelog:** [" + previousTag + "..." + tag + "](" + url + "/compare/" + previousTag + "..." + tag + ")");
  generated.push(endMarker);

  const all = sections(changelog);
  const existing = all.filter((section) => section.version === tag.slice(1));
  const unreleased = all.filter((section) => section.version === "Unreleased");
  assert.ok(existing.length <= 1 && unreleased.length <= 1, "Duplicate changelog version");
  assert.ok(all.length > 0, "Changelog must contain release or Unreleased sections");
  const target = existing[0];
  const highlights = target ? curated(changelog.slice(target.body, target.end)) :
    unreleased[0] ? curated(changelog.slice(unreleased[0].body, unreleased[0].end)) : "";
  const heading = target?.heading ?? "## [" + tag.slice(1) + "] - " + date;
  const entry = heading + "\n\n" + (highlights ? highlights + "\n\n" : "") + generated.join("\n\n") + "\n\n";
  let updated;
  if (target) updated = changelog.slice(0, target.start) + entry + changelog.slice(target.end);
  else if (unreleased[0]) {
    const section = unreleased[0];
    updated = changelog.slice(0, section.body) + "\n" + entry + changelog.slice(section.end);
  } else updated = changelog.slice(0, all[0].start) + entry + changelog.slice(all[0].start);
  return { changelog: updated, notes: entry.trimEnd() + "\n" };
}

function command(program, args) {
  const result = spawnSync(program, args, { encoding: "utf8", timeout: 30_000, maxBuffer: 8 * 1024 * 1024 });
  assert.equal(result.status, 0, program + " failed: " + (result.error?.message ?? result.stderr));
  return result.stdout;
}

function git(...args) {
  return command("git", ["--no-pager", ...args]);
}

function options(args) {
  const values = {};
  const fields = new Set(["--tag", "--source", "--repository", "--previous-tag", "--releases", "--changelog", "--changelog-output", "--notes-output"]);
  for (let i = 0; i < args.length; i++) {
    const key = args[i];
    assert.ok(fields.has(key) || key === "--write-changelog", "Unknown option: " + key);
    assert.ok(!(key in values), "Duplicate option: " + key);
    if (key === "--write-changelog") values[key] = true;
    else {
      assert.ok(args[i + 1] && !args[i + 1].startsWith("--"), "Missing value: " + key);
      values[key] = args[++i];
    }
  }
  parseTag(values["--tag"]);
  assert.ok(!(values["--write-changelog"] && values["--changelog-output"]), "Choose one changelog output");
  return values;
}

function main(args) {
  const values = options(args);
  const tag = values["--tag"];
  const repository = values["--repository"] ?? process.env.GITHUB_REPOSITORY ?? "obscuritylabs/Colossus";
  fullMatch(repository, /^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/u, "Expected GitHub owner/repository");
  assert.equal(git("rev-parse", "--is-shallow-repository").trim(), "false", "Fetch complete Git history before generating release notes");
  const sourceCommit = git("rev-parse", "--verify", "--end-of-options", (values["--source"] ?? "HEAD") + "^{commit}").trim();
  assert.equal(sourceCommit, git("rev-parse", "HEAD").trim(), "Check out the exact source commit before generating release history");
  const input = values["--changelog"] ?? "CHANGELOG.md";
  const changelogOutput = values["--write-changelog"] ? input : values["--changelog-output"];
  if (changelogOutput && resolve(changelogOutput) === resolve(input)) {
    assert.equal(git("tag", "--list", tag).trim(), "", "Version tag already exists; prepare a new version or preview notes without rewriting history");
  }
  if (values["--notes-output"]) assert.notEqual(resolve(input), resolve(values["--notes-output"]), "Release notes cannot overwrite the input changelog");
  if (values["--notes-output"] && values["--changelog-output"]) {
    assert.notEqual(resolve(values["--notes-output"]), resolve(values["--changelog-output"]), "Release notes and changelog need separate output files");
  }
  const changelog = readFileSync(input, "utf8");
  let previousTag = values["--previous-tag"] ?? pinnedBase(changelog, tag);
  if (previousTag === undefined) {
    const releases = JSON.parse(values["--releases"] ? readFileSync(values["--releases"], "utf8") :
      command("gh", ["api", "--paginate", "--slurp", "repos/" + repository + "/releases?per_page=100"]));
    const known = new Set(git("tag", "--list", "v*").trim().split("\n"));
    const reachable = new Set(git("tag", "--merged", sourceCommit, "--list", "v*").trim().split("\n"));
    previousTag = selectPreviousTag(tag, releases, known, reachable);
  }
  if (previousTag !== null) {
    assert.ok(earlierStable(tag, previousTag), "The baseline must be an earlier stable release");
    git("merge-base", "--is-ancestor", previousTag, sourceCommit);
  }
  const baseCommit = previousTag ? git("rev-parse", "--verify", "--end-of-options", previousTag + "^{commit}").trim() : null;
  const log = git("log", "--no-merges", "--reverse", "--format=%H%x00%s%x00%b%x00", baseCommit ? baseCommit + ".." + sourceCommit : sourceCommit).split("\0");
  const commits = [];
  for (let i = 0; i + 2 < log.length; i += 3) commits.push({ sha: log[i].trim(), subject: log[i + 1], body: log[i + 2] });
  const epoch = git("show", "-s", "--format=%ct", sourceCommit).trim();
  assert.match(epoch, /^\d+$/u, "Invalid source timestamp");
  const date = new Date(Number(epoch) * 1000).toISOString().slice(0, 10);
  const history = renderHistory({ tag, repository, sourceCommit, date, previousTag, commits, changelog });
  for (const [path, content] of [
    [changelogOutput, history.changelog],
    [values["--notes-output"], history.notes],
  ]) {
    if (path) {
      mkdirSync(dirname(resolve(path)), { recursive: true });
      writeFileSync(path, content);
    }
  }
  if (!values["--write-changelog"] && !values["--changelog-output"] && !values["--notes-output"]) process.stdout.write(history.notes);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { main(process.argv.slice(2)); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
