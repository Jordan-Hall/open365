"""Builds input.xlsx: a conditional-format border against the neighbour's own or conditional border.

Pair k = (a, b) over STYLES x STYLES. Horizontal pairs sit in row 2 + 2k, vertical pairs in rows
2 + 3k and 3 + 3k. `a` is red, `b` is blue. A "CF" border is a dxf applied by a cellIs rule
(the cell holds 1), an "own" border is in the cell format.
  columns B C : CF right (a)  | own left (b)       column L : CF bottom (a) / own top (b)
  columns E F : own right (a) | CF left (b)        column N : own bottom (a) / CF top (b)
  columns H I : CF right (a)  | CF left (b)        column P : CF bottom (a) / CF top (b)
  column R    : one cell, own right (b, blue) under a CF right (a, red)
"""
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'styles-border-colors'))
from xlsxlib import write_xlsx, office_theme

STYLES = ['none', 'thin', 'medium', 'dashed', 'thick', 'double', 'hair']
RED, BLUE = 'FFFF0000', 'FF0000FF'
borders = ['<border><left/><right/><top/><bottom/><diagonal/></border>']
xfs = ['<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>']
own = {}
for side in ('left', 'right', 'top', 'bottom'):
    for st in STYLES:
        colour = RED if side in ('right', 'bottom') else BLUE
        parts = {n: f'<{n}/>' for n in ('left', 'right', 'top', 'bottom')}
        if st != 'none':
            parts[side] = f'<{side} style="{st}"><color rgb="{colour}"/></{side}>'
        borders.append('<border>' + parts['left'] + parts['right'] + parts['top'] + parts['bottom'] + '<diagonal/></border>')
        xfs.append(f'<xf numFmtId="0" fontId="0" fillId="0" borderId="{len(borders) - 1}" xfId="0" applyBorder="1"/>')
        own[(side, st)] = len(xfs) - 1
dxfs, dxf = [], {}
for side in ('left', 'right', 'top', 'bottom'):
    for st in STYLES[1:]:
        colour = RED if side in ('right', 'bottom') else BLUE
        dxfs.append(f'<dxf><border><{side} style="{st}"><color rgb="{colour}"/></{side}></border></dxf>')
        dxf[(side, st)] = len(dxfs) - 1
cells, cf, prio = {}, [], [0]


def put_cf(r, c, side, st, own_side=None, own_st=None):
    """A cell holding 1 with a rule giving `side` the border `st` (none: no rule)."""
    xf = own[(own_side, own_st)] if own_side else 0
    cells[(r, c)] = (xf, 1)
    if st != 'none':
        prio[0] += 1
        ref = f'{chr(64 + c)}{r}'
        cf.append(f'<conditionalFormatting sqref="{ref}"><cfRule type="cellIs" dxfId="{dxf[(side, st)]}" priority="{prio[0]}" operator="equal"><formula>1</formula></cfRule></conditionalFormatting>')


def put_own(r, c, side, st):
    cells[(r, c)] = (own[(side, st)], None)


k = 0
for a in STYLES:
    for b in STYLES:
        r = 2 + 2 * k
        put_cf(r, 2, 'right', a);            put_own(r, 3, 'left', b)
        put_own(r, 5, 'right', a);           put_cf(r, 6, 'left', b)
        put_cf(r, 8, 'right', a);            put_cf(r, 9, 'left', b)
        v = 2 + 3 * k
        put_cf(v, 12, 'bottom', a);          put_own(v + 1, 12, 'top', b)
        put_own(v, 14, 'bottom', a);         put_cf(v + 1, 14, 'top', b)
        put_cf(v, 16, 'bottom', a);          put_cf(v + 1, 16, 'top', b)
        put_cf(r, 18, 'right', a, 'right', b)
        k += 1
write_xlsx(os.path.join(os.path.dirname(__file__), 'input.xlsx'), borders=borders, xfs=xfs, cells=cells,
           dxfs=dxfs, sheet_extra=''.join(cf), theme=office_theme())
print(k, 'pairs', len(cf), 'rules')
