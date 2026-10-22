#!/usr/bin/env bash
# Publication consumes an exact successful tag candidate, never a rebuilt chart.
set -euo pipefail
umask 077
tag=${RELEASE_TAG:?Release tag is required}
node --input-type=module - "$tag" <<'JS'
import { validateRequest } from './scripts/ci/verify-control-plane-assets.mjs';
validateRequest(process.argv[2]);
JS
test "$(gh release view "$tag" --json isDraft --jq .isDraft)" = false
# Use authenticated API reads; checkout deliberately retains no Git credentials.
annotation=$(gh api "repos/obscuritylabs/Colossus/git/ref/tags/$tag" --jq 'select(.object.type == "tag") | .object.sha')
[[ "$annotation" =~ ^[0-9a-f]{40}$ ]]
commit=$(gh api "repos/obscuritylabs/Colossus/git/tags/$annotation" --jq 'select(.object.type == "commit") | .object.sha')
[[ "$commit" =~ ^[0-9a-f]{40}$ ]]
test "$(gh api "repos/obscuritylabs/Colossus/compare/$commit...main" --jq .merge_base_commit.sha)" = "$commit"
run=$(gh run list --workflow control-plane-chart.yml --branch "$tag" --event push --status success \
  --json databaseId,headSha --jq ".[] | select(.headSha == \"$commit\") | .databaseId" | head -n 1)
test -n "$run"
test "$(gh api "repos/obscuritylabs/Colossus/actions/runs/$run" --jq .head_sha)" = "$commit"
test "$(gh api "repos/obscuritylabs/Colossus/actions/runs/$run" --jq .event)" = push
test "$(gh api "repos/obscuritylabs/Colossus/actions/runs/$run" --jq .conclusion)" = success
output=${RUNNER_TEMP:?Runner temporary directory is required}/control-plane-chart-publication
mkdir -p "$output/trusted" "$output/remote"
gh run download "$run" --name "colossus-control-plane-chart-$tag" --dir "$output/trusted"
node scripts/ci/control-plane-chart.mjs verify "$output/trusted" "$tag" "$commit"
version=${tag#v}
archive=colossus-control-plane-chart-${version}.tgz
ref=oci://registry-1.docker.io/obscuritylabs/colossus-control-plane-chart
if helm pull "$ref" --version "$version" --destination "$output/remote" 2> "$output/pull-error"; then
  cmp "$output/trusted/$archive" "$output/remote/$archive"
else
  # Only an explicit missing manifest permits a new immutable version.
  if ! grep -Eq 'MANIFEST_UNKNOWN|manifest unknown|404 Not Found|: not found' "$output/pull-error"; then
    cat "$output/pull-error" >&2
    exit 1
  fi
  helm push "$output/trusted/$archive" oci://registry-1.docker.io/obscuritylabs
  helm pull "$ref" --version "$version" --destination "$output/remote"
  cmp "$output/trusted/$archive" "$output/remote/$archive"
fi
cp "$output/trusted/candidate.json" "$output/publication.json"
