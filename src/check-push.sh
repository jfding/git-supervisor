#!/usr/bin/env bash
# LICENSE: MIT

set -u
set -o pipefail

## global config settings
# logging level, default 2=verbose
: "${LOGLEVEL:=2}"
# timeout SEC for long running ops
: "${TIMEOUT:=600}"
# timer loop time in SEC
: "${SLEEP_TIME:=120}"

# file lock, normally need NOT change
: "${CI_LOCK:=/tmp/.git-supervisor-lock.d}"
# working base dir
: "${DIR_BASE:=/work}"
# whitelist of repos to checkout
: "${BR_WHITELIST:=main master dev test alpha}"
# whitelist of repos to check (empty = scan all repos in work dir)
: "${REPO_WHITELIST:=}"

# release tag pattern (ERE): only tags matching this are deployed as releases
# default: v plus one or more of 0-9, Q, or dot (e.g. v1.0, v1.2.Q3, v2025Q4.2.0)
: "${RELEASE_TAG_PATTERN:=^v[0-9Q.]+$}"
# optional exclude pattern (ERE): tags matching this are skipped (e.g. pre-releases)
# default empty = no exclusion. Example: -alpha|-beta|-rc|-SNAPSHOT
: "${RELEASE_TAG_EXCLUDE_PATTERN:=}"
# Need only handle top-N releases only (0 means all, default is 4)
: "${RELEASE_TAG_TOPN:=4}"

BASHPID=$(echo $$ | tr -d '\n')

function _color_enabled {
    [[ -n "${NO_COLOR:-}" ]] && return 1
    [[ -n "${FORCE_COLOR:-}" ]] && [[ "${FORCE_COLOR}" != "0" ]] && return 0
    [[ -t 1 ]] || [[ -t 2 ]]
}

function _color_wrap {
    local _color=$1
    local _text=$2
    local _code=

    case "${_color}" in
      red) _code='31' ;;
      yellow) _code='33' ;;
      green) _code='32' ;;
      blue) _code='34' ;;
      grey) _code='90' ;;
      *) printf '%s' "${_text}"; return ;;
    esac

    printf '\033[%sm%s\033[0m' "${_code}" "${_text}"
}

function _logging {
    local _level=$1; shift
    local _color=${1:-}; shift
    local _prefix=$(/bin/date '+%m-%d %H:%M:%S>')
    if [ $_level -le $LOGLEVEL ]; then
      local _message="$*"
      local _line
      [[ -n ${HOST_ID:-} ]] && _prefix="${_prefix} {${HOST_ID}}"
      [[ -n "${LOG_PREFIX:-}" ]] && _prefix="${_prefix} ${LOG_PREFIX}"
      _line="${_prefix} ${_message}"

      if _color_enabled && [[ -n "${_color}" ]]; then
        _line=$(_color_wrap "${_color}" "${_line}")
      fi
      if _color_enabled; then
        printf '%b\n' "${_line}"
      else
        printf '%s\n' "${_line}"
      fi
    fi
}
function highlight {
    _logging 0 green "$@"
}
function info {
    _logging 1 "" "$@"
}
function verbose {
    _logging 2 "grey" "$@"
}
function debug {
    _logging 2 "blue" "$@"
}
function err {
  _logging 0 red "ERROR: $*"
}
function warn {
  _logging 0 yellow "WARN: $*"
}

# Sort version-like tags in descending order
# Tag components are split on '.' and 'Q'; empty segments are treated as 0.
function sort_version_tags_desc {
  awk '
    function parse(tag, idx,    s, len, i, ch, buf) {
      s = tag
      sub(/^v/, "", s)
      len = 0
      buf = ""

      for (i = 1; i <= length(s); i++) {
        ch = substr(s, i, 1)
        if (ch == "." || ch == "Q") {
          if (buf == "") {
            buf = "0"
          }
          nums[idx, ++len] = buf + 0
          buf = ""
        } else {
          buf = buf ch
        }
      }

      if (buf == "") {
        buf = "0"
      }
      nums[idx, ++len] = buf + 0
      parts_len[idx] = len

      if (len > max_len) {
        max_len = len
      }
    }

    NF {
      tags[++count] = $0
      parse($0, count)
    }

    END {
      for (i = 1; i <= count; i++) {
        key = ""
        for (j = 1; j <= max_len; j++) {
          val = ((i SUBSEP j) in nums) ? nums[i, j] : 0
          key = key sprintf("%020d", val)
        }
        printf "%s\t%s\n", key, tags[i]
      }
    }
  ' | sort -r | cut -f2-
}

# Print release tags for current repo (must be in repo dir).
# Uses RELEASE_TAG_PATTERN (ERE) and optionally filters out RELEASE_TAG_EXCLUDE_PATTERN.
# optional arg: top-N releases only (default to use global RELEASE_TAG_TOPN)
function get_release_tags {
  local _tags
  local _topn=${1:-$RELEASE_TAG_TOPN}

  _tags=$(git tag -l | grep -E -- "${RELEASE_TAG_PATTERN}" || true)
  [[ -n "${RELEASE_TAG_EXCLUDE_PATTERN:-}" ]] && \
    _tags=$(echo "${_tags}" | grep -v -E -- "${RELEASE_TAG_EXCLUDE_PATTERN}" || true)

  [[ -z "${_tags}" ]] && return

  # (reverse)sort tags by version number
  _tags=$(printf '%s\n' "${_tags}" | grep -v '^[[:space:]]*$' | sort_version_tags_desc)

  # get topN only
  [[ $_topn -gt 0 ]] && _tags=$(echo "${_tags}" | head -n $_topn)

  echo "${_tags}"
}

# Return space-separated branch list for the given repo.
# Uses BR_WHITELIST_PER_REPO if set (format: "repo1 br1 br2|repo2 br3"),
# else BR_WHITELIST.
function get_branches_for_repo {
  local _repo_name=$1

  if [[ -z "${BR_WHITELIST_PER_REPO:-}" ]]; then
    echo "$BR_WHITELIST"
    return
  fi

  local _segment
  while IFS= read -r _segment; do
    if [[ $_segment == $_repo_name\ * ]] || [[ $_segment == "$_repo_name" ]]; then
      echo "${_segment#$_repo_name }"
      return
    fi
  done <<< "${BR_WHITELIST_PER_REPO//|/$'\n'}"
  echo "$BR_WHITELIST"
}

# file lock
function acquire_lock {
  local _lock_path=$1
  # mkdir is atomic; only one process can create the dir
  # max waiting times will be 100
  for _i in {1..100}; do
    if mkdir "$_lock_path" 2>/dev/null; then
      echo "${BASHPID}" > "${_lock_path}/owner.pid"
      return 0
    fi
    sleep 1
  done

  err "failed to acquire lock after many tries: ${_lock_path}"
  return 1
}
function release_lock {
  local _lock_path=$1
  local _owner_file="${_lock_path}/owner.pid"

  [[ -d "${_lock_path}" ]] || return 0
  [[ -f "${_owner_file}" ]] || return 0
  [[ $(cat "${_owner_file}") == "${BASHPID}" ]] || return 0

  rm -f "${_owner_file}"
  rmdir "${_lock_path}" 2>/dev/null || true
}
# make sure clean up locks on exit
trap 'release_lock "${CI_LOCK}"' EXIT INT TERM

function _timeout {
    if command -v timeout &>/dev/null; then
        timeout $TIMEOUT "$@"
    else
        "$@"
    fi
}

function _handle_post {
    # post scripts
    local _post_path=$1
    local _cp_path=$2

    if [[ -f "${_post_path}" ]]; then
      highlight "..running post scripts [ $_post_path ]"
      cd "${_cp_path}"
      bash "${_post_path}"
      cd - > /dev/null
    fi
}

function _handle_docker {
    # restart docker instance
    local _docker_path=$1

    command -v docker >/dev/null || {
      warn "docker cli not found, skip docker restart"
      return
    }

    if [[ -f "${_docker_path}" ]]; then
      local _docker_name=$(cat "${_docker_path}")

      highlight "..restarting docker [ $_docker_name ]"
      _timeout docker restart "${_docker_name}" > /dev/null || \
          err "failed to restart docker [ $_docker_name ]"
      unset _docker_name
    fi
}

# extract ref (tag or branch) to dest dir
function _git_checkout_ref_to {
  local _ref=$1
  local _dest=$2
  git archive "$_ref" | tar -x -C "$_dest"
}

# expect one argument "tag_name"
function checkout_and_copy_tag {
  local _repo=$1
  local _tag=$2

  local _cp_path="${DIR_COPIES}/${_repo}.prod.${_tag}"

  # if path exists, skip but consider successful
  [[ -d $_cp_path ]] && return

  # extract tag tree directly to target dir (no checkout in repo, ref unchanged)
  highlight "..copying files for new RELEASE [ $_tag ]"
  mkdir -p $_cp_path &&
    _git_checkout_ref_to $_tag $_cp_path || {
      rm -rf $_cp_path
      err "failed to copy files for new RELEASE [ $_tag ]"
      return 1
    }
}

# expect repo, branch, and optional per-repo branch list (default BR_WHITELIST)
function checkout_and_copy_br {
  local _repo=$1
  local _br=$2
  local _br_list="${3:-$BR_WHITELIST}"

  local _cp_path="${DIR_COPIES}/${_repo}.${_br}"
  local _post_path="${_cp_path}.post"
  local _docker_path="${_cp_path}.docker"

  # if no copy of this br, create dir; whitelisted branches get checkout in same run,
  # others get .skipping (opt-in)
  if [[ ! -d $_cp_path ]]; then
    mkdir -p "$_cp_path"
    if [[ $_br_list =~ (^|[[:space:]])$_br($|[[:space:]]) ]]; then
      highlight "..init dir of [ $_br ] (whitelisted, copying files)"
    else
      touch "$_cp_path/.skipping"
      highlight "..init (empty)dir for [ $_br ]"
    fi
  fi

  # checking flags
  if [[ -f "${_cp_path}/.debugging" ]]; then
    debug "..skip debugging work copy of branch [ $_br ]"
    return
  fi
  if [[ -f "${_cp_path}/.skipping" ]]; then
    verbose "..skip unused branch [ $_br ]"
    return
  fi

  # current commit at origin (no checkout in repo, ref unchanged)
  local _origin_ref
  _origin_ref=$(git rev-parse origin/$_br 2>/dev/null) || {
    warn "..no origin/$_br, skip"
    return
  }

  # initial copy when dir is empty
  if [[ -z $(/bin/ls -A $_cp_path 2>/dev/null) ]]; then
    highlight "..copying files for [ $_br ]"
    _git_checkout_ref_to origin/$_br $_cp_path || {
      err "failed to copy files for [ $_br ]"
      return 1
    }
    echo -n "$_origin_ref" > "${_cp_path}/.git-rev"
  fi

  local _stored_ref
  _stored_ref=$(cat "${_cp_path}/.git-rev" 2>/dev/null)
  local _need_update=0

  # add a debug trigger
  if [[ -f "${_cp_path}/.trigger" ]]; then
    rm -f "${_cp_path}/.trigger" # burn after reading
    debug "..having a debug try"
    _need_update=1
  fi

  # remote has new commits? (count commits on origin not reachable from stored ref)
  if [[ $_need_update -eq 0 ]] && [[ -n "${_stored_ref}" ]]; then
    local _behind
    _behind=$(git rev-list --count "${_stored_ref}..origin/$_br" 2>/dev/null)
    if [[ "${_behind:-1}" == "0" ]]; then
      verbose "..no change of branch [ $_br ], skip"
      return
    fi
  fi
  _need_update=1

  # only refresh when copy dir already has content (initial copy is handled above)
  if [[ $_need_update -eq 1 ]] && [[ -n $(/bin/ls -A $_cp_path 2>/dev/null) ]]; then
    highlight "..UPDATING branch [ $_br ]"
    if [[ -f "${_cp_path}/.no-cleanup" ]]; then
      # overwrite only, do not remove extra files
      _git_checkout_ref_to origin/$_br $_cp_path || {
        err "failed to copy files for [ $_br ]"
        return 1
      }
    else
      # full refresh: extract to new dir, preserve flags, then mv into place
      local _staging="${_cp_path}.staging.$$"
      mkdir -p "$_staging"
      _git_checkout_ref_to origin/$_br "$_staging" || {
        rm -rf $_staging
        err "failed to copy files for [ $_br ]"
        return 1
      }

      for _f in .no-cleanup .living .skipping .debugging; do
        [[ -e "${_cp_path}/${_f}" ]] && cp -p "${_cp_path}/${_f}" "${_staging}/"
      done
      rm -rf "${_cp_path}"
      mv "${_staging}" "${_cp_path}"
    fi
    echo -n "$_origin_ref" > "${_cp_path}/.git-rev"

    # post scripts
    _handle_post ${_post_path} ${_cp_path}

    # restart docker instance
    _handle_docker ${_docker_path}
  fi
}

# expect one argument "branch_name"
function fetch_and_check {
  local _repo=$1
  local _br
  local _release
  local _bp
  local _br_whitelist
  _br_whitelist=$(get_branches_for_repo "$_repo")

  cd $_repo || { err "failed to cd to $_repo, critical issue, skip"; return 1; }

  # clean up trash file from last time crash
  [[ -f .git/index.lock ]] && rm -f .git/index.lock

  info "..fetching repo, for branches [$_br_whitelist]..."
  _timeout git fetch -q --all --tags --prune --prune-tags || {
    err "failed to fetch repo $_repo, skip"
    return 1
  }

  ## iterate each branch

  # get list via git for-each-ref, remove 'origin/' prefix, skip HEAD and symbolic refs
  for _br in $(git for-each-ref --format='%(refname:strip=3)' refs/remotes/origin); do
    # filters
    [[ $_br = 'HEAD' ]] && continue
    (echo "$_br" | grep -q '/') && continue

    # check branch whitelist || repo dir exists already
    if [[ $_br_whitelist =~ (^|[[:space:]])$_br($|[[:space:]]) ]] || \
       [[ -d "${DIR_COPIES}/${_repo}.${_br}" ]]; then

        checkout_and_copy_br $_repo $_br "$_br_whitelist" || continue

        # heart beat
        touch "${DIR_COPIES}/${_repo}.${_br}/.living"
    fi
  done

  ## iterate each release tag

  local _releases=$(get_release_tags)
  local _latest_release=$(echo $_releases | cut -d' ' -f1)

  for _release in $_releases; do
    [[ -z "$_release" ]] && continue
    checkout_and_copy_tag $_repo $_release || continue

    # update latest version path symlink
    if [[ $_release == $_latest_release ]]; then
      local _latest_link="${DIR_COPIES}/${_repo}.prod.latest"
      local _latest_path=$(readlink $_latest_link || echo "")
      local _cur_release_path="${DIR_COPIES}/${_repo}.prod.${_release}"

      [[ $_latest_path != $_cur_release_path ]] && {
        rm -f $_latest_link
        ln -sf $(basename $_cur_release_path) $_latest_link
      }

      # post scripts
      _handle_post "${DIR_COPIES}/${_repo}.prod.post" ${_cur_release_path}
      # restart docker instance
      _handle_docker "${DIR_COPIES}/${_repo}.prod.docker"
    fi

    # heart beat
    if [[ -d "${DIR_COPIES}/${_repo}.prod.${_release}" ]]; then
      touch "${DIR_COPIES}/${_repo}.prod.${_release}/.living"
    fi
  done

  # clean up deprected dirs in "work/copies"
  for _bp in $(/bin/ls -d ${DIR_COPIES}/${_repo}.*/); do

      (echo $_bp | grep -q to-be-removed) && continue
      (echo $_bp | grep -q .latest) && continue

      _bp=${_bp%/}

      # manually marked as deprecated
      if [ -f "${_bp}/.stopping" ]; then
        # clean up all content
        rm -rf "${_bp}"
        mkdir -p "${_bp}"
        touch "${_bp}/.skipping"
        touch "${_bp}/.living"
      fi

      if [ -f "${_bp}/.living" ]; then
        rm -f "${_bp}/.living"
      else
        debug "..cleaning up deprecated dir: ${_bp}"
        #rm -rf $_bp
        #rm -f ${_bp}.*
        mv "$_bp" "${_bp}.to-be-removed"
      fi
  done

  cd - > /dev/null
}

function main_loop {
  local _repo
  local _worker_pid
  local _worker_failed=0

  cd $DIR_REPOS || {
    err "failed to cd to DIR_REPOS: $DIR_REPOS, critical issue, abort"
    exit 1
  }

  # loop like a daemon
  while true; do

    # Acquire lock
    acquire_lock "${CI_LOCK}" || exit 1

    # build list of repo dirs to check (whitelist or all git dirs)
    REPOS_TO_CHECK=$(
      if [[ -n "${REPO_WHITELIST}" ]]; then
        for _repo in $REPO_WHITELIST; do
          if [[ -d "${_repo}/.git" ]]; then
            echo "$_repo"
          else
            if [[ -d "${_repo}" ]]; then
              warn "[${_repo}] not a git repo, skip"
            else
              warn "[${_repo}] not found in $DIR_REPOS, skip"
            fi
          fi
        done
      else
        for _repo in $(/bin/ls -d */ 2>/dev/null); do
          _repo=${_repo%/}
          [[ -d "${_repo}/.git" ]] && echo "$_repo"
        done
      fi
    )

    for _repo in $REPOS_TO_CHECK; do
      info "[${_repo}] checking git status ..."
      ( LOG_PREFIX="[${_repo}]"; fetch_and_check "${_repo}" ) &
    done

    for _worker_pid in $(jobs -pr); do
      wait "${_worker_pid}" || _worker_failed=1
    done
    [[ "${_worker_failed}" == "1" ]] && err "one or more repo workers failed in this round"
    _worker_failed=0

    # Release lock
    release_lock "${CI_LOCK}"

    if [[ "${1:-}" == "once" ]]; then
      exit 0
    fi

    # if SLEEP_TIME value is empty or value is 0, means run once and exit
    [[ $SLEEP_TIME == "" ]] || [[ $SLEEP_TIME == "0" ]] && exit 0

    info "waiting for next check ..."
    sleep $SLEEP_TIME
  done
}

### __main__ ###

if [[ "${1:-}" == "--version" ]] || [[ "${1:-}" == "-V" ]]; then
  SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)
  for d in "$SCRIPT_DIR" "$SCRIPT_DIR/.." "/scripts"; do
    if [[ -f "$d/VERSION" ]]; then
      echo "check-push $(cat "$d/VERSION")"
      exit 0
    fi
  done
  echo "check-push unknown"
  exit 0
fi

# check for required commands
for c in git tar; do
  command -v "$c" >/dev/null || { err "missing command: $c"; exit 1; }
done
# check for optional 'docker' support
command -v docker >/dev/null || warn "docker cli not found, will skip docker restart handling"

## check and init all working dirs
# 1. check the DIR_BASE is available (sanitize&check at the time)
_ORIG_DIR_BASE=$DIR_BASE
DIR_BASE=$(realpath $DIR_BASE 2>/dev/null) || {
  err "base working dir not found: $_ORIG_DIR_BASE"
  exit 1
}
# subdirs
DIR_REPOS=${DIR_BASE}/git_repos
DIR_COPIES=${DIR_BASE}/copies

# 2. DIR_BASE/copies is writable
[[ -d $DIR_COPIES ]] || mkdir -p $DIR_COPIES || { err "failed to create COPIES dir: $DIR_COPIES"; exit 1; }
[[ -w $DIR_COPIES ]] ||                         { err "COPIES dir not writable: $DIR_COPIES"; exit 1; }

# 3. init repo dir
[[ -d $DIR_REPOS ]] || mkdir -p $DIR_REPOS

if [[ "${1:-}" == "--once" ]]; then
  main_loop once
else
  main_loop
fi
