# Auto reload scripts and web hook for Rushi App DevOps

## Scripts

- `src/check-push.sh`: **main** logic of the engine, can be called by web hook or by timer loop
- `src/prod2latest.sh`: shell script to be run in **HOST** env, to figure out the latest version
  release code copy and update the latest symlinks.
- `src/cleanup-archives.sh`: shell script to clean up archive files under *<work>/copies/.archives/*

## Sub-project

- gh-webhook: hook service to listen for github.com callbacks. Once triggered, will run
  check-push.sh shell script to have one-shot check.
- check-push-rs: Re-implimentation of check-push.sh in Rust
- supervisor: central supervisor for to control all remote hosts and repos

## Usage

### Setup

- Sample settings in docker-compose.yml in the code tree.
- Volume <work> to store all the data: git_repos, (code)copies, scripts.
- Volumn <keys> to store the ssh keys to access github.com repos.

### Web hook for github repos

It's the default command entry for docker image, will listen on :9870 port.

### Timer loop to check status of repos

If want to run a timer loop instead of web-hook, need to:

- Must set SLEEP_TIME env for docker-run, to specify the timeout values(seconds)
- Specify the **command** as `/srcripts/check-push.sh` for docker-run
- If no SLEEP_TIME env, the script will be run as one-shot checking.

### Configuration (check-push.sh)

- **BR_WHITELIST**: Space-separated branch names to track and copy by default (e.g. `main master dev`). Override via env; default in script: `main master dev test alpha`. Whitelisted branches get their copy dir created and populated on first run; other branches are only tracked if a copy dir already exists (and then start with a `.skipping` flag until you remove it).

### Init working git repos

In HOST, under the path *<work-volume>/git_repos/*, just use the regular `git clone` the target repos.

### How to run util scripts in HOST

All the scripts will be visible to HOST in the path: *<work-volume>/scripts/*.

## how to test

- first time to launch all tests: `./tests/launch-testing.sh`
- if testing env is ready, to run: `./tests/scripts/test-check-push.sh`
- to clean up test env, to run: `./tests/scripts/cleanup-test.sh`

After everytime to run the test scripts, the results can be checked in `./tests/work.test/copies/`
