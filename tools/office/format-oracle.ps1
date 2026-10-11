<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365 cell formatting: what Excel writes
into styles.xml when a formatting operation is applied to a range, and what its COM object model reports.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed output (fixtures/reference/xlsx/format-*/expected.json) through
crates/xlsx/tests/format_oracle.rs.

Modes
  -MakeBlank       (re)create fixtures/xlsx/format-blank.xlsx (an empty one-sheet workbook saved by Excel).
  -BuiltinStyles   apply every built-in cell style to its own cell and write
                   fixtures/reference/xlsx/format-builtin-styles/styles.json (the definitions the engine embeds).
  (default)        run every scenario of fixtures/xlsx/format-scenarios.json (or those matching -Only) on a copy of
                   the blank workbook, save it, and write fixtures/reference/xlsx/format-<name>/expected.json:
                     counts   fonts/fills/borders/cellXfs/cellStyleXfs/cellStyles/numFmts of the saved styles.xml
                     tables   the raw font, fill, border and xf elements Excel wrote
                     cells    per probe cell: the canonical effective format read back from the saved file (font,
                              fill, border, numFmt, xf attributes, alignment, protection, style name) and the COM
                              properties (Font, Interior, Borders, NumberFormat, Style.Name, ...)
                   Only generated data is used.

Scenario operations (all ranges are A1 / A1:B2 / C:C / 2:2 text; colours are {rgb:"RRGGBB"} or {theme:n,tint:t}
with the OOXML theme index):
  font, fill, border, align, numfmt, protect, style, decimals, percent, comma, accounting, clear_formats,
  painter, value, row_height, col_width.

Process safety: starts its own hidden Excel instance and aborts if COM hands back an instance that was already
running. AutomationSecurity is forced to 3 (disable macros). Excel is Quit in a finally block; the process id is taken
from our own window handle and only that process is ever stopped. Other EXCEL processes are never touched.

  pwsh -NoProfile -File tools/office/format-oracle.ps1 -MakeBlank
  pwsh -NoProfile -File tools/office/format-oracle.ps1 -BuiltinStyles
  pwsh -NoProfile -File tools/office/format-oracle.ps1 [-Only '^bold']
#>
param(
  [string]$Only = '',
  [switch]$MakeBlank,
  [switch]$BuiltinStyles
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$blank = Join-Path $fixtureDir 'format-blank.xlsx'
$scenarioFile = Join-Path $fixtureDir 'format-scenarios.json'
$work = Join-Path ([IO.Path]::GetTempPath()) ('open365-format-oracle-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
$inv = [Globalization.CultureInfo]::InvariantCulture

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@

$script:xl = $null
$script:xlPid = 0

function Start-Excel {
  $existing = @(Get-Process -Name EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $app = New-Object -ComObject Excel.Application
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  if ($existing -contains [int]$procId) {
    # COM attached to an Excel somebody else owns. Do not touch it.
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app)
    throw "COM returned the already running Excel instance $procId; aborting without touching it."
  }
  $script:xlPid = [int]$procId
  $script:xl = $app
  $app.Visible = $false
  $app.DisplayAlerts = $false
  $app.ScreenUpdating = $false
  $app.EnableEvents = $false
  $app.AutomationSecurity = 3
}

function Stop-Excel {
  $app = $script:xl
  $script:xl = $null
  if ($app) {
    try { foreach ($wb in @($app.Workbooks)) { try { $wb.Saved = $true; $wb.Close($false) } catch {} } } catch {}
    try { $app.Quit() } catch {}
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
  }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  if ($script:xlPid -gt 0) {
    $deadline = (Get-Date).AddSeconds(8)
    while ((Get-Date) -lt $deadline -and (Get-Process -Id $script:xlPid -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
    $p = Get-Process -Id $script:xlPid -ErrorAction SilentlyContinue
    if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $script:xlPid -Force -ErrorAction SilentlyContinue }
  }
  $script:xlPid = 0
}

function Scrub-Core([string]$path) {
  $zip = [IO.Compression.ZipFile]::Open($path, 'Update')
  try {
    $entry = $zip.GetEntry('docProps/core.xml')
    if ($entry) {
      $reader = New-Object IO.StreamReader($entry.Open(), (New-Object Text.UTF8Encoding $false))
      $xml = $reader.ReadToEnd(); $reader.Close()
      $xml = [regex]::Replace($xml, '<dc:creator>.*?</dc:creator>', '<dc:creator></dc:creator>')
      $xml = [regex]::Replace($xml, '<cp:lastModifiedBy>.*?</cp:lastModifiedBy>', '<cp:lastModifiedBy></cp:lastModifiedBy>')
      $entry.Delete()
      $new = $zip.CreateEntry('docProps/core.xml')
      $w = New-Object IO.StreamWriter($new.Open(), (New-Object Text.UTF8Encoding $false))
      $w.Write($xml); $w.Close()
    }
  }
  finally { $zip.Dispose() }
}

function Read-ZipText([string]$path, [string]$name) {
  $zip = [IO.Compression.ZipFile]::OpenRead($path)
  try {
    $e = $zip.GetEntry($name)
    if (-not $e) { return $null }
    $r = New-Object IO.StreamReader($e.Open(), [Text.Encoding]::UTF8)
    $t = $r.ReadToEnd(); $r.Close(); return $t
  }
  finally { $zip.Dispose() }
}

# ---------------------------------------------------------------------------------------------------------------
# COM mappings
# ---------------------------------------------------------------------------------------------------------------
# OOXML theme index -> the XlThemeColor value that makes Excel write that index (measured: 1 -> 1, 2 -> 0, 3 -> 2, 4 -> 3)
$ThemeCom = @{ 0 = 2; 1 = 1; 2 = 3; 3 = 4; 4 = 5; 5 = 6; 6 = 7; 7 = 8; 8 = 9; 9 = 10; 10 = 11; 11 = 12 }
$Underline = @{ none = -4142; single = 2; double = -4119; singleAccounting = 4; doubleAccounting = 5 }
$Pattern = @{
  none = -4142; solid = 1; darkGray = -4126; mediumGray = -4125; lightGray = -4124; gray125 = 17; gray0625 = 18
  darkHorizontal = -4128; darkVertical = -4166; darkDown = -4121; darkUp = -4162; darkGrid = 9; darkTrellis = 10
  lightHorizontal = 11; lightVertical = 12; lightDown = 13; lightUp = 14; lightGrid = 15; lightTrellis = 16
}
$Side = @{ left = 7; top = 8; bottom = 9; right = 10; insideVertical = 11; insideHorizontal = 12; diagonalDown = 5; diagonalUp = 6 }
# OOXML border style -> (LineStyle, Weight)
$LineStyleMap = @{
  thin = @(1, 2); medium = @(1, -4138); thick = @(1, 4); hair = @(1, 1); dashed = @(-4115, 2); mediumDashed = @(-4115, -4138)
  dotted = @(-4118, 2); dashDot = @(4, 2); mediumDashDot = @(4, -4138); dashDotDot = @(5, 2); mediumDashDotDot = @(5, -4138)
  double = @(-4119, 4); slantDashDot = @(13, -4138); none = @(-4142, 2)
}
$HAlign = @{ general = 1; left = -4131; center = -4108; right = -4152; fill = 5; justify = -4130; centerContinuous = 7; distributed = -4117 }
$VAlign = @{ top = -4160; center = -4108; bottom = -4107; justify = -4130; distributed = -4117 }

function Rgb-Int([string]$hex) {
  $r = [Convert]::ToInt32($hex.Substring(0, 2), 16); $g = [Convert]::ToInt32($hex.Substring(2, 2), 16); $b = [Convert]::ToInt32($hex.Substring(4, 2), 16)
  return $r + 256 * $g + 65536 * $b
}

# Sets a colour on an Excel object whose properties are named like Font.Color / ThemeColor / TintAndShade.
function Set-Col($obj, $spec, [string]$colorProp, [string]$themeProp, [string]$tintProp) {
  if ($spec.PSObject.Properties['rgb']) { $obj.$colorProp = Rgb-Int $spec.rgb; return }
  if ($spec.PSObject.Properties['theme']) {
    $obj.$themeProp = $ThemeCom[[int]$spec.theme]
    if ($spec.PSObject.Properties['tint'] -and [double]$spec.tint -ne 0) { $obj.$tintProp = [double]$spec.tint }
    return
  }
  if ($spec.PSObject.Properties['auto']) { $obj.ColorIndex = -4105; return }
}

function Has($o, [string]$n) { return $null -ne $o.PSObject.Properties[$n] }

function Apply-Op($app, $wb, $ws, $op) {
  $name = $op.op
  switch ($name) {
    'font' {
      $f = $ws.Range($op.range).Font
      if (Has $op 'name') { $f.Name = $op.name }
      if (Has $op 'size') { $f.Size = [double]$op.size }
      if (Has $op 'bold') { $f.Bold = [bool]$op.bold }
      if (Has $op 'italic') { $f.Italic = [bool]$op.italic }
      if (Has $op 'underline') { $f.Underline = $Underline[$op.underline] }
      if (Has $op 'strike') { $f.Strikethrough = [bool]$op.strike }
      if (Has $op 'color') { Set-Col $f $op.color 'Color' 'ThemeColor' 'TintAndShade' }
    }
    'fill' {
      $i = $ws.Range($op.range).Interior
      $p = $op.pattern
      if ($p -eq 'none') { $i.Pattern = -4142; return }
      if ($p -eq 'solid') {
        if (Has $op 'fg') { Set-Col $i $op.fg 'Color' 'ThemeColor' 'TintAndShade' }
        $i.Pattern = 1
        return
      }
      $i.Pattern = $Pattern[$p]
      if (Has $op 'fg') { Set-Col $i $op.fg 'PatternColor' 'PatternThemeColor' 'PatternTintAndShade' }
      if (Has $op 'bg') { Set-Col $i $op.bg 'Color' 'ThemeColor' 'TintAndShade' }
    }
    'border' {
      $r = $ws.Range($op.range)
      foreach ($s in $op.sides) {
        if ($s -eq 'outline') { $targets = @($r.Borders.Item(7), $r.Borders.Item(8), $r.Borders.Item(9), $r.Borders.Item(10)) }
        elseif ($s -eq 'all') { $targets = @($r.Borders.Item(7), $r.Borders.Item(8), $r.Borders.Item(9), $r.Borders.Item(10), $r.Borders.Item(11), $r.Borders.Item(12)) }
        else { $targets = @($r.Borders.Item($Side[$s])) }
        foreach ($b in $targets) {
          $m = $LineStyleMap[$op.style]
          $b.LineStyle = $m[0]
          if ($op.style -ne 'none') { $b.Weight = $m[1] }
          if (Has $op 'color') { Set-Col $b $op.color 'Color' 'ThemeColor' 'TintAndShade' }
        }
      }
    }
    'align' {
      $r = $ws.Range($op.range)
      if (Has $op 'horizontal') { $r.HorizontalAlignment = $HAlign[$op.horizontal] }
      if (Has $op 'vertical') { $r.VerticalAlignment = $VAlign[$op.vertical] }
      if (Has $op 'wrap') { $r.WrapText = [bool]$op.wrap }
      if (Has $op 'indent') { $r.IndentLevel = [int]$op.indent }
      if (Has $op 'rotation') { if ($op.rotation -eq 'vertical') { $r.Orientation = -4166 } else { $r.Orientation = [int]$op.rotation } }
      if (Has $op 'shrink') { $r.ShrinkToFit = [bool]$op.shrink }
    }
    'numfmt' { $ws.Range($op.range).NumberFormat = $op.code }
    'protect' {
      $r = $ws.Range($op.range)
      if (Has $op 'locked') { $r.Locked = [bool]$op.locked }
      if (Has $op 'hidden') { $r.FormulaHidden = [bool]$op.hidden }
    }
    'style' { $ws.Range($op.range).Style = $script:StyleLocal[$op.name] }
    'decimals' {
      $ws.Activate(); $ws.Range($op.range).Select() | Out-Null
      $id = if ([int]$op.delta -gt 0) { 'DecimalsIncrease' } else { 'DecimalsDecrease' }
      $app.CommandBars.ExecuteMso($id)
    }
    'percent' { $ws.Activate(); $ws.Range($op.range).Select() | Out-Null; $app.CommandBars.ExecuteMso('PercentStyle') }
    'comma' { $ws.Activate(); $ws.Range($op.range).Select() | Out-Null; $app.CommandBars.ExecuteMso('CommaStyle') }
    'accounting' { $ws.Activate(); $ws.Range($op.range).Select() | Out-Null; $app.CommandBars.ExecuteMso('AccountingFormat') }
    'clear_formats' { $ws.Range($op.range).ClearFormats() | Out-Null }
    'painter' {
      $ws.Range($op.from).Copy() | Out-Null
      $ws.Range($op.to).PasteSpecial(-4122) | Out-Null
      $app.CutCopyMode = 0
    }
    'value' { $ws.Range($op.range).Value2 = [double]$op.value }
    'row_height' { $ws.Range($op.range).RowHeight = [double]$op.points }
    'col_width' { $ws.Range($op.range).ColumnWidth = [double]$op.width }
    default { throw "unknown op $name" }
  }
}

# ---------------------------------------------------------------------------------------------------------------
# Reading styles.xml back
# ---------------------------------------------------------------------------------------------------------------
# English name of a built-in cell style by builtinId (the file stores the localised name).
function Canonical-Style-Name([int]$id, [string]$fallback) {
  $fixed = @{ 0 = 'Normal'; 3 = 'Comma'; 4 = 'Currency'; 5 = 'Percent'; 6 = 'Comma [0]'; 7 = 'Currency [0]'; 8 = 'Hyperlink'; 9 = 'Followed Hyperlink'
    10 = 'Note'; 11 = 'Warning Text'; 15 = 'Title'; 16 = 'Heading 1'; 17 = 'Heading 2'; 18 = 'Heading 3'; 19 = 'Heading 4'; 20 = 'Input'
    21 = 'Output'; 22 = 'Calculation'; 23 = 'Check Cell'; 24 = 'Linked Cell'; 25 = 'Total'; 26 = 'Good'; 27 = 'Bad'; 28 = 'Neutral'; 53 = 'Explanatory Text' }
  if ($fixed.ContainsKey($id)) { return $fixed[$id] }
  if ($id -ge 29 -and $id -le 52) {
    $k = ($id - 29) % 4; $acc = [math]::Floor(($id - 29) / 4) + 1
    $pct = @('', '20% - ', '40% - ', '60% - ')[$k]
    return "${pct}Accent$acc"
  }
  return $fallback
}

function Items([string]$xml, [string]$section, [string]$item) {
  $m = [regex]::Match($xml, "(?s)<$section(\s[^>]*)?>(.*?)</$section>")
  if (-not $m.Success) { return @() }
  $body = $m.Groups[2].Value
  $pat = "(?s)<$item\b[^>]*?/>|<$item\b[^>]*?>.*?</$item>"
  return @([regex]::Matches($body, $pat) | ForEach-Object { $_.Value })
}

function Attr([string]$tag, [string]$name) {
  $m = [regex]::Match($tag, "^<\w+\b[^>]*?\b$name=`"([^`"]*)`"")
  if ($m.Success) { return $m.Groups[1].Value }
  return $null
}

function Parse-Styles([string]$xml) {
  $s = [ordered]@{}
  $s.fonts = @(Items $xml 'fonts' 'font')
  $s.fills = @(Items $xml 'fills' 'fill')
  $s.borders = @(Items $xml 'borders' 'border')
  $s.cellXfs = @(Items $xml 'cellXfs' 'xf')
  $s.cellStyleXfs = @(Items $xml 'cellStyleXfs' 'xf')
  $s.cellStyles = @(Items $xml 'cellStyles' 'cellStyle')
  $s.numFmts = @(Items $xml 'numFmts' 'numFmt')
  return $s
}

function Xf-Inner([string]$xf) {
  $m = [regex]::Match($xf, '(?s)^<xf\b[^>]*?>(.*)</xf>$')
  if ($m.Success) { return $m.Groups[1].Value }
  return ''
}

# Canonical effective format of cellXfs[i]: nothing that depends on table order.
function Canon-Xf($st, [int]$i) {
  $xf = $st.cellXfs[$i]
  $fontId = [int](Attr $xf 'fontId'); $fillId = [int](Attr $xf 'fillId'); $borderId = [int](Attr $xf 'borderId')
  $numFmtId = [int](Attr $xf 'numFmtId'); $xfId = [int](Attr $xf 'xfId')
  $code = $null
  if ($numFmtId -ge 164) {
    foreach ($n in $st.numFmts) { if ((Attr $n 'numFmtId') -eq "$numFmtId") { $code = (Attr $n 'formatCode') } }
  }
  $styleName = $null
  foreach ($cs in $st.cellStyles) { if ((Attr $cs 'xfId') -eq "$xfId") { $b = Attr $cs 'builtinId'; $styleName = $(if ($b) { "builtin:$b" } else { Attr $cs 'name' }) } }
  $attrs = [regex]::Replace($xf, '\b(fontId|fillId|borderId|numFmtId|xfId)="[^"]*"\s*', '')
  return [ordered]@{
    font = $st.fonts[$fontId]; fill = $st.fills[$fillId]; border = $st.borders[$borderId]
    numFmt = $(if ($code) { "custom:$code" } else { "id:$numFmtId" })
    style = $styleName; xf = $attrs
  }
}

function Effective-Xf([string]$sheetXml, [string]$cellRef) {
  $m = [regex]::Match($cellRef, '^([A-Z]+)(\d+)$')
  $colName = $m.Groups[1].Value; $row = [int]$m.Groups[2].Value
  $col = 0; foreach ($ch in $colName.ToCharArray()) { $col = $col * 26 + ([int]$ch - 64) }
  $cm = [regex]::Match($sheetXml, "<c r=`"$cellRef`"([^>]*?)/?>")
  if ($cm.Success) { $s = [regex]::Match($cm.Groups[1].Value, '\bs="(\d+)"'); if ($s.Success) { return [int]$s.Groups[1].Value } else { return 0 } }
  $rm = [regex]::Match($sheetXml, "<row r=`"$row`"([^>]*?)/?>")
  if ($rm.Success -and $rm.Groups[1].Value -match 'customFormat="1"') { $s = [regex]::Match($rm.Groups[1].Value, '\bs="(\d+)"'); if ($s.Success) { return [int]$s.Groups[1].Value } }
  foreach ($c in [regex]::Matches($sheetXml, '<col\b[^>]*?/>')) {
    $min = [int](Attr $c.Value 'min'); $max = [int](Attr $c.Value 'max')
    if ($col -ge $min -and $col -le $max) { $st = Attr $c.Value 'style'; if ($st) { return [int]$st } }
  }
  return 0
}

function Com-Cell($ws, [string]$a) {
  $c = $ws.Range($a)
  $o = [ordered]@{}
  $get = { param($b) try { & $b } catch { $null } }
  $o.font = [ordered]@{
    name = & $get { $c.Font.Name }; size = & $get { $c.Font.Size }; bold = & $get { $c.Font.Bold }; italic = & $get { $c.Font.Italic }
    underline = & $get { $c.Font.Underline }; strike = & $get { $c.Font.Strikethrough }; color = & $get { $c.Font.Color }
  }
  $o.interior = [ordered]@{ pattern = & $get { $c.Interior.Pattern }; color = & $get { $c.Interior.Color }; patternColor = & $get { $c.Interior.PatternColor } }
  $b = [ordered]@{}
  foreach ($k in 'left', 'right', 'top', 'bottom', 'diagonalDown', 'diagonalUp') {
    $idx = $Side[$k]
    $b[$k] = [ordered]@{ lineStyle = & $get { $c.Borders.Item($idx).LineStyle }; weight = & $get { $c.Borders.Item($idx).Weight }; color = & $get { $c.Borders.Item($idx).Color } }
  }
  $o.borders = $b
  $o.align = [ordered]@{
    horizontal = & $get { $c.HorizontalAlignment }; vertical = & $get { $c.VerticalAlignment }; wrap = & $get { $c.WrapText }
    indent = & $get { $c.IndentLevel }; orientation = & $get { $c.Orientation }; shrink = & $get { $c.ShrinkToFit }
  }
  $o.numberFormat = & $get { $c.NumberFormat }
  $o.locked = & $get { $c.Locked }; $o.formulaHidden = & $get { $c.FormulaHidden }
  $o.styleName = & $get { $c.Style.Name }
  return $o
}

function Cells-In([string]$spec) {
  $out = @()
  foreach ($part in $spec -split ',') {
    $m = [regex]::Match($part.Trim(), '^([A-Z]+)(\d+)(?::([A-Z]+)(\d+))?$')
    if (-not $m.Success) { throw "bad probe $part" }
    $cn = { param($s) $n = 0; foreach ($ch in $s.ToCharArray()) { $n = $n * 26 + ([int]$ch - 64) }; $n }
    $c1 = & $cn $m.Groups[1].Value; $r1 = [int]$m.Groups[2].Value
    if ($m.Groups[3].Success) { $c2 = & $cn $m.Groups[3].Value; $r2 = [int]$m.Groups[4].Value } else { $c2 = $c1; $r2 = $r1 }
    for ($r = $r1; $r -le $r2; $r++) {
      for ($c = $c1; $c -le $c2; $c++) {
        $name = ''; $n = $c; while ($n -gt 0) { $n--; $name = [string][char](65 + $n % 26) + $name; $n = [math]::Floor($n / 26) }
        $out += "$name$r"
      }
    }
  }
  return $out
}

function To-Json($o) { return ($o | ConvertTo-Json -Depth 12) }

# ---------------------------------------------------------------------------------------------------------------
$script:StyleLocal = @{}
New-Item -ItemType Directory -Force -Path $work | Out-Null
try {
  Start-Excel
  $app = $script:xl

  if ($MakeBlank) {
    $wb = $app.Workbooks.Add()
    while ($wb.Worksheets.Count -gt 1) { $wb.Worksheets.Item($wb.Worksheets.Count).Delete() }
    $wb.Worksheets.Item(1).Name = 'Sheet1'
    $wb.SaveAs($blank, 51)
    $wb.Close($false)
    Scrub-Core $blank
    @"
feature = "format-blank"
made_with = "Microsoft Excel $($app.Version) (Windows)"
author = "project member"
expect_dropped = []
notes = "empty one-sheet workbook: the starting point of every cell-formatting oracle scenario"
"@ | Set-Content -Path (Join-Path $fixtureDir 'format-blank.toml') -Encoding UTF8
    Write-Host "wrote $blank"
    return
  }

  # Map canonical built-in style names to this Excel's (possibly localised) names, through the saved file.
  $probe = $app.Workbooks.Open($blank, 0, $true)
  $pws = $probe.Worksheets.Item(1)
  $localNames = @(); foreach ($s in $probe.Styles) { $localNames += $s.Name }
  for ($i = 0; $i -lt $localNames.Count; $i++) { $pws.Cells.Item($i + 1, 1).Style = $localNames[$i] }
  $mapPath = Join-Path $work 'styles-map.xlsx'
  $probe.SaveAs($mapPath, 51)
  $probe.Close($false)
  $mst = Parse-Styles (Read-ZipText $mapPath 'xl/styles.xml')
  $msheet = Read-ZipText $mapPath 'xl/worksheets/sheet1.xml'
  $builtin = @()
  for ($i = 0; $i -lt $localNames.Count; $i++) {
    $xfi = Effective-Xf $msheet "A$($i + 1)"
    $xf = $mst.cellXfs[$xfi]
    $xfId = [int](Attr $xf 'xfId')
    $cs = $null; foreach ($c in $mst.cellStyles) { if ((Attr $c 'xfId') -eq "$xfId") { $cs = $c } }
    $sx = $mst.cellStyleXfs[$xfId]
    $fontId = [int](Attr $sx 'fontId'); $fillId = [int](Attr $sx 'fillId'); $borderId = [int](Attr $sx 'borderId'); $numFmtId = [int](Attr $sx 'numFmtId')
    $code = $null
    if ($numFmtId -ge 164) { foreach ($n in $mst.numFmts) { if ((Attr $n 'numFmtId') -eq "$numFmtId") { $code = (Attr $n 'formatCode') } } }
    $canon = if ($cs) { Canonical-Style-Name ([int](Attr $cs 'builtinId')) (Attr $cs 'name') } else { $null }
    if ($canon) { $script:StyleLocal[$canon] = $localNames[$i] }
    $builtin += [ordered]@{
      local = $localNames[$i]; name = $canon; builtinId = $(if ($cs) { Attr $cs 'builtinId' } else { $null })
      cellStyle = $cs; xf = $sx; font = $mst.fonts[$fontId]; fill = $mst.fills[$fillId]; border = $mst.borders[$borderId]
      numFmtId = $numFmtId; numFmtCode = $code
    }
  }
  $script:StyleLocal['Normal'] = $localNames | Where-Object { $_ -eq 'Normal' }
  if ($BuiltinStyles) {
    $dir = Join-Path $refDir 'format-builtin-styles'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    To-Json ([ordered]@{ excel = $app.Version; styles = $builtin }) | Set-Content -Path (Join-Path $dir 'styles.json') -Encoding UTF8
    Write-Host "wrote $dir/styles.json ($($builtin.Count) styles)"
    return
  }

  $spec = Get-Content -Raw -Path $scenarioFile | ConvertFrom-Json
  $done = 0
  foreach ($sc in $spec.scenarios) {
    if ($Only -and $sc.name -notmatch $Only) { continue }
    $wb = $app.Workbooks.Open($blank, 0, $true)
    $ws = $wb.Worksheets.Item(1)
    $err = $null
    try { foreach ($op in $sc.ops) { Apply-Op $app $wb $ws $op } }
    catch { $err = $_.Exception.Message; Write-Warning "$($sc.name): $err" }
    $probe = if ($sc.probe) { $sc.probe } else { 'A1:F8' }
    $addrs = Cells-In $probe
    $com = [ordered]@{}
    foreach ($a in $addrs) { $com[$a] = Com-Cell $ws $a }
    $outPath = Join-Path $work ($sc.name + '.xlsx')
    $wb.SaveAs($outPath, 51)
    $wb.Close($false)
    $st = Parse-Styles (Read-ZipText $outPath 'xl/styles.xml')
    $sheetXml = Read-ZipText $outPath 'xl/worksheets/sheet1.xml'
    $cells = [ordered]@{}
    foreach ($a in $addrs) {
      $xi = Effective-Xf $sheetXml $a
      $cells[$a] = [ordered]@{ canon = (Canon-Xf $st $xi); com = $com[$a] }
    }
    $colsXml = [regex]::Match($sheetXml, '(?s)<cols>.*?</cols>').Value
    $rowAttrs = @([regex]::Matches($sheetXml, '<row\b[^>]*>') | ForEach-Object { $_.Value } | Where-Object { $_ -match '\bs="' })
    $out = [ordered]@{
      name = $sc.name; excel = $app.Version; error = $err
      counts = [ordered]@{
        fonts = $st.fonts.Count; fills = $st.fills.Count; borders = $st.borders.Count; cellXfs = $st.cellXfs.Count
        cellStyleXfs = $st.cellStyleXfs.Count; cellStyles = $st.cellStyles.Count; numFmts = $st.numFmts.Count
      }
      tables = [ordered]@{ fonts = $st.fonts; fills = $st.fills; borders = $st.borders; cellXfs = $st.cellXfs; cellStyleXfs = $st.cellStyleXfs; cellStyles = $st.cellStyles; numFmts = $st.numFmts }
      cols = $colsXml; styledRows = $rowAttrs
      cells = $cells
    }
    $dir = Join-Path $refDir ('format-' + $sc.name)
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    To-Json $out | Set-Content -Path (Join-Path $dir 'expected.json') -Encoding UTF8
    $done++
    Write-Host ("{0,-28} fonts={1} fills={2} borders={3} xfs={4}" -f $sc.name, $st.fonts.Count, $st.fills.Count, $st.borders.Count, $st.cellXfs.Count)
  }
  Write-Host "$done scenarios"
}
finally {
  Stop-Excel
  if (Test-Path $work) { Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue }
}
