<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365 worksheet drawing placement.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this
script is never run in CI. CI consumes the committed output (fixtures/reference/xlsx/<name>/drawing.json)
through crates/xlsx/tests/drawing_oracle.rs.

For every fixtures/xlsx/drawing-*.xlsx it opens the workbook read-only (never saved) and records, per worksheet
and per shape in z order: name, Type, Placement, Visible, Left, Top, Width, Height (points, as Excel returns
them at 100 % zoom), TopLeftCell and BottomRightCell (A1 addresses), plus the dpi of the run. Column widths and
row heights come from grid-geometry-oracle.ps1 (geometry.json), values from office-oracle.ps1.

  pwsh -NoProfile -File tools/office/drawing-oracle.ps1                 # all drawing-* fixtures
  pwsh -NoProfile -File tools/office/drawing-oracle.ps1 -Only '^drawing-hidden'

Process safety: starts its own hidden Excel instance, records its process id and only ever stops that
process (in a finally block). Other EXCEL and WINWORD processes are never touched.

Output is stable: LF line endings, ASCII only, no absolute paths and no user information.
#>
param([string]$Only)
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
  $app.AutomationSecurity = 3
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  $script:xl = $app
  $script:xlPid = [int]$procId
  $script:dpi = [int][Native.Win32]::GetDpiForWindow([IntPtr]$app.Hwnd)
  Write-Output "Excel PID $procId (started by this script)"
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

function Num($v) { return ([double]$v).ToString('R', $inv) }
function Esc($s) { return ([string]$s).Replace('\', '\\').Replace('"', '\"') }

function Cell-Addr($r) {
  if ($null -eq $r) { return '' }
  return [string]$r.Address($false, $false)
}

function Probe($path, $name) {
  $wb = $script:xl.Workbooks.Open($path, 0, $true)
  $sheets = @()
  foreach ($ws in $wb.Worksheets) {
    $items = @()
    foreach ($s in $ws.Shapes) {
      $items += ('      {{"name": "{0}", "type": {1}, "placement": {2}, "visible": {3}, "left": {4}, "top": {5}, "width": {6}, "height": {7}, "topLeftCell": "{8}", "bottomRightCell": "{9}"}}' -f
        (Esc $s.Name), [int]$s.Type, [int]$s.Placement, ([string]([bool]$s.Visible)).ToLower(),
        (Num $s.Left), (Num $s.Top), (Num $s.Width), (Num $s.Height),
        (Cell-Addr $s.TopLeftCell), (Cell-Addr $s.BottomRightCell))
    }
    $sheets += '    {"name": "' + (Esc $ws.Name) + '", "shapes": [' + "`n" + ($items -join ",`n") + "`n    ]}"
  }
  $wb.Close($false)
  [void][Runtime.InteropServices.Marshal]::ReleaseComObject($wb)
  $json = "{`n  `"dpi`": $($script:dpi),`n  `"sheets`": [`n" + ($sheets -join ",`n") + "`n  ]`n}`n"
  $dir = Join-Path $refDir $name
  New-Item -ItemType Directory -Force $dir | Out-Null
  [IO.File]::WriteAllText((Join-Path $dir 'drawing.json'), $json, (New-Object Text.UTF8Encoding($false)))
  Write-Output "wrote $name/drawing.json"
}

try {
  Start-Excel
  foreach ($f in Get-ChildItem $fixtureDir -Filter 'drawing-*.xlsx' | Sort-Object Name) {
    $name = $f.BaseName
    if ($Only -and $name -notmatch $Only) { continue }
    Probe $f.FullName $name
  }
} finally {
  Stop-Excel
}
