#!/bin/sh
set -eu

usage() {
  echo "usage: $0 MESSAGE_FILE | --stdin | --range COMMIT_RANGE" >&2
  exit 2
}

subject_from_stream() {
  awk '
    {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      sub(/[[:space:]]+$/, "", line)
      if (line != "" && substr(line, 1, 1) != "#") {
        print line
        exit
      }
    }
  '
}

validate_subject() {
  subject=$1
  case "$subject" in
    "Merge "*|"Revert "*|"fixup! "*|"squash! "*) return 0 ;;
  esac

  if printf '%s\n' "$subject" | grep -Eq '^(build|chore|ci|docs|feat|fix|perf|refactor|revert|security|style|test)(\([a-z0-9._-]+\))?!?: [^[:space:]].+$'; then
    return 0
  fi

  echo "Commit message is not Conventional Commits compliant." >&2
  echo "Found: ${subject:-<empty>}" >&2
  echo "Expected: <type>[optional scope][!]: <description>" >&2
  echo "Allowed types: build, chore, ci, docs, feat, fix, perf, refactor, revert, security, style, test" >&2
  echo "Examples: feat(tui): add themes | fix: handle denied approvals" >&2
  return 1
}

if [ "$#" -eq 1 ] && [ "$1" = "--stdin" ]; then
  validate_subject "$(subject_from_stream)"
elif [ "$#" -eq 2 ] && [ "$1" = "--range" ]; then
  status=0
  while IFS= read -r subject; do
    validate_subject "$subject" || status=1
  done <<EOF
$(git log --format=%s "$2")
EOF
  exit "$status"
elif [ "$#" -eq 1 ]; then
  validate_subject "$(subject_from_stream < "$1")"
else
  usage
fi
