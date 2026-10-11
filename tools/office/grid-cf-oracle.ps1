<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365 conditional-format display.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed output (fixtures/reference/xlsx/grid-cf/displayformat.json) through
crates/commands/tests/conditional.rs and crates/ui-egui/tests/grid_cf.rs.

-MakeFixture (re)creates fixtures/xlsx/grid-cf.xlsx and its .toml through COM: one column of twelve values per rule
type (cellIs, expression, top10, aboveAverage, duplicate/unique, text rules, blanks, errors, timePeriod, colorScale,
dataBar, iconSet, rule priority and stopIfTrue, dxf number formats). Excel writes the file; docProps/core.xml
dc:creator and cp:lastModifiedBy are scrubbed to empty strings. Nothing is hand-edited.

The oracle then opens the workbook read-only (never saved) and records, for every cell of the used range, what Excel
displays: Range.Text and Range.DisplayFormat (Interior.Pattern / Color, Font.Color / Bold / Italic / Strikethrough /
Underline, NumberFormat, the four edge Borders). Colours are Excel's resolved BGR integers. "today" is the TODAY()
serial of the run: the time-period fixtures hold constant dates around it and the Rust test pins the clock to it.

  pwsh -NoProfile -File tools/office/grid-cf-oracle.ps1                 # record displayformat.json
  pwsh -NoProfile -File tools/office/grid-cf-oracle.ps1 -MakeFixture    # rebuild the fixture first

Process safety: starts its own hidden Excel instance, remembers its process id and only ever stops that process
(in a finally block). Other EXCEL/WINWORD processes are never touched.
#>
param(
  [switch]$MakeFixture
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture

Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@

$script:xl = $null
$script:xlPid = 0

function Start-Excel {
  $app = New-Object -ComObject Excel.Application
  $app.Visible = $false
  $app.DisplayAlerts = $false
  $app.EnableEvents = $false
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  $script:xl = $app
  $script:xlPid = [int]$procId
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
    # Only the process this script started; the id was taken from our own window handle.
    if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $script:xlPid -Force -ErrorAction SilentlyContinue }
  }
  $script:xlPid = 0
}

function Json-Str([string]$s) {
  $sb = New-Object System.Text.StringBuilder
  [void]$sb.Append('"')
  foreach ($ch in $s.ToCharArray()) {
    $code = [int]$ch
    if ($ch -eq '"') { [void]$sb.Append('\"') }
    elseif ($ch -eq '\') { [void]$sb.Append('\\') }
    elseif ($code -lt 0x20 -or $code -gt 0x7e) { [void]$sb.AppendFormat('\u{0:x4}', $code) }
    else { [void]$sb.Append($ch) }
  }
  [void]$sb.Append('"')
  return $sb.ToString()
}

function Rgb([int]$r, [int]$g, [int]$b) { return $r + 256 * $g + 65536 * $b }

function Scrub-Core([string]$path) {
  Add-Type -AssemblyName System.IO.Compression
  Add-Type -AssemblyName System.IO.Compression.FileSystem
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

$M = [Type]::Missing
$fillRed = Rgb 255 199 206
$fontRed = Rgb 156 0 6
$fillYellow = Rgb 255 235 156
$fontYellow = Rgb 156 87 0
$fillGreen = Rgb 198 239 206
$fillBlue = Rgb 189 215 238

# Style a format condition: fill (BGR or $null), font colour, bold, italic, underline, strike, number format.
function Style($fc, $fill, $font, [bool]$bold = $false, [bool]$italic = $false, [bool]$under = $false, [bool]$strike = $false, [string]$nf = '') {
  try { $fc.StopIfTrue = $false } catch {}
  if ($null -ne $fill) { $fc.Interior.Color = $fill }
  if ($null -ne $font) { $fc.Font.Color = $font }
  if ($bold) { $fc.Font.Bold = $true }
  if ($italic) { $fc.Font.Italic = $true }
  if ($under) { $fc.Font.Underline = 2 }
  if ($strike) { $fc.Font.Strikethrough = $true }
  if ($nf -ne '') { $fc.NumberFormat = $nf }
}

$numbers = @(12, 87, 45, 3, 66, 50, 21, 99, 45, 74, 8, 30)
$texts = @('apple', 'banana', 'Cherry', 'grape', 'pineapple', 'Orange', 'anna', '', 'APPLE pie', 'kiwi', 'band', 'ban')
$dateOffsets = @(-40, -30, -8, -7, -6, -1, 0, 1, 2, 8, 20, 40)

function Fill-Column($ws, [int]$col, [string]$title, $values) {
  $ws.Cells.Item(1, $col).Value2 = $title
  for ($i = 0; $i -lt $values.Count; $i++) {
    $v = $values[$i]
    if ($null -eq $v -or "$v" -eq '') { continue }
    if ($v -is [string] -and $v.StartsWith('=')) { $ws.Cells.Item(2 + $i, $col).Formula = $v }
    elseif ($v -is [string]) { $ws.Cells.Item(2 + $i, $col).Formula = $v }
    else { $ws.Cells.Item(2 + $i, $col).Value2 = [double]$v }
  }
}

function Col-Range($ws, [int]$col) { return ,$ws.Range($ws.Cells.Item(2, $col), $ws.Cells.Item(13, $col)) }

function Write-Fixture($app, [string]$path) {
  $wb = $app.Workbooks.Add()
  $ws = $wb.Worksheets.Item(1); $ws.Name = 'Rules'
  $todaySerial = [double]$app.Evaluate('TODAY()')
  $next = { $script:col = $script:col + 1; return $script:col }
  $script:col = 0

  # --- number rules
  $c = & $next; Fill-Column $ws $c 'cellIs >50' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.Add(1, 5, '=50'); Style $fc $fillRed $fontRed $true
  $c = & $next; Fill-Column $ws $c 'cellIs between' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.Add(1, 1, '=20', '=60'); Style $fc $fillYellow $fontYellow $false $true $true
  $c = & $next; Fill-Column $ws $c 'cellIs =45 numFmt' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.Add(1, 3, '=45'); Style $fc $fillGreen $null $false $false $false $false '0.00"x"'
  $fc.Borders.Item(-4131).LineStyle = 1; $fc.Borders.Item(-4152).LineStyle = 1
  $fc.Borders.Item(-4160).LineStyle = 1; $fc.Borders.Item(-4107).LineStyle = 1
  $fc.Borders.Item(-4107).Color = Rgb 255 0 0
  $c = & $next; Fill-Column $ws $c 'cellIs <=21' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.Add(1, 8, '=21'); Style $fc $null $fontRed $false $false $false $true
  $c = & $next; Fill-Column $ws $c 'cellIs notBetween' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.Add(1, 2, '=10', '=80'); Style $fc $fillBlue
  # expression: relative reference into the first column, and a cell-relative test of the cell itself.
  $c = & $next; Fill-Column $ws $c 'expr own+col A' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.Add(2, $M, '=AND(MOD(F2,2)=0,$A2>40)'); Style $fc $fillGreen $null $true
  $c = & $next; Fill-Column $ws $c 'expr banding' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.Add(2, $M, '=MOD(ROW(),2)=0'); Style $fc $fillYellow
  # --- top10 / average / duplicates
  $c = & $next; Fill-Column $ws $c 'top 3' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddTop10(); $fc.TopBottom = 1; $fc.Rank = 3; $fc.Percent = $false; Style $fc $fillGreen
  $c = & $next; Fill-Column $ws $c 'bottom 25 pct' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddTop10(); $fc.TopBottom = 0; $fc.Rank = 25; $fc.Percent = $true; Style $fc $fillRed
  $c = & $next; Fill-Column $ws $c 'above avg' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddAboveAverage(); $fc.AboveBelow = 0; Style $fc $fillGreen
  $c = & $next; Fill-Column $ws $c 'below or eq avg' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddAboveAverage(); $fc.AboveBelow = 3; Style $fc $fillRed
  $c = & $next; Fill-Column $ws $c 'above 1 stddev' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddAboveAverage(); $fc.AboveBelow = 4; $fc.NumStdDev = 1; Style $fc $fillYellow
  $c = & $next; Fill-Column $ws $c 'duplicates' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddUniqueValues(); $fc.DupeUnique = 1; Style $fc $fillRed $fontRed
  $c = & $next; Fill-Column $ws $c 'unique' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddUniqueValues(); $fc.DupeUnique = 0; Style $fc $fillGreen
  # --- text
  $c = & $next; Fill-Column $ws $c 'contains an' $texts
  $fc = (Col-Range $ws $c).FormatConditions.Add(9, $M, $M, $M, 'an', 0); Style $fc $fillYellow
  $c = & $next; Fill-Column $ws $c 'begins ap' $texts
  $fc = (Col-Range $ws $c).FormatConditions.Add(9, $M, $M, $M, 'ap', 2); Style $fc $fillGreen
  $c = & $next; Fill-Column $ws $c 'ends e' $texts
  $fc = (Col-Range $ws $c).FormatConditions.Add(9, $M, $M, $M, 'e', 3); Style $fc $fillBlue
  $c = & $next; Fill-Column $ws $c 'not contains an' $texts
  $fc = (Col-Range $ws $c).FormatConditions.Add(9, $M, $M, $M, 'an', 1); Style $fc $fillRed
  $c = & $next; Fill-Column $ws $c 'blanks' $texts
  $fc = (Col-Range $ws $c).FormatConditions.Add(10); Style $fc $fillYellow
  $c = & $next; Fill-Column $ws $c 'no blanks' $texts
  $fc = (Col-Range $ws $c).FormatConditions.Add(13); Style $fc $fillGreen
  $errs = @(1, '=1/0', 'x', '=NA()', 5, '=SQRT(-1)', 7, 8, '=1/0', 10, 11, 12)
  $c = & $next; Fill-Column $ws $c 'errors' $errs
  $fc = (Col-Range $ws $c).FormatConditions.Add(16); Style $fc $fillRed $fontRed
  $c = & $next; Fill-Column $ws $c 'no errors' $errs
  $fc = (Col-Range $ws $c).FormatConditions.Add(17); Style $fc $fillGreen
  # --- time periods: constant dates around today
  $dates = $dateOffsets | ForEach-Object { $todaySerial + $_ }
  $periods = @(@('today', 0), @('yesterday', 1), @('last7Days', 2), @('thisWeek', 3), @('lastWeek', 4), @('lastMonth', 5),
    @('tomorrow', 6), @('nextWeek', 7), @('nextMonth', 8), @('thisMonth', 9))
  foreach ($p in $periods) {
    $c = & $next; Fill-Column $ws $c ('date ' + $p[0]) $dates
    (Col-Range $ws $c).NumberFormat = 'yyyy-mm-dd'
    $fc = (Col-Range $ws $c).FormatConditions.Add(11, $M, $M, $M, $M, $M, $p[1]); Style $fc $fillGreen
  }
  # --- colour scales
  $c = & $next; Fill-Column $ws $c 'scale 2' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddColorScale(2)
  $fc.ColorScaleCriteria.Item(1).FormatColor.Color = Rgb 255 255 255
  $fc.ColorScaleCriteria.Item(2).FormatColor.Color = Rgb 248 105 107
  $c = & $next; Fill-Column $ws $c 'scale 3' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddColorScale(3)
  $fc.ColorScaleCriteria.Item(1).FormatColor.Color = Rgb 248 105 107
  $fc.ColorScaleCriteria.Item(2).FormatColor.Color = Rgb 255 235 132
  $fc.ColorScaleCriteria.Item(3).FormatColor.Color = Rgb 99 190 123
  $c = & $next; Fill-Column $ws $c 'scale 3 custom' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddColorScale(3)
  $fc.ColorScaleCriteria.Item(1).Type = 0; $fc.ColorScaleCriteria.Item(1).Value = 10
  $fc.ColorScaleCriteria.Item(1).FormatColor.Color = Rgb 90 138 198
  $fc.ColorScaleCriteria.Item(2).Type = 5; $fc.ColorScaleCriteria.Item(2).Value = 40
  $fc.ColorScaleCriteria.Item(2).FormatColor.Color = Rgb 255 255 255
  $fc.ColorScaleCriteria.Item(3).Type = 3; $fc.ColorScaleCriteria.Item(3).Value = 80
  $fc.ColorScaleCriteria.Item(3).FormatColor.Color = Rgb 248 105 107
  # --- data bar and icon sets (no DisplayFormat equivalent: preserved and drawn, not compared)
  $c = & $next; Fill-Column $ws $c 'data bar' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddDatabar()
  $c = & $next; Fill-Column $ws $c 'icons 3' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddIconSetCondition(); $fc.IconSet = $wb.IconSets.Item(1)
  $c = & $next; Fill-Column $ws $c 'icons 5' $numbers
  $fc = (Col-Range $ws $c).FormatConditions.AddIconSetCondition(); $fc.IconSet = $wb.IconSets.Item(14)
  # --- priority and stopIfTrue
  $c = & $next; Fill-Column $ws $c 'two rules merge' $numbers
  $r = Col-Range $ws $c
  $fc = $r.FormatConditions.Add(1, 5, '=50'); Style $fc $fillRed
  $fc = $r.FormatConditions.Add(1, 5, '=30'); Style $fc $fillBlue $fontRed $true
  $c = & $next; Fill-Column $ws $c 'stopIfTrue' $numbers
  $r = Col-Range $ws $c
  $fc = $r.FormatConditions.Add(1, 5, '=50'); Style $fc $fillRed; $fc.StopIfTrue = $true
  $fc = $r.FormatConditions.Add(1, 5, '=30'); Style $fc $fillBlue $fontRed $true
  # --- dxf number formats on dates and percentages
  $c = & $next; Fill-Column $ws $c 'dxf date numFmt' ($dateOffsets | ForEach-Object { 45000 + 37 * $_ })
  $fc = (Col-Range $ws $c).FormatConditions.Add(1, 7, '=0'); Style $fc $null $null $false $false $false $false 'mmmm\ \''yy'
  $c = & $next; Fill-Column $ws $c 'dxf pct numFmt' @(0.1, 0.256, 0.5, 0.75, 1, 0.01, 0.333, 0.9, 0.05, 0.6, 0.45, 0.8)
  $fc = (Col-Range $ws $c).FormatConditions.Add(1, 5, '=0.4'); Style $fc $null $fontRed $false $false $false $false '0.0%'

  $ws.Range($ws.Columns.Item(1), $ws.Columns.Item($script:col)).ColumnWidth = 14
  $ws.Cells.Item(1, 1).Select()
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
  $wb.SaveAs($path, 51)
  $wb.Close($false)
  Scrub-Core $path
  $toml = [IO.Path]::ChangeExtension($path, '.toml')
  $made = "Microsoft Excel $($app.Build) (Windows)"
  $lines = @(
    'feature = "grid-cf"',
    ('made_with = ' + (Json-Str $made)),
    'author = "project member"',
    'expect_dropped = []',
    'notes = "one column of twelve values per conditional-format rule type: cellIs, expression, top10, aboveAverage, duplicate/unique, text, blanks, errors, timePeriod, colorScale, dataBar, iconSet, priority, stopIfTrue and dxf number formats"'
  )
  [IO.File]::WriteAllText($toml, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  Write-Host "grid-cf-oracle: wrote $path"
}

function Edge($d, [int]$idx) {
  $b = $d.Borders.Item($idx)
  return '[' + [int]$b.LineStyle + ',' + [int]$b.Color + ']'
}

function Read-Display($app, [string]$path) {
  $wb = $app.Workbooks.Open($path, 0, $true)
  try {
    $ws = $wb.Worksheets.Item(1)
    $app.Calculate()
    $today = [double]$app.Evaluate('TODAY()')
    $ur = $ws.UsedRange
    $rows = $ur.Row + $ur.Rows.Count - 1
    $cols = $ur.Column + $ur.Columns.Count - 1
    $cells = @()
    for ($r = 2; $r -le $rows; $r++) {
      for ($c = 1; $c -le $cols; $c++) {
        $cell = $ws.Cells.Item($r, $c)
        $d = $cell.DisplayFormat
        $ref = $cell.Address($false, $false)
        $text = [string]$cell.Text
        $sb = '{"ref":' + (Json-Str $ref) + ',"text":' + (Json-Str $text) +
          ',"pattern":' + [int]$d.Interior.Pattern + ',"fill":' + [int]$d.Interior.Color +
          ',"font_color":' + [int]$d.Font.Color +
          ',"bold":' + ([bool]$d.Font.Bold).ToString().ToLower() +
          ',"italic":' + ([bool]$d.Font.Italic).ToString().ToLower() +
          ',"strike":' + ([bool]$d.Font.Strikethrough).ToString().ToLower() +
          ',"underline":' + [int]$d.Font.Underline +
          ',"numfmt":' + (Json-Str ([string]$d.NumberFormat)) +
          ',"borders":{"left":' + (Edge $d 7) + ',"top":' + (Edge $d 8) + ',"bottom":' + (Edge $d 9) + ',"right":' + (Edge $d 10) + '}}'
        $cells += $sb
      }
    }
    return @{ today = $today; cells = $cells; rows = $rows; cols = $cols }
  }
  finally { $wb.Close($false) }
}

try {
  Start-Excel
  $path = Join-Path $fixtureDir 'grid-cf.xlsx'
  if ($MakeFixture) { Write-Fixture $script:xl $path }
  $res = Read-Display $script:xl $path
  $lines = @(
    '{',
    '  "source": "fixtures/xlsx/grid-cf.xlsx",',
    '  "oracle": "Microsoft Excel via COM: Range.Text and Range.DisplayFormat (Interior, Font, NumberFormat, Borders); colours are BGR integers",',
    ('  "excel_build": ' + (Json-Str ([string]$script:xl.Build)) + ','),
    ('  "today": ' + ([double]$res.today).ToString('R', $inv) + ','),
    ('  "rows": ' + $res.rows + ','),
    ('  "cols": ' + $res.cols + ','),
    '  "cells": ['
  )
  $lines += ($res.cells | ForEach-Object { '    ' + $_ }) -join ",`n"
  $lines += '  ]'
  $lines += '}'
  $outDir = Join-Path $refDir 'grid-cf'
  New-Item -ItemType Directory -Force -Path $outDir | Out-Null
  [IO.File]::WriteAllText((Join-Path $outDir 'displayformat.json'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  Write-Host "grid-cf-oracle: recorded $($res.cells.Count) cells"
}
finally {
  Stop-Excel
}
