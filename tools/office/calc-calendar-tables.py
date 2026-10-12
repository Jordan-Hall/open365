"""Builds the calendar tables of crates/calc/src/numfmt (lunisolar_f.bin, lunisolar_e.bin, persian_leap.bin) from Excel's own output.

Input: the file written by `make-numfmt-calc-refs.ps1 -Mode Series` for the cases
  {"id": "fullF", "code": "[$-F0409]yyyy-m-d", "start": 1, "count": 2958465}
  {"id": "fullE", "code": "[$-E0409]yyyy-m-d", "start": 1, "count": 2958465}
  {"id": "per",   "code": "[$-160409]yyyy-m-d", "start": 1, "count": 2958465}   (Persian: the leap years)
(one file with all ids, or one file per id). Every value is "year-month-day" in the lunisolar calendar of that tag, or
"#VALUE!" for serial 60.

Months are keyed by real day number (serial 60 does not exist; serial 61 is day 60).

Output: a little-endian u32 count, that many u32 real days (bit 31 set when the day number stays the same) on which the year number goes up in the middle of a month
(Gregorian 1 January of 1960, 2051, 2142, ... every 91 years; bit 31 set when the day number repeats there), then one byte per lunisolar month (bits 0-4 length in days, bit 7 the year number goes up by one, bit 6 the month
number restarts at 1, otherwise it goes up by one; the two bits are set together for an ordinary new year, but Excel's
approximated stretches also have a new year whose first month is numbered 13). The script rebuilds every
measured value from the table and stops if one differs, so the table is a lossless copy of what Excel returned.

  python tools/office/calc-calendar-tables.py series-file.json [more-series-files.json ...]
"""

import json
import os
import struct
import sys

FIRST_START = -14
FIRST_YEAR, FIRST_MONTH = 1899, 12
OUT = os.path.join(os.path.dirname(__file__), "..", "..", "crates", "calc", "src", "numfmt")


def real_day(serial):
    """Excel counts the fictitious 29 February 1900 as serial 60; the calendars count real days."""
    return serial if serial < 60 else serial - 1


def parse(v):
    y, m, d = v.split("-")
    return int(y), int(m), int(d)


def build(values, start):
    """Returns (bytes, bumps, months): `bumps` are the real days on which the year number goes up in the middle of a month."""
    months = []  # (start day, year at the start, month number)
    bumps = []
    prev = None
    for i, v in enumerate(values):
        if v.startswith("#"):
            continue
        day = real_day(start + i)
        y, m, d = parse(v)
        if d == 1 or not months:
            months.append((day - (d - 1), y, m))
        elif prev is not None and y != prev[0]:
            assert y == prev[0] + 1 and m == prev[1] and d in (prev[2] + 1, prev[2]), ("mid-month year change", day, prev, v)
            bumps.append((day, d == prev[2]))  # the day number may repeat (a 31-day month)
        prev = (y, m, d)
    data = bytearray()
    for i, (s, y, m) in enumerate(months):
        nxt = months[i + 1][0] if i + 1 < len(months) else s + 30
        length = nxt - s
        assert 0 < length < 32, (s, y, m, length)
        flag = 0
        if i > 0:
            ps, py, pm = months[i - 1]
            end_year = py + len(bumps_in(bumps, ps, s))
            assert y in (end_year, end_year + 1), ("year sequence", s, y, m, end_year, pm)
            if y == end_year + 1:
                flag |= 0x80
            if m == 1:
                flag |= 0x40
            else:
                assert m == pm + 1, ("month sequence", s, y, m, py, pm)
        data.append(length | flag)
    assert (months[0][0], months[0][1], months[0][2]) == (FIRST_START, FIRST_YEAR, FIRST_MONTH), months[0]
    return bytes(data), bumps, months


def bumps_in(bumps, lo, hi):
    return [b for b in bumps if lo <= b[0] < hi]


def check(values, start, data, bumps):
    s, y, m = FIRST_START, FIRST_YEAR, FIRST_MONTH
    out = {}
    for i, b in enumerate(data):
        if i > 0:
            y += 1 if b & 0x80 else 0
            m = 1 if b & 0x40 else m + 1
        for d in range(b & 0x1F):
            out[s + d] = (y, m, d + 1)
        bump = bumps_in(bumps, s, s + (b & 0x1F))
        for day, rep in bump:
            for d in range(day, s + (b & 0x1F)):
                out[d] = (y + 1, m, d - s + 1 - (1 if rep else 0))
        s += b & 0x1F
        y += len(bump)
    for i, v in enumerate(values):
        if v.startswith("#"):
            continue
        serial = real_day(start + i)
        if serial not in out:
            continue  # the last month is open ended
        assert out[serial] == parse(v), (serial, v, out[serial])


def persian(values, start):
    """Bit i (LSB first) of the output is 1 when Persian year 1279 + i has 366 days in Excel (years 1279 to 9377; Excel's last
    year, 9378, is cut off by the end of the date range)."""
    starts = {}
    months = {}
    for i, v in enumerate(values):
        if v.startswith("#"):
            continue
        y, m, d = parse(v)
        months[(y, m)] = max(months.get((y, m), 0), d)
        if m == 1 and d == 1:
            starts[y] = start + i
    first, last = min(starts), max(starts) - 1
    assert first == 1279 and last == 9377, (first, last)
    leap = {y: starts[y + 1] - starts[y] == 366 for y in range(first, last + 1)}
    for (y, m), d in months.items():
        if y in leap:
            assert d == (31 if m <= 6 else 30 if m <= 11 or leap[y] else 29), ("month length", y, m, d)
    bits = bytearray((last - first + 1 + 7) // 8)
    for y in range(first, last + 1):
        if leap[y]:
            bits[(y - first) // 8] |= 1 << ((y - first) % 8)
    return bytes(bits)


def main(paths):
    found = {}
    for p in paths:
        for case in json.load(open(p, encoding="utf8")):
            found[case["id"]] = case
    if "per" in found:
        case = found["per"]
        data = persian(case["values"], int(case["start"]))
        with open(os.path.join(OUT, "persian_leap.bin"), "wb") as f:
            f.write(data)
        print("persian_leap.bin", len(data), "bytes")
    for ident, name in (("fullF", "lunisolar_f.bin"), ("fullE", "lunisolar_e.bin")):
        if ident not in found:
            continue
        case = found[ident]
        data, bumps, months = build(case["values"], int(case["start"]))
        check(case["values"], int(case["start"]), data, bumps)
        with open(os.path.join(OUT, name), "wb") as f:
            f.write(struct.pack("<I", len(bumps)))
            f.write(b"".join(struct.pack("<I", day | (0x80000000 if rep else 0)) for day, rep in bumps))
            f.write(data)
        print(name, len(data), "months,", len(bumps), "mid-month year changes")


if __name__ == "__main__":
    main(sys.argv[1:])
