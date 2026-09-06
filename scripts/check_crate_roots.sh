#!/usr/bin/env bash
set -euo pipefail

maximum_lines="${COLOSSUS_MAX_CRATE_ROOT_LINES:-250}"
failed=0

while IFS= read -r crate_root; do
  [[ -f "$crate_root" ]] || continue
  line_count="$(wc -l < "$crate_root")"
  line_count="${line_count//[[:space:]]/}"
  if (( line_count > maximum_lines )); then
    printf '%s has %s lines; crate roots must stay at or below %s lines\n' \
      "$crate_root" "$line_count" "$maximum_lines" >&2
    failed=1
  fi
done < <(
  git ls-files --cached --others --exclude-standard \
    'crates/*/src/lib.rs' \
    'crates/*/src/main.rs' \
    'xtask/src/lib.rs' \
    'xtask/src/main.rs'
)

if (( failed != 0 )); then
  printf '%s\n' \
    'Move implementation into responsibility-focused modules; do not raise the limit to fit new logic.' >&2
  exit 1
fi

printf 'crate root structure check passed (maximum %s lines)\n' "$maximum_lines"
