"""Builds input.xlsx: every pair of border styles on the two sides of a shared edge.

Pair k = (a, b) over STYLES x STYLES. Horizontal neighbours: row 2 + 2k, column B draws `a` on its
right edge in red, column C draws `b` on its left edge in blue. Vertical neighbours: rows 2 + 3k and
3 + 3k of column F, the upper cell draws `a` on its bottom edge in red, the lower `b` on its top
edge in blue.
"""
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'styles-border-colors'))
from xlsxlib import write_xlsx, office_theme

STYLES = ['none', 'thin', 'medium', 'dashed', 'dotted', 'thick', 'double', 'hair', 'mediumDashed',
          'dashDot', 'mediumDashDot', 'dashDotDot', 'mediumDashDotDot', 'slantDashDot']
RED, BLUE = 'FFFF0000', 'FF0000FF'
borders = ['<border><left/><right/><top/><bottom/><diagonal/></border>']
xfs = ['<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>']
index = {}
for side, colour in (('right', RED), ('left', BLUE), ('bottom', RED), ('top', BLUE)):
    for st in STYLES:
        if st == 'none':
            edge = f'<{side} style="none"/>' if False else f'<{side}/>'
        else:
            edge = f'<{side} style="{st}"><color rgb="{colour}"/></{side}>'
        parts = {n: f'<{n}/>' for n in ('left', 'right', 'top', 'bottom')}
        parts[side] = edge
        borders.append('<border>' + parts['left'] + parts['right'] + parts['top'] + parts['bottom'] + '<diagonal/></border>')
        xfs.append(f'<xf numFmtId="0" fontId="0" fillId="0" borderId="{len(borders) - 1}" xfId="0" applyBorder="1"/>')
        index[(side, st)] = len(xfs) - 1
cells = {}
k = 0
for a in STYLES:
    for b in STYLES:
        r = 2 + 2 * k
        cells[(r, 2)] = (index[('right', a)], None)
        cells[(r, 3)] = (index[('left', b)], None)
        r = 2 + 3 * k
        cells[(r, 6)] = (index[('bottom', a)], None)
        cells[(r + 1, 6)] = (index[('top', b)], None)
        k += 1
write_xlsx(os.path.join(os.path.dirname(__file__), 'input.xlsx'), borders=borders, xfs=xfs, cells=cells,
           theme=office_theme())
print(k, 'pairs')
