"""Builds minimal OOXML workbooks from raw XML fragments (probe inputs for the Excel oracle).

Only generated data: no personal content. Used by the build.py of every styles-* fixture:

    import sys, os; sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'styles-border-colors'))
    from xlsxlib import write_xlsx
"""
import os
import zipfile

NS = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
RNS = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'


def col_name(c):
    s = ''
    while c:
        c, r = divmod(c - 1, 26)
        s = chr(65 + r) + s
    return s


def write_xlsx(path, *, borders=('<border><left/><right/><top/><bottom/><diagonal/></border>',),
               fills=None, fonts=None, xfs=('<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>',),
               dxfs=(), cells=None, sheet_extra='', table_parts=(), table_styles='', cols='',
               styles_extra_after_xfs='', cell_style_xfs=None, extra_sheet_prefix='', theme=None):
    """cells: {(row, col): (xf_index, value_or_None)}; tables: [(name, xml)] in table_parts."""
    fills = fills or ('<fill><patternFill patternType="none"/></fill>', '<fill><patternFill patternType="gray125"/></fill>')
    fonts = fonts or ('<font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font>',)
    cell_style_xfs = cell_style_xfs or ('<xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>',)
    styles = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        f'<styleSheet xmlns="{NS}">'
        f'<fonts count="{len(fonts)}">{"".join(fonts)}</fonts>'
        f'<fills count="{len(fills)}">{"".join(fills)}</fills>'
        f'<borders count="{len(borders)}">{"".join(borders)}</borders>'
        f'<cellStyleXfs count="{len(cell_style_xfs)}">{"".join(cell_style_xfs)}</cellStyleXfs>'
        f'<cellXfs count="{len(xfs)}">{"".join(xfs)}</cellXfs>'
        '<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>'
        f'<dxfs count="{len(dxfs)}">{"".join(dxfs)}</dxfs>'
        f'{table_styles or "<tableStyles count=\"0\" defaultTableStyle=\"TableStyleMedium2\" defaultPivotStyle=\"PivotStyleLight16\"/>"}'
        '</styleSheet>')
    rows = {}
    for (r, c), (xf, v) in (cells or {}).items():
        rows.setdefault(r, {})[c] = (xf, v)
    sd = []
    for r in sorted(rows):
        cs = []
        for c in sorted(rows[r]):
            xf, v = rows[r][c]
            ref = f'{col_name(c)}{r}'
            s = f' s="{xf}"' if xf else ''
            if v is None:
                cs.append(f'<c r="{ref}"{s}/>')
            elif isinstance(v, str):
                cs.append(f'<c r="{ref}"{s} t="inlineStr"><is><t>{v}</t></is></c>')
            else:
                cs.append(f'<c r="{ref}"{s}><v>{v}</v></c>')
        sd.append(f'<row r="{r}">{"".join(cs)}</row>')
    legacy = ''.join(f'<tablePart r:id="rId{i + 1}"/>' for i in range(len(table_parts)))
    if legacy:
        legacy = f'<tableParts count="{len(table_parts)}">{legacy}</tableParts>'
    sheet = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        f'<worksheet xmlns="{NS}" xmlns:r="{RNS}">{extra_sheet_prefix}'
        f'{cols}<sheetData>{"".join(sd)}</sheetData>{sheet_extra}{legacy}</worksheet>')
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as z:
        ct = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
              '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
              '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
              '<Default Extension="xml" ContentType="application/xml"/>'
              '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
              '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
              '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>'
              + ('<Override PartName="/xl/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>' if theme else '')
              + ''.join(f'<Override PartName="/xl/tables/table{i + 1}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>' for i in range(len(table_parts)))
              + '</Types>')
        z.writestr('[Content_Types].xml', ct)
        z.writestr('_rels/.rels', '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>')
        z.writestr('xl/workbook.xml', f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<workbook xmlns="{NS}" xmlns:r="{RNS}"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>')
        z.writestr('xl/_rels/workbook.xml.rels', '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>' + ('<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>' if theme else '') + '</Relationships>')
        z.writestr('xl/styles.xml', styles)
        if theme:
            z.writestr('xl/theme/theme1.xml', theme)
        z.writestr('xl/worksheets/sheet1.xml', sheet)
        if table_parts:
            z.writestr('xl/worksheets/_rels/sheet1.xml.rels', '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
                       + ''.join(f'<Relationship Id="rId{i + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table{i + 1}.xml"/>' for i in range(len(table_parts))) + '</Relationships>')
            for i, t in enumerate(table_parts):
                z.writestr(f'xl/tables/table{i + 1}.xml', t)


def office_theme():
    """The theme part of fixtures/xlsx/format-blank.xlsx (written by Excel, generated data)."""
    here = os.path.dirname(os.path.abspath(__file__))
    blank = os.path.join(here, '..', '..', '..', 'xlsx', 'format-blank.xlsx')
    with zipfile.ZipFile(blank) as z:
        return z.read('xl/theme/theme1.xml').decode('utf8')
