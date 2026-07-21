#!/bin/sh

set -eu

usage() {
    printf 'usage: %s plan|evaluate|activate [OWNER/REPOSITORY]\n' "$0"
}

mode=${1:-}
repository=${2:-obscuritylabs/Colossus}
case "$mode" in
    plan|evaluate|activate) ;;
    *) usage >&2; exit 2 ;;
esac

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
template="$root/.github/rulesets/main.json"
jq empty "$template"

existing_id=$(gh api --paginate "repos/$repository/rulesets" \
    --jq '.[] | select(.name == "Colossus main protection") | .id' \
    | head -n 1)

if [ "$mode" = plan ]; then
    printf 'repository=%s\n' "$repository"
    printf 'label=ci:full\n'
    printf 'ruleset_id=%s\n' "${existing_id:-absent}"
    printf 'next_enforcement=evaluate\n'
    exit 0
fi

gh label create ci:full --repo "$repository" --force \
    --color 5319E7 \
    --description 'Run cost-bounded full pre-merge acceptance on the current PR head'

temporary=$(mktemp)
response=$(mktemp)
trap 'rm -f "$temporary" "$response"' EXIT HUP INT TERM
case "$mode" in
    evaluate) enforcement=evaluate ;;
    activate) enforcement=active ;;
esac
jq --arg enforcement "$enforcement" '.enforcement = $enforcement' "$template" >"$temporary"

if [ -n "$existing_id" ]; then
    gh api --method PUT "repos/$repository/rulesets/$existing_id" \
        --input "$temporary" >"$response"
else
    gh api --method POST "repos/$repository/rulesets" \
        --input "$temporary" >"$response"
fi

configured_id=$(jq -r .id "$response")
actual=$(gh api "repos/$repository/rulesets/$configured_id" --jq .enforcement)
if [ "$actual" != "$enforcement" ]; then
    printf 'ruleset verification failed: expected %s, found %s\n' "$enforcement" "$actual" >&2
    exit 1
fi
printf 'configured %s with %s enforcement\n' "$repository" "$actual"
