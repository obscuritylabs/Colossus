import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import {
  readSavedImageConfig,
  validateRequest,
} from "./verify-control-plane-assets.mjs";

export const documentationImage = "colossus-documentation:candidate";
export const documentationArchitectures = ["amd64", "arm64"];
const digestPattern = /^sha256:[a-f0-9]{64}$/u;
const indexMediaTypes = new Set([
  "application/vnd.oci.image.index.v1+json",
  "application/vnd.docker.distribution.manifest.list.v2+json",
]);

export function candidateNames(tag, architecture) {
  validateRequest(tag);
  if (!documentationArchitectures.includes(architecture))
    throw new Error("Unsupported documentation architecture.");
  const name = `Colossus-Documentation-${tag}-linux-${architecture}`;
  return { archive: `${name}.docker.tar.gz`, manifest: `${name}.json` };
}

export function verifyPublishedDocumentationRelease(release, tag) {
  validateRequest(tag);
  if (
    release?.tag_name !== tag ||
    release.draft !== false ||
    release.prerelease !== tag.includes("-preview.") ||
    !Number.isSafeInteger(release.id) ||
    release.id < 1 ||
    typeof release.published_at !== "string" ||
    !Number.isFinite(Date.parse(release.published_at))
  )
    throw new Error(
      "A published exact-tag stable or preview release is required.",
    );
  return { release_id: release.id, tag, published_at: release.published_at };
}

export function verifyDocumentationSourceRun(run, workflow, tag, commit) {
  validateRequest(tag, commit);
  if (!["release.yml", "documentation-candidate.yml"].includes(workflow))
    throw new Error("Unsupported source proof workflow.");
  if (
    !Number.isSafeInteger(run?.id) ||
    run.id < 1 ||
    run.event !== "push" ||
    run.status !== "completed" ||
    run.conclusion !== "success" ||
    run.head_sha !== commit ||
    run.head_branch !== tag ||
    run.path !== `.github/workflows/${workflow}` ||
    run.repository?.full_name !== "obscuritylabs/Colossus" ||
    run.head_repository?.full_name !== "obscuritylabs/Colossus"
  )
    throw new Error(
      "Successful exact-tag repository source proof is required.",
    );
  return {
    run_id: run.id,
    workflow,
    event: run.event,
    tag,
    source_commit: commit,
  };
}

export function matchingDocumentationSourceRun(runs, workflow, tag, commit) {
  if (!Array.isArray(runs) || runs.length > 100)
    throw new Error("Source proof results are unavailable or unbounded.");
  for (const run of runs) {
    try {
      return verifyDocumentationSourceRun(run, workflow, tag, commit);
    } catch {
      // Other completed tag builds are not proof for this exact source request.
    }
  }
  throw new Error("No successful exact-tag source build is available.");
}

async function archiveDigest(path) {
  const file = await lstat(path);
  if (!file.isFile() || file.size < 1 || file.size > 512 * 1024 * 1024)
    throw new Error("Documentation image must be a bounded regular archive.");
  const hash = createHash("sha256");
  for await (const block of createReadStream(path)) hash.update(block);
  return { sha256: hash.digest("hex"), size: file.size };
}

export async function createDocumentationCandidate(
  directory,
  tag,
  commit,
  architecture,
) {
  validateRequest(tag, commit);
  const names = candidateNames(tag, architecture);
  const archive = join(directory, names.archive);
  const digest = await archiveDigest(archive);
  const saved = readSavedImageConfig(archive, {
    tag,
    commit,
    arch: architecture,
    imageName: documentationImage,
  });
  const candidate = {
    schema_version: 1,
    tag,
    source_commit: commit,
    architecture,
    image_name: documentationImage,
    config_digest: saved.config_digest,
    archive: { filename: names.archive, ...digest },
    smoke_suite: "documentation-container-v1",
  };
  await writeFile(
    join(directory, names.manifest),
    JSON.stringify(candidate, null, 2) + "\n",
    {
      flag: "wx",
      mode: 0o600,
    },
  );
  return candidate;
}

export async function verifyDocumentationCandidate(
  directory,
  tag,
  commit,
  architecture,
) {
  validateRequest(tag, commit);
  const names = candidateNames(tag, architecture);
  const manifestPath = join(directory, names.manifest);
  const file = await lstat(manifestPath);
  if (!file.isFile() || file.size > 16 * 1024)
    throw new Error(
      "Documentation candidate metadata must be bounded and regular.",
    );
  const candidate = JSON.parse(
    new TextDecoder("utf-8", { fatal: true }).decode(
      await readFile(manifestPath),
    ),
  );
  if (
    candidate?.schema_version !== 1 ||
    candidate.tag !== tag ||
    candidate.source_commit !== commit ||
    candidate.architecture !== architecture ||
    candidate.image_name !== documentationImage ||
    candidate.smoke_suite !== "documentation-container-v1" ||
    !digestPattern.test(candidate.config_digest) ||
    candidate.archive?.filename !== names.archive
  )
    throw new Error(
      "Documentation candidate differs from the verified tag source.",
    );
  const archive = join(directory, names.archive);
  const actual = await archiveDigest(archive);
  if (
    candidate.archive.sha256 !== actual.sha256 ||
    candidate.archive.size !== actual.size
  )
    throw new Error(
      "Documentation image differs from its tested candidate bytes.",
    );
  const saved = readSavedImageConfig(archive, {
    tag,
    commit,
    arch: architecture,
    imageName: documentationImage,
  });
  if (candidate.config_digest !== saved.config_digest)
    throw new Error(
      "Documentation candidate config digest disagrees with the image.",
    );
  return candidate;
}

export function verifyDocumentationIndex(index, variants) {
  if (
    !digestPattern.test(index?.digest) ||
    !indexMediaTypes.has(index.mediaType) ||
    index.schemaVersion !== 2 ||
    !Array.isArray(index.manifests) ||
    index.manifests.length !== 2
  )
    throw new Error(
      "Documentation index must contain exactly the tested platforms.",
    );
  for (const architecture of documentationArchitectures) {
    const digest = variants[architecture];
    if (
      !digestPattern.test(digest) ||
      index.manifests.filter(
        (item) =>
          item.digest === digest &&
          item.platform?.os === "linux" &&
          item.platform?.architecture === architecture &&
          (item.platform.variant === undefined ||
            (architecture === "arm64" && item.platform.variant === "v8")),
      ).length !== 1
    )
      throw new Error("Refusing a conflicting documentation image version.");
  }
  return index.digest;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const [operation, directory, tag, commit, architecture] =
    process.argv.slice(2);
  if (
    !directory ||
    !tag ||
    !commit ||
    !architecture ||
    process.argv.length !== 7 ||
    !["create", "verify"].includes(operation)
  )
    throw new Error(
      "usage: documentation-candidate.mjs create|verify DIRECTORY TAG COMMIT ARCH",
    );
  console.log(
    JSON.stringify(
      await (
        operation === "create"
          ? createDocumentationCandidate
          : verifyDocumentationCandidate
      )(directory, tag, commit, architecture),
    ),
  );
}
