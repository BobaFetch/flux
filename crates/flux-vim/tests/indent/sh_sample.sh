#!/bin/bash
# A sample for comparing highlights.
set -euo pipefail

NAME="${1:-world}"
COUNT=$((2 + 3))

greet() {
  local who="$1"
  echo "hello, $who" >&2
}

for f in *.txt; do
  if [[ -f "$f" && $COUNT -gt 1 ]]; then
    greet "$f" | tr a-z A-Z
  fi
done

case "$NAME" in
  world) exit 0 ;;
  *) echo "$(date +%s)" ;;
esac
