"""Private strict JSON decoding shared by storage and snapshot readers."""

import json
import math


def _object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate object key {key!r}")
        value[key] = item
    return value


def _constant(value):
    raise ValueError(f"nonstandard JSON number {value}")


def _float(value):
    number = float(value)
    if not math.isfinite(number):
        raise ValueError(f"JSON number exceeds finite float64 range: {value}")
    return number


def decode(data):
    return json.loads(
        data,
        object_pairs_hook=_object,
        parse_constant=_constant,
        parse_float=_float,
    )
