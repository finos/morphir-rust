from __future__ import annotations
from dataclasses import dataclass

@dataclass(frozen=True)
class Address:
    street: str
    zip_code: int

@dataclass(frozen=True)
class Pending:
    pass

@dataclass(frozen=True)
class Approved:
    amount: float
    address: Address

type Decision = Pending | Approved

@dataclass(frozen=True)
class Application:
    decision: Decision
    verified: bool
    coordinates: tuple[float, float]
