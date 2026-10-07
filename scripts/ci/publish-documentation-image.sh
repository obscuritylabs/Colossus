#!/usr/bin/env bash
set -euo pipefail
umask 077

tag=${RELEASE_TAG:?Release tag is required}
image=ghcr.io/obscuritylabs/colossus-documentation
node --input-type=module - "$tag" <<'JS'
import { validateRequest } from './scripts/ci/verify-control-plane-assets.mjs';
validateRequest(process.argv[2]);
JS
node --input-type=module - "$(docker version --format '{{.Server.APIVersion}}')" <<'JS'
const version = process.argv[2];
if (!/^\d+\.\d+$/u.test(version)) throw new Error('Docker server API version is unavailable.');
const [major, minor] = version.split('.').map(Number);
if (major < 1 || (major === 1 && minor < 46)) throw new Error('Docker API 1.46 or later is required for explicit platform publication.');
JS

git fetch origin refs/heads/main:refs/remotes/origin/main "refs/tags/$tag:refs/tags/$tag"
test "$(git cat-file -t "$tag")" = tag
commit=$(git rev-parse "$tag^{commit}")
git merge-base --is-ancestor "$commit" origin/main
output=${RUNNER_TEMP:?Runner temporary directory is required}/documentation-image
mkdir -p "$output/trusted"
gh api "repos/obscuritylabs/Colossus/releases/tags/$tag" > "$output/release.json"

for workflow in release.yml documentation-candidate.yml; do
  gh api "repos/obscuritylabs/Colossus/actions/workflows/$workflow/runs?event=push&status=success&head_sha=$commit&per_page=100" > "$output/$workflow-runs.json"
  run=$(node --input-type=module - "$output/$workflow-runs.json" "$workflow" "$tag" "$commit" <<'JS'
import { readFile } from 'node:fs/promises';
import { matchingDocumentationSourceRun } from './scripts/ci/documentation-candidate.mjs';
const [path, workflow, tag, commit] = process.argv.slice(2);
const response = JSON.parse(await readFile(path, 'utf8'));
console.log(matchingDocumentationSourceRun(response.workflow_runs, workflow, tag, commit).run_id);
JS
  )
  gh api "repos/obscuritylabs/Colossus/actions/runs/$run" > "$output/$workflow-run.json"
  if [[ "$workflow" == documentation-candidate.yml ]]; then
    for arch in amd64 arm64; do
      gh run download "$run" --repo obscuritylabs/Colossus --name "colossus-documentation-$arch" --dir "$output/trusted"
    done
  fi
done

# Validate both complete source-bound candidates before the first registry write.
node --input-type=module - "$output" "$tag" "$commit" "$(git rev-parse HEAD)" <<'JS'
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { verifyPublishedDocumentationRelease, verifyDocumentationSourceRun, verifyDocumentationCandidate } from './scripts/ci/documentation-candidate.mjs';
const [directory, tag, commit, publisher] = process.argv.slice(2);
const load = async name => JSON.parse(await readFile(join(directory, name), 'utf8'));
const release = verifyPublishedDocumentationRelease(await load('release.json'), tag);
const releaseBuild = verifyDocumentationSourceRun(await load('release.yml-run.json'), 'release.yml', tag, commit);
const candidateBuild = verifyDocumentationSourceRun(await load('documentation-candidate.yml-run.json'), 'documentation-candidate.yml', tag, commit);
const candidates = await Promise.all(['amd64', 'arm64'].map(arch => verifyDocumentationCandidate(join(directory, 'trusted'), tag, commit, arch)));
const evidence = { ...release, source_commit: commit, publisher_commit: publisher, release_build: releaseBuild, candidate_build: candidateBuild, candidates };
await writeFile(join(directory, 'provenance.json'), JSON.stringify(evidence, null, 2) + '\n');
JS

# Only an explicit registry not-found response allows creation. Authentication,
# rate-limit, transport, and conflicting immutable versions all fail closed.
inspect() {
  local ref=$1 destination=$2 error
  if docker buildx imagetools inspect "$ref" --format '{{json .Manifest}}' > "$destination" 2> "$output/inspect-error"; then return 0; fi
  error=$(<"$output/inspect-error")
  if [[ "$error" == *'manifest unknown'* || "$error" == *'MANIFEST_UNKNOWN'* || "$error" == *'404 Not Found'* || "$error" == *"$ref: not found"* ]]; then return 1; fi
  cat "$output/inspect-error" >&2
  return 2
}

for arch in amd64 arm64; do
  archive="$output/trusted/Colossus-Documentation-${tag}-linux-${arch}.docker.tar.gz"
  expected=$(node --input-type=module - "$output/trusted" "$tag" "$commit" "$arch" <<'JS'
import { verifyDocumentationCandidate } from './scripts/ci/documentation-candidate.mjs';
const [directory, tag, commit, arch] = process.argv.slice(2);
console.log((await verifyDocumentationCandidate(directory, tag, commit, arch)).config_digest);
JS
  )
  docker load --input "$archive"
  test "$(docker image inspect colossus-documentation:candidate --format '{{.Architecture}}')" = "$arch"
  test "$(docker image inspect colossus-documentation:candidate --format '{{.Config.User}}')" = 10001:10001
  test "$(docker image inspect colossus-documentation:candidate --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')" = "$commit"
  test "$(docker image inspect colossus-documentation:candidate --format '{{index .Config.Labels "org.opencontainers.image.version"}}')" = "$tag"
  ref="$image:$tag-$arch"
  if inspect "$ref" "$output/$arch.json"; then
    : # Existing version tags are verified below and never overwritten.
  else
    status=$?
    test "$status" = 1
    docker tag colossus-documentation:candidate "$ref"
    docker push --platform "linux/$arch" "$ref"
    inspect "$ref" "$output/$arch.json"
  fi
  executable=$(node --input-type=module - "$output/$arch.json" <<'JS'
import { readFile } from 'node:fs/promises';
import { executableDescriptorDigest } from './scripts/ci/verify-control-plane-assets.mjs';
console.log(executableDescriptorDigest(JSON.parse(await readFile(process.argv[2], 'utf8'))));
JS
  )
  docker buildx imagetools inspect "$image@$executable" --raw > "$output/$arch-manifest.json"
  node --input-type=module - "$output/$arch-manifest.json" "$expected" <<'JS'
import { readFile } from 'node:fs/promises';
import { verifyExecutableManifest } from './scripts/ci/verify-control-plane-assets.mjs';
verifyExecutableManifest(JSON.parse(await readFile(process.argv[2], 'utf8')), process.argv[3]);
JS
  docker pull --platform "linux/$arch" "$image@$executable"
  printf '%s\n' "$executable" > "$output/$arch.digest"
done

# Recheck release publication before advertising either platform at the exact tag.
gh api "repos/obscuritylabs/Colossus/releases/tags/$tag" > "$output/release-current.json"
node --input-type=module - "$output" "$tag" <<'JS'
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { verifyPublishedDocumentationRelease } from './scripts/ci/documentation-candidate.mjs';
const [directory, tag] = process.argv.slice(2);
const previous = JSON.parse(await readFile(join(directory, 'provenance.json'), 'utf8'));
const current = verifyPublishedDocumentationRelease(JSON.parse(await readFile(join(directory, 'release-current.json'), 'utf8')), tag);
if (current.release_id !== previous.release_id || current.published_at !== previous.published_at) throw new Error('The reviewed release changed during publication.');
JS
if inspect "$image:$tag" "$output/index.json"; then
  : # The exact existing index must match both verified platform digests.
else
  status=$?
  test "$status" = 1
  docker buildx imagetools create --tag "$image:$tag" "$image@$(<"$output/amd64.digest")" "$image@$(<"$output/arm64.digest")"
  inspect "$image:$tag" "$output/index.json"
fi
node --input-type=module - "$output" "$image:$tag" <<'JS'
import { readFile, writeFile, appendFile } from 'node:fs/promises';
import { join } from 'node:path';
import { verifyDocumentationIndex } from './scripts/ci/documentation-candidate.mjs';
const [directory, image] = process.argv.slice(2);
const index = JSON.parse(await readFile(join(directory, 'index.json'), 'utf8'));
const variants = Object.fromEntries(await Promise.all(['amd64', 'arm64'].map(async arch => [arch, (await readFile(join(directory, arch + '.digest'), 'utf8')).trim()])));
const digest = verifyDocumentationIndex(index, variants);
const provenance = JSON.parse(await readFile(join(directory, 'provenance.json'), 'utf8'));
const evidence = { image, digest, source_commit: provenance.source_commit, publisher_commit: provenance.publisher_commit, release_id: provenance.release_id, candidate_build: provenance.candidate_build, platforms: index.manifests.map(item => ({digest: item.digest, platform: item.platform})) };
await writeFile(join(directory, 'publication.json'), JSON.stringify(evidence, null, 2) + '\n');
if (process.env.GITHUB_STEP_SUMMARY) await appendFile(process.env.GITHUB_STEP_SUMMARY, `Documentation image: \`${image}\`\n\nVerified digest: \`${digest}\`\n\nAnonymous digest pull must pass before advertising public availability.\n`);
console.log(JSON.stringify(evidence));
JS
