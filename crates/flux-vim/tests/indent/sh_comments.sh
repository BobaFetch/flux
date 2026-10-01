#!/bin/sh
# Header comment.

setup() {
    echo setting up

    # In a block, after a blank line.
    echo more
    # Right after a line.
    if [ "$x" = "{" ]; then
        echo "brace in a string: }"
    fi
}

while :; do
    # comment in a loop
    sleep 1
done

echo "if inside quotes"
echo 'fi'
