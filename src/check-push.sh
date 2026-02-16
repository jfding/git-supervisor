#!/usr/bin/env bash
set -u
set -o pipefail

## global config settings
: "${VERB:=1}"
: "${TIMEOUT:=600}"
: "${SLEEP_TIME:=360}"
: "${CI_LOCK:=/tmp/.auto-reloader-lock.d}"
: "${DIR_BASE:=/work}"

## derived vars
DIR_REPOS=${DIR_BASE}/git_repos
DIR_COPIES=${DIR_BASE}/copies
DIR_SCRIPTS=${DIR_BASE}/scripts

## hard coded settings
BR_WHITELIST="main master dev test alpha"

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
    local _datetime=`/bin/date '+%m-%d %H:%M:%S>'`
    if [ $_level -le $VERB ]; then
        echo $_datetime $*
    fi
}
function mustsay {
    _logging 0 $*
}
function say {
    _logging 1 $*
}
function verbose {
    _logging 2 $*
}
function err {
  mustsay "ERROR: $*"
}

# file lock
function acquire_lock {
  # mkdir is atomic; only one process can create the dir
  while ! mkdir "$CI_LOCK" 2>/dev/null; do sleep 1; done
}
function release_lock {
  rmdir "$CI_LOCK" 2>/dev/null || true
}
# make sure clean up locks on exit
trap release_lock EXIT INT TERM

function _timeout {
    if command -v timeout &>/dev/null; then
        timeout $TIMEOUT $*
    else
        $*
    fi
}

function _handle_post {
    # post scripts
    local _post_path=$1
    local _cp_path=$2

    if [[ -f ${_post_path} ]]; then
      say "..running post scripts [ $_post_path ]"
      cd ${_cp_path}
      bash "${_post_path}"
      cd - > /dev/null
    fi
}

function _handle_docker {
    # restart docker instance
    local _docker_path=$1

    if [[ -f ${_docker_path} ]]; then
      local _docker_name=`cat ${_docker_path}`

      say "..restarting docker [ $_docker_name ]"
      _timeout docker restart $_docker_name > /dev/null || err "failed to restart docker [ $_docker_name ]"
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
  
  # start to work on this br
  git checkout -q -f $_tag

  # check whether need to init all files at first
  mkdir -p $_cp_path && rsync -a --delete --exclude .git . $_cp_path && say "..copy files for new RELEASE [ $_tag ]"

  if [[ -L $_latest_path ]]; then
    local _cur_latest_path=$(readlink $_latest_path)
    local _cur_latest_tag=$(basename $_cur_latest_path | sed 's/.*.prod.//')

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
  if [[ -f ${_cp_path}/.debugging ]]; then
    verbose "..skip debugging work copy of branch [ $_br ]"
    return
  fi
  if [[ -f ${_cp_path}/.skipping ]]; then
    verbose "..skip unused branch [ $_br ]"
    return
  fi

  # start to work on this br
  git checkout -q -f $_br

  # check whether need to init all files at first
  [[ -z `/bin/ls $_cp_path` ]] && rsync -a --delete --exclude .git . $_cp_path && say "..copy files for [ $_br ]"

  local _diff=`git diff --name-only $_br origin/$_br`

  # add a debug trigger
  if [[ -f ${_cp_path}/.trigger ]]; then
    rm -f ${_cp_path}/.trigger # burn after reading

    if [[ -z $_diff ]]; then
      say "..having a debug try"
      _diff="debugging"
    fi
  fi

  if [[ -n $_diff ]]; then
      say "..UPDATING branch [ $_br ]"
      git checkout -q -B $_br origin/$_br || {
          mustsay "..failed git checkout and skip"
          return
      }
      if [[ -f ${_cp_path}/.no-cleanup ]]; then
        # if ./no-cleanup existing, do not clean up cached or built files
        rsync -a --exclude .git . $_cp_path
      else
        rsync -a --delete --exclude .git . $_cp_path
      fi

      # post scripts
      _handle_post ${_post_path} ${_cp_path}

      # restart docker instance
      _handle_docker ${_docker_path}

      else
    verbose "..no change of branch [ $_br ], skip"
  fi
}

# expect one argument "branch_name"
function fetch_and_check {
  local _repo=$1
  local _br
  local _release
  local _bp

  cd $_repo

  # clean up trash file from last time crash
  [[ -f .git/index.lock ]] && rm -f .git/index.lock

  say "..fetching repo ..."
  _timeout git fetch -q --all --tags --prune || err "failed to fetch repo $_repo"

  #for _br in `ls .git/refs/remotes/origin/`; do
  for _br in `git branch -r  | grep -v HEAD | sed -e 's/.*origin\///'`; do
    [[ $_br = 'HEAD' ]] && continue
    (echo $_br | grep -q '/') && continue

    # check branch whitelist || repo dir exists already
    if [[ $BR_WHITELIST =~ (^|[[:space:]])$_br($|[[:space:]]) ]] || [[ -d "${DIR_COPIES}/${_repo}.${_br}" ]]; then
        checkout_and_copy_br $_repo $_br

        # heart beat
        touch "${DIR_COPIES}/${_repo}.${_br}/.living"
    fi
  done

  for _release in `git tag -l  | grep '^v[Q0-9.]\+$' `; do
    checkout_and_copy_tag $_repo $_release

    # heart beat
    if [[ -d "${DIR_COPIES}/${_repo}.prod.${_release}" ]]; then
      touch "${DIR_COPIES}/${_repo}.prod.${_release}/.living"
    fi
  done

  # clean up deprected dirs in "work/copies"
  for _bp in `/bin/ls -d ${DIR_COPIES}/${_repo}.*/`; do

      (echo $_bp | grep -q to-be-removed) && continue
      (echo $_bp | grep -q .latest) && continue

      _bp=${_bp%/}

      # manually marked as deprecated
      if [ -f $_bp/.stopping ]; then
        # clean up all content
        rm -rf $_bp
        mkdir -p $_bp
        touch $_bp/.skipping
        touch $_bp/.living
      fi

      if [ -f $_bp/.living ]; then
        rm -f "$_bp/.living"
      else
        say "..cleaning up deprecated dir: $_bp"
        #rm -rf $_bp
        #rm -f ${_bp}.*
        mv $_bp $_bp.to-be-removed
      fi
  done

  cd - > /dev/null
}

function main {
  local _repo
  
  # working dir
  [[ -d $DIR_REPOS ]] || mkdir -p $DIR_REPOS

  # check scripts dir and copy scripts in docker to external
  [[ ! -d $DIR_SCRIPTS ]] && {
    # only copy scripts once when creating the dir
    mkdir -p $DIR_SCRIPTS
    rsync -a /scripts/* $DIR_SCRIPTS
  }

  # loop like a daemon
  while true; do
    # Acquire lock
    acquire_lock

    cd $DIR_REPOS
    for _repo in * ; do
      if [[ -d $_repo/.git ]]; then
        mustsay "checking git status for <$_repo>"
        fetch_and_check $_repo
      fi
    done

    # Release lock
    release_lock

    # if SLEEP_TIME value is 0, means run once and exit
    [[ $SLEEP_TIME == 0 ]] && exit 0

    say "waiting for next check ..."
    sleep $SLEEP_TIME
  done
}

## __main__ start here

# if VERB=0, keep super silent
[[ $VERB = 0 ]] && exec >/dev/null 2>&1

for c in git rsync docker; do
  command -v "$c" >/dev/null || { err "missing command: $c"; exit 1; }
done

if [[ "${1:-}" == "once" ]]; then
  SLEEP_TIME=0
  main
else
  main
fi