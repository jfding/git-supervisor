#!/bin/sh

TODAY=$(date +%Y%m%d)

BUILDDIR=$(pwd)
TOPDIR=$(cd ../../; pwd)

cp $TOPDIR/src/*.sh $BUILDDIR
cp $TOPDIR/VERSION $BUILDDIR

# Copy Rust source code for Docker build
mkdir -p $BUILDDIR/src/check-push-rs
cp -r $TOPDIR/check-push-rs/* $BUILDDIR/src/check-push-rs/

cd $TOPDIR/src/gh-webhook/
uv build
cp dist/*.whl $BUILDDIR
cp hook.py $BUILDDIR

cd $BUILDDIR
docker build -t rushiai/auto-reloader:$TODAY .

# clean up
rm -f *.whl hook.py check-push.sh prod2latest.sh cleanup-archives.sh VERSION
rm -rf src/
