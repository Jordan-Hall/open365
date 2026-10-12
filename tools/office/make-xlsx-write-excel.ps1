<#
Builds, in the locally installed Microsoft Excel (COM), the same objects the Open365 xlsx write API creates, and
saves them under fixtures/reference/xlsx/write-excel/ so the XML Excel itself writes can be compared with ours
(crates/xlsx/examples/write_ops.rs):

  databar-default / databar-custom   Data bars (Excel 2010 x14 extension)
  timeperiod-<name>                  one conditional format per date-occurring period
  hyperlinks                         URL, place in this document, e-mail (display attribute)
  table-noheader / table-header      Insert > Table over data without / with a header row

  pwsh -NoProfile -File tools/office/make-xlsx-write-excel.ps1

LOCAL DEVELOPER TOOL ONLY (Microsoft does not support unattended server-side Office automation).
Starts its own Excel instance (macros force-disabled), never attaches to or kills another one, quits it in a
finally block and stops only the EXCEL process it started (its PID is printed). The workbooks hold generic demo
data and no personal information; the author is blanked.
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$out = Join-Path $root 'fixtures\reference\xlsx\write-excel'
New-Item -ItemType Directory -Force $out | Out-Null
function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }

$before = Get-Pids 'EXCEL'
$xl = $null; $xlPids = @()
try {
  $xl = New-Object -ComObject Excel.Application
  $xlPids = @(Get-Pids 'EXCEL' | Where-Object { $_ -notin $before })
  if ($xlPids.Count -eq 0) { throw 'COM attached to an Excel instance this script did not start; refusing to touch it' }
  Write-Output "excel pid(s) started by this script: $($xlPids -join ', ')"
  $xl.Visible = $false; $xl.DisplayAlerts = $false
  $xl.AutomationSecurity = 3

  function New-Probe([string]$name, [scriptblock]$body) {
    $wb = $xl.Workbooks.Add()
    try {
      $ws = $wb.Worksheets.Item(1)
      & $body $ws
      $file = Join-Path $out "$name.xlsx"
      if (Test-Path $file) { Remove-Item $file }
      $wb.SaveAs($file, 51)
      Write-Output "saved $name"
    } finally { $wb.Close($false); Release $wb }
  }
  function Block($ws, [bool]$header) {
    $r = 1
    if ($header) { $ws.Range('F1').Value2 = 'Item'; $ws.Range('G1').Value2 = 'Qty'; $ws.Range('H1').Value2 = 'Price'; $r = 2 }
    $rows = @(@('Apple', 3, 1.5), @('Pear', 5, 2.25), @('Plum', 8, 0.75))
    foreach ($row in $rows) { $ws.Cells.Item($r, 6).Value2 = $row[0]; $ws.Cells.Item($r, 7).Value2 = [double]$row[1]; $ws.Cells.Item($r, 8).Value2 = [double]$row[2]; $r++ }
  }

  New-Probe 'databar-default' { param($ws) Block $ws $true; [void]$ws.Range('G2:H4').FormatConditions.AddDatabar() }
  New-Probe 'databar-custom' {
    param($ws) Block $ws $true
    $db = $ws.Range('G2:H4').FormatConditions.AddDatabar()
    $db.BarFillType = 0              # xlDataBarFillSolid
    $db.BarBorder.Type = 1           # xlDataBarBorderSolid
    $db.BarColor.Color = 0xC68E63    # BGR of RGB(63,8E,C6)
    $db.NegativeBarFormat.ColorType = 1   # xlDataBarColor
    $db.AxisPosition = 0             # xlDataBarAxisAutomatic
    $db.MinPoint.Modify(1)           # xlConditionValueLowestValue
    $db.MaxPoint.Modify(2)           # xlConditionValueHighestValue
  }
  $periods = [ordered]@{ today = 0; yesterday = 1; last7 = 2; thisweek = 3; lastweek = 4; lastmonth = 5; tomorrow = 6; nextweek = 7; nextmonth = 8; thismonth = 9 }
  foreach ($k in $periods.Keys) {
    $v = $periods[$k]
    $sb = {
      param($ws) Block $ws $true
      $fcs = $ws.Range('G2:H4').FormatConditions
      $fc = $fcs.GetType().InvokeMember('Add', [Reflection.BindingFlags]::InvokeMethod, $null, $fcs, [object[]]@(11, $v), $null, $null, [string[]]@('Type', 'DateOperator'))
      $fc.Interior.Color = 0xCEC7FF
    }.GetNewClosure()
    New-Probe "timeperiod-$k" $sb
  }
  New-Probe 'hyperlinks' {
    param($ws)
    [void]$ws.Hyperlinks.Add($ws.Range('F1'), 'https://example.com/a?x=1&y=2')
    [void]$ws.Hyperlinks.Add($ws.Range('F2'), '', 'Sheet1!B2')
    [void]$ws.Hyperlinks.Add($ws.Range('F3'), 'mailto:a@example.com?subject=Hi there', [Type]::Missing, [Type]::Missing, 'mail')
  }
  New-Probe 'hyperlinks-text' {
    param($ws)
    $m = [Type]::Missing
    [void]$ws.Hyperlinks.Add($ws.Range('F1'), '', 'Sheet1!B2', $m, 'go')
    [void]$ws.Hyperlinks.Add($ws.Range('F2'), 'https://example.com/b', $m, 'a tip', $m)
    $ws.Range('F3').Value2 = 'Apple'; [void]$ws.Hyperlinks.Add($ws.Range('F3'), 'https://example.com/c')
    $ws.Range('F4').Value2 = [double]5; [void]$ws.Hyperlinks.Add($ws.Range('F4'), 'https://example.com/d')
    [void]$ws.Hyperlinks.Add($ws.Range('F5'), '', 'Sheet1!B2', 'loc tip', $m)
    [void]$ws.Hyperlinks.Add($ws.Range('F6'), 'https://example.com/e', $m, 'tip e', 'text e')
    $ws.Range('F7').Value2 = 'Pear'; [void]$ws.Hyperlinks.Add($ws.Range('F7'), 'https://example.com/f', $m, $m, 'zz')
    $ws.Range('F8').Value2 = 'Plum'; [void]$ws.Hyperlinks.Add($ws.Range('F8'), '', 'Sheet1!B2')
    $ws.Range('F9:F10').Value2 = [double]1; [void]$ws.Hyperlinks.Add($ws.Range('F9:F10'), '', 'Sheet1!B3')
    [void]$ws.Hyperlinks.Add($ws.Range('F11:F12'), 'https://example.com/g')
  }
  New-Probe 'table-noheader' {
    param($ws) Block $ws $false
    [void]$ws.ListObjects.Add(1, $ws.Range('F1:H3'), [Type]::Missing, 2)
  }
  New-Probe 'table-header' {
    param($ws) Block $ws $true
    [void]$ws.ListObjects.Add(1, $ws.Range('F1:H4'), [Type]::Missing, 1)
  }
} finally {
  if ($xl) { try { $xl.Quit() } catch {}; Release $xl }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  Start-Sleep -Milliseconds 500
  foreach ($id in $xlPids) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
}
