#!/bin/sh
./configure \
    --prefix=/usr \
    --enable-foo
make

grep foo file.txt |
    sort |
    uniq -c
echo next

test -f a.txt &&
    echo exists
echo after

[ -d dir ] || {
    mkdir dir
}

command_one && command_two \
    && command_three
echo end
