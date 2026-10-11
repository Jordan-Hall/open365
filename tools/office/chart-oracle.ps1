<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365 chart rendering: automatic
value-axis scaling, plot-area rectangles and legend rectangles.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed output through crates/chart/tests.

Modes (any combination, default is all four):
  -Fixtures  (re)creates fixtures/xlsx/chart-*.xlsx and their .toml manifests through COM. Excel writes the
             files; nothing is hand-edited. Afterwards regenerate the references the crates/xlsx tests require:
               pwsh -NoProfile -File tools/office/grid-geometry-oracle.ps1 -Only '^chart-'
               pwsh -NoProfile -File tools/office/office-oracle.ps1 -Mode reference -Path <each fixtures/xlsx/chart-*.xlsx>
  -Layout    opens every fixtures/xlsx/chart-*.xlsx read-only (never saved) and records, per chart, the chart
             area size, PlotArea.Inside{Left,Top,Width,Height}, PlotArea.{Left,Top,Width,Height},
             Legend.{Left,Top,Width,Height}, the title rectangle and the resolved value-axis scale, all in
             points as Excel reports them. Output: fixtures/reference/xlsx/<fixture>/layout.json
  -Axes      builds scratch charts (never saved) over generated data ranges and records Axes(xlValue) and
             Axes(xlCategory) MinimumScale, MaximumScale, MajorUnit and MinorUnit as Excel resolves them.
             Output: fixtures/reference/xlsx/chart-axes/axes.json

             -Seed N picks another random sweep (default 365 writes axes.json; any other seed writes
             axes-seedN.json, a hold-out set the rules were not fitted to).
  -Sweep     records the resolved value-axis scale of a (0, v) data range for v = 5..30 at chart heights
             (column) and widths (bar) from 80 to 640 pt: how many major intervals Excel allows at each plot
             size. Output: fixtures/reference/xlsx/chart-axes/sweep.json

Process safety: starts its own Excel instance with AutomationSecurity = 3 (disable macros), remembers its process id
and only ever stops that process (in a finally block). Other EXCEL processes are never touched.

Output is stable: LF line endings, ASCII only, no absolute paths and no user information. Data is synthetic.
#>
param(
  [switch]$Fixtures,
  [switch]$Layout,
  [switch]$Axes,
  [switch]$Sweep,
  [string]$Only,
  [int]$Seed = 365
)
$ErrorActionPreference = 'Stop'
if (-not ($Fixtures -or $Layout -or $Axes -or $Sweep)) { $Fixtures = $true; $Layout = $true; $Axes = $true; $Sweep = $true }
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture

Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class ChWin {
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
}
'@

$script:xl = $null
$script:xlPid = 0

function Start-Excel {
  $app = New-Object -ComObject Excel.Application
  $app.AutomationSecurity = 3
  $app.Visible = $true
  $app.DisplayAlerts = $false
  $app.EnableEvents = $false
  $procId = [uint32]0
  [void][ChWin]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
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
    if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $script:xlPid -Force -ErrorAction SilentlyContinue }
  }
  $script:xlPid = 0
}

function Num($v) {
  if ($null -eq $v) { return 'null' }
  $d = [double]$v
  if ([double]::IsNaN($d) -or [double]::IsInfinity($d)) { return 'null' }
  return $d.ToString('R', $inv)
}

function Write-Lf($path, $text) {
  New-Item -ItemType Directory -Force (Split-Path -Parent $path) | Out-Null
  [IO.File]::WriteAllText($path, ($text -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
}

function Write-Manifest($name, $notes) {
  $t = "feature = `"chart`"`nmade_with = `"Microsoft Excel $($script:xl.Build) (Windows)`"`nauthor = `"project member`"`nexpect_dropped = []`nnotes = `"$notes`"`n"
  Write-Lf (Join-Path $fixtureDir "$name.toml") $t
}

# ---------------------------------------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------------------------------------
$categories = 'North', 'South', 'East', 'West', 'Centre'
$seriesNames = 'Alpha', 'Beta', 'Gamma'
$seriesData = @(
  @(12, 19, 7, 15, 22),
  @(8, 14, 11, 9, 17),
  @(5, 6, 13, 10, 4)
)

function New-ChartFixture($name, $notes, [int]$type, [scriptblock]$setup, [string]$range = 'G2:N19') {
  if ($Only -and $name -notmatch $Only) { return }
  $path = Join-Path $fixtureDir "$name.xlsx"
  if (Test-Path $path) { Remove-Item $path -Force }
  $wb = $script:xl.Workbooks.Add()
  $ws = $wb.Worksheets.Item(1); $ws.Name = 'Data'
  $ws.Cells.Item(1, 1).Value2 = 'Region'
  for ($s = 0; $s -lt 3; $s++) { $ws.Cells.Item(1, $s + 2).Value2 = $seriesNames[$s] }
  for ($i = 0; $i -lt 5; $i++) {
    $ws.Cells.Item($i + 2, 1).Value2 = $categories[$i]
    for ($s = 0; $s -lt 3; $s++) { $ws.Cells.Item($i + 2, $s + 2).Value2 = [double]$seriesData[$s][$i] }
  }
  $anchor = $ws.Range($range)
  $co = $ws.ChartObjects().Add($anchor.Left, $anchor.Top, $anchor.Width, $anchor.Height)
  $co.Name = 'Chart 1'
  $ch = $co.Chart
  $ch.ChartType = $type
  $ch.SetSourceData($ws.Range('A1:D6'))
  & $setup $ws $ch
  # The 2023+ Office theme uses Aptos Narrow, which cannot be bundled; Calibri has a metric-compatible
  # open font (Carlito), so layout can be checked on any machine.
  $ch.ChartArea.Font.Name = 'Calibri'
  $wb.RemovePersonalInformation = $true
  $wb.SaveAs($path, 51)
  Write-Manifest $name $notes
  $wb.Close($false)
  [void][Runtime.InteropServices.Marshal]::ReleaseComObject($wb)
}

function Invoke-Fixtures {
  New-ChartFixture 'chart-column-clustered' 'clustered column, three series, title, legend right' 51 {
    param($ws, $ch) $ch.HasTitle = $true; $ch.ChartTitle.Text = 'Sales by region'; $ch.HasLegend = $true; $ch.Legend.Position = -4152 }
  New-ChartFixture 'chart-column-stacked' 'stacked column, legend bottom, data labels' 52 {
    param($ws, $ch) $ch.HasTitle = $true; $ch.ChartTitle.Text = 'Stacked'; $ch.Legend.Position = -4107; $ch.SeriesCollection(1).HasDataLabels = $true }
  New-ChartFixture 'chart-column-percent' 'percent stacked column, legend top' 53 {
    param($ws, $ch) $ch.HasTitle = $false; $ch.Legend.Position = -4160 }
  New-ChartFixture 'chart-bar-clustered' 'clustered bar, legend right, no title' 57 {
    param($ws, $ch) $ch.HasTitle = $false }
  New-ChartFixture 'chart-line' 'line with markers, title, legend bottom' 65 {
    param($ws, $ch) $ch.HasTitle = $true; $ch.ChartTitle.Text = 'Trend'; $ch.Legend.Position = -4107 }
  New-ChartFixture 'chart-pie' 'pie of the first series, percent data labels, legend right' 5 {
    param($ws, $ch)
    $ch.SetSourceData($ws.Range('A1:B6'))
    $ch.HasTitle = $true; $ch.ChartTitle.Text = 'Share'
    $ch.SeriesCollection(1).HasDataLabels = $true
    $ch.SeriesCollection(1).DataLabels().ShowPercentage = $true
    $ch.SeriesCollection(1).DataLabels().ShowValue = $false }
  New-ChartFixture 'chart-doughnut' 'doughnut of the first series, legend right' -4120 {
    param($ws, $ch) $ch.SetSourceData($ws.Range('A1:B6')); $ch.HasTitle = $false }
  New-ChartFixture 'chart-scatter' 'scatter with markers only; X values in column B, Y in column C' -4169 {
    param($ws, $ch)
    $ch.SetSourceData($ws.Range('B1:C6'))
    $ch.HasTitle = $true; $ch.ChartTitle.Text = 'Scatter'; $ch.HasLegend = $false }
  New-ChartFixture 'chart-area' 'area, legend bottom' 1 {
    param($ws, $ch) $ch.HasTitle = $false; $ch.Legend.Position = -4107 }
  New-ChartFixture 'chart-fixed-axis' 'column with fixed value axis 0..40 step 10, axis titles, no legend' 51 {
    param($ws, $ch)
    $ch.HasTitle = $true; $ch.ChartTitle.Text = 'Fixed'; $ch.HasLegend = $false
    $ax = $ch.Axes(2); $ax.MinimumScale = 0; $ax.MaximumScale = 40; $ax.MajorUnit = 10
    $ax.HasTitle = $true; $ax.AxisTitle.Text = 'Units'
    $cx = $ch.Axes(1); $cx.HasTitle = $true; $cx.AxisTitle.Text = 'Region' }
  # Hold-out fixtures: different sizes, legend positions and series counts than the ones the layout
  # constants were fitted on (docs/architecture/charts.md).
  New-ChartFixture 'chart-holdout-bar-stacked' 'hold-out: wide stacked bar, legend top, title' 58 {
    param($ws, $ch) $ch.HasTitle = $true; $ch.ChartTitle.Text = 'Wide bars'; $ch.Legend.Position = -4160 } 'G2:S30'
  New-ChartFixture 'chart-holdout-line-left' 'hold-out: small line chart, two series, legend left, no title' 65 {
    param($ws, $ch) $ch.SetSourceData($ws.Range('A1:C6')); $ch.HasTitle = $false; $ch.Legend.Position = -4131 } 'G2:L12'
  New-ChartFixture 'chart-holdout-column-titles' 'hold-out: two-series column, legend bottom, axis titles' 51 {
    param($ws, $ch)
    $ch.SetSourceData($ws.Range('A1:C6'))
    $ch.HasTitle = $true; $ch.ChartTitle.Text = 'Quarterly output by region'; $ch.Legend.Position = -4107
    $ch.Axes(2).HasTitle = $true; $ch.Axes(2).AxisTitle.Text = 'Output'
    $ch.Axes(1).HasTitle = $true; $ch.Axes(1).AxisTitle.Text = 'Region name' } 'G2:P22'
  Write-Output 'fixtures written'
}

# ---------------------------------------------------------------------------------------------------------
# Layout
# ---------------------------------------------------------------------------------------------------------
function Rect-Json($o) {
  try { return ('{{"left":{0},"top":{1},"width":{2},"height":{3}}}' -f (Num $o.Left), (Num $o.Top), (Num $o.Width), (Num $o.Height)) } catch { return 'null' }
}

function Invoke-Layout {
  $files = Get-ChildItem $fixtureDir -Filter 'chart-*.xlsx' | Sort-Object Name
  if ($Only) { $files = $files | Where-Object { $_.BaseName -match $Only } }
  foreach ($f in $files) {
    $wb = $script:xl.Workbooks.Open($f.FullName, 0, $true)
    try {
      $ws = $wb.Worksheets.Item(1)
      $co = $ws.ChartObjects(1)
      $ch = $co.Chart
      $ch.Refresh()
      $pa = $ch.PlotArea
      $inside = ('{{"left":{0},"top":{1},"width":{2},"height":{3}}}' -f (Num $pa.InsideLeft), (Num $pa.InsideTop), (Num $pa.InsideWidth), (Num $pa.InsideHeight))
      $outer = Rect-Json $pa
      $legend = 'null'; if ($ch.HasLegend) { $legend = Rect-Json $ch.Legend }
      $title = 'null'; if ($ch.HasTitle) { $title = Rect-Json $ch.ChartTitle }
      $scale = 'null'
      if ($ch.ChartType -notin 5, -4120) {
        $v = $ch.Axes(2)
        $scale = ('{{"min":{0},"max":{1},"major":{2}}}' -f (Num $v.MinimumScale), (Num $v.MaximumScale), (Num $v.MajorUnit))
      }
      $json = "{`n  `"source`": `"fixtures/xlsx/$($f.Name)`",`n  `"oracle`": `"Microsoft Excel via COM, points, origin at the chart area top left`",`n  `"excel_build`": `"$($script:xl.Build)`",`n" +
        "  `"object`": {`"width`":$(Num $co.Width),`"height`":$(Num $co.Height)},`n" +
        "  `"chart_area`": {`"width`":$(Num $ch.ChartArea.Width),`"height`":$(Num $ch.ChartArea.Height)},`n" +
        "  `"plot_inside`": $inside,`n  `"plot_outer`": $outer,`n  `"legend`": $legend,`n  `"title`": $title,`n  `"value_axis`": $scale`n}`n"
      Write-Lf (Join-Path $refDir "$($f.BaseName)/layout.json") $json
      Write-Output "layout $($f.BaseName)"
    } finally { $wb.Saved = $true; $wb.Close($false) }
  }
}

# ---------------------------------------------------------------------------------------------------------
# Axes
# ---------------------------------------------------------------------------------------------------------
function Round-To($v, $dec) { [Math]::Round($v, $dec, [MidpointRounding]::AwayFromZero) }

function New-AxisCases {
  $rng = New-Object System.Random $Seed
  $cases = New-Object System.Collections.ArrayList
  function Add-Case($kind, $h, $series, $xs) {
    [void]$cases.Add([pscustomobject]@{ kind = $kind; h = $h; series = $series; xs = $xs })
  }
  $heights = 216, 150, 300, 420
  $kinds = 'column', 'line'
  $edge = @(
    @(1, 2, 3, 4), @(10, 20, 30), @(0, 0, 0), @(5, 5, 5), @(100), @(1), @(0.5, 0.25, 0.75), @(99, 100, 101),
    @(1000, 1005, 1010), @(95, 97, 100), @(-3, -2, -1), @(-5, 5, 0), @(-100, 250, 40), @(7, 8, 9, 10, 11),
    @(0.001, 0.002, 0.003), @(12345, 67890, 23456), @(1e6, 2.5e6, 3e6), @(0.9, 1.0, 1.1), @(2, 4, 6, 8, 10),
    @(100, 100, 100, 99), @(53, 56, 51, 58), @(1e-5, 3e-5, 2e-5), @(-0.5, -0.25, -0.1), @(80, 90, 85, 95),
    @(10, 10.5, 11), @(1e9, 3e9, 2e9), @(0, 1), @(0, 100), @(0, 1000), @(-1, 1), @(33, 66, 99), @(14, 17, 21, 25)
  )
  foreach ($e in $edge) {
    foreach ($k in $kinds) { Add-Case $k 216 @(, ([double[]]$e)) $null }
  }
  for ($i = 0; $i -lt 140; $i++) {
    $mag = [Math]::Pow(10, $rng.Next(-3, 7))
    $n = $rng.Next(3, 9)
    $style = $rng.Next(0, 4)
    $vals = New-Object double[] $n
    for ($j = 0; $j -lt $n; $j++) {
      switch ($style) {
        0 { $x = $rng.NextDouble() * 10 }
        1 { $x = 8 + $rng.NextDouble() * 2 }
        2 { $x = ($rng.NextDouble() * 20) - 10 }
        3 { $x = -1 * (1 + $rng.NextDouble() * 9) }
      }
      $vals[$j] = Round-To ($x * $mag) 6
    }
    $h = $heights[$rng.Next(0, $heights.Count)]
    Add-Case ($kinds[$rng.Next(0, 2)]) $h @(, $vals) $null
  }
  foreach ($h in $heights) {
    foreach ($e in @(@(12, 19, 7, 15, 22), @(980, 1020, 1100), @(0.2, 0.9, 0.5))) { Add-Case 'column' $h @(, ([double[]]$e)) $null }
  }
  for ($i = 0; $i -lt 30; $i++) {
    $mag = [Math]::Pow(10, $rng.Next(-1, 5)); $n = $rng.Next(3, 7)
    $a = New-Object double[] $n; $b = New-Object double[] $n
    for ($j = 0; $j -lt $n; $j++) { $a[$j] = Round-To ($rng.NextDouble() * 10 * $mag) 4; $b[$j] = Round-To ($rng.NextDouble() * 10 * $mag) 4 }
    Add-Case 'stacked' 216 @($a, $b) $null
  }
  for ($i = 0; $i -lt 20; $i++) {
    $mag = [Math]::Pow(10, $rng.Next(-1, 5)); $n = $rng.Next(3, 8)
    $a = New-Object double[] $n
    for ($j = 0; $j -lt $n; $j++) { $a[$j] = Round-To ((5 + $rng.NextDouble() * 5) * $mag) 4 }
    Add-Case 'area' 216 @(, $a) $null
  }
  for ($i = 0; $i -lt 40; $i++) {
    $mag = [Math]::Pow(10, $rng.Next(-1, 5)); $n = $rng.Next(4, 9)
    $xm = [Math]::Pow(10, $rng.Next(-1, 4))
    $a = New-Object double[] $n; $x = New-Object double[] $n
    for ($j = 0; $j -lt $n; $j++) { $a[$j] = Round-To ((($rng.NextDouble() * 12) - 2) * $mag) 4; $x[$j] = Round-To (($rng.NextDouble() * 10) * $xm) 4 }
    Add-Case 'scatter' 216 @(, $a) $x
  }
  return $cases
}

function Invoke-Axes {
  $wb = $script:xl.Workbooks.Add()
  try {
    $ws = $wb.Worksheets.Item(1)
    $co = $ws.ChartObjects().Add(10, 10, 360, 216)
    $ch = $co.Chart
    $cases = New-AxisCases
    $out = New-Object System.Collections.ArrayList
    $idx = 0
    foreach ($c in $cases) {
      $idx++
      [void]$ws.Cells.Clear()
      $n = $c.series[0].Count
      $nser = $c.series.Count
      if ($c.kind -eq 'scatter') {
        for ($j = 0; $j -lt $n; $j++) { $ws.Cells.Item($j + 1, 1).Value2 = [double]$c.xs[$j]; $ws.Cells.Item($j + 1, 2).Value2 = [double]$c.series[0][$j] }
      } else {
        for ($j = 0; $j -lt $n; $j++) { for ($s = 0; $s -lt $nser; $s++) { $ws.Cells.Item($j + 1, $s + 1).Value2 = [double]$c.series[$s][$j] } }
      }
      $co.Height = $c.h; $co.Width = 360
      $type = switch ($c.kind) { 'column' { 51 } 'line' { 4 } 'stacked' { 52 } 'area' { 1 } 'scatter' { -4169 } }
      $ch.ChartType = $type
      $ch.SetSourceData($ws.Range($ws.Cells.Item(1, 1), $ws.Cells.Item($n, [Math]::Max($nser, 2))))
      if ($c.kind -ne 'scatter') { $ch.PlotBy = 2 }
      $ch.ChartType = $type
      $ch.Refresh()
      $v = $ch.Axes(2)
      $rec = '{{"kind":"{0}","height":{1},"series":[{2}],' -f $c.kind, (Num $c.h), (($c.series | ForEach-Object { '[' + (($_ | ForEach-Object { Num $_ }) -join ',') + ']' }) -join ',')
      if ($c.kind -eq 'scatter') {
        $rec += '"xs":[' + (($c.xs | ForEach-Object { Num $_ }) -join ',') + '],'
        $x = $ch.Axes(1)
        $rec += ('"x_axis":{{"min":{0},"max":{1},"major":{2}}},' -f (Num $x.MinimumScale), (Num $x.MaximumScale), (Num $x.MajorUnit))
      }
      $rec += ('"inside_w":{0},"inside_h":{1},' -f (Num $ch.PlotArea.InsideWidth), (Num $ch.PlotArea.InsideHeight))
      $rec += ('"value_axis":{{"min":{0},"max":{1},"major":{2},"minor":{3}}}}}' -f (Num $v.MinimumScale), (Num $v.MaximumScale), (Num $v.MajorUnit), (Num $v.MinorUnit))
      [void]$out.Add($rec)
      if ($idx % 50 -eq 0) { Write-Output "axes $idx/$($cases.Count)" }
    }
    $json = "{`n  `"oracle`": `"Microsoft Excel via COM, Axes(xlValue).MinimumScale/MaximumScale/MajorUnit/MinorUnit, chart object 360 pt wide, automatic scaling`",`n  `"excel_build`": `"$($script:xl.Build)`",`n  `"cases`": [`n    " +
      ($out -join ",`n    ") + "`n  ]`n}`n"
    $axesName = $(if ($Seed -eq 365) { 'axes.json' } else { "axes-seed$Seed.json" })
    Write-Lf (Join-Path $refDir "chart-axes/$axesName") $json
  } finally { $wb.Saved = $true; $wb.Close($false) }
}

function Invoke-Sweep {
  $wb = $script:xl.Workbooks.Add()
  try {
    $ws = $wb.Worksheets.Item(1)
    $co = $ws.ChartObjects().Add(10, 10, 360, 216)
    $ch = $co.Chart
    $out = New-Object System.Collections.ArrayList
    foreach ($kind in 'column', 'bar') {
      foreach ($size in 80, 120, 160, 200, 240, 280, 320, 360, 400, 440, 480, 520, 560, 600, 640) {
        if ($kind -eq 'column') { $co.Width = 360; $co.Height = $size } else { $co.Height = 216; $co.Width = $size }
        for ($v = 5; $v -le 30; $v++) {
          [void]$ws.Cells.Clear()
          $ws.Cells.Item(1, 1).Value2 = 0.0; $ws.Cells.Item(2, 1).Value2 = [double]$v
          $ch.ChartType = $(if ($kind -eq 'column') { 51 } else { 57 })
          $ch.SetSourceData($ws.Range('A1:A2'))
          $ch.Refresh()
          $a = $ch.Axes(2)
          [void]$out.Add(('{{"kind":"{0}","size":{1},"v":{2},"min":{3},"max":{4},"major":{5},"inside_w":{6},"inside_h":{7}}}' -f $kind, $size, $v, (Num $a.MinimumScale), (Num $a.MaximumScale), (Num $a.MajorUnit), (Num $ch.PlotArea.InsideWidth), (Num $ch.PlotArea.InsideHeight)))
        }
      }
      Write-Output "sweep $kind"
    }
    $json = "{`n  `"oracle`": `"Microsoft Excel via COM, Axes(xlValue) for data (0, v); size is the chart object height (column) or width (bar) in points`",`n  `"excel_build`": `"$($script:xl.Build)`",`n  `"cases`": [`n    " + ($out -join ",`n    ") + "`n  ]`n}`n"
    Write-Lf (Join-Path $refDir 'chart-axes/sweep.json') $json
  } finally { $wb.Saved = $true; $wb.Close($false) }
}

try {
  Start-Excel
  if ($Fixtures) { Invoke-Fixtures }
  if ($Layout) { Invoke-Layout }
  if ($Axes) { Invoke-Axes }
  if ($Sweep) { Invoke-Sweep }
} finally {
  Stop-Excel
}
