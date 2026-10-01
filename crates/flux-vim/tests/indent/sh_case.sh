#!/bin/sh
case "$1" in
    start)
        echo starting
        ;;
    stop|halt)
        echo stopping
        ;;
    restart) echo restarting ;;
    *)
        echo "usage: $0 start|stop" >&2
        exit 1
        ;;
esac

case $x in
    (a) echo a ;;
    (b)
        echo b
        ;;
esac

case "$mode" in
    debug)
        set -x
        ;&
    verbose)
        echo verbose
        ;;
esac
echo done
