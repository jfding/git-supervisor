#!/bin/bash

# Script to cleanup the files in <volume-work>/copies/.archives/*/*, to save disk space.

# helper function to ask yes/no input
function asking() {
    local question=$1
    local default=Y
    [[ -n $2 ]] && default=$2

    while true; do
        if [[ $YES == "1" ]]; then
            yn=$default
        else
            read -p "$question (y/n) [$default]: " yn
        fi
        yn=${yn:-$default}  # use default value if no input
        case $yn in
            [Yy]* ) return 0;;
            [Nn]* ) return 1;;
            * ) echo "Please answer yes (y) or no (n).";;
        esac
    done
}

function usage {
    echo "Usage: $0 [--yes] <work-path>"
    echo "  <work-path>: the path to the work volume"
    echo "  Will cleanup the files in <work-path>/copies/.archives/*/*"
    echo "  [--yes]: answer yes to all questions"
    exit 1
}

## main start here

if [[ $1 == "--yes" ]]; then
    YES="1"
    shift
fi

if [[ -z $1 ]]; then
    usage
fi

WORK_PATH=$(realpath $1)
ARCHIVES_PATH="$WORK_PATH/copies/.archives"

if [[ ! -d $WORK_PATH ]]; then
    echo "Error: <work-path> does not exist"
    exit 1
fi

if [[ ! -d $WORK_PATH/copies/ ]]; then
    echo "Error: <work-path>/copies/ does not exist, not a valid path?"
    exit 1
fi

if [[ ! -d $ARCHIVES_PATH ]]; then
    # nothing to do, quit quietly
    mkdir -p $ARCHIVES_PATH
    exit 0
fi

cd $ARCHIVES_PATH

_total_freed_size=0
for copy_dir in $(find . -mindepth 1 -maxdepth 1 -type d); do
    cd $copy_dir
    if asking "Cleaning up $(realpath $copy_dir)"; then
        # get the du of this dir
        _du_size=$(du -sb | awk '{print $1}')
        _total_freed_size=$((_total_freed_size + _du_size))
        rm -rf *
    fi
    cd ..
done

# make $total_freed_size human readable
_total_freed_size=$(numfmt --to=iec $_total_freed_size)
echo "Total freed space: $_total_freed_size"

unset _du_size
unset _total_freed_size