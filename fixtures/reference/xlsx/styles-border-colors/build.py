"""Builds input.xlsx: one cell per border-colour form (all four edges + diagonal) and a CF dxf per form."""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from xlsxlib import write_xlsx, office_theme

FORMS = [
    '<color auto="1"/>',
    '<color indexed="64"/>',
    '<color indexed="65"/>',
    '<color indexed="10"/>',
    '<color indexed="8"/>',
    '<color indexed="0"/>',
    '<color indexed="63"/>',
    '<color rgb="FF3F3F3F"/>',
    '<color rgb="003F3F3F"/>',
    '<color rgb="803F3F3F"/>',
    '<color rgb="FFFF0000"/>',
    '<color theme="0"/>',
    '<color theme="1"/>',
    '<color theme="4"/>',
    '<color theme="4" tint="0.39994506668294322"/>',
    '<color theme="2" tint="-0.499984740745262"/>',
    '<color theme="6" tint="0.79998168889431442"/>',
    '<color theme="5" tint="0"/>',
    '<color theme="5" tint="0.5"/>',
    '<color theme="3" tint="-9.9978637043366805E-2"/>',
    '<color indexed="10" tint="0.5"/>',
    '<color rgb="FF00FF00" tint="-0.25"/>',
    '<color auto="1" tint="0.5"/>',
    '<color/>',
    '',  # no colour element
]
borders = ['<border><left/><right/><top/><bottom/><diagonal/></border>']
for f in FORMS:
    e = lambda n: f'<{n} style="thin">{f}</{n}>'
    borders.append(f'<border diagonalUp="1" diagonalDown="1">{e("left")}{e("right")}{e("top")}{e("bottom")}{e("diagonal")}</border>')
xfs = ['<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>']
cells = {}
for i, f in enumerate(FORMS):
    xfs.append(f'<xf numFmtId="0" fontId="0" fillId="0" borderId="{i + 1}" xfId="0" applyBorder="1"/>')
    cells[(2 + 2 * i, 2)] = (i + 1, None)
dxfs = []
cf = ''
for i, f in enumerate(FORMS):
    e = lambda n: f'<{n} style="thin">{f}</{n}>'
    dxfs.append(f'<dxf><border>{e("left")}{e("right")}{e("top")}{e("bottom")}</border></dxf>')
    r = 2 + 2 * i
    cells[(r, 5)] = (0, 1)
    cf += f'<conditionalFormatting sqref="E{r}"><cfRule type="cellIs" dxfId="{i}" priority="{i + 1}" operator="equal"><formula>1</formula></cfRule></conditionalFormatting>'
write_xlsx(os.path.join(os.path.dirname(__file__), 'input.xlsx'), borders=borders, xfs=xfs, cells=cells,
           dxfs=dxfs, sheet_extra=cf, theme=office_theme())
print(len(FORMS), 'forms')
