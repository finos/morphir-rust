from collections.abc import Callable
def identity(value: int) -> int:
    return value
def apply(transform: Callable[[int], int], value: int) -> int:
    return transform(value)
def capture(value: int) -> Callable[[int], int]:
    return lambda ignored: value
def shadow(value: int) -> Callable[[int], int]:
    return lambda value: value
def curry(flag: bool) -> Callable[[int], Callable[[int], int]]:
    return lambda first: lambda second: first if flag else second
def choose(flag: bool, first: int, second: int) -> int:
    return first if flag else second
def run(value: int) -> int:
    return choose(True, apply(lambda item: identity(item), value), curry(False)(1)(2))
def recurse(flag: bool, value: int) -> int:
    return recurse(False, value) if flag else value
def constant() -> int:
    return 42
def read_constant(value: int) -> int:
    return constant()

def immediate(value: int) -> int:
    return (lambda item: item)(value)

def tupled(value: int) -> tuple[Callable[[int], int], int]:
    return (lambda ignored: value, value)

def conditional(flag: bool) -> Callable[[int], int]:
    return identity if flag else lambda item: item

def forward(value: int) -> int:
    return later(value)

def later(value: int) -> int:
    return value
