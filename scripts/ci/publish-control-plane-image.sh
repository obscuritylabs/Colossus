#!/usr/bin/env bash
set -euo pipefail
umask 077

tag=${RELEASE_TAG:?Release tag is required}
image=ghcr.io/obscuritylabs/colossus-control-plane
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
test "$(gh release view "$tag" --json isDraft --jq .isDraft)" = false
git fetch origin main "refs/tags/$tag:refs/tags/$tag"
test "$(git cat-file -t "$tag")" = tag
commit=$(git rev-list -n 1 "$tag")
git merge-base --is-ancestor "$commit" origin/main

run=$(gh run list --workflow release.yml --branch "$tag" --event push --status success \
  --json databaseId,headSha --jq ".[] | select(.headSha == \"$commit\") | .databaseId" | head -n 1)
test -n "$run"
test "$(gh api "repos/obscuritylabs/Colossus/actions/runs/$run" --jq .head_sha)" = "$commit"
test "$(gh api "repos/obscuritylabs/Colossus/actions/runs/$run" --jq .event)" = push
test "$(gh api "repos/obscuritylabs/Colossus/actions/runs/$run" --jq .conclusion)" = success

output=${RUNNER_TEMP:?Runner temporary directory is required}/control-plane-image
mkdir -p "$output/trusted" "$output/published"
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  gh run download "$run" --name "colossus-control-plane-$target" --dir "$output/trusted"
done
gh release download "$tag" --pattern 'Colossus-Control-Plane-*' --dir "$output/published"
node scripts/ci/verify-control-plane-assets.mjs "$output/trusted" "$output/published" "$tag" > "$output/assets.json"

# Only explicit not-found responses permit creation. Authorization, rate-limit and
# transport failures do not become permission to overwrite an existing immutable tag.
inspect() {
  local ref=$1 destination=$2
  if docker buildx imagetools inspect "$ref" --format '{{json .Manifest}}' > "$destination" 2> "$output/inspect-error"; then
    return 0
  fi
  local error
  error=$(<"$output/inspect-error")
  if [[ "$error" == *'manifest unknown'* || "$error" == *'MANIFEST_UNKNOWN'* || "$error" == *'404 Not Found'* || "$error" == *"$ref: not found"* ]]; then return 1; fi
  cat "$output/inspect-error" >&2
  return 2
}

for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  if [[ "$target" == x86_64-* ]]; then arch=amd64; else arch=arm64; fi
  archive="$output/trusted/Colossus-Control-Plane-${tag}-${target}.docker.tar.gz"
  expected=$(node --input-type=module - "$archive" "$arch" "$tag" "$commit" <<'JS'
import { readSavedImageConfig } from './scripts/ci/verify-control-plane-assets.mjs';
const [archive, arch, tag, commit] = process.argv.slice(2);
console.log(readSavedImageConfig(archive, {arch, tag, commit}).config_digest);
JS
  )
  docker load --input "$archive"
  test "$(docker image inspect colossus-control-plane:release --format '{{.Architecture}}')" = "$arch"
  test "$(docker image inspect colossus-control-plane:release --format '{{.Config.User}}')" = 10001:10001
  test "$(docker image inspect colossus-control-plane:release --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')" = "$commit"
  test "$(docker image inspect colossus-control-plane:release --format '{{index .Config.Labels "org.opencontainers.image.version"}}')" = "$tag"
  ref="$image:$tag-$arch"
  if inspect "$ref" "$output/$arch.json"; then
    : # Verify the existing immutable executable below; never overwrite it.
  else
    status=$?
    test "$status" = 1
    docker tag colossus-control-plane:release "$ref"
    # Publish the executable manifest explicitly. Containerd archives can contain a
    # parent index and default provenance; those are not executable platform digests.
    docker push --platform "linux/$arch" "$ref"
    inspect "$ref" "$output/$arch.json"
  fi
  executable=$(node --input-type=module - "$output/$arch.json" <<'JS'
import { readFile } from 'node:fs/promises';
import { executableDescriptorDigest } from './scripts/ci/verify-control-plane-assets.mjs';
console.log(executableDescriptorDigest(JSON.parse(await readFile(process.argv[2], 'utf8'))));
JS
  )
  # Resolve metadata by the captured digest, not a mutable architecture tag.
  docker buildx imagetools inspect "$image@$executable" --raw > "$output/$arch-manifest.json"
  node --input-type=module - "$output/$arch-manifest.json" "$expected" <<'JS'
import { readFile } from 'node:fs/promises';
import { verifyExecutableManifest } from './scripts/ci/verify-control-plane-assets.mjs';
verifyExecutableManifest(JSON.parse(await readFile(process.argv[2], 'utf8')), process.argv[3]);
JS
  # The config binds uncompressed rootfs digests, but a manifest can claim that config
  # with conflicting layer descriptors. Pull the immutable executable so the engine
  # verifies the actual layer downloads and rootfs before the final index advertises it.
  docker pull --platform "linux/$arch" "$image@$executable"
  printf '%s\n' "$executable" > "$output/$arch.digest"
done

if inspect "$image:$tag" "$output/index.json"; then
  : # Verify the existing exact-platform index below; never overwrite it.
else
  status=$?
  test "$status" = 1
  docker buildx imagetools create --tag "$image:$tag" "$image@$(<"$output/amd64.digest")" "$image@$(<"$output/arm64.digest")"
  inspect "$image:$tag" "$output/index.json"
fi
node --input-type=module - "$output" "$image:$tag" "$commit" <<'JS'
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
const [directory, image, commit] = process.argv.slice(2);
const load = async name => JSON.parse(await readFile(join(directory,name),'utf8'));
const index = await load('index.json');
const variants = await Promise.all(['amd64.json','arm64.json'].map(load));
const manifests = index.manifests ?? [];
if (manifests.length !== 2 || !/^sha256:[a-f0-9]{64}$/u.test(index.digest)) throw new Error('Published index is invalid.');
for (const [position, arch] of ['amd64','arm64'].entries()) {
  if (!manifests.some(item => item.digest === variants[position].digest && item.platform?.os === 'linux' && item.platform?.architecture === arch))
    throw new Error('Published image differs from the verified release platforms.');
}
const evidence = {image,digest:index.digest,source_commit:commit,platforms:manifests.map(item=>({digest:item.digest,platform:item.platform}))};
await writeFile(join(directory,'publication.json'),JSON.stringify(evidence,null,2)+'\n');
console.log(JSON.stringify(evidence));
JS
