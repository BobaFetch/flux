#!/bin/sh
greet() {
    local who="$1"
    echo "hello, $who"
}

function cleanup {
    rm -f "$tmp"
}

function usage() {
    cat >&2 <<EOF2
usage: $0 [options]
  -h  help
EOF2
}

trap cleanup EXIT

main()
{
    greet world
    (
        cd /tmp
        ls
    )
    { echo grouped; echo block; }
    {
        echo braces
    } > out.txt
}

main "$@"
