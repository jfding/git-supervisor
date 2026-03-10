#!/bin/sh

METADIR=$(pwd)
TOPDIR=$(cd ../../; pwd)
BUILDDIR=$TOPDIR

TODAY=$(date +%Y%m%d)
LATEST_TAG="v$(cat $TOPDIR/VERSION)"

# if current commit of git is not at the same as latest_tag, then append TODAY to the tag
# Use 'git rev-list -n 1' to handle both lightweight and annotated tags
if [ "$(git rev-parse HEAD)" != "$(git rev-list -n 1 $LATEST_TAG)" ]; then
    TAG="${LATEST_TAG}-${TODAY}"
else
    TAG="${LATEST_TAG}"
fi

cd $BUILDDIR
docker build -f ${METADIR}/Dockerfile -t rushiai/auto-reloader:$TAG .