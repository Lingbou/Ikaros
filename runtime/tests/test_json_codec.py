from __future__ import annotations

import pytest

from ikaros_runtime.json_codec import MAX_SAFE_INTEGER, dumps, loads


@pytest.mark.parametrize(
    "source",
    [
        str(MAX_SAFE_INTEGER + 1),
        str(-MAX_SAFE_INTEGER - 1),
        f"{MAX_SAFE_INTEGER + 2}.0",
        f"{-MAX_SAFE_INTEGER - 2}.0",
        "9.007199254740993e15",
        "-9.007199254740993e15",
        "1" + ("0" * 400),
        "NaN",
        "Infinity",
        "-Infinity",
        "1e9999",
        "-1e9999",
    ],
)
def test_loads_rejects_numbers_that_are_unsafe_for_javascript(source: str) -> None:
    with pytest.raises(ValueError):
        loads('{"value":' + source + "}")


def test_loads_accepts_safe_integer_boundaries() -> None:
    assert loads(f"[{MAX_SAFE_INTEGER},{-MAX_SAFE_INTEGER}]") == [
        MAX_SAFE_INTEGER,
        -MAX_SAFE_INTEGER,
    ]


def test_loads_accepts_safe_integer_boundaries_written_as_floats() -> None:
    assert loads(f"[{MAX_SAFE_INTEGER}.0,{-MAX_SAFE_INTEGER}.0]") == [
        float(MAX_SAFE_INTEGER),
        float(-MAX_SAFE_INTEGER),
    ]


@pytest.mark.parametrize(
    "value",
    [
        MAX_SAFE_INTEGER + 1,
        -MAX_SAFE_INTEGER - 1,
        float(MAX_SAFE_INTEGER + 1),
        float(-MAX_SAFE_INTEGER - 1),
        float("nan"),
        float("inf"),
    ],
)
def test_dumps_rejects_unsafe_numbers_at_any_depth(value: int | float) -> None:
    with pytest.raises(ValueError):
        dumps({"nested": [{"value": value}]})


def test_dumps_accepts_safe_integer_boundaries() -> None:
    assert dumps(
        {
            "values": [
                MAX_SAFE_INTEGER,
                -MAX_SAFE_INTEGER,
                float(MAX_SAFE_INTEGER),
                float(-MAX_SAFE_INTEGER),
            ]
        }
    ) == (
        f'{{"values": [{MAX_SAFE_INTEGER}, {-MAX_SAFE_INTEGER}, '
        f"{MAX_SAFE_INTEGER}.0, {-MAX_SAFE_INTEGER}.0]}}"
    )
