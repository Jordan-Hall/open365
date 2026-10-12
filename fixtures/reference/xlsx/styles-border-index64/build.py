"""Builds input.xlsx: one bordered cell whose edges use indexed 64 only (no auto colour in the file)."""
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'styles-border-colors'))
from xlsxlib import write_xlsx, office_theme

e = lambda n: f'<{n} style="thin"><color indexed="64"/></{n}>'
borders = ['<border><left/><right/><top/><bottom/><diagonal/></border>',
           f'<border>{e("left")}{e("right")}{e("top")}{e("bottom")}<diagonal/></border>']
xfs = ['<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>',
       '<xf numFmtId="0" fontId="0" fillId="0" borderId="1" xfId="0" applyBorder="1"/>']
write_xlsx(os.path.join(os.path.dirname(__file__), 'input.xlsx'), borders=borders, xfs=xfs,
           cells={(2, 2): (1, None)}, theme=office_theme())
