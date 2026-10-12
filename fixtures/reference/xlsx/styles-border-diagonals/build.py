"""Builds input.xlsx: diagonal borders. Row 2 + 2k, columns B..E draw the diagonal style k with the
four flag combinations (none, up, down, both) in red; column G does it for a cell whose neighbour to
the right (H) carries a left border and column J has a diagonal plus a conflicting right edge."""
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'styles-border-colors'))
from xlsxlib import write_xlsx, office_theme

STYLES = ['thin', 'medium', 'dashed', 'dotted', 'thick', 'double', 'hair', 'mediumDashed',
          'dashDot', 'mediumDashDot', 'dashDotDot', 'mediumDashDotDot', 'slantDashDot']
borders = ['<border><left/><right/><top/><bottom/><diagonal/></border>']
xfs = ['<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>']
cells = {}
FLAGS = [('', ''), (' diagonalUp="1"', ''), ('', ' diagonalDown="1"'), (' diagonalUp="1"', ' diagonalDown="1"')]
for k, st in enumerate(STYLES):
    for j, (up, down) in enumerate(FLAGS):
        borders.append(f'<border{up}{down}><left/><right/><top/><bottom/><diagonal style="{st}"><color rgb="FFFF0000"/></diagonal></border>')
        xfs.append(f'<xf numFmtId="0" fontId="0" fillId="0" borderId="{len(borders) - 1}" xfId="0" applyBorder="1"/>')
        cells[(2 + 2 * k, 2 + j)] = (len(xfs) - 1, None)
# a diagonal cell between neighbours that draw the shared edges: nothing is shared with a diagonal
borders.append('<border diagonalUp="1" diagonalDown="1"><left style="thin"><color rgb="FF0000FF"/></left><right style="thick"><color rgb="FF0000FF"/></right><top/><bottom/><diagonal style="medium"><color rgb="FFFF0000"/></diagonal></border>')
xfs.append(f'<xf numFmtId="0" fontId="0" fillId="0" borderId="{len(borders) - 1}" xfId="0" applyBorder="1"/>')
cells[(2, 8)] = (len(xfs) - 1, None)
# the same diagonal flags but diagonal element without style (flags alone draw nothing)
borders.append('<border diagonalUp="1" diagonalDown="1"><left/><right/><top/><bottom/><diagonal/></border>')
xfs.append(f'<xf numFmtId="0" fontId="0" fillId="0" borderId="{len(borders) - 1}" xfId="0" applyBorder="1"/>')
cells[(4, 8)] = (len(xfs) - 1, None)
# a merged cell with a diagonal: A2:B3 style drawn on every member
write_xlsx(os.path.join(os.path.dirname(__file__), 'input.xlsx'), borders=borders, xfs=xfs, cells=cells,
           sheet_extra='', theme=office_theme())
