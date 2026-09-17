def choose(flag: bool, first: int, second: int) -> int:
    if flag:
        return first
    else:
        return second


def classify(value: int) -> str:
    if value < 0:
        return "negative"
    elif value == 0:
        return "zero"
    else:
        return "positive"


def nested(first: bool, second: bool) -> int:
    if first:
        if second:
            return 1
        else:
            return 2
    else:
        return 3


def guarded(first: bool, second: bool) -> int:
    if first:
        if second:
            return 1
    return 2


def expression(flag: bool) -> float:
    return -1.5 if flag else 2.5
