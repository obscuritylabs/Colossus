import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  rm,
  symlink,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
  candidateNames,
  createDocumentationCandidate,
  documentationImage,
  matchingDocumentationSourceRun,
  verifyDocumentationCandidate,
  verifyDocumentationIndex,
  verifyDocumentationSourceRun,
  verifyPublishedDocumentationRelease,
} from "./documentation-candidate.mjs";

const tag = "v0.11.7-preview.3";
const commit = "a".repeat(40);
const digest = (letter) => `sha256:${letter.repeat(64)}`;

async function imageArchive(directory, architecture, user = "10001:10001") {
  const names = candidateNames(tag, architecture);
  const stage = join(directory, `stage-${architecture}`);
  await mkdir(stage, { recursive: true });
  const config = Buffer.from(
    JSON.stringify({
      os: "linux",
      architecture,
      config: {
        User: user,
        Labels: {
          "org.opencontainers.image.version": tag,
          "org.opencontainers.image.revision": commit,
        },
      },
    }),
  );
  const filename = createHash("sha256").update(config).digest("hex") + ".json";
  await writeFile(join(stage, filename), config);
  await writeFile(
    join(stage, "manifest.json"),
    JSON.stringify([
      { Config: filename, RepoTags: [documentationImage], Layers: [] },
    ]),
  );
  execFileSync("tar", [
    "-czf",
    join(directory, names.archive),
    "-C",
    stage,
    "manifest.json",
    filename,
  ]);
  return names;
}

test("tested image bytes and exact source identity survive the retained candidate round trip", async () => {
  const directory = await mkdtemp(
    join(tmpdir(), "colossus-documentation-candidate-"),
  );
  try {
    for (const architecture of ["amd64", "arm64"]) {
      const names = await imageArchive(directory, architecture);
      const original = await createDocumentationCandidate(
        directory,
        tag,
        commit,
        architecture,
      );
      assert.deepEqual(
        await verifyDocumentationCandidate(
          directory,
          tag,
          commit,
          architecture,
        ),
        original,
      );
      await assert.rejects(
        createDocumentationCandidate(directory, tag, commit, architecture),
        { code: "EEXIST" },
      );
      await assert.rejects(
        verifyDocumentationCandidate(
          directory,
          tag,
          "b".repeat(40),
          architecture,
        ),
        /tag source/u,
      );
      const bytes = await readFile(join(directory, names.archive));
      await writeFile(
        join(directory, names.archive),
        Buffer.concat([bytes, Buffer.from("substitution")]),
      );
      await assert.rejects(
        verifyDocumentationCandidate(directory, tag, commit, architecture),
        /tested candidate bytes/u,
      );
      await writeFile(join(directory, names.archive), bytes);
      await writeFile(
        join(directory, names.manifest),
        JSON.stringify({ ...original, config_digest: digest("d") }),
      );
      await assert.rejects(
        verifyDocumentationCandidate(directory, tag, commit, architecture),
        /config digest/u,
      );
      await writeFile(
        join(directory, names.manifest),
        JSON.stringify({
          ...original,
          archive: { ...original.archive, filename: "../../image.tar.gz" },
        }),
      );
      await assert.rejects(
        verifyDocumentationCandidate(directory, tag, commit, architecture),
        /tag source/u,
      );
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("root images, aliases, symlinks and malformed candidate requests cannot reach publication", async () => {
  assert.throws(() => candidateNames("../../secret", "amd64"));
  assert.throws(() => candidateNames("v0.11.7-preview.0", "amd64"));
  assert.throws(() => candidateNames(tag, "unknown"));
  const directory = await mkdtemp(
    join(tmpdir(), "colossus-documentation-invalid-"),
  );
  try {
    const names = await imageArchive(directory, "amd64", "0:0");
    await assert.rejects(
      createDocumentationCandidate(directory, tag, commit, "amd64"),
      /release identity/u,
    );
    await rm(join(directory, names.archive));
    await writeFile(join(directory, "elsewhere"), "not an archive");
    await symlink(join(directory, "elsewhere"), join(directory, names.archive));
    await assert.rejects(
      createDocumentationCandidate(directory, tag, commit, "amd64"),
      /regular archive/u,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("source proof requires successful exact tag runs from this repository and a published release", () => {
  const release = {
    id: 42,
    tag_name: tag,
    draft: false,
    prerelease: true,
    published_at: "2026-10-06T12:00:00Z",
  };
  assert.equal(
    verifyPublishedDocumentationRelease(release, tag).release_id,
    42,
  );
  for (const overrides of [
    { draft: true },
    { prerelease: false },
    { published_at: null },
    { tag_name: "v0.11.7" },
  ])
    assert.throws(
      () =>
        verifyPublishedDocumentationRelease({ ...release, ...overrides }, tag),
      /published exact-tag/u,
    );
  for (const workflow of ["release.yml", "documentation-candidate.yml"]) {
    const run = {
      id: 43,
      event: "push",
      status: "completed",
      conclusion: "success",
      head_sha: commit,
      head_branch: tag,
      path: `.github/workflows/${workflow}`,
      repository: { full_name: "obscuritylabs/Colossus" },
      head_repository: { full_name: "obscuritylabs/Colossus" },
    };
    assert.equal(
      matchingDocumentationSourceRun([run], workflow, tag, commit).run_id,
      43,
    );
    for (const overrides of [
      { event: "workflow_dispatch" },
      { head_sha: "b".repeat(40) },
      { conclusion: "failure" },
      { head_branch: "main" },
      { path: ".github/workflows/arbitrary.yml" },
      { head_repository: { full_name: "someone/fork" } },
    ])
      assert.throws(
        () =>
          verifyDocumentationSourceRun(
            { ...run, ...overrides },
            workflow,
            tag,
            commit,
          ),
        /exact-tag repository source/u,
      );
    assert.throws(
      () => matchingDocumentationSourceRun([], workflow, tag, commit),
      /No successful/u,
    );
  }
});

test("an existing documentation index is reusable only for the two exact tested executables", () => {
  const variants = { amd64: digest("c"), arm64: digest("d") };
  const index = {
    schemaVersion: 2,
    mediaType: "application/vnd.oci.image.index.v1+json",
    digest: digest("a"),
    manifests: [
      {
        digest: variants.amd64,
        platform: { os: "linux", architecture: "amd64" },
      },
      {
        digest: variants.arm64,
        platform: { os: "linux", architecture: "arm64", variant: "v8" },
      },
    ],
  };
  assert.equal(verifyDocumentationIndex(index, variants), index.digest);
  assert.throws(
    () => verifyDocumentationIndex(index, { ...variants, arm64: digest("e") }),
    /conflicting/u,
  );
  assert.throws(
    () =>
      verifyDocumentationIndex(
        { ...index, manifests: [index.manifests[0], index.manifests[0]] },
        variants,
      ),
    /conflicting/u,
  );
  assert.throws(
    () =>
      verifyDocumentationIndex(
        {
          ...index,
          manifests: [
            ...index.manifests,
            {
              digest: digest("e"),
              platform: { os: "unknown", architecture: "unknown" },
            },
          ],
        },
        variants,
      ),
    /exactly/u,
  );
});

test("publication contracts and write authority stay separate from tag and PR candidate code", async () => {
  const candidate = await readFile(
    new URL(
      "../../.github/workflows/documentation-candidate.yml",
      import.meta.url,
    ),
    "utf8",
  );
  const publisher = await readFile(
    new URL("../../.github/workflows/documentation-image.yml", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(candidate, /packages:\s*write|docker login|secrets\./u);
  assert.match(
    candidate,
    /node deploy\/documentation\/smoke\.mjs colossus-documentation:candidate/u,
  );
  assert.match(candidate, /if: github\.event_name == 'push'/u);
  assert.match(candidate, /if \[\[ "\$EVENT_NAME" == push \]\]; then/u);
  assert.equal(
    publisher.match(
      /ref: \$\{\{ needs\.publisher-revision\.outputs\.revision \}\}/gu,
    )?.length,
    2,
  );
  assert.equal(publisher.match(/packages: write/gu)?.length, 1);
  assert.match(publisher, /github\.ref == 'refs\/heads\/main'/u);
  assert.match(publisher, /needs: \[publisher-revision, contracts\]/u);
});
