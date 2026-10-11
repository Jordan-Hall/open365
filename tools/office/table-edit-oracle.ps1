<#
Word table-editing oracle: what Microsoft Word 16.0 does when a table is edited.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this is
never run in CI. CI consumes the committed outputs: fixtures/reference/docx/tbledit-<scenario>/table.tsv.

  pwsh -NoProfile -File tools/office/table-edit-oracle.ps1 -Make          # (re)creates fixtures/docx/tbledit-base-*.docx
  pwsh -NoProfile -File tools/office/table-edit-oracle.ps1                # runs every scenario in Word
  pwsh -NoProfile -File tools/office/table-edit-oracle.ps1 -Only insert-3x3

Each scenario copies one base fixture into a private temp folder, opens the copy in a Word instance this script
starts (AutomationSecurity = 3: macros never run), applies the edit through the Word object model (the same calls
a recorded UI macro makes), saves the copy and reads the saved word/document.xml. The fixtures in fixtures/docx
are never opened for writing. Only the Word process this script started is stopped, in the finally block; the
temp copies (explicit files this script created) are deleted at the end. No Windows or Office setting is changed:
tracked changes are recorded under whatever user name Word already has, and the author, date and id of every
revision are left out of the dump.

table.tsv (tab separated; text escapes backslash, tab, CR, LF and non-ASCII as \\, \t, \r, \n, \uXXXX):
  # base <fixture>        the starting document
  # op <text>             the Word call
  p      <text>                          body-level paragraph
  tbl    <n>  grid=<twips,...>  rows=<n> pr=<tblPr>   table n (document order, top level only)
  row    <i>  pr=<trPr>  ex=<tblPrEx>
  cell   <i>  <k>  pr=<tcPr>  ppr=<pPr of the first paragraph>  txt=<text of the cell's paragraphs joined by U+00B6>
Property strings are the element tree of the property element: name(attr=value,...){children}, attributes sorted
by local name; w:id, w:author, w:date and rsid attributes are left out.
The Rust side (Table::property_dump in crates/docx/src/table_edit.rs) writes the same format.
#>
param(
  [switch]$Make,
  [string]$Only
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixDir = Join-Path $root 'fixtures\docx'
$refDir = Join-Path $root 'fixtures\reference\docx'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

# ---------------------------------------------------------------------------
# Base fixtures (generated, no personal data). Letter page, 1" margins: text width 9360 twips.
# ---------------------------------------------------------------------------
$NS = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
$SECT = '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>'
function P($t) { if ($t -eq '') { '<w:p/>' } else { "<w:p><w:r><w:t xml:space=`"preserve`">$t</w:t></w:r></w:p>" } }
function Tc($w, $inner, $extra = '') { "<w:tc><w:tcPr><w:tcW w:w=`"$w`" w:type=`"dxa`"/>$extra</w:tcPr>$inner</w:tc>" }
function Tr($cells, $trpr = '') { $p = if ($trpr) { "<w:trPr>$trpr</w:trPr>" } else { '' }; "<w:tr>$p$cells</w:tr>" }
function Tbl($pr, $grid, $rows) {
  $g = ($grid | ForEach-Object { "<w:gridCol w:w=`"$_`"/>" }) -join ''
  "<w:tbl><w:tblPr>$pr</w:tblPr><w:tblGrid>$g</w:tblGrid>$rows</w:tbl>"
}
$GRIDPR = '<w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/><w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/>'
$W3 = @(3116, 3117, 3117)   # what Word itself writes for three columns in 9350 twips
function Grid3x3 {
  $rows = ''
  foreach ($r in 1..3) { $rows += Tr ((1..3 | ForEach-Object { Tc $W3[$_ - 1] (P "r${r}c$_") }) -join '') }
  Tbl $GRIDPR $W3 $rows
}
function SectXml($left = 1440, $right = 1440, $w = 12240) { "<w:sectPr><w:pgSz w:w=`"$w`" w:h=`"15840`"/><w:pgMar w:top=`"1440`" w:right=`"$right`" w:bottom=`"1440`" w:left=`"$left`" w:header=`"720`" w:footer=`"720`" w:gutter=`"0`"/></w:sectPr>" }
$script:sects = @{}
function Bases {
  $b = [ordered]@{}
  $b['empty'] = (P 'Before') + (P '') + (P 'After')
  # Same body on other page setups: A4 with 1" margins (9026 twips of text) and an odd 9406.
  $b['empty-a4'] = $b['empty']; $script:sects['empty-a4'] = SectXml 1440 1440 11906
  $b['empty-odd'] = $b['empty']; $script:sects['empty-odd'] = SectXml 1701 1133
  $b['grid3x3'] = (P 'Before') + (Grid3x3) + (P 'After')
  # Fixed layout, narrower than the text area.
  $rows = ''
  foreach ($r in 1..3) { $rows += Tr ((1..3 | ForEach-Object { Tc 2000 (P "r${r}c$_") }) -join '') }
  $b['fixed-narrow'] = (P 'Before') + (Tbl ('<w:tblStyle w:val="TableGrid"/><w:tblW w:w="6000" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/>') @(2000, 2000, 2000) $rows) + (P 'After')
  # Row 1: a cell spanning columns 1-2 and one cell. Rows 2-3: a vertical merge in column 1.
  $rows = (Tr ((Tc 6233 (P 'span12') '<w:gridSpan w:val="2"/>') + (Tc 3117 (P 'c3')))) +
          (Tr ((Tc 3116 (P 'vtop') '<w:vMerge w:val="restart"/>') + (Tc 3117 (P 'b2')) + (Tc 3117 (P 'c2')))) +
          (Tr ((Tc 3116 (P '') '<w:vMerge/>') + (Tc 3117 (P 'b3')) + (Tc 3117 (P 'c3b'))))
  $b['merged'] = (P 'Before') + (Tbl $GRIDPR $W3 $rows) + (P 'After')
  # Styled rows and cells: header row, cantSplit, row height, shading, borders, centred bold paragraph.
  $cp = '<w:p><w:pPr><w:jc w:val="center"/><w:rPr><w:b/></w:rPr></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>r2c1</w:t></w:r></w:p>'
  $rows = (Tr ((1..3 | ForEach-Object { Tc $W3[$_ - 1] (P "r1c$_") }) -join '') '<w:cantSplit/><w:tblHeader/>') +
          (Tr ((Tc 3116 $cp) + (Tc 3117 (P 'r2c2') '<w:tcBorders><w:top w:val="double" w:sz="6" w:space="0" w:color="FF0000"/></w:tcBorders><w:shd w:val="clear" w:color="auto" w:fill="FFFF00"/>') + (Tc 3117 (P 'r2c3'))) '<w:trHeight w:val="600"/>') +
          (Tr ((1..3 | ForEach-Object { Tc $W3[$_ - 1] (P "r3c$_") }) -join ''))
  $b['styled'] = (P 'Before') + (Tbl $GRIDPR $W3 $rows) + (P 'After')
  # Auto-fit to window: percentage widths.
  $rows = ''
  foreach ($r in 1..3) { $rows += Tr ((@(1666, 1667, 1667) | ForEach-Object -Begin { $i = 0 } -Process { "<w:tc><w:tcPr><w:tcW w:w=`"$_`" w:type=`"pct`"/></w:tcPr>$(P "r${r}c$((++$i))")</w:tc>" }) -join '') }
  $b['window'] = (P 'Before') + (Tbl '<w:tblStyle w:val="TableGrid"/><w:tblW w:w="5000" w:type="pct"/><w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/>' $W3 $rows) + (P 'After')
  # Auto-fit to contents: automatic widths.
  $rows = ''
  foreach ($r in 1..3) { $rows += Tr ((1..3 | ForEach-Object { "<w:tc><w:tcPr><w:tcW w:w=`"0`" w:type=`"auto`"/></w:tcPr>$(P "r${r}c$_")</w:tc>" }) -join '') }
  $b['content'] = (P 'Before') + (Tbl $GRIDPR @(900, 900, 900) $rows) + (P 'After')
  # Automatic table width (tblW auto), cell widths in dxa, narrower than the text area.
  $rows = ''
  foreach ($r in 1..3) { $rows += Tr ((1..3 | ForEach-Object { Tc 2000 (P "r${r}c$_") }) -join '') }
  $b['auto-narrow'] = (P 'Before') + (Tbl $GRIDPR @(2000, 2000, 2000) $rows) + (P 'After')
  $b['text-para'] = (P 'Before') + (P 'Hello world') + (P 'After')
  $b['lines-tab'] = (P 'Before') + (P "a`tb`tc") + (P "d`te`tf") + (P "g`th`ti") + (P 'After')
  $b['lines-comma'] = (P 'Before') + (P 'a,b,c') + (P 'd,e,f') + (P 'After')
  $b['lines-plain'] = (P 'Before') + (P 'one') + (P 'two') + (P 'three') + (P 'four') + (P 'After')
  $b
}
$STYLES = '<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="en-US" w:eastAsia="en-US" w:bidi="ar-SA"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style><w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style><w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:uiPriority w:val="39"/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/></w:tblBorders></w:tblPr></w:style></w:styles>'
$SETTINGS = '<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:defaultTabStop w:val="720"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>'

function Write-Zip($path, $entries) {
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
  $fs = [IO.File]::Create($path)
  try {
    $zip = New-Object IO.Compression.ZipArchive($fs, [IO.Compression.ZipArchiveMode]::Create)
    try {
      foreach ($k in $entries.Keys) {
        $e = $zip.CreateEntry($k, [IO.Compression.CompressionLevel]::Optimal)
        $s = $e.Open()
        try { $b = (New-Object Text.UTF8Encoding $false).GetBytes($entries[$k]); $s.Write($b, 0, $b.Length) } finally { $s.Dispose() }
      }
    } finally { $zip.Dispose() }
  } finally { $fs.Dispose() }
}

function Make-Fixtures {
  $decl = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' + "`n"
  foreach ($kv in (Bases).GetEnumerator()) {
    $name = "tbledit-base-$($kv.Key)"
    $ct = $decl + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/></Types>'
    $rels = $decl + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>'
    $drels = $decl + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>'
    $entries = [ordered]@{
      '[Content_Types].xml' = $ct
      '_rels/.rels' = $rels
      'word/_rels/document.xml.rels' = $drels
      'word/styles.xml' = $decl + $STYLES
      'word/settings.xml' = $decl + $SETTINGS
      'word/document.xml' = $decl + "<w:document $NS><w:body>" + $kv.Value + $(if ($script:sects.ContainsKey($kv.Key)) { $script:sects[$kv.Key] } else { $SECT }) + '</w:body></w:document>'
    }
    Write-Zip (Join-Path $fixDir "$name.docx") $entries
    Write-Output "made $name"
  }
}
if ($Make) { Make-Fixtures; exit 0 }

# ---------------------------------------------------------------------------
# Scenarios: name -> base, description, Word edit. $t = Table 1, $d = document, $w = Word.
# Word constants: wdAutoFitFixed 0, wdAutoFitContent 1, wdAutoFitWindow 2; wdBorderTop -1, Left -2, Bottom -3, Right -4.
# ---------------------------------------------------------------------------
function Sel-Cell($d, $w, $r, $c) { $d.Tables.Item(1).Cell($r, $c).Select() }
function Sel-Range($d, $w, $r1, $c1, $r2, $c2) {
  $t = $d.Tables.Item(1)
  $rng = $d.Range($t.Cell($r1, $c1).Range.Start, $t.Cell($r2, $c2).Range.End)
  $rng.Select()
}
# Range.ConvertToTable with the UI defaults (fixed column width "auto", Word 2000+ table behaviour); the other
# 12 parameters are left out by name because COM interop cannot pass "missing" from PowerShell.
function Convert-ToTable($r, [int]$sep, [int]$rows, [int]$cols) {
  $r.GetType().InvokeMember('ConvertToTable', [Reflection.BindingFlags]::InvokeMethod, $null, $r,
    [object[]]@($sep, $rows, $cols, 0, 1), $null, $null,
    [string[]]@('Separator', 'NumRows', 'NumColumns', 'AutoFitBehavior', 'DefaultTableBehavior')) | Out-Null
}
function Para-Range($d, $i) { $d.Paragraphs.Item($i).Range }
function Para-Start($d, $i) { $r = $d.Paragraphs.Item($i).Range; $r.Collapse(1); $r }
$S = [ordered]@{}
function Add-S($name, $base, $desc, [scriptblock]$edit, [bool]$track = $false) { $script:S[$name] = @{ base = $base; desc = $desc; edit = $edit; track = $track } }

# Insert table
Add-S 'insert-3x3' 'empty' 'Tables.Add(range, 3, 3, wdWord9TableBehavior, wdAutoFitFixed)' { $d.Tables.Add((Para-Start $d 2), 3, 3, 1, 0) | Out-Null }
Add-S 'insert-2x5' 'empty' 'Tables.Add(range, 2, 5, 1, 0)' { $d.Tables.Add((Para-Start $d 2), 2, 5, 1, 0) | Out-Null }
Add-S 'insert-1x1' 'empty' 'Tables.Add(range, 1, 1, 1, 0)' { $d.Tables.Add((Para-Start $d 2), 1, 1, 1, 0) | Out-Null }
Add-S 'insert-3x3-default' 'empty' 'Tables.Add(range, 3, 3)' { $d.Tables.Add((Para-Start $d 2), 3, 3) | Out-Null }
Add-S 'insert-3x3-word8' 'empty' 'Tables.Add(range, 3, 3, wdWord8TableBehavior, wdAutoFitFixed)' { $d.Tables.Add((Para-Start $d 2), 3, 3, 0, 0) | Out-Null }
Add-S 'insert-3x3-content' 'empty' 'Tables.Add(range, 3, 3, 1, wdAutoFitContent)' { $d.Tables.Add((Para-Start $d 2), 3, 3, 1, 1) | Out-Null }
Add-S 'insert-3x3-window' 'empty' 'Tables.Add(range, 3, 3, 1, wdAutoFitWindow)' { $d.Tables.Add((Para-Start $d 2), 3, 3, 1, 2) | Out-Null }
Add-S 'insert-3x3-a4' 'empty-a4' 'Tables.Add(range, 3, 3, 1, 0) on A4, 9026 twips of text' { $d.Tables.Add((Para-Start $d 2), 3, 3, 1, 0) | Out-Null }
Add-S 'insert-3x3-odd' 'empty-odd' 'Tables.Add(range, 3, 3, 1, 0), 9406 twips of text' { $d.Tables.Add((Para-Start $d 2), 3, 3, 1, 0) | Out-Null }
Add-S 'insert-7x2-odd' 'empty-odd' 'Tables.Add(range, 2, 7, 1, 0), 9406 twips of text' { $d.Tables.Add((Para-Start $d 2), 2, 7, 1, 0) | Out-Null }
Add-S 'insert-4x4-a4' 'empty-a4' 'Tables.Add(range, 4, 4, 1, 0) on A4' { $d.Tables.Add((Para-Start $d 2), 4, 4, 1, 0) | Out-Null }
Add-S 'insert-at-start' 'text-para' 'Tables.Add(collapsed range at offset 0 of "Hello world", 2, 2, 1, 0)' { $r = Para-Start $d 2; $d.Tables.Add($r, 2, 2, 1, 0) | Out-Null }
Add-S 'insert-at-middle' 'text-para' 'Tables.Add(collapsed range at offset 5 of "Hello world", 2, 2, 1, 0)' { $r = Para-Start $d 2; $r.Move(1, 5) | Out-Null; $d.Tables.Add($r, 2, 2, 1, 0) | Out-Null }
Add-S 'insert-at-end' 'text-para' 'Tables.Add(collapsed range at the end of "Hello world", 2, 2, 1, 0)' { $r = Para-Range $d 2; $r.MoveEnd(1, -1) | Out-Null; $r.Collapse(0); $d.Tables.Add($r, 2, 2, 1, 0) | Out-Null }
Add-S 'insert-3x3-tracked' 'empty' 'Tables.Add(range, 3, 3, 1, 0) with track changes' { $d.Tables.Add((Para-Start $d 2), 3, 3, 1, 0) | Out-Null } $true

# Rows
Add-S 'row-above-first' 'grid3x3' 'Selection.InsertRowsAbove(1) in row 1' { Sel-Cell $d $w 1 2; $w.Selection.InsertRowsAbove(1) }
Add-S 'row-above-mid' 'grid3x3' 'Selection.InsertRowsAbove(1) in row 2' { Sel-Cell $d $w 2 2; $w.Selection.InsertRowsAbove(1) }
Add-S 'row-below-mid' 'grid3x3' 'Selection.InsertRowsBelow(1) in row 2' { Sel-Cell $d $w 2 2; $w.Selection.InsertRowsBelow(1) }
Add-S 'row-below-last' 'grid3x3' 'Selection.InsertRowsBelow(1) in row 3' { Sel-Cell $d $w 3 1; $w.Selection.InsertRowsBelow(1) }
Add-S 'row-above-two' 'grid3x3' 'Selection.InsertRowsAbove(2) in row 2' { Sel-Cell $d $w 2 1; $w.Selection.InsertRowsAbove(2) }
Add-S 'row-below-merged' 'merged' 'InsertRowsBelow(1) in row 2 (vertical merge restart row)' { Sel-Cell $d $w 2 1; $w.Selection.InsertRowsBelow(1) }
Add-S 'row-above-merged' 'merged' 'InsertRowsAbove(1) in row 3 (vertical merge continuation row)' { Sel-Cell $d $w 3 2; $w.Selection.InsertRowsAbove(1) }
Add-S 'row-below-spanned' 'merged' 'InsertRowsBelow(1) in row 1 (gridSpan row)' { Sel-Cell $d $w 1 1; $w.Selection.InsertRowsBelow(1) }
Add-S 'row-below-mid-tracked' 'grid3x3' 'InsertRowsBelow(1) in row 2 with track changes' { Sel-Cell $d $w 2 2; $w.Selection.InsertRowsBelow(1) } $true
Add-S 'row-above-mid-tracked' 'grid3x3' 'InsertRowsAbove(1) in row 2 with track changes' { Sel-Cell $d $w 2 2; $w.Selection.InsertRowsAbove(1) } $true

Add-S 'row-below-merged-mid' 'merged' 'InsertRowsBelow(1) in cell (2,2): inside the vertical merge' { Sel-Cell $d $w 2 2; $w.Selection.InsertRowsBelow(1) }
Add-S 'row-above-restart' 'merged' 'InsertRowsAbove(1) in cell (2,2): the row where a vertical merge restarts' { Sel-Cell $d $w 2 2; $w.Selection.InsertRowsAbove(1) }
Add-S 'row-below-last-merged' 'merged' 'InsertRowsBelow(1) in cell (3,2): the last row of a vertical merge' { Sel-Cell $d $w 3 2; $w.Selection.InsertRowsBelow(1) }
Add-S 'row-below-styled' 'styled' 'InsertRowsBelow(1) in cell (2,2) (row height, shading, borders)' { Sel-Cell $d $w 2 2; $w.Selection.InsertRowsBelow(1) }
Add-S 'row-above-styled-col1' 'styled' 'InsertRowsAbove(1) in cell (2,1) (centred bold paragraph)' { Sel-Cell $d $w 2 1; $w.Selection.InsertRowsAbove(1) }
Add-S 'row-below-header' 'styled' 'InsertRowsBelow(1) in the header row' { Sel-Cell $d $w 1 1; $w.Selection.InsertRowsBelow(1) }
Add-S 'row-above-header' 'styled' 'InsertRowsAbove(1) in the header row' { Sel-Cell $d $w 1 1; $w.Selection.InsertRowsAbove(1) }

# Columns
Add-S 'col-left-first' 'grid3x3' 'Selection.InsertColumnsLeft in column 1 (full-width table)' { Sel-Cell $d $w 2 1; $w.Selection.InsertColumns() }
Add-S 'col-right-mid' 'grid3x3' 'Selection.InsertColumnsRight in column 2 (full-width table)' { Sel-Cell $d $w 2 2; $w.Selection.InsertColumnsRight() }
Add-S 'col-right-last' 'grid3x3' 'Selection.InsertColumnsRight in column 3 (full-width table)' { Sel-Cell $d $w 2 3; $w.Selection.InsertColumnsRight() }
Add-S 'col-left-mid' 'grid3x3' 'Selection.InsertColumns (left) in column 2' { Sel-Cell $d $w 2 2; $w.Selection.InsertColumns() }
Add-S 'col-right-narrow' 'fixed-narrow' 'InsertColumnsRight in column 2 (fixed, narrower than the text area)' { Sel-Cell $d $w 2 2; $w.Selection.InsertColumnsRight() }
Add-S 'col-left-narrow' 'fixed-narrow' 'InsertColumns (left) in column 1 (fixed, narrower than the text area)' { Sel-Cell $d $w 2 1; $w.Selection.InsertColumns() }
Add-S 'col-right-window' 'window' 'InsertColumnsRight in column 2 (auto-fit to window)' { Sel-Cell $d $w 2 2; $w.Selection.InsertColumnsRight() }
Add-S 'col-right-content' 'content' 'InsertColumnsRight in column 2 (auto-fit to contents)' { Sel-Cell $d $w 2 2; $w.Selection.InsertColumnsRight() }
Add-S 'col-right-styled' 'styled' 'InsertColumnsRight in cell (2,2) (shading, borders)' { Sel-Cell $d $w 2 2; $w.Selection.InsertColumnsRight() }
Add-S 'col-left-styled' 'styled' 'InsertColumns (left) in cell (2,1) (centred bold paragraph)' { Sel-Cell $d $w 2 1; $w.Selection.InsertColumns() }
Add-S 'col-right-auto-narrow' 'auto-narrow' 'InsertColumnsRight in column 2 (tblW auto, 6000 of 9360 twips)' { Sel-Cell $d $w 2 2; $w.Selection.InsertColumnsRight() }
Add-S 'col-right-mid-tracked' 'grid3x3' 'InsertColumnsRight in column 2 with track changes' { Sel-Cell $d $w 2 2; $w.Selection.InsertColumnsRight() } $true
Add-S 'col-right-merged' 'merged' 'InsertColumnsRight in column 1 (span and vertical merge)' { Sel-Cell $d $w 2 1; $w.Selection.InsertColumnsRight() }
Add-S 'col-left-merged' 'merged' 'InsertColumns (left) in column 2 (row 2)' { Sel-Cell $d $w 2 2; $w.Selection.InsertColumns() }

# Delete
Add-S 'delete-row-mid' 'grid3x3' 'Rows(2).Delete()' { $d.Tables.Item(1).Rows.Item(2).Delete() }
Add-S 'delete-row-first' 'grid3x3' 'Rows(1).Delete()' { $d.Tables.Item(1).Rows.Item(1).Delete() }
Add-S 'delete-row-last' 'grid3x3' 'Rows(3).Delete()' { $d.Tables.Item(1).Rows.Item(3).Delete() }
Add-S 'delete-row-merged' 'merged' 'Selection.Rows.Delete() in cell (2,2) (row of a vertical merge restart)' { Sel-Cell $d $w 2 2; $w.Selection.Rows.Delete() }
Add-S 'delete-row-merged-cont' 'merged' 'Selection.Rows.Delete() in cell (3,2) (vertical merge continuation row)' { Sel-Cell $d $w 3 2; $w.Selection.Rows.Delete() }
Add-S 'delete-row-spanned' 'merged' 'Selection.Rows.Delete() in cell (1,1) (gridSpan row)' { Sel-Cell $d $w 1 1; $w.Selection.Rows.Delete() }
Add-S 'delete-row-styled' 'styled' 'Rows(1).Delete() (header row)' { $d.Tables.Item(1).Rows.Item(1).Delete() }
Add-S 'delete-col-mid' 'grid3x3' 'Columns(2).Delete()' { $d.Tables.Item(1).Columns.Item(2).Delete() }
Add-S 'delete-col-first' 'grid3x3' 'Columns(1).Delete()' { $d.Tables.Item(1).Columns.Item(1).Delete() }
Add-S 'delete-col-last' 'grid3x3' 'Columns(3).Delete()' { $d.Tables.Item(1).Columns.Item(3).Delete() }
Add-S 'delete-col-narrow' 'fixed-narrow' 'Columns(2).Delete() (fixed, narrower than the text area)' { $d.Tables.Item(1).Columns.Item(2).Delete() }
Add-S 'delete-col-window' 'window' 'Columns(2).Delete() (auto-fit to window)' { $d.Tables.Item(1).Columns.Item(2).Delete() }
Add-S 'delete-col-auto-narrow' 'auto-narrow' 'Columns(2).Delete() (tblW auto)' { $d.Tables.Item(1).Columns.Item(2).Delete() }
Add-S 'delete-col-mid-tracked' 'grid3x3' 'Columns(2).Delete() with track changes' { $d.Tables.Item(1).Columns.Item(2).Delete() } $true
Add-S 'delete-col-merged' 'merged' 'Selection.Columns.Delete in column 2 (row 2)' { Sel-Cell $d $w 2 2; $w.Selection.Columns.Delete() }
Add-S 'delete-table' 'grid3x3' 'Tables(1).Delete()' { $d.Tables.Item(1).Delete() }
Add-S 'delete-row-mid-tracked' 'grid3x3' 'Rows(2).Delete() with track changes' { $d.Tables.Item(1).Rows.Item(2).Delete() } $true
Add-S 'delete-row-first-tracked' 'grid3x3' 'Rows(1).Delete() with track changes' { $d.Tables.Item(1).Rows.Item(1).Delete() } $true
Add-S 'delete-table-tracked' 'grid3x3' 'Tables(1).Delete() with track changes' { $d.Tables.Item(1).Delete() } $true

# Merge and split
Add-S 'merge-h' 'grid3x3' 'Cell(1,1).Merge(Cell(1,2))' { $t = $d.Tables.Item(1); $t.Cell(1, 1).Merge($t.Cell(1, 2)) }
Add-S 'merge-h-row' 'grid3x3' 'Cell(2,1).Merge(Cell(2,3))' { $t = $d.Tables.Item(1); $t.Cell(2, 1).Merge($t.Cell(2, 3)) }
Add-S 'merge-v' 'grid3x3' 'Cell(1,1).Merge(Cell(2,1))' { $t = $d.Tables.Item(1); $t.Cell(1, 1).Merge($t.Cell(2, 1)) }
Add-S 'merge-v-col' 'grid3x3' 'Cell(1,2).Merge(Cell(3,2))' { $t = $d.Tables.Item(1); $t.Cell(1, 2).Merge($t.Cell(3, 2)) }
Add-S 'merge-block' 'grid3x3' 'Cell(1,1).Merge(Cell(2,2))' { $t = $d.Tables.Item(1); $t.Cell(1, 1).Merge($t.Cell(2, 2)) }
Add-S 'merge-all' 'grid3x3' 'Cell(1,1).Merge(Cell(3,3))' { $t = $d.Tables.Item(1); $t.Cell(1, 1).Merge($t.Cell(3, 3)) }
Add-S 'merge-h-beside-vmerge' 'merged' 'Cell(2,2).Merge(Cell(2,3)) (next to a vertical merge)' { Sel-Range $d $w 2 2 2 3; $w.Selection.Cells.Merge() }
Add-S 'merge-v-col3' 'merged' 'Cell(1,2).Merge(Cell(2,3)) (c3 with c2)' { $t = $d.Tables.Item(1); $t.Cell(1, 2).Merge($t.Cell(2, 3)) }
Add-S 'merge-styled' 'styled' 'Cell(2,1).Merge(Cell(2,2)) (shading, borders, paragraph formatting)' { $t = $d.Tables.Item(1); $t.Cell(2, 1).Merge($t.Cell(2, 2)) }
Add-S 'split-1x2' 'grid3x3' 'Cell(2,2).Split(1, 2)' { $d.Tables.Item(1).Cell(2, 2).Split(1, 2) }
Add-S 'split-2x1' 'grid3x3' 'Cell(2,2).Split(2, 1)' { $d.Tables.Item(1).Cell(2, 2).Split(2, 1) }
Add-S 'split-2x2' 'grid3x3' 'Cell(2,2).Split(2, 2)' { $d.Tables.Item(1).Cell(2, 2).Split(2, 2) }
Add-S 'split-1x3-first' 'grid3x3' 'Cell(1,1).Split(1, 3)' { $d.Tables.Item(1).Cell(1, 1).Split(1, 3) }
Add-S 'split-span' 'merged' 'Cell(1,1).Split(1, 2) (spanning cell back to two)' { $d.Tables.Item(1).Cell(1, 1).Split(1, 2) }
Add-S 'split-span-3' 'merged' 'Cell(1,1).Split(1, 3) (spanning cell into three)' { $d.Tables.Item(1).Cell(1, 1).Split(1, 3) }
Add-S 'split-vmerge' 'merged' 'Cell(2,1).Split(2, 1) (vertical merge back to two cells)' { $d.Tables.Item(1).Cell(2, 1).Split(2, 1) }
Add-S 'split-styled' 'styled' 'Cell(2,2).Split(1, 2) (shading and borders)' { $d.Tables.Item(1).Cell(2, 2).Split(1, 2) }

# Properties
Add-S 'col-width-none' 'grid3x3' 'Columns(2).SetWidth(100pt, wdAdjustNone)' { $d.Tables.Item(1).Columns.Item(2).SetWidth(100, 0) }
Add-S 'col-width-prop' 'grid3x3' 'Columns(2).SetWidth(100pt, wdAdjustProportional)' { $d.Tables.Item(1).Columns.Item(2).SetWidth(100, 1) }
Add-S 'col-width-same' 'grid3x3' 'Columns(2).SetWidth(100pt, wdAdjustSameWidth)' { $d.Tables.Item(1).Columns.Item(2).SetWidth(100, 3) }
Add-S 'col-width-first' 'grid3x3' 'Columns(1).SetWidth(100pt, wdAdjustFirstColumn)' { $d.Tables.Item(1).Columns.Item(1).SetWidth(100, 2) }
Add-S 'col-width-narrow' 'fixed-narrow' 'Columns(2).SetWidth(150pt, wdAdjustNone) (grows, still inside the text area)' { $d.Tables.Item(1).Columns.Item(2).SetWidth(150, 0) }
Add-S 'col-width-auto-narrow-grow' 'auto-narrow' 'Columns(2).SetWidth(150pt, wdAdjustNone) (tblW auto, 7000 of 9360 twips after)' { $d.Tables.Item(1).Columns.Item(2).SetWidth(150, 0) }
Add-S 'col-width-auto-narrow-over' 'auto-narrow' 'Columns(2).SetWidth(400pt, wdAdjustNone) (tblW auto, wider than the text area)' { $d.Tables.Item(1).Columns.Item(2).SetWidth(400, 0) }
Add-S 'col-width-narrow-shrink' 'fixed-narrow' 'Columns(2).SetWidth(50pt, wdAdjustNone)' { $d.Tables.Item(1).Columns.Item(2).SetWidth(50, 0) }
Add-S 'col-width-narrow-prop' 'fixed-narrow' 'Columns(1).SetWidth(150pt, wdAdjustProportional)' { $d.Tables.Item(1).Columns.Item(1).SetWidth(150, 1) }
Add-S 'col-width-window' 'window' 'Columns(2).SetWidth(100pt, wdAdjustNone) (auto-fit to window)' { $d.Tables.Item(1).Columns.Item(2).SetWidth(100, 0) }
Add-S 'col-width-last' 'grid3x3' 'Columns(3).SetWidth(200pt, wdAdjustNone)' { $d.Tables.Item(1).Columns.Item(3).SetWidth(200, 0) }
Add-S 'row-height-atleast' 'grid3x3' 'Rows(2).Height = 40, HeightRule = wdRowHeightAtLeast' { $r = $d.Tables.Item(1).Rows.Item(2); $r.HeightRule = 1; $r.Height = 40 }
Add-S 'row-height-exact' 'grid3x3' 'Rows(2).Height = 40, HeightRule = wdRowHeightExactly' { $r = $d.Tables.Item(1).Rows.Item(2); $r.HeightRule = 2; $r.Height = 40 }
Add-S 'row-height-auto' 'grid3x3' 'Rows(2).HeightRule = wdRowHeightAuto' { $r = $d.Tables.Item(1).Rows.Item(2); $r.HeightRule = 1; $r.Height = 40; $r.HeightRule = 0 }
Add-S 'row-header' 'grid3x3' 'Rows(1).HeadingFormat = True' { $d.Tables.Item(1).Rows.Item(1).HeadingFormat = -1 }
Add-S 'row-cantsplit' 'grid3x3' 'Rows(2).AllowBreakAcrossPages = False' { $d.Tables.Item(1).Rows.Item(2).AllowBreakAcrossPages = 0 }
Add-S 'cell-shading' 'grid3x3' 'Cell(1,1).Shading.BackgroundPatternColor = yellow' { $d.Tables.Item(1).Cell(1, 1).Shading.BackgroundPatternColor = 65535 }
Add-S 'cell-shading-pattern' 'grid3x3' 'Cell(1,2).Shading.Texture = wdTexture25Percent, foreground red, background yellow' { $s = $d.Tables.Item(1).Cell(1, 2).Shading; $s.Texture = 250; $s.ForegroundPatternColor = 255; $s.BackgroundPatternColor = 65535 }
Add-S 'row-shading' 'grid3x3' 'Rows(2).Shading.BackgroundPatternColor = green' { $d.Tables.Item(1).Rows.Item(2).Shading.BackgroundPatternColor = 65280 }
Add-S 'cell-border-top' 'grid3x3' 'Cell(2,2).Borders(wdBorderTop): single 1.5pt red' { $b = $d.Tables.Item(1).Cell(2, 2).Borders.Item(-1); $b.LineStyle = 1; $b.LineWidth = 12; $b.Color = 255 }
Add-S 'cell-border-all' 'grid3x3' 'Cell(2,2).Borders.Enable with double lines' { $b = $d.Tables.Item(1).Cell(2, 2).Borders; $b.OutsideLineStyle = 7; $b.OutsideLineWidth = 6 }
Add-S 'cell-border-none' 'grid3x3' 'Cell(2,2).Borders(wdBorderLeft).LineStyle = none' { $d.Tables.Item(1).Cell(2, 2).Borders.Item(-2).LineStyle = 0 }
Add-S 'table-borders-none' 'grid3x3' 'Tables(1).Borders.Enable = False' { $d.Tables.Item(1).Borders.Enable = 0 }
Add-S 'table-borders-outside' 'grid3x3' 'Tables(1).Borders.OutsideLineStyle = double, InsideLineStyle = single' { $b = $d.Tables.Item(1).Borders; $b.OutsideLineStyle = 7; $b.InsideLineStyle = 1 }
Add-S 'table-align-center' 'grid3x3' 'Tables(1).Rows.Alignment = wdAlignRowCenter' { $d.Tables.Item(1).Rows.Alignment = 1 }
Add-S 'table-align-right' 'fixed-narrow' 'Tables(1).Rows.Alignment = wdAlignRowRight' { $d.Tables.Item(1).Rows.Alignment = 2 }
Add-S 'table-align-left-narrow' 'fixed-narrow' 'Tables(1).Rows.Alignment = wdAlignRowLeft' { $d.Tables.Item(1).Rows.Alignment = 0 }
Add-S 'table-indent' 'fixed-narrow' 'Tables(1).Rows.LeftIndent = 36' { $d.Tables.Item(1).Rows.LeftIndent = 36 }
Add-S 'table-pref-width' 'grid3x3' 'Tables(1).PreferredWidthType = points, PreferredWidth = 300' { $t = $d.Tables.Item(1); $t.PreferredWidthType = 3; $t.PreferredWidth = 300 }
Add-S 'table-pref-percent' 'grid3x3' 'Tables(1).PreferredWidthType = percent, PreferredWidth = 80' { $t = $d.Tables.Item(1); $t.PreferredWidthType = 2; $t.PreferredWidth = 80 }

# Autofit
Add-S 'autofit-fixed' 'grid3x3' 'Tables(1).AutoFitBehavior(wdAutoFitFixed)' { $d.Tables.Item(1).AutoFitBehavior(0) }
Add-S 'autofit-content' 'grid3x3' 'Tables(1).AutoFitBehavior(wdAutoFitContent)' { $d.Tables.Item(1).AutoFitBehavior(1) }
Add-S 'autofit-window' 'grid3x3' 'Tables(1).AutoFitBehavior(wdAutoFitWindow)' { $d.Tables.Item(1).AutoFitBehavior(2) }
Add-S 'autofit-fixed-narrow' 'fixed-narrow' 'Tables(1).AutoFitBehavior(wdAutoFitFixed)' { $d.Tables.Item(1).AutoFitBehavior(0) }
Add-S 'autofit-content-narrow' 'fixed-narrow' 'Tables(1).AutoFitBehavior(wdAutoFitContent)' { $d.Tables.Item(1).AutoFitBehavior(1) }
Add-S 'autofit-window-narrow' 'fixed-narrow' 'Tables(1).AutoFitBehavior(wdAutoFitWindow)' { $d.Tables.Item(1).AutoFitBehavior(2) }
Add-S 'allow-autofit-off' 'grid3x3' 'Tables(1).AllowAutoFit = False' { $d.Tables.Item(1).AllowAutoFit = 0 }

# Convert
Add-S 'table-to-text-multipara' 'grid3x3' 'Cell(1,1).Merge(Cell(1,2)) then Tables(1).ConvertToText(wdSeparateByTabs)' { $t = $d.Tables.Item(1); $t.Cell(1, 1).Merge($t.Cell(1, 2)); $d.Tables.Item(1).ConvertToText(1) | Out-Null }
Add-S 'text-to-table-tab' 'lines-tab' 'Range(paragraphs 2-4).ConvertToTable(wdSeparateByTabs, 3, 3, ..., AutoFitBehavior wdAutoFitFixed, DefaultTableBehavior wdWord9TableBehavior)' { $r = $d.Range((Para-Range $d 2).Start, (Para-Range $d 4).End); Convert-ToTable $r 1 3 3 }
Add-S 'text-to-table-comma' 'lines-comma' 'Range(paragraphs 2-3).ConvertToTable(wdSeparateByCommas, 2, 3, ..., fixed, Word9)' { $r = $d.Range((Para-Range $d 2).Start, (Para-Range $d 3).End); Convert-ToTable $r 2 2 3 }
Add-S 'text-to-table-para' 'lines-plain' 'Range(paragraphs 2-5).ConvertToTable(wdSeparateByParagraphs, 2, 2, ..., fixed, Word9)' { $r = $d.Range((Para-Range $d 2).Start, (Para-Range $d 5).End); Convert-ToTable $r 0 2 2 }
Add-S 'text-to-table-para-1col' 'lines-plain' 'Range(paragraphs 2-5).ConvertToTable(wdSeparateByParagraphs, 4, 1, ..., fixed, Word9)' { $r = $d.Range((Para-Range $d 2).Start, (Para-Range $d 5).End); Convert-ToTable $r 0 4 1 }
Add-S 'table-to-text-tab' 'grid3x3' 'Tables(1).ConvertToText(wdSeparateByTabs)' { $d.Tables.Item(1).ConvertToText(1) | Out-Null }
Add-S 'table-to-text-para' 'grid3x3' 'Tables(1).ConvertToText(wdSeparateByParagraphs)' { $d.Tables.Item(1).ConvertToText(0) | Out-Null }
Add-S 'table-to-text-comma' 'grid3x3' 'Tables(1).ConvertToText(wdSeparateByCommas)' { $d.Tables.Item(1).ConvertToText(2) | Out-Null }
Add-S 'table-to-text-merged' 'merged' 'Tables(1).ConvertToText(wdSeparateByTabs)' { $d.Tables.Item(1).ConvertToText(1) | Out-Null }

# ---------------------------------------------------------------------------
# Dump of a saved word/document.xml
# ---------------------------------------------------------------------------
$WNS = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
function Esc([string]$s) {
  $sb = New-Object Text.StringBuilder
  foreach ($ch in $s.ToCharArray()) {
    $c = [int]$ch
    if ($ch -eq '\') { [void]$sb.Append('\\') } elseif ($c -eq 9) { [void]$sb.Append('\t') }
    elseif ($c -eq 13) { [void]$sb.Append('\r') } elseif ($c -eq 10) { [void]$sb.Append('\n') }
    elseif ($c -lt 32 -or $c -gt 126) { [void]$sb.Append('\u' + $c.ToString('x4')) } else { [void]$sb.Append($ch) }
  }
  $sb.ToString()
}
function Node-Str($e) {
  $attrs = New-Object 'System.Collections.Generic.SortedDictionary[string,string]' ([StringComparer]::Ordinal)
  foreach ($a in $e.Attributes) {
    if ($a.NamespaceURI -ne $WNS) { continue }
    $n = $a.LocalName
    if ($n -eq 'id' -or $n -eq 'author' -or $n -eq 'date' -or $n.StartsWith('rsid')) { continue }
    $attrs[$n] = $a.Value
  }
  $s = $e.LocalName
  if ($attrs.Count -gt 0) {
    $s += '(' + (($attrs.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ',') + ')'
  }
  $kids = @($e.ChildNodes | Where-Object { $_.NodeType -eq 'Element' })
  if ($kids.Count -gt 0) { $s += '{' + (($kids | ForEach-Object { Node-Str $_ }) -join ' ') + '}' }
  $s
}
function Para-Text($p) {
  $sb = New-Object Text.StringBuilder
  foreach ($n in $p.SelectNodes('.//w:r/w:t | .//w:r/w:tab | .//w:r/w:br', $script:nsm)) {
    if ($n.LocalName -eq 't') { [void]$sb.Append($n.InnerText) } elseif ($n.LocalName -eq 'tab') { [void]$sb.Append("`t") } else { [void]$sb.Append("`n") }
  }
  $sb.ToString()
}
function Prop-Str($parent, $local) {
  $e = $parent.SelectSingleNode("w:$local", $script:nsm)
  if ($e) { Node-Str $e } else { '-' }
}
function Dump-Doc([string]$xmlText) {
  $x = New-Object Xml.XmlDocument
  $x.PreserveWhitespace = $false
  $x.LoadXml($xmlText)
  $script:nsm = New-Object Xml.XmlNamespaceManager($x.NameTable)
  $script:nsm.AddNamespace('w', $WNS)
  $body = $x.SelectSingleNode('//w:body', $script:nsm)
  $out = New-Object System.Collections.Generic.List[string]
  $ti = 0
  foreach ($n in $body.ChildNodes) {
    if ($n.NodeType -ne 'Element') { continue }
    if ($n.LocalName -eq 'p') { $out.Add("p`t$(Esc (Para-Text $n))") }
    elseif ($n.LocalName -eq 'tbl') {
      $grid = @($n.SelectNodes('w:tblGrid/w:gridCol', $script:nsm) | ForEach-Object { $_.GetAttribute('w', $WNS) }) -join ','
      $rows = @($n.SelectNodes('w:tr', $script:nsm))
      $out.Add("tbl`t$ti`tgrid=$grid`trows=$($rows.Count)`tpr=$(Prop-Str $n 'tblPr')")
      $ri = 0
      foreach ($tr in $rows) {
        $out.Add("row`t$ri`tpr=$(Prop-Str $tr 'trPr')`tex=$(Prop-Str $tr 'tblPrEx')")
        $ci = 0
        foreach ($tc in @($tr.SelectNodes('w:tc', $script:nsm))) {
          $txt = (@($tc.SelectNodes('w:p', $script:nsm)) | ForEach-Object { Para-Text $_ }) -join [string][char]0xB6
          $p1 = $tc.SelectSingleNode('w:p/w:pPr', $script:nsm)
          $ppr = if ($p1) { Node-Str $p1 } else { '-' }
          $out.Add("cell`t$ri`t$ci`tpr=$(Prop-Str $tc 'tcPr')`tppr=$ppr`ttxt=$(Esc $txt)")
          $ci++
        }
        $ri++
      }
      $ti++
    }
  }
  $out
}

# ---------------------------------------------------------------------------
# Measurement
# ---------------------------------------------------------------------------
function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }
$wordBefore = Get-Pids 'WINWORD'
$word = $null; $wordPids = @()
function Get-Word {
  if ($null -eq $script:word) {
    $script:word = New-Object -ComObject Word.Application
    $script:wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $script:wordBefore })
    if ($script:wordPids.Count -eq 0) { throw 'COM attached to a Word instance this script did not start; refusing to touch it' }
    $script:word.Visible = $false; $script:word.DisplayAlerts = 0
    $script:word.AutomationSecurity = 3  # msoAutomationSecurityForceDisable: macros never run
  }
  $script:word
}
function Open-Doc($path) {
  $w = Get-Word
  # FileName, ConfirmConversions, ReadOnly, AddToRecentFiles
  $w.Documents.Open($path, $false, $false, $false)
}
function Read-DocumentXml([string]$docx) {
  $zip = [IO.Compression.ZipFile]::OpenRead($docx)
  try {
    $e = $zip.GetEntry('word/document.xml')
    $sr = New-Object IO.StreamReader($e.Open(), [Text.Encoding]::UTF8)
    try { $sr.ReadToEnd() } finally { $sr.Dispose() }
  } finally { $zip.Dispose() }
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("open365-tbledit-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
$made = @()
$names = @($S.Keys)
if ($Only) { $names = @($names | Where-Object { $_ -eq $Only }) }
if (-not $names) { Write-Output 'no such scenario'; exit 1 }
$w = $null
try {
  $w = Get-Word
  foreach ($name in $names) {
    $sc = $S[$name]
    $src = Join-Path $fixDir "tbledit-base-$($sc.base).docx"
    if (-not (Test-Path -LiteralPath $src)) { throw "missing fixture $src; run with -Make first" }
    $copy = Join-Path $tmp "$name.docx"
    Copy-Item -LiteralPath $src -Destination $copy; $made += $copy
    $d = Open-Doc $copy
    try {
      $d.TrackRevisions = [bool]$sc.track
      & $sc.edit
      $d.TrackRevisions = $false
      $d.Save()
    } catch {
      Write-Output "FAIL $name : $($_.Exception.Message)"
      try { $d.Close(0) } catch {}
      Release $d
      continue
    }
    try { $d.Close(0) } catch {}
    Release $d
    $lines = @("# base tbledit-base-$($sc.base)", "# op $($sc.desc)") + @(Dump-Doc (Read-DocumentXml $copy))
    $outDir = Join-Path $refDir "tbledit-$name"
    New-Item -ItemType Directory -Force $outDir | Out-Null
    [IO.File]::WriteAllText((Join-Path $outDir 'table.tsv'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
    Write-Output "ok   $name"
  }
} finally {
  if ($word) { try { $word.Quit(0) } catch {}; Release $word }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($id in $wordPids) { Get-Process -Id $id -ErrorAction SilentlyContinue | Stop-Process -Force }
  foreach ($f in $made) { if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f -Force } }
  if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp -Force }
}
