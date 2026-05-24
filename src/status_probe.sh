#!/usr/bin/env bash
# Emits one TSV line per branch/release/latest/stale/unknown finding under $DIR_COPIES.
# Inputs (env): DIR_BASE, HOST_ID.
# Schema: <host>\t<kind>\t<repo>\t<name>\t<sha>\t<mtime_unix>\t<flags>
# Exit: 0 always, even with zero findings; non-zero only on unreadable $DIR_COPIES.
set -u
export LC_ALL=C

: "${HOST_ID:=unknown}"
: "${DIR_BASE:=/work}"
DIR_REPOS="${DIR_BASE}/git_repos"
DIR_COPIES="${DIR_BASE}/copies"

# Missing copies dir is a legitimate empty (host not yet bootstrapped).
[[ -d "$DIR_COPIES" ]] || exit 0
# Unreadable copies dir is a failure — distinguish from "empty".
if [[ ! -r "$DIR_COPIES" ]]; then
  echo "probe: $DIR_COPIES not readable" >&2
  exit 1
fi
cd "$DIR_COPIES" || { echo "probe: cd $DIR_COPIES failed" >&2; exit 1; }

emit() {
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" "$6" "$7"
}

collect_flags() {
  local dir=$1
  local out=""
  local f
  for f in skipping debugging no-cleanup stopping trigger; do
    if [[ -f "$dir/.$f" ]]; then
      [[ -n "$out" ]] && out="$out,"
      out="$out$f"
    fi
  done
  [[ -z "$out" ]] && out="-"
  printf '%s' "$out"
}

# Print Unix-seconds mtime, or 0 if missing/unreadable.
mtime_or_zero() {
  local f=$1
  [[ -e "$f" ]] || { echo 0; return; }
  local t
  t=$(stat -c %Y "$f" 2>/dev/null) && { echo "$t"; return; }
  t=$(stat -f %m "$f" 2>/dev/null) && { echo "$t"; return; }
  echo 0
}

# Build the repo-name list, longest first. `awk` sorts by length-desc so
# multi-dot repo names ("my.api") match before shorter prefixes ("my").
REPO_NAMES=()
if [[ -d "$DIR_REPOS" ]]; then
  while IFS= read -r r; do
    [[ -n "$r" ]] && REPO_NAMES+=("$r")
  done < <(ls -1 "$DIR_REPOS" 2>/dev/null \
            | awk '{print length, $0}' \
            | sort -rn -k1,1 \
            | cut -d' ' -f2-)
fi

# Match $1 (copy-dir name) against REPO_NAMES; on success sets two globals.
MATCHED_REPO=""
MATCHED_REST=""
match_repo() {
  local d=$1 r
  for r in "${REPO_NAMES[@]}"; do
    if [[ "$d" == "$r" ]]; then
      MATCHED_REPO=$r; MATCHED_REST=""; return 0
    fi
    if [[ "$d" == "$r".* ]]; then
      MATCHED_REPO=$r; MATCHED_REST=${d#"$r".}; return 0
    fi
  done
  MATCHED_REPO=""; MATCHED_REST=""
  return 1
}

shopt -s nullglob
for d in */; do
  d=${d%/}

  if ! match_repo "$d"; then
    # Unrecognized directory — surface so users can spot drift.
    emit "$HOST_ID" unknown "-" "$d" "-" "$(mtime_or_zero "$d")" "-"
    continue
  fi

  # Stale handling (preserve repo attribution). Stale dirs can carry flags
  # like .stopping/.skipping per check-push.sh:648-657, so collect them too.
  if [[ "$d" == *.to-be-removed ]]; then
    emit "$HOST_ID" stale "$MATCHED_REPO" "$d" "-" "$(mtime_or_zero "$d")" "$(collect_flags "$d")"
    continue
  fi

  sha=$(cat "$d/.git-rev" 2>/dev/null | tr -d '\r\n\t' || true)
  [[ -z "$sha" ]] && sha="-"
  mtime=$(mtime_or_zero "$d/.living")
  flags=$(collect_flags "$d")

  case "$MATCHED_REST" in
    "")
      # bare repo dir copied as-is — uncommon. Treat as unknown.
      emit "$HOST_ID" unknown "$MATCHED_REPO" "$d" "$sha" "$mtime" "$flags"
      ;;
    prod.latest)
      target=$(readlink "$d" 2>/dev/null || echo "-")
      [[ -z "$target" ]] && target="-"
      emit "$HOST_ID" latest "$MATCHED_REPO" "$target" "$sha" "$mtime" "$flags"
      ;;
    prod.*)
      tag=${MATCHED_REST#prod.}
      emit "$HOST_ID" release "$MATCHED_REPO" "$tag" "$sha" "$mtime" "$flags"
      ;;
    *)
      emit "$HOST_ID" branch "$MATCHED_REPO" "$MATCHED_REST" "$sha" "$mtime" "$flags"
      ;;
  esac
done
