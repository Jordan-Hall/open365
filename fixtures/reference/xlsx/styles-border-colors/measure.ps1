<#
Records what Excel does with the styles-* probe workbooks (fixtures/reference/xlsx/styles-<name>/input.xlsx):
  displayformat.json  Range.DisplayFormat of every cell of the used range plus one cell of margin: Interior, Font and the
                      seven border indexes (left, top, bottom, right, diagonal down, diagonal up) with LineStyle, Weight,
                      Color (BGR), ColorIndex and, when a theme colour, ThemeColor and TintAndShade.
  saved/              the parts Excel writes when it opens the input and saves a copy (styles.xml, sheet1.xml, tables).
The input is opened read-only from a temporary copy and never saved in place. LOCAL DEVELOPER TOOL ONLY.

  pwsh -NoProfile -File measure.ps1 -Name border-colors [-Name2 ...]

Process safety: starts its own hidden Excel, aborts if COM returns an Excel that was already running, keeps its PID and in
the finally block quits it and stops only that PID. Other EXCEL processes are never touched.
#>
param([Parameter(Mandatory)][string[]]$Name, [int]$MaxRows = 0, [int]$MaxCols = 0, [string]$RestyleColumn = '')
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$refDir = Split-Path -Parent $here
$inv = [Globalization.CultureInfo]::InvariantCulture
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@
$script:xl = $null; $script:xlPid = 0
function Start-Excel {
  $existing = @(Get-Process -Name EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $app = New-Object -ComObject Excel.Application
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  if ($existing -contains [int]$procId) { throw "COM returned the already running Excel $procId; aborting." }
  $script:xlPid = [int]$procId; $script:xl = $app
  $app.Visible = $false; $app.DisplayAlerts = $false; $app.ScreenUpdating = $false; $app.EnableEvents = $false
  $app.AutomationSecurity = 3
  Write-Host "excel pid $($script:xlPid)"
}
function Stop-Excel {
  $app = $script:xl; $script:xl = $null
  if ($app) { try { $app.Quit() } catch {} ; try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {} }
  if ($script:xlPid -gt 0) {
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Date) -lt $deadline -and (Get-Process -Id $script:xlPid -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
    $p = Get-Process -Id $script:xlPid -ErrorAction SilentlyContinue
    if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $script:xlPid -Force -ErrorAction SilentlyContinue }
    $script:xlPid = 0
  }
}
function Edge($d, $i) {
  $b = $d.Borders.Item($i)
  $o = [ordered]@{ ls = [int]$b.LineStyle }
  if ($o.ls -ne -4142) {
    $o.w = [int]$b.Weight; $o.c = [int]$b.Color; $o.ci = [int]$b.ColorIndex
    try { $t = $b.ThemeColor; if ($null -ne $t) { $o.tc = [int]$t; $o.ts = [double]$b.TintAndShade } } catch {}
  }
  return $o
}
function Export-Parts($file, $out) {
  if (Test-Path $out) { Remove-Item -Recurse -Force $out }
  New-Item -ItemType Directory -Path $out | Out-Null
  $zip = [IO.Compression.ZipFile]::OpenRead($file)
  try {
    foreach ($e in $zip.Entries) {
      if ($e.FullName -match '^xl/(styles\.xml|worksheets/sheet\d+\.xml|tables/table\d+\.xml)$') {
        $dest = Join-Path $out ($e.FullName -replace '/', '_')
        [IO.Compression.ZipFileExtensions]::ExtractToFile($e, $dest, $true)
      }
    }
  } finally { $zip.Dispose() }
}
function Measure-One($xl, $dir) {
  $input = Join-Path $dir 'input.xlsx'
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ('open365-styles-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
  New-Item -ItemType Directory -Path $tmp | Out-Null
  $copy = Join-Path $tmp 'input.xlsx'
  Copy-Item -LiteralPath $input -Destination $copy
  $wb = $xl.Workbooks.Open($copy, 0, $true)
  try {
    $ws = $wb.Worksheets.Item(1)
    $ur = $ws.UsedRange
    $r0 = [Math]::Max(1, $ur.Row - 1); $c0 = [Math]::Max(1, $ur.Column - 1)
    $r1 = $ur.Row + $ur.Rows.Count; $c1 = $ur.Column + $ur.Columns.Count
    if ($MaxRows -gt 0) { $r1 = [Math]::Min($r1, $MaxRows) }
    if ($MaxCols -gt 0) { $c1 = [Math]::Min($c1, $MaxCols) }
    $cells = [ordered]@{}
    for ($r = $r0; $r -le $r1; $r++) {
      for ($c = $c0; $c -le $c1; $c++) {
        $cell = $ws.Cells.Item($r, $c); $d = $cell.DisplayFormat
        $o = [ordered]@{
          fill = [int]$d.Interior.Color; pat = [int]$d.Interior.Pattern; font = [int]$d.Font.Color
          b = [bool]$d.Font.Bold; i = [bool]$d.Font.Italic
          left = Edge $d 7; top = Edge $d 8; bottom = Edge $d 9; right = Edge $d 10; dd = Edge $d 5; du = Edge $d 6
        }
        $cells[$cell.Address($false, $false)] = $o
      }
    }
    $doc = [ordered]@{ oracle = 'Microsoft Excel COM Range.DisplayFormat'; build = [string]$xl.Build; cells = $cells }
    $json = $doc | ConvertTo-Json -Depth 8 -Compress
    [IO.File]::WriteAllText((Join-Path $dir 'displayformat.json'), $json + "`n", (New-Object Text.UTF8Encoding $false))
    $saved = Join-Path $tmp 'saved.xlsx'
    $wb.SaveCopyAs($saved)
    Export-Parts $saved (Join-Path $dir 'saved')
    if ($RestyleColumn -ne '') {
      # Make the left edge of every bordered cell of the column thick, as the border toolbar would.
      $ci = [int]$RestyleColumn
      for ($r = $r0; $r -le $r1; $r++) {
        $cell = $ws.Cells.Item($r, $ci)
        if ([int]$cell.Borders.Item(7).LineStyle -ne -4142) { $cell.Borders.Item(7).Weight = 4 }
      }
      $saved2 = Join-Path $tmp 'saved2.xlsx'
      $wb.SaveCopyAs($saved2)
      Export-Parts $saved2 (Join-Path $dir 'saved-restyle')
    }
  } finally { $wb.Close($false) }
  Remove-Item -Recurse -Force $tmp
}
try {
  Start-Excel
  foreach ($n in $Name) { Measure-One $script:xl (Join-Path $refDir ('styles-' + $n)); Write-Host "measured $n" }
} finally { Stop-Excel }
