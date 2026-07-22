#!/bin/bash

set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "usage: $0 TARGET" >&2
    exit 2
fi

target=$1
binary="$GITHUB_WORKSPACE/target/$target/release/colossus"
version=$(cargo metadata --locked --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "colossus-cli") | .version')
package="colossus-$version-$target"

smoke="$RUNNER_TEMP/colossus-release-smoke-$target"
rm -rf "$smoke"
mkdir -p "$smoke/workflows"
cp release/smoke-config.yaml "$smoke/config.yaml"
(
    cd "$smoke"
    "$binary" --version | grep '^colossus '
    "$binary" --config config.yaml config show >/dev/null
    "$binary" --config config.yaml run connected >result.json
    jq -e '.output == "connected" and .profile == "echo" and .event_count >= 3' result.json >/dev/null
    "$binary" --config config.yaml audit verify >audit.json
    jq -e '.last_sequence >= 1 and .checkpoint.global_sequence == .last_sequence' audit.json >/dev/null
)

stage="$RUNNER_TEMP/$package"
rm -rf "$stage"
mkdir -p "$stage" dist
install -m 0755 "$binary" "$stage/colossus"
install -m 0755 release/install.sh "$stage/install.sh"
case "$target" in
    *-linux-*)
        install -m 0755 release/install-apparmor.sh "$stage/install-apparmor.sh"
        install -m 0644 release/colossus.apparmor.in "$stage/colossus.apparmor.in"
        ;;
esac
cp LICENSE README.md "$stage/"
tar -C "$RUNNER_TEMP" -czf "dist/$package.tar.gz" "$package"
if command -v sha256sum >/dev/null; then
    (cd dist && sha256sum "$package.tar.gz" >"$package.tar.gz.sha256")
else
    (cd dist && shasum -a 256 "$package.tar.gz" >"$package.tar.gz.sha256")
fi

extract="$RUNNER_TEMP/colossus-install-extract-$target"
prefix="$RUNNER_TEMP/colossus-install-prefix-$target"
installed_smoke="$RUNNER_TEMP/colossus-install-smoke-$target"
rm -rf "$extract" "$prefix" "$installed_smoke"
mkdir -p "$extract" "$installed_smoke/workflows"
tar -xzf "dist/$package.tar.gz" -C "$extract"
"$extract/$package/install.sh" --prefix "$prefix"
cp release/smoke-config.yaml "$installed_smoke/config.yaml"
(
    cd "$installed_smoke"
    "$prefix/bin/colossus" --version | grep '^colossus '
    "$prefix/bin/colossus" --config config.yaml run installed-offline >result.json
    jq -e '.output == "installed-offline" and .profile == "echo" and .event_count >= 3' result.json >/dev/null
    "$prefix/bin/colossus" --config config.yaml audit verify >audit.json
    jq -e '.last_sequence >= 1 and .checkpoint.global_sequence == .last_sequence' audit.json >/dev/null
)

bundle_root="$RUNNER_TEMP/colossus-bundle-smoke-$target"
bundle_stage="$bundle_root/stage"
bundle="$bundle_root/bundle"
bundle_prefix="$bundle_root/prefix"
rm -rf "$bundle_root"
mkdir -p "$bundle_stage/artifacts/$target" "$bundle_root/workflows"
install -m 0755 "$binary" "$bundle_stage/artifacts/$target/colossus"
cp LICENSE "$bundle_stage/LICENSE"
cat >"$bundle_root/config.yaml" <<EOF
schemaVersion: 1
access:
  profile: pinned
  tools:
    include: [echo]
    exclude: []
  actions:
    allow: [bundle.verify]
    requireApproval: [bundle.key.inspect, pack.trust.add, bundle.build, bundle.install]
    deny: []
storage:
  path: $bundle_root/state.redb
  keys:
    kind: environment
    journal_variable: COLOSSUS_BUNDLE_JOURNAL_KEY
    journal_key_id: release-bundle-journal-v1
    signing_variable: COLOSSUS_BUNDLE_CHECKPOINT_KEY
    anchor_path: $bundle_root/anchor.json
policy:
  kind: built_in
  require_post_effect: false
workflows:
  repository: $bundle_root/workflows
  user: $bundle_root/workflows
providers:
  profiles:
    echo:
      kind: echo
      model: echo
      baseUrl: null
      credentialReference: null
      timeoutMs: 5000
  roles:
    primary: echo
agent:
  maxTurns: 2
subagents:
  maxConcurrent: 1
sandbox:
  backend: native
  profile: release-bundle-smoke-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: $bundle_root
      mode: write
  executables: []
  environment: []
  networkDestinations: []
  timeoutMs: 30000
  maxOutputBytes: 1048576
  maxProcesses: 1
  maxMemoryBytes: 67108864
  maxConcurrency: 1
EOF

config="$bundle_root/config.yaml"
"$binary" --config "$config" --approval-mode full-access bundle key-info \
    --signing-key-reference env:COLOSSUS_BUNDLE_SIGNING_SEED >"$bundle_root/key.json"
public=$(jq -r .public_key "$bundle_root/key.json")
"$binary" --config "$config" --approval-mode full-access packs trust add colossus \
    --public-key "$public" >/dev/null
"$binary" --config "$config" --approval-mode full-access bundle build \
    "$bundle_stage" "$bundle" --name colossus-offline --version "$version" \
    --publisher colossus --created-at 2026-07-11T00:00:00Z \
    --source-revision "$GITHUB_SHA" \
    --signing-key-reference env:COLOSSUS_BUNDLE_SIGNING_SEED >"$bundle_root/build.json"
"$binary" --config "$config" bundle verify "$bundle" >/dev/null
"$binary" --config "$config" --approval-mode full-access bundle install \
    "$bundle" --prefix "$bundle_prefix" >"$bundle_root/install.json"
installed="$bundle_prefix/bin/colossus"
"$installed" --config "$config" run bundle-installed >"$bundle_root/result.json"
jq -e --arg target "$target" \
    '.targets == [$target]' "$bundle_root/build.json" >/dev/null
jq -e --arg target "$target" \
    '.target == $target' "$bundle_root/install.json" >/dev/null
jq -e '.output == "bundle-installed"' "$bundle_root/result.json" >/dev/null
"$installed" --config "$config" audit verify >/dev/null
