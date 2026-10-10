<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365 spreadsheet grid geometry.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this
script is never run in CI. CI consumes the committed output (fixtures/reference/xlsx/<name>/geometry.json)
through crates/xlsx/tests/geometry.rs.

For every fixtures/xlsx/*.xlsx it opens the workbook read-only (never saved) and records, per visible-or-hidden
worksheet, Range.Width of every cell in row 1 (one value per column) and Range.Height of every cell in column A
(one value per row), in points exactly as Excel returns them. A hidden row or column reads 0. The grid recorded
is the used range plus three, and at least 12 rows by 12 columns, so default (untouched) rows and columns are
covered too. Values are points at 100% zoom, with the window DPI of the run recorded as "dpi" (120 on the dev machine, 150% scaling); the Rust test converts its pixel results to
points (px * 72 / dpi) and compares with ==, no tolerance.

-MakeFixture first (re)creates fixtures/xlsx/grid-geometry.xlsx and its .toml: a workbook built through COM with
custom and hidden columns, a changed default column width, custom, hidden and auto-grown rows, merges and
frozen panes, none of it personal data. Excel writes the file; nothing is hand-edited.

  pwsh -NoProfile -File tools/office/grid-geometry-oracle.ps1                 # all fixtures
  pwsh -NoProfile -File tools/office/grid-geometry-oracle.ps1 -MakeFixture    # also rebuild grid-geometry.xlsx
  pwsh -NoProfile -File tools/office/grid-geometry-oracle.ps1 -Only '^merged' # fixtures whose name matches

Process safety: starts its own hidden Excel instance, remembers its process id and only ever stops that
process (in a finally block). Other EXCEL/WINWORD processes are never touched.

Output is stable: LF line endings, ASCII only, no absolute paths and no user information.
#>
param(
  [string]$Only,
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

function Write-Fixture($app, [string]$path) {
  $wb = $app.Workbooks.Add()
  while ($wb.Worksheets.Count -lt 2) { [void]$wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
  $s1 = $wb.Worksheets.Item(1); $s1.Name = 'Custom'
  $s2 = $wb.Worksheets.Item(2); $s2.Name = 'Defaults'
  # Sheet 1: custom widths, a hidden column, custom, hidden and auto-grown rows, merges, frozen panes.
  $widths = @(3, 8.43, 10, 12.5, 20, 30, 50, 100, 0.5, 14.25)
  for ($i = 0; $i -lt $widths.Count; $i++) { $s1.Cells.Item(1, $i + 1).EntireColumn.ColumnWidth = [double]$widths[$i] }
  $s1.Cells.Item(1, 11).EntireColumn.Hidden = $true
  $s1.Cells.Item(1, 1).Value2 = 'Item'
  $s1.Cells.Item(1, 2).Value2 = 'Qty'
  $s1.Cells.Item(2, 1).Value2 = 'tall'
  $s1.Cells.Item(2, 1).Font.Size = 28
  $s1.Cells.Item(3, 2).Value2 = 12.5
  $s1.Cells.Item(4, 3).Value2 = 'wrapped text that needs several lines in a narrow cell'
  $s1.Cells.Item(4, 3).WrapText = $true
  $s1.Cells.Item(5, 1).EntireRow.RowHeight = 30
  $s1.Cells.Item(6, 1).EntireRow.RowHeight = 7.5
  $s1.Cells.Item(7, 1).EntireRow.RowHeight = 40.5
  $s1.Cells.Item(8, 1).EntireRow.Hidden = $true
  $s1.Cells.Item(9, 1).EntireRow.RowHeight = 15
  $s1.Cells.Item(10, 1).EntireRow.RowHeight = 0.75
  $s1.Range('A12:C12').Merge()
  $s1.Cells.Item(12, 1).Value2 = 'merged across'
  $s1.Range('E12:E14').Merge()
  $s1.Range('G13:H14').Merge()
  $s1.Activate()
  $app.ActiveWindow.FreezePanes = $false
  $app.ActiveWindow.ScrollRow = 1
  $app.ActiveWindow.ScrollColumn = 1
  $s1.Range('C3').Select()
  $app.ActiveWindow.FreezePanes = $true
  # Sheet 2: a changed default column width and default row height, nothing else.
  $s2.StandardWidth = 12
  $s2.Cells.RowHeight = 20.25
  $s2.Cells.Item(1, 1).Value2 = 'default sizes'
  $s2.Range('B2:C2').Merge()
  $s2.Activate()
  $app.ActiveWindow.FreezePanes = $false
  $s2.Range('B1').Select()
  $app.ActiveWindow.FreezePanes = $true
  $s1.Activate()
  $s1.Range('A1').Select()
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
  $wb.SaveAs($path, 51)
  $wb.Close($false)
  $toml = [IO.Path]::ChangeExtension($path, '.toml')
  $made = "Microsoft Excel $($app.Build) (Windows)"
  $lines = @(
    'feature = "grid-geometry"',
    ('made_with = ' + (Json-Str $made)),
    'author = "project member"',
    'expect_dropped = []',
    'notes = "custom, hidden and default column widths; custom, hidden and auto-grown row heights; merges; frozen panes"'
  )
  [IO.File]::WriteAllText($toml, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
}

function Read-Geometry($app, [string]$path) {
  $wb = $app.Workbooks.Open($path, 0, $true)
  try {
    $sheets = @()
    foreach ($ws in $wb.Worksheets) {
      $used = $ws.UsedRange
      $rowsN = [Math]::Max($used.Row + $used.Rows.Count - 1 + 3, 12)
      $colsN = [Math]::Max($used.Column + $used.Columns.Count - 1 + 3, 12)
      $cols = for ($c = 1; $c -le $colsN; $c++) { Num $ws.Cells.Item(1, $c).Width }
      $rows = for ($r = 1; $r -le $rowsN; $r++) { Num $ws.Cells.Item($r, 1).Height }
      $sheets += ('    {"name": ' + (Json-Str $ws.Name) + ', "col_widths_pt": [' + ($cols -join ', ') + '], "row_heights_pt": [' + ($rows -join ', ') + ']}')
    }
    return $sheets
  }
  finally {
    $wb.Saved = $true
    $wb.Close($false)
  }
}

try {
  Start-Excel
  if ($MakeFixture) { Write-Fixture $script:xl (Join-Path $fixtureDir 'grid-geometry.xlsx') }
  $files = Get-ChildItem -LiteralPath $fixtureDir -Filter '*.xlsx' | Sort-Object Name
  $count = 0
  foreach ($f in $files) {
    $name = [IO.Path]::GetFileNameWithoutExtension($f.Name)
    if ($Only -and $name -notmatch $Only) { continue }
    $sheets = Read-Geometry $script:xl $f.FullName
    $lines = @(
      '{',
      ('  "source": "fixtures/xlsx/' + $f.Name + '",'),
      '  "oracle": "Microsoft Excel via COM, Range.Width / Range.Height in points at 100% zoom",',
      ('  "dpi": ' + $script:dpi + ','),
      ('  "excel_build": ' + (Json-Str ([string]$script:xl.Build)) + ','),
      '  "sheets": ['
    )
    $lines += ($sheets -join ",`n")
    $lines += '  ]'
    $lines += '}'
    $outDir = Join-Path $refDir $name
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    [IO.File]::WriteAllText((Join-Path $outDir 'geometry.json'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
    $count++
    Write-Host "grid-geometry-oracle: $name"
  }
  Write-Host "grid-geometry-oracle: wrote $count geometry.json files"
}
finally {
  Stop-Excel
}
