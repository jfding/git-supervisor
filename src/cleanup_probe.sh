#!/usr/bin/env bash
# Reaps stale "*.to-be-removed" copy dirs under $DIR_COPIES.
# Dry-run by default (APPLY=0): lists what WOULD be removed, deletes nothing.
# APPLY=1: deletes each stale dir via a safe-rm guard, reports per-dir outcome.
# Inputs (env): DIR_BASE, HOST_ID, APPLY.
# Schema: <host>\t<repo>\t<name>\t<mtime_unix>\t<outcome>\t<reason>
#   outcome ∈ would-remove | removed | failed ; reason is "-" unless failed.
# Exit: 0 on success (incl. zero findings); non-zero if any deletion failed
#       or $DIR_COPIES is unreadable.
set -u
export LC_ALL=C

: "${HOST_ID:=unknown}"
: "${DIR_BASE:=/work}"
: "${APPLY:=0}"
DIR_REPOS="${DIR_BASE}/git_repos"
DIR_COPIES="${DIR_BASE}/copies"

# Missing copies dir → nothing to clean (host not yet bootstrapped).
[[ -d "$DIR_COPIES" ]] || exit 0
if [[ ! -r "$DIR_COPIES" ]]; then
  echo "cleanup: $DIR_COPIES not readable" >&2
  exit 1
fi

emit() {
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" "$6"
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

# Build the repo-name list, longest first, so multi-dot repo names ("my.api")
# match before shorter prefixes ("my"). Mirrors status_probe.sh.
REPO_NAMES=()
if [[ -d "$DIR_REPOS" ]]; then
  while IFS= read -r r; do
    [[ -n "$r" ]] && REPO_NAMES+=("$r")
  done < <(ls -1 "$DIR_REPOS" 2>/dev/null \
            | awk '{print length, $0}' \
            | sort -rn -k1,1 \
            | cut -d' ' -f2-)
fi

# Echo the matching repo name for a copy-dir name, or "-" if none matches.
match_repo() {
  local d=$1 r
  for r in "${REPO_NAMES[@]}"; do
    if [[ "$d" == "$r" || "$d" == "$r".* ]]; then
      echo "$r"; return 0
    fi
  done
  echo "-"; return 0
}

cd "$DIR_COPIES" || { echo "cleanup: cd $DIR_COPIES failed" >&2; exit 1; }

rc=0
shopt -s nullglob
for d in *.to-be-removed/; do
  d=${d%/}
  [[ -d "$d" ]] || continue
  repo=$(match_repo "$d")
  mtime=$(mtime_or_zero "$d")
  # Dry-run only for now; APPLY handling added in Task 2.
  emit "$HOST_ID" "$repo" "$d" "$mtime" would-remove "-"
done

exit $rc
