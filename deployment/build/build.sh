#!/bin/sh

METADIR=$(realpath $(dirname "$0"))
TOPDIR=$(realpath "$METADIR"/../..)
BUILDDIR=$TOPDIR

TODAY=$(date +%Y%m%d)
LATEST_TAG="v$(grep '^version' $TOPDIR/Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"

# if current commit of git is not at the same as latest_tag, then append TODAY to the tag
# Use 'git rev-list -n 1' to handle both lightweight and annotated tags
if [ "$(git rev-parse HEAD)" != "$(git rev-list -n 1 $LATEST_TAG)" ]; then
    TAG="${LATEST_TAG}-${TODAY}"
else
    TAG="${LATEST_TAG}"
fi

cd $BUILDDIR
docker build -f ${METADIR}/Dockerfile -t rushiai/git-supervisor:$TAG .
