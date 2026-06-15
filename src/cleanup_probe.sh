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

# rm -rf only when $1 resolves strictly under $DIR_COPIES (real path). Refuses
# the copies root itself and anything outside the tree. Ported from
# check-push.sh:_safe_rm_rf_copies. On failure: echo a one-line reason, return 1.
safe_rm_rf_copies() {
  local _target=$1 _base _resolved _err
  [[ -n "$_target" ]] || { echo "empty target"; return 1; }
  [[ -n "${DIR_COPIES:-}" ]] || { echo "DIR_COPIES unset, refusing rm -rf"; return 1; }
  _base=$(cd "$DIR_COPIES" && pwd -P) || { echo "cannot resolve DIR_COPIES"; return 1; }
  # Resolve the target's own real path. A symlink resolves to its destination,
  # so a link pointing outside the tree is correctly refused below.
  if [[ -e "$_target" || -L "$_target" ]]; then
    _resolved=$(cd "$_target" 2>/dev/null && pwd -P) || { echo "cannot resolve target"; return 1; }
  else
    # Deliberate divergence from check-push.sh: the reap loop only passes dirs that
    # just matched the glob and passed `[[ -d ]]`, so a missing target signals a
    # TOCTOU race — refusing to delete is the safe choice here.
    echo "target missing"; return 1
  fi
  [[ "$_resolved" == "$_base" ]] && { echo "refusing copies root"; return 1; }
  case "${_resolved}/" in
    "${_base}/"*) ;;
    *) echo "outside copies tree"; return 1 ;;
  esac
  _err=$(rm -rf -- "$_target" 2>&1) || { echo "${_err:-rm failed}"; return 1; }
  return 0
}

# Collapse tabs/CR/newlines in a reason string to spaces so it stays one TSV field.
sanitize() { printf '%s' "$1" | tr '\t\r\n' '   '; }

cd "$DIR_COPIES" || { echo "cleanup: cd $DIR_COPIES failed" >&2; exit 1; }

rc=0
shopt -s nullglob
for d in *.to-be-removed/; do
  d=${d%/}
  [[ -d "$d" ]] || continue
  repo=$(match_repo "$d")
  mtime=$(mtime_or_zero "$d")
  if [[ "$APPLY" == "1" ]]; then
    if reason=$(safe_rm_rf_copies "$d"); then
      emit "$HOST_ID" "$repo" "$d" "$mtime" removed "-"
    else
      emit "$HOST_ID" "$repo" "$d" "$mtime" failed "$(sanitize "$reason")"
      rc=1
    fi
  else
    emit "$HOST_ID" "$repo" "$d" "$mtime" would-remove "-"
  fi
done

exit $rc
