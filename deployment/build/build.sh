#!/bin/sh

TODAY=$(date +%Y%m%d)

BUILDDIR=$(pwd)
TOPDIR=$(cd ../../; pwd)

cp $TOPDIR/src/*.sh $BUILDDIR
cp $TOPDIR/VERSION $BUILDDIR

# Copy supervisor for Docker build (exclude target to keep context small)
rsync -a --exclude target "$TOPDIR/supervisor" "$BUILDDIR/"

cd $TOPDIR/gh-webhook/
uv build
cp dist/*.whl $BUILDDIR
cp hook.py $BUILDDIR

cd $BUILDDIR
docker build -t rushiai/auto-reloader:$TODAY .

# clean up
rm -f *.whl hook.py check-push.sh prod2latest.sh cleanup-archives.sh VERSION
rm -rf supervisor/