type Point = tuple[float, float]
type LabeledPoint = tuple[Point, str]


def make_point(x: float, y: float) -> Point:
    return (x, y)


def located(flag: bool, location: Point) -> LabeledPoint:
    if flag:
        return (location, "known")
    else:
        return ((0.0, 0.0), "unknown")
