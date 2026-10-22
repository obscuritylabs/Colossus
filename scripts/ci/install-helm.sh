#!/usr/bin/env bash
# CI-only Linux amd64 bootstrap. Contributors may supply their own Helm via HELM/PATH.
set -euo pipefail
version=3.19.0
checksum=a7f81ce08007091b86d8bd696eb4d86b8d0f2e1b9f6c714be62f82f96a594496
output=${RUNNER_TEMP:?RUNNER_TEMP is required}/colossus-helm
mkdir -p "$output"
curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
  "https://get.helm.sh/helm-v${version}-linux-amd64.tar.gz" --output "$output/helm.tar.gz"
printf '%s  %s\n' "$checksum" "$output/helm.tar.gz" | sha256sum --check --strict
tar -xzf "$output/helm.tar.gz" -C "$output" linux-amd64/helm
printf '%s\n' "$output/linux-amd64" >> "${GITHUB_PATH:?GITHUB_PATH is required}"
