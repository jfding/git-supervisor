#!/usr/bin/env bash
set -u
set -o pipefail

## global config settings
: "${VERB:=1}"
: "${TIMEOUT:=600}"
: "${SLEEP_TIME:=120}"
: "${CI_LOCK:=/tmp/.auto-reloader-lock.d}"
: "${DIR_BASE:=/work}"

## hard coded settings
BR_WHITELIST="main master dev test alpha"
BASHPID=$(echo $$ | tr -d '\n')

function _version_less_than {
  if [[ -z $1 ]] || [[ -z $2 ]]; then
    return 100
  fi
  if [[ $1 == $2 ]]; then
    return 2
  fi

  python3 -c "
import sys
v1, v2 = sys.argv[1].lstrip('v'), sys.argv[2].lstrip('v')
n1 = [[int(y) for y in x.split('Q')] for x in v1.split('.')]
n1 = [item for sublist in n1 for item in sublist]
n2 = [[int(y) for y in x.split('Q')] for x in v2.split('.')]
n2 = [item for sublist in n2 for item in sublist]
max_len = max(len(n1), len(n2))
n1.extend([0] * (max_len - len(n1)))
n2.extend([0] * (max_len - len(n2)))
sys.exit(0 if n1 < n2 else (1 if n1 > n2 else 2))
" $1 $2
}

function _logging {
    local _level=$1; shift
    local _prefix=$(/bin/date '+%m-%d %H:%M:%S>')
    if [ $_level -le $VERB ]; then
      [[ -n ${HOST_ID:-} ]] && _prefix="${_prefix} {${HOST_ID}}"
      [[ -n "${LOG_PREFIX:-}" ]] && _prefix="${_prefix} ${LOG_PREFIX}"

      echo $_prefix "$@"
    fi
}
function mustsay {
    _logging 0 "$@"
}
function say {
    _logging 1 "$@"
}
function verbose {
    _logging 2 "$@"
}
function err {
  mustsay "ERROR: $@"
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
      say "..running post scripts [ $_post_path ]"
      cd "${_cp_path}"
      bash "${_post_path}"
      cd - > /dev/null
    fi
}

function _handle_docker {
    # restart docker instance
    local _docker_path=$1

    command -v docker >/dev/null || { say "WARN: docker cli not found, skip docker restart"; return; }

    if [[ -f "${_docker_path}" ]]; then
      local _docker_name=$(cat "${_docker_path}")

      say "..restarting docker [ $_docker_name ]"
      _timeout docker restart "${_docker_name}" > /dev/null || err "failed to restart docker [ $_docker_name ]"
      unset _docker_name
    fi
}

# expect one argument "tag_name"
function checkout_and_copy_tag {
  local _repo=$1
  local _tag=$2

  local _cp_path="${DIR_COPIES}/${_repo}.prod.${_tag}"
  local _arch_path="${DIR_COPIES}/.archives/${_repo}.prod.${_tag}"
  local _post_path="${DIR_COPIES}/${_repo}.prod.post"
  local _docker_path="${DIR_COPIES}/${_repo}.prod.docker"
  local _latest_path="${DIR_COPIES}/${_repo}.prod.latest"

  # if path exists, skip
  [[ -d $_cp_path ]] && return

  # if path exists with dot prefix, skip
  [[ -d $_arch_path ]] && return
  
  # extract tag tree directly to target dir (no checkout in repo, ref unchanged)
  mkdir -p $_cp_path && git archive $_tag | tar -x -C $_cp_path && say "..copy files for new RELEASE [ $_tag ]"

  if [[ -L $_latest_path ]]; then
    local _cur_latest_path=$(readlink $_latest_path)
    local _cur_latest_tag=$(basename $_cur_latest_path | sed 's/.*\.prod\.//')

    if _version_less_than $_cur_latest_tag $_tag; then
      rm -f $_latest_path
      ln -sf $(basename $_cp_path) $_latest_path
    fi
  else
    ln -sf $(basename $_cp_path) $_latest_path
  fi

  # post scripts
  _handle_post ${_post_path} ${_cp_path}

  # restart docker instance
  _handle_docker ${_docker_path}
}

# expect one argument "branch_name"
function checkout_and_copy_br {
  local _repo=$1
  local _br=$2

  local _cp_path="${DIR_COPIES}/${_repo}.${_br}"
  local _post_path="${_cp_path}.post"
  local _docker_path="${_cp_path}.docker"

  # if no copy of this br, just mkdir with a skipping flag file
  # (do not actual copying files, unless admin specify it explicitly)
  [[ ! -d $_cp_path ]] && mkdir -p $_cp_path && touch $_cp_path/.skipping && say "..init dir of [ $_br ]"

  # checking flags
  if [[ -f "${_cp_path}/.debugging" ]]; then
    verbose "..skip debugging work copy of branch [ $_br ]"
    return
  fi
  if [[ -f "${_cp_path}/.skipping" ]]; then
    verbose "..skip unused branch [ $_br ]"
    return
  fi

  # current commit at origin (no checkout in repo, ref unchanged)
  local _origin_ref
  _origin_ref=$(git rev-parse origin/$_br 2>/dev/null) || { mustsay "..no origin/$_br, skip"; return; }

  # initial copy when dir is empty
  if [[ -z $(/bin/ls -A $_cp_path 2>/dev/null) ]]; then
    git archive origin/$_br | tar -x -C $_cp_path && echo -n "$_origin_ref" > "${_cp_path}/.git-rev" && say "..copy files for [ $_br ]"
  fi

  local _stored_ref
  _stored_ref=$(cat "${_cp_path}/.git-rev" 2>/dev/null)
  local _need_update=0

  # add a debug trigger
  if [[ -f "${_cp_path}/.trigger" ]]; then
    rm -f "${_cp_path}/.trigger" # burn after reading
    say "..having a debug try"
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
    say "..UPDATING branch [ $_br ]"
    if [[ -f "${_cp_path}/.no-cleanup" ]]; then
      # overwrite only, do not remove extra files
      git archive origin/$_br | tar -x -C $_cp_path
    else
      # full refresh: extract to new dir, preserve flags, then mv into place
      local _staging="${_cp_path}.staging.$$"
      mkdir -p "$_staging"
      git archive origin/$_br | tar -x -C "$_staging"
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

  cd $_repo || { err "failed to cd to $_repo, critical issue, skip"; return 1; }

  # clean up trash file from last time crash
  [[ -f .git/index.lock ]] && rm -f .git/index.lock

  say "..fetching repo ..."
  _timeout git fetch -q --all --tags --prune || { err "failed to fetch repo $_repo, skip"; return 1; }

  #for _br in `ls .git/refs/remotes/origin/`; do
  for _br in $(git branch -r | grep -v HEAD | sed -e 's/.*origin\///'); do
    [[ $_br = 'HEAD' ]] && continue
    (echo "$_br" | grep -q '/') && continue

    # check branch whitelist || repo dir exists already
    if [[ $BR_WHITELIST =~ (^|[[:space:]])$_br($|[[:space:]]) ]] || [[ -d "${DIR_COPIES}/${_repo}.${_br}" ]]; then
        checkout_and_copy_br $_repo $_br

        # heart beat
        touch "${DIR_COPIES}/${_repo}.${_br}/.living"
    fi
  done

  for _release in $(git tag -l | grep '^v[Q0-9.]\+$'); do
    checkout_and_copy_tag $_repo $_release

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
        say "..cleaning up deprecated dir: ${_bp}"
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
  
  cd $DIR_REPOS || { err "failed to cd to DIR_REPOS: $DIR_REPOS, critical issue, abort"; exit 1; }

  # loop like a daemon
  while true; do

    # Acquire lock
    acquire_lock "${CI_LOCK}" || exit 1

    for _repo in $(/bin/ls -d *); do
      if [[ -d "${_repo}/.git" ]]; then
        mustsay "[${_repo}] checking git status ..."
        (
          LOG_PREFIX="[${_repo}]"
          fetch_and_check "${_repo}"
        ) &
      fi
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

    say "waiting for next check ..."
    sleep $SLEEP_TIME
  done
}

### __main__ ###

# check for required commands
for c in git tar; do
  command -v "$c" >/dev/null || { err "missing command: $c"; exit 1; }
done
# check for optional 'docker' support
command -v docker >/dev/null || { say "docker cli not found, will skip docker restart handling"; }

## check and init all working dirs
# 1. check the DIR_BASE is available (sanitize&check at the time)
_ORIG_DIR_BASE=$DIR_BASE
DIR_BASE=$(realpath $DIR_BASE 2>/dev/null) || { err "base working dir not found: $_ORIG_DIR_BASE"; exit 1; }
# subdirs
DIR_REPOS=${DIR_BASE}/git_repos
DIR_COPIES=${DIR_BASE}/copies

# 2. DIR_BASE/copies is writable
[[ -d $DIR_COPIES ]] || mkdir -p $DIR_COPIES || { err "failed to create COPIES dir: $DIR_COPIES"; exit 1; }
[[ -w $DIR_COPIES ]] ||                         { err "COPIES dir not writable: $DIR_COPIES"; exit 1; }

# 3. init repo dir
[[ -d $DIR_REPOS ]] || mkdir -p $DIR_REPOS

# if VERB=0, keep super silent
[[ $VERB = 0 ]] && exec >/dev/null 2>&1

if [[ "${1:-}" == "--once" ]]; then
  main_loop once
else
  main_loop
fi
