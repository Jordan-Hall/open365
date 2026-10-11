<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365 grid auto-fit (double-clicking a
column or row header border).

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this
script is never run in CI. CI consumes the committed output (fixtures/reference/xlsx/grid-autofit/autofit.json)
through crates/ui-egui/tests/grid_fit.rs.

-MakeFixture first (re)creates fixtures/xlsx/grid-autofit.xlsx and its .toml: sheet "Cols" has one column per
case (short and long text, fonts, bold, italic, numbers in General and formatted, dates, wrapped, merged,
empty, indented, rotated), sheet "Rows" has one row per case (font sizes and families, wrapped text, newlines).
The workbook is saved before any auto-fit, so the fixture keeps Excel's default sizes. Nothing is hand-edited.

Then, on a copy it never saves, it calls Columns(c).AutoFit() for every case column and Rows(r).AutoFit() for
every case row and records ColumnWidth (characters), Range.Width and Range.Height (points) exactly as Excel
returns them, plus the window DPI of the run (120 on the dev machine, 150% scaling). The Rust test converts
points to pixels with that DPI and compares with ==, no tolerance.

  pwsh -NoProfile -File tools/office/grid-autofit-oracle.ps1 -MakeFixture

Process safety: starts its own hidden Excel instance, remembers its process id and only ever stops that
process (in a finally block). Other EXCEL/WINWORD processes are never touched.

Output is stable: LF line endings, ASCII only, no absolute paths and no user information.
#>
param(
  [switch]$MakeFixture
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx/grid-autofit'
$inv = [Globalization.CultureInfo]::InvariantCulture

Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetDpiForWindow(System.IntPtr hWnd);
'@

$script:xl = $null
$script:xlPid = 0
$script:dpi = 96

function Start-Excel {
  $app = New-Object -ComObject Excel.Application
  $app.Visible = $false
  $app.DisplayAlerts = $false
  $app.ScreenUpdating = $false
  $app.EnableEvents = $false
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  $script:xl = $app
  $script:xlPid = [int]$procId
  $script:dpi = [int][Native.Win32]::GetDpiForWindow([IntPtr]$app.Hwnd)
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

function Num($v) { return ([double]$v).ToString('R', $inv) }

function Json-Str([string]$s) {
  return '"' + ($s -replace '\\', '\\' -replace '"', '\"') + '"'
}

# One column per case: label, then the cells to put in it (row 1 downwards). A cell is
# @{ v = value; f = formula; font = name; size = n; bold; italic; fmt = number format; wrap; indent; rot; shrink }.
function Cell-Spec($v, $o) { $o['v'] = $v; return $o }

$colCases = @(
  @{ label = 'short text'; cells = @(@{ v = 'Hello' }) },
  @{ label = 'long text'; cells = @(@{ v = 'The quick brown fox jumps over the lazy dog' }) },
  @{ label = 'bold'; cells = @(@{ v = 'Bold text'; bold = $true }) },
  @{ label = 'italic'; cells = @(@{ v = 'Italic text sample'; italic = $true }) },
  @{ label = 'cambria 14'; cells = @(@{ v = 'Cambria fourteen'; font = 'Cambria'; size = 14 }) },
  @{ label = 'general number'; cells = @(@{ v = 1234567.891 }) },
  @{ label = 'thousands format'; cells = @(@{ v = 1234567.891; fmt = '#,##0.00' }) },
  @{ label = 'iso date'; cells = @(@{ v = 45366; fmt = 'yyyy-mm-dd' }, @{ v = 45367 }) },
  @{ label = 'currency negative'; cells = @(@{ v = -1234.5; fmt = '$#,##0.00;[Red]($#,##0.00)' }) },
  @{ label = 'wrapped'; cells = @(@{ v = 'wrapped text that is long enough to need several lines'; wrap = $true }) },
  @{ label = 'several cells'; cells = @(@{ v = 'a' }, @{ v = 'medium length text' }, @{ v = 'tiny' }) },
  @{ label = 'empty with fill'; cells = @(@{ fill = 255 }) },
  @{ label = 'merged only'; cells = @(@{ v = 'merged text that is rather long'; merge = 'M1:N1' }, @{ v = 'x' }) },
  @{ label = 'merged only 2'; cells = @() },
  @{ label = 'formula'; cells = @(@{ f = '=SUM(1,2)' }, @{ f = '="a long formula result text"' }) },
  @{ label = 'indent 2'; cells = @(@{ v = 'Indented'; indent = 2 }) },
  @{ label = 'cambria 11'; cells = @(@{ v = 'Cambria eleven text'; font = 'Cambria' }) },
  @{ label = 'calibri 9'; cells = @(@{ v = 'Small nine'; size = 9 }) },
  @{ label = 'calibri 20'; cells = @(@{ v = 'Big'; size = 20 }) },
  @{ label = 'bool and error'; cells = @(@{ v = $true }, @{ f = '=1/0' }) },
  @{ label = 'calibri 14 bold'; cells = @(@{ v = 'Bold fourteen'; size = 14; bold = $true }) },
  @{ label = 'small decimal'; cells = @(@{ v = 0.000123456789 }) },
  @{ label = 'percent'; cells = @(@{ v = 0.256; fmt = '0.0%' }) },
  @{ label = 'text number'; cells = @(@{ v = '00123'; fmt = '@' }) },
  @{ label = 'leading spaces'; cells = @(@{ v = '   leading' }) },
  @{ label = 'rotated'; cells = @(@{ v = 'Rotated text'; rot = 90 }) },
  @{ label = 'shrink'; cells = @(@{ v = 'Shrink me please'; shrink = $true }) },
  @{ label = 'bold number'; cells = @(@{ v = 12345; bold = $true }) },
  @{ label = 'accents'; cells = @(@{ v = 'Unicode accents eaiou' }) },
  @{ label = 'very long'; cells = @(@{ v = ('W' * 300) }) },
  @{ label = 'twelve digits'; cells = @(@{ v = 123456789012 }) },
  @{ label = 'float sum'; cells = @(@{ f = '=0.1+0.2' }) },
  @{ label = 'one third'; cells = @(@{ f = '=1/3' }) },
  @{ label = 'negative decimal'; cells = @(@{ v = -5.5 }) },
  @{ label = 'text with newline'; cells = @(@{ v = "line one`nline two longer"; wrap = $true }) },
  @{ label = 'calibri 11 digits'; cells = @(@{ v = 8 }, @{ v = 88 }, @{ v = 888 }) },
  @{ label = 'calibri 10 italic'; cells = @(@{ v = 'Italic ten point text'; size = 10; italic = $true }) },
  @{ label = 'bold italic'; cells = @(@{ v = 'Bold italic text'; bold = $true; italic = $true }) },
  @{ label = 'underline'; cells = @(@{ v = 'Underlined text'; underline = $true }) }
)

$rowCases = @(
  @{ label = 'calibri 11'; text = 'x' },
  @{ label = 'calibri 20'; text = 'x'; size = 20 },
  @{ label = 'cambria 10'; text = 'x'; font = 'Cambria'; size = 10 },
  @{ label = 'cambria 14'; text = 'x'; font = 'Cambria'; size = 14 },
  @{ label = 'calibri 8'; text = 'x'; size = 8 },
  @{ label = 'calibri 36'; text = 'x'; size = 36 },
  @{ label = 'wrapped 3 lines'; text = 'alpha beta gamma delta epsilon zeta eta theta'; wrap = $true },
  @{ label = 'wrapped big'; text = 'alpha beta gamma delta epsilon'; wrap = $true; size = 16 },
  @{ label = 'empty'; text = $null },
  @{ label = 'bold 11'; text = 'x'; bold = $true },
  @{ label = 'cambria 12'; text = 'x'; font = 'Cambria'; size = 12 },
  @{ label = 'newline'; text = "one`ntwo"; wrap = $true },
  @{ label = 'cambria 11'; text = 'x'; font = 'Cambria'; size = 11 },
  @{ label = 'calibri 9 italic'; text = 'x'; size = 9; italic = $true },
  @{ label = 'calibri 10 bold'; text = 'x'; size = 10; bold = $true },
  @{ label = 'calibri 12'; text = 'x'; size = 12 },
  @{ label = 'calibri 14'; text = 'x'; size = 14 },
  @{ label = 'calibri 9'; text = 'x'; size = 9 },
  @{ label = 'calibri 10'; text = 'x'; size = 10 },
  @{ label = 'calibri 16'; text = 'x'; size = 16 },
  @{ label = 'cambria 8'; text = 'x'; font = 'Cambria'; size = 8 },
  @{ label = 'cambria 20'; text = 'x'; font = 'Cambria'; size = 20 }
)

function Apply($cell, $o) {
  if ($o.ContainsKey('font')) { $cell.Font.Name = $o['font'] }
  if ($o.ContainsKey('size')) { $cell.Font.Size = [double]$o['size'] }
  if ($o.ContainsKey('bold')) { $cell.Font.Bold = $true }
  if ($o.ContainsKey('italic')) { $cell.Font.Italic = $true }
  if ($o.ContainsKey('underline')) { $cell.Font.Underline = 2 }
  if ($o.ContainsKey('fmt')) { $cell.NumberFormat = $o['fmt'] }
  if ($o.ContainsKey('wrap')) { $cell.WrapText = $true }
  if ($o.ContainsKey('indent')) { $cell.HorizontalAlignment = -4131; $cell.IndentLevel = [int]$o['indent'] }
  if ($o.ContainsKey('rot')) { $cell.Orientation = [int]$o['rot'] }
  if ($o.ContainsKey('shrink')) { $cell.ShrinkToFit = $true }
  if ($o.ContainsKey('fill')) { $cell.Interior.Color = [int]$o['fill'] }
}

function Build($app) {
  $wb = $app.Workbooks.Add()
  # Calibri 11 as the default font: a bundled font, so the grid measures the same glyphs Excel does.
  $wb.Styles.Item('Normal').Font.Name = 'Calibri'
  $wb.Styles.Item('Normal').Font.Size = 11
  while ($wb.Worksheets.Count -lt 2) { [void]$wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
  $cs = $wb.Worksheets.Item(1); $cs.Name = 'Cols'
  $rs = $wb.Worksheets.Item(2); $rs.Name = 'Rows'
  for ($i = 0; $i -lt $colCases.Count; $i++) {
    $col = $i + 1
    $r = 0
    foreach ($o in $colCases[$i].cells) {
      $r++
      $cell = $cs.Cells.Item($r, $col)
      if ($o.ContainsKey('f')) { $cell.Formula = $o['f'] }
      elseif ($o.ContainsKey('v')) {
        if ($o.ContainsKey('fmt') -and $o['fmt'] -eq '@') { $cell.NumberFormat = '@' }
        $v = $o['v']
        if ($v -is [string]) { $cell.Value2 = $v }
        elseif ($v -is [bool]) { $cell.Formula = $(if ($v) { 'TRUE' } else { 'FALSE' }) }
        else { $cell.Formula = ([double]$v).ToString('R', $inv) }
      }
      Apply $cell $o
      if ($o.ContainsKey('merge')) { $cs.Range($o['merge']).Merge() }
    }
  }
  $rs.Columns.Item(1).ColumnWidth = 12
  for ($i = 0; $i -lt $rowCases.Count; $i++) {
    $row = $i + 1
    $o = $rowCases[$i]
    $cell = $rs.Cells.Item($row, 1)
    if ($null -ne $o.text) { $cell.Value2 = $o.text }
    Apply $cell $o
  }
  return $wb
}

function Measure-Fit($wb) {
  $cs = $wb.Worksheets.Item('Cols')
  $rs = $wb.Worksheets.Item('Rows')
  $cols = @()
  for ($i = 0; $i -lt $colCases.Count; $i++) {
    $c = $i + 1
    $cs.Columns.Item($c).AutoFit() | Out-Null
    $w = $cs.Columns.Item($c).ColumnWidth
    $pt = $cs.Cells.Item(1, $c).Width
    $cols += ('    {"col": ' + $c + ', "label": ' + (Json-Str $colCases[$i].label) + ', "width_chars": ' + (Num $w) + ', "width_pt": ' + (Num $pt) + '}')
  }
  $rows = @()
  for ($i = 0; $i -lt $rowCases.Count; $i++) {
    $r = $i + 1
    $rs.Rows.Item($r).AutoFit() | Out-Null
    $h = $rs.Cells.Item($r, 1).Height
    $rows += ('    {"row": ' + $r + ', "label": ' + (Json-Str $rowCases[$i].label) + ', "height_pt": ' + (Num $h) + '}')
  }
  return @($cols, $rows)
}

try {
  Start-Excel
  $path = Join-Path $fixtureDir 'grid-autofit.xlsx'
  $wb = Build $script:xl
  if ($MakeFixture) {
    if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    $wb.SaveAs($path, 51)
    $toml = [IO.Path]::ChangeExtension($path, '.toml')
    $made = "Microsoft Excel $($script:xl.Build) (Windows)"
    $lines = @(
      'feature = "grid-autofit"',
      ('made_with = ' + (Json-Str $made)),
      'author = "project member"',
      'expect_dropped = []',
      'notes = "one column per auto-fit case (text, fonts, numbers, formats, wrapped, merged, empty) and one row per row auto-fit case"'
    )
    [IO.File]::WriteAllText($toml, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  }
  $res = Measure-Fit $wb
  $wb.Saved = $true
  $wb.Close($false)
  $lines = @(
    '{',
    '  "source": "fixtures/xlsx/grid-autofit.xlsx",',
    '  "oracle": "Microsoft Excel via COM: Columns(c).AutoFit() then ColumnWidth and Range.Width; Rows(r).AutoFit() then Range.Height",',
    ('  "dpi": ' + $script:dpi + ','),
    ('  "excel_build": ' + (Json-Str ([string]$script:xl.Build)) + ','),
    '  "cols": [',
    ($res[0] -join ",`n"),
    '  ],',
    '  "rows": [',
    ($res[1] -join ",`n"),
    '  ]',
    '}'
  )
  New-Item -ItemType Directory -Force -Path $refDir | Out-Null
  [IO.File]::WriteAllText((Join-Path $refDir 'autofit.json'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  Write-Host 'grid-autofit-oracle: wrote autofit.json'
}
finally {
  Stop-Excel
}
