<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365 data bars: where the axis sits,
which way negative bars grow and how long every bar is.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed output (fixtures/reference/xlsx/grid-databar/databars.json) through
crates/ui-egui/tests/grid_databar.rs.

-MakeFixture (re)creates fixtures/xlsx/grid-databar.xlsx and its .toml through COM: one data bar rule per column
over eight numbers, covering all-positive, mixed-sign and all-negative ranges, the three axis positions, a
distinct negative colour, right-to-left bars, solid bars and fixed and automatic minimum and maximum. Excel writes
the file; docProps/core.xml dc:creator and cp:lastModifiedBy are scrubbed. Nothing is hand-edited.

The oracle opens the workbook read-only (never saved) in its own visible Excel window at 100 percent zoom, switches
each rule to solid fill with no border for the measurement only, renders the window with PrintWindow (no clipboard)
and reads, for every value, the pixel runs of the bar across the cell at a fixed height: where the bar starts and
ends relative to the cell's grid lines, and its colour. It also records Excel's own DataBar properties per rule.

  pwsh -NoProfile -STA -File tools/office/grid-databar-oracle.ps1                 # record databars.json
  pwsh -NoProfile -STA -File tools/office/grid-databar-oracle.ps1 -MakeFixture    # rebuild the fixture first

Process safety: starts its own Excel instance with AutomationSecurity = 3 (disable macros), remembers its process id
and only ever stops that process (in a finally block). Other EXCEL processes are never touched.
#>
param(
  [switch]$MakeFixture
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class DbWin {
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
'@

# Physical pixels everywhere: window rectangles and PointsToScreenPixels agree with the rendered bitmap.
[void][DbWin]::SetProcessDpiAwarenessContext([IntPtr](-4))

$script:xl = $null
$script:xlPid = 0

function Start-Excel {
  $app = New-Object -ComObject Excel.Application
  $app.AutomationSecurity = 3
  $app.Visible = $true
  $app.DisplayAlerts = $false
  $app.EnableEvents = $false
  $procId = [uint32]0
  [void][DbWin]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
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

# axis: 0 automatic, 1 midpoint, 2 none. dir: -5002 context, -5003 left to right, -5004 right to left.
# min/max: @(type, value); types 1 lowest, 2 highest, 0 number, 6 automatic min, 7 automatic max.
$mixed = @(-50, -25, 0, 25, 50, 75, 100, 10)
$cases = @(
  @{ name = 'positive automatic'; values = @(10, 20, 30, 40, 50, 60, 70, 80); axis = 0 },
  @{ name = 'mixed automatic'; values = $mixed; axis = 0 },
  @{ name = 'mostly negative automatic'; values = @(-100, -80, -40, -10, 5, 20, -60, 0); axis = 0 },
  @{ name = 'all negative automatic'; values = @(-10, -20, -30, -40, -50, -60, -70, -80); axis = 0 },
  @{ name = 'mixed midpoint'; values = $mixed; axis = 1 },
  @{ name = 'mixed no axis'; values = $mixed; axis = 2 },
  @{ name = 'mixed red negative'; values = $mixed; axis = 0; negative = (Rgb 255 0 0) },
  @{ name = 'mixed right to left'; values = $mixed; axis = 0; dir = -5004 },
  @{ name = 'mixed solid'; values = $mixed; axis = 0; solid = $true },
  @{ name = 'mixed fixed -50 to 50'; values = @(-80, -50, -25, 0, 25, 50, 80, 10); axis = 0; min = @(0, -50); max = @(0, 50) },
  @{ name = 'positive fixed 20 to 60'; values = @(10, 20, 30, 40, 50, 60, 70, 80); axis = 0; min = @(0, 20); max = @(0, 60) },
  @{ name = 'tiny negative'; values = @(-1, 100, 50, 25, 0, 75, 10, 90); axis = 0 },
  @{ name = 'positive automatic min'; values = @(10, 20, 30, 40, 50, 60, 70, 80); axis = 0; min = @(6, 0); max = @(7, 0) },
  @{ name = 'mixed automatic min'; values = $mixed; axis = 0; min = @(6, 0); max = @(7, 0) },
  @{ name = 'all negative automatic min'; values = @(-10, -20, -30, -40, -50, -60, -70, -80); axis = 0; min = @(6, 0); max = @(7, 0) },
  @{ name = 'positive midpoint'; values = @(10, 20, 30, 40, 50, 60, 70, 80); axis = 1 },
  @{ name = 'all negative midpoint'; values = @(-10, -20, -30, -40, -50, -60, -70, -80); axis = 1 }
)
$perBlock = 7
function Title-Row([int]$i) { return [int](1 + 11 * [math]::Floor($i / $perBlock)) }
function Case-Col([int]$i) { return 1 + ($i % $perBlock) }

function Write-Fixture($app, [string]$path) {
  $wb = $app.Workbooks.Add()
  $ws = $wb.Worksheets.Item(1)
  $ws.Range('A:G').ColumnWidth = 16
  for ($i = 0; $i -lt $cases.Count; $i++) {
    $k = $cases[$i]
    $t = Title-Row $i; $c = Case-Col $i
    $ws.Cells.Item($t, $c).Value2 = $k.name
    for ($j = 0; $j -lt $k.values.Count; $j++) { $ws.Cells.Item($t + 1 + $j, $c).Value2 = [double]$k.values[$j] }
    $rng = $ws.Range($ws.Cells.Item($t + 1, $c), $ws.Cells.Item($t + $k.values.Count, $c))
    $fc = $rng.FormatConditions.AddDatabar()
    if ($k.min) { $fc.MinPoint.Modify([int]$k.min[0], [double]$k.min[1]) }
    if ($k.max) { $fc.MaxPoint.Modify([int]$k.max[0], [double]$k.max[1]) }
    $fc.AxisPosition = [int]$k.axis
    if ($k.dir) { $fc.Direction = [int]$k.dir }
    if ($k.negative) {
      $fc.NegativeBarFormat.ColorType = 0
      $fc.NegativeBarFormat.Color.Color = [int]$k.negative
    } else {
      $fc.NegativeBarFormat.ColorType = 1
    }
    if ($k.solid) { $fc.BarFillType = 0 }
  }
  $wb.SaveAs($path, 51)
  $wb.Close($false)
  Scrub-Core $path
  $toml = [IO.Path]::ChangeExtension($path, '.toml')
  $made = "Microsoft Excel $($app.Build) (Windows)"
  $lines = @(
    'feature = "grid-databar"',
    ('made_with = "' + $made + '"'),
    'author = "project member"',
    'expect_dropped = []',
    'notes = "one data bar rule per column over eight numbers: all-positive, mixed-sign and all-negative ranges, the three axis positions, a distinct negative colour, right-to-left bars, solid bars, fixed and automatic minimum and maximum"'
  )
  [IO.File]::WriteAllText($toml, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  Write-Host "grid-databar-oracle: wrote $path"
}

function Dist($a, $b) { return [math]::Abs($a.R - $b.R) + [math]::Abs($a.G - $b.G) + [math]::Abs($a.B - $b.B) }
function Hex($p) { return ('{0:X2}{1:X2}{2:X2}' -f $p.R, $p.G, $p.B) }

function Measure-Rules($app, [string]$path) {
  # A scratch copy opened writable so the rules can be switched to solid; it is never saved.
  $scratch = Join-Path $env:TEMP 'grid-databar-scratch.xlsx'
  Copy-Item -LiteralPath $path -Destination $scratch -Force
  $wb = $app.Workbooks.Open($scratch)
  try {
    $ws = $wb.Worksheets.Item(1)
    $ws.Activate()
    $app.WindowState = -4137
    $app.ActiveWindow.Zoom = 100
    $win = $app.ActiveWindow
    $h = [IntPtr]$app.Hwnd
    $out = @()
    for ($i = 0; $i -lt $cases.Count; $i++) {
      $k = $cases[$i]
      $t = Title-Row $i; $c = Case-Col $i
      $n = $k.values.Count
      $rng = $ws.Range($ws.Cells.Item($t + 1, $c), $ws.Cells.Item($t + $n, $c))
      $fc = $rng.FormatConditions.Item(1)
      $props = [ordered]@{
        axis_position = [int]$fc.AxisPosition; bar_fill_type = [int]$fc.BarFillType; direction = [int]$fc.Direction
        negative_color_type = [int]$fc.NegativeBarFormat.ColorType
        bar_color = [int]$fc.BarColor.Color; negative_color = [int]$fc.NegativeBarFormat.Color.Color
        axis_color = [int]$fc.AxisColor.Color
        min_type = [int]$fc.MinPoint.Type; min_value = $fc.MinPoint.Value; max_type = [int]$fc.MaxPoint.Type; max_value = $fc.MaxPoint.Value
      }
      # Solid fill without a border for the measurement; the file on disk is never saved.
      $fc.BarFillType = 0
      try { $fc.BarBorder.Type = 0 } catch {}
      $app.Calculate()
      Start-Sleep -Milliseconds 400
      $r = New-Object DbWin+RECT; [void][DbWin]::GetWindowRect($h, [ref]$r)
      $bmp = New-Object Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
      $g = [Drawing.Graphics]::FromImage($bmp); $dc = $g.GetHdc()
      [void][DbWin]::PrintWindow($h, $dc, 2); $g.ReleaseHdc($dc); $g.Dispose()
      # Geometry comes from the image itself: the grid lines on a blank row (vertical) and a blank column
      # (horizontal). COM point-to-pixel conversions disagree with the rendered bitmap on scaled displays.
      $white = [Drawing.Color]::FromArgb(255, 255, 255)
      $isLine = { param($p) ((Dist $p $white) -gt 20) -and $p.R -gt 150 -and [math]::Abs($p.R - $p.G) -lt 6 }
      $vlines = @(); $prevLine = $false
      for ($x = 44; $x -lt $bmp.Width - 60; $x++) {
        $on = & $isLine ($bmp.GetPixel($x, 1000))
        if ($on -and -not $prevLine) { $vlines += $x }
        $prevLine = $on
      }
      $x1probe = $bmp.Width - 330
      $hlines = @(); $prevLine = $false
      for ($y = 229; $y -lt 1070; $y++) {
        $on = & $isLine ($bmp.GetPixel($x1probe, $y))
        if ($on -and -not $prevLine) { $hlines += $y }
        $prevLine = $on
      }
      $colW = $vlines[1] - $vlines[0]
      # Boundary k lies between column k and column k+1; column 1 has no recorded left line.
      $gl = if ($c -eq 1) { $vlines[0] - $colW } else { $vlines[$c - 2] }
      $gr = $vlines[$c - 1]
      $bg = $white
      $rows = @()
      for ($j = 0; $j -lt $n; $j++) {
        $rowNo = $t + 1 + $j
        $y = $hlines[$rowNo - 2] + 4
        $runs = @(); $start = -1; $first = $null; $last = $null
        for ($x = $gl + 1; $x -lt $gr; $x++) {
          $px = $bmp.GetPixel($x, $y)
          if ((Dist $px $bg) -gt 24) {
            if ($start -lt 0) { $start = $x; $first = $px }
            $last = $px
          } elseif ($start -ge 0) {
            $runs += [ordered]@{ from = $start - $gl; to = $x - $gl; first = (Hex $first); last = (Hex $last) }
            $start = -1
          }
        }
        if ($start -ge 0) { $runs += [ordered]@{ from = $start - $gl; to = $gr - $gl; first = (Hex $first); last = (Hex $last) } }
        $rows += [ordered]@{ row = $rowNo; value = [double]$k.values[$j]; runs = $runs }
      }
      if ($i -eq 1) { $bmp.Save((Join-Path $env:TEMP 'grid-databar-sample.png')) }
      $bmp.Dispose()
      $out += [ordered]@{ name = $k.name; col = $c; first_row = $t + 1; rows_n = $n; cell_width_px = $gr - $gl; excel = $props; rows = $rows }
    }
    return $out
  }
  finally { $wb.Saved = $true; $wb.Close($false) }
}

try {
  Start-Excel
  $path = Join-Path $fixtureDir 'grid-databar.xlsx'
  if ($MakeFixture) { Write-Fixture $script:xl $path }
  $res = Measure-Rules $script:xl $path
  $doc = [ordered]@{
    source = 'fixtures/xlsx/grid-databar.xlsx'
    oracle = 'Microsoft Excel via COM: FormatConditions DataBar properties, and the pixel runs of each bar (solid, no border) in a PrintWindow render at 100 percent zoom; from/to are pixels from the cell left grid line'
    excel_build = [string]$script:xl.Build
    cases = $res
  }
  $outDir = Join-Path $refDir 'grid-databar'
  New-Item -ItemType Directory -Force -Path $outDir | Out-Null
  $json = ($doc | ConvertTo-Json -Depth 10)
  [IO.File]::WriteAllText((Join-Path $outDir 'databars.json'), $json + "`n", (New-Object Text.UTF8Encoding $false))
  Write-Host "grid-databar-oracle: recorded $($res.Count) rules"
}
finally {
  Stop-Excel
}
