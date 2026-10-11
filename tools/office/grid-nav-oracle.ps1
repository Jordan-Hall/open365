<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365 grid keyboard navigation.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this
script is never run in CI. CI consumes the committed output (fixtures/reference/xlsx/grid-nav/nav.json)
through crates/ui-egui/tests/grid_nav.rs.

Ctrl+Arrow is Range.End(direction) from the active cell (Microsoft documents End as the same as pressing
END+ARROW, and Ctrl+Arrow is the same jump). For every start cell in rows 1..60 and columns 1..13 of the
fixture's "Nav" sheet, and for a few cells on the sheet edges, it records the row Ctrl+Up and Ctrl+Down
reach and the column Ctrl+Left and Ctrl+Right reach. It also records Ctrl+End (the last used cell).
Enter, Tab and Page keys cannot be sent to a hidden COM instance (SendKeys has no effect there), so their
rules are not measured here; grid_nav.rs documents the rules it assumes for them.

-MakeFixture first (re)creates fixtures/xlsx/grid-nav.xlsx and its .toml through COM: blocks of values
with gaps, hidden rows and columns inside and at the end of blocks, a merged cell, a formatted-only
cell, a formula returning "", and a frozen-pane sheet. No personal data.

  pwsh -NoProfile -File tools/office/grid-nav-oracle.ps1 -MakeFixture

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
$refDir = Join-Path $root 'fixtures/reference/xlsx/grid-nav'
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
  $app.ScreenUpdating = $false
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
  return '"' + ($s -replace '\\', '\\' -replace '"', '\"') + '"'
}

function Fill($ws, [int]$col, [int[]]$rows) {
  foreach ($r in $rows) { $ws.Cells.Item($r, $col).Value2 = [double]($r * 100 + $col) }
}

function Write-Fixture($app, [string]$path) {
  $wb = $app.Workbooks.Add()
  while ($wb.Worksheets.Count -lt 2) { [void]$wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
  $s = $wb.Worksheets.Item(1); $s.Name = 'Nav'
  $f = $wb.Worksheets.Item(2); $f.Name = 'Frozen'
  # Column A: blocks, gaps, hidden rows inside (14-15) and at the end (22) of a block.
  Fill $s 1 @(1, 2, 3, 6, 7, 8, 10, 12, 13, 14, 15, 16, 18, 20, 21, 22, 25, 33, 41, 52)
  $s.Rows.Item(14).Hidden = $true; $s.Rows.Item(15).Hidden = $true
  $s.Rows.Item(17).Hidden = $true
  $s.Rows.Item(22).Hidden = $true
  $s.Rows.Item(25).Hidden = $true
  # A merged cell A30:A32, a formatted-only cell A40, a formula returning "" in A50.
  $s.Range('A30:A32').Merge(); $s.Cells.Item(30, 1).Value2 = 'merged'
  $s.Cells.Item(40, 1).Interior.Color = 255
  $s.Cells.Item(50, 1).Formula = '=""'
  # Row 2: A..D, G, I..L; column H (empty) and K (inside a block) hidden.
  foreach ($c in 2, 3, 4, 7, 9, 10, 11, 12) { $s.Cells.Item(2, $c).Value2 = [double](200 + $c) }
  $s.Columns.Item(8).Hidden = $true
  $s.Columns.Item(11).Hidden = $true
  # Lone cells and a second block lower down for column C and E.
  Fill $s 3 @(5, 6, 7, 30)
  Fill $s 5 @(1, 59)
  Fill $s 13 @(60)
  # Frozen sheet: a full block with panes frozen below row 2 and right of column 2.
  for ($r = 1; $r -le 10; $r++) { for ($c = 1; $c -le 6; $c++) { $f.Cells.Item($r, $c).Value2 = [double]($r * 10 + $c) } }
  $f.Activate()
  $app.ActiveWindow.FreezePanes = $false
  $f.Range('C3').Select()
  $app.ActiveWindow.FreezePanes = $true
  $s.Activate()
  $s.Range('A1').Select()
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
  $wb.SaveAs($path, 51)
  $wb.Close($false)
  $toml = [IO.Path]::ChangeExtension($path, '.toml')
  $made = "Microsoft Excel $($app.Build) (Windows)"
  $lines = @(
    'feature = "grid-nav"',
    ('made_with = ' + (Json-Str $made)),
    'author = "project member"',
    'expect_dropped = []',
    'notes = "blocks of values with gaps, hidden rows and columns, a merged cell, a formatted-only cell, a formula returning an empty string; frozen panes"'
  )
  [IO.File]::WriteAllText($toml, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
}

function Read-Nav($app, [string]$path) {
  $wb = $app.Workbooks.Open($path, 0, $true)
  try {
    $ws = $wb.Worksheets.Item('Nav')
    $starts = New-Object System.Collections.Generic.List[object]
    for ($r = 1; $r -le 60; $r++) { for ($c = 1; $c -le 13; $c++) { $starts.Add(@($r, $c)) } }
    foreach ($e in @(@(1048576, 1), @(1048576, 5), @(1, 16384), @(30, 16384), @(1048575, 1), @(1, 16383), @(59, 5), @(60, 13))) { $starts.Add($e) }
    $rows = @()
    foreach ($st in $starts) {
      $r = [int]$st[0]; $c = [int]$st[1]
      $cell = $ws.Cells.Item($r, $c)
      $up = $cell.End(-4162).Row
      $down = $cell.End(-4121).Row
      $left = $cell.End(-4159).Column
      $right = $cell.End(-4161).Column
      $rows += ('      "' + $r + ',' + $c + '": [' + $up + ', ' + $down + ', ' + $left + ', ' + $right + ']')
    }
    $last = $ws.Cells.SpecialCells(11)
    $fr = $wb.Worksheets.Item('Frozen')
    $flast = $fr.Cells.SpecialCells(11)
    $lines = @(
      '{',
      '  "source": "fixtures/xlsx/grid-nav.xlsx",',
      '  "oracle": "Microsoft Excel via COM, Range.End(xlUp/xlDown/xlToLeft/xlToRight) from each start cell",',
      ('  "excel_build": ' + (Json-Str ([string]$app.Build)) + ','),
      '  "format": "\"row,col\": [up_row, down_row, left_col, right_col]",',
      ('  "last_cell": [' + $last.Row + ', ' + $last.Column + '],'),
      ('  "frozen_last_cell": [' + $flast.Row + ', ' + $flast.Column + '],'),
      '  "ends": {',
      ($rows -join ",`n"),
      '  }',
      '}'
    )
    return $lines
  }
  finally {
    $wb.Saved = $true
    $wb.Close($false)
  }
}

try {
  Start-Excel
  $path = Join-Path $fixtureDir 'grid-nav.xlsx'
  if ($MakeFixture) { Write-Fixture $script:xl $path }
  $lines = Read-Nav $script:xl $path
  New-Item -ItemType Directory -Force -Path $refDir | Out-Null
  [IO.File]::WriteAllText((Join-Path $refDir 'nav.json'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  Write-Host 'grid-nav-oracle: wrote nav.json'
}
finally {
  Stop-Excel
}
