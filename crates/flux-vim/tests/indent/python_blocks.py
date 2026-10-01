import os
import sys


class Stack:
    """A stack.

    With a docstring that spans lines.
    """

    def __init__(self, items=None):
        self.items = list(items or [])

    def push(self, item):
        self.items.append(item)
        return self

    def pop(self):
        if not self.items:
            raise IndexError("pop from empty stack")
        return self.items.pop()

    @property
    def size(self):
        return len(self.items)


def classify(n):
    if n < 0:
        return "negative"
    elif n == 0:
        return "zero"
    else:
        pass
    for i in range(n):
        if i % 2:
            continue
        if i > 10:
            break
        print(i)
    while True:
        n -= 1
        if n < 0:
            break
    return "positive"


def safe(path):
    try:
        with open(path) as f:
            return f.read()
    except OSError as e:
        print(e, file=sys.stderr)
    except ValueError:
        pass
    finally:
        print("done")


if __name__ == "__main__":
    print(classify(int(sys.argv[1])))
