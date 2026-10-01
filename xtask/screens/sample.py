#!/usr/bin/env python3
"""A sample for comparing highlights."""
import os
from typing import Optional


class Greeter:
    """Says hello."""

    count: int = 0

    def __init__(self, name: str, loud: bool = False) -> None:
        self.name = name
        self.loud = loud

    @property
    def greeting(self) -> str:
        msg = f"Hello, {self.name}!"
        return msg.upper() if self.loud else msg


def main(argv: Optional[list] = None) -> int:
    for i in range(3):
        if i % 2 == 0 and not None:
            print(i, True, 3.14, [1, 2], {"a": 1})
    try:
        os.getcwd()
    except OSError as e:
        raise RuntimeError("no cwd") from e
    lam = lambda x: x * 2  # noqa
    return 0
