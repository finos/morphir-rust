from __future__ import annotations
from models import Decision, Point


def select_decision(flag: bool, first: Decision, second: Decision) -> Decision:
    if flag:
        return first
    else:
        return second


def origin() -> Point:
    return (0.0, 0.0)
