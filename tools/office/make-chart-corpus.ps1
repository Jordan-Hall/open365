<#
Generates the randomised chart measurement corpora that crates/chart is fitted on and checked against.
Excel (local, COM) is the oracle; every number in the output is what Excel reported for a generated chart.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed JSON through crates/chart/tests.

  -Set layout   random chart type, size, legend position, titles, series and categories. Records the resolved
                value axis (MinimumScale, MaximumScale, MajorUnit, MinorUnit), PlotArea.Inside*, PlotArea.*,
                Legend.* and ChartTitle.* in points.
  -Set log      value axis with ScaleType = xlLogarithmic over random positive data (LogBase 10, sometimes 2/3/5).
  -Set date     category axis of real dates (time-scale axis): MinimumScale, MaximumScale, MajorUnit,
                MajorUnitScale, MinorUnit, MinorUnitScale, BaseUnit.
  -Set lognarrow  log axes over data inside a narrow band at large magnitudes (the regime where the axis may not start at 1).
  -Set labels   tick-label number formats, tick values and texts on linear and log value axes, and the DataLabel
                rectangles (Left/Top/Width/Height) at every valid label position for column, column_stacked, bar,
                bar_stacked, line, scatter and pie. Output: fixtures/reference/xlsx/chart-labels/<tag>.json
  -Set dates    date (time-scale) category axes: tick dates, the label text Excel formats for each, and tick positions in
                points along the plot area (fitted from DataLabel centres, see "fit"). Output: .../chart-dates/<tag>.json
  -Set all      layout, log, lognarrow and date (not labels or dates, which are slower).

  -Seed N       random sequence; the committed files use 1001 (train, fitted on) and 2002 (hold-out, never fitted).
  -Count N      cases per set (default: layout 1500, log 300, date 300; labels and dates 200 for seed 1001, else 100).
  -OutRoot dir  write chart-labels/ and chart-dates/ under this directory instead of the repository (trial runs).
  -Tag name     output suffix: fixtures/reference/xlsx/chart-scale/<set>-<tag>.json (default train/holdout by seed).

Process safety: starts its own Excel instance; aborts when the new instance has the PID of an Excel that was already
running (so a user's own session is never touched); AutomationSecurity = 3; Quit in finally; only the PID this
script launched is ever stopped. Generated workbooks only, nothing is saved, no personal data. Output is stable:
LF line endings, ASCII only, no absolute paths.
#>
param(
  [ValidateSet('layout', 'log', 'lognarrow', 'date', 'labels', 'dates', 'all')][string]$Set = 'all',
  [int]$Seed = 1001,
  [int]$Count = 0,
  [string]$Tag = '',
  [string]$OutRoot = ''
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$outDir = Join-Path $root 'fixtures/reference/xlsx/chart-scale'
$labelsDir = Join-Path $root 'fixtures/reference/xlsx/chart-labels'
$datesDir = Join-Path $root 'fixtures/reference/xlsx/chart-dates'
if ($OutRoot) { $labelsDir = Join-Path $OutRoot 'chart-labels'; $datesDir = Join-Path $OutRoot 'chart-dates' }
$inv = [Globalization.CultureInfo]::InvariantCulture
if (-not $Tag) { $Tag = $(if ($Seed -eq 1001) { 'train' } elseif ($Seed -eq 2002) { 'holdout' } else { "seed$Seed" }) }

Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class McWin {
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
}
'@

$script:xl = $null
$script:xlPid = 0
$script:owned = $false

function Start-Excel {
  $before = @(Get-Process -Name EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $app = New-Object -ComObject Excel.Application
  $procId = [uint32]0
  [void][McWin]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  if ($before -contains [int]$procId) {
    # Attached to somebody else's Excel: let go without quitting or changing anything.
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($app)
    throw "Attached to an Excel instance (PID $procId) that was already running; aborting."
  }
  $script:xlPid = [int]$procId
  Write-Output "Excel PID $procId (started by this script)"
  $script:owned = $true
  $script:xl = $app
  $app.AutomationSecurity = 3
  $app.Visible = $true
  $app.DisplayAlerts = $false
  $app.EnableEvents = $false
  $app.ScreenUpdating = $false
}

# Writes a rectangular block with one COM call. $rows is an array of arrays (first cell at A1).
function Write-Block($ws, $rows) {
  $nr = $rows.Count; $nc = 0
  foreach ($r in $rows) { if ($r.Count -gt $nc) { $nc = $r.Count } }
  $arr = New-Object 'object[,]' $nr, $nc
  for ($i = 0; $i -lt $nr; $i++) { for ($j = 0; $j -lt $rows[$i].Count; $j++) { $arr[$i, $j] = $rows[$i][$j] } }
  $ws.Range($ws.Cells.Item(1, 1), $ws.Cells.Item($nr, $nc)).Value2 = $arr
}

function Stop-Excel {
  $app = $script:xl
  $script:xl = $null
  if ($app -and $script:owned) {
    try { foreach ($wb in @($app.Workbooks)) { try { $wb.Saved = $true; $wb.Close($false) } catch {} } } catch {}
    try { $app.Quit() } catch {}
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
  }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  if ($script:owned -and $script:xlPid -gt 0) {
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
function Arr($a) { return '[' + (($a | ForEach-Object { Num $_ }) -join ',') + ']' }
function StrArr($a) { return '[' + (($a | ForEach-Object { '"' + $_ + '"' }) -join ',') + ']' }
function Rect($o) {
  try { return ('{{"left":{0},"top":{1},"width":{2},"height":{3}}}' -f (Num $o.Left), (Num $o.Top), (Num $o.Width), (Num $o.Height)) } catch { return 'null' }
}
function Write-Lf($path, $text) {
  New-Item -ItemType Directory -Force (Split-Path -Parent $path) | Out-Null
  [IO.File]::WriteAllText($path, ($text -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
}
function Round-To($v, $dec) { [Math]::Round($v, $dec, [MidpointRounding]::AwayFromZero) }

function New-Scratch {
  $wb = $script:xl.Workbooks.Add()
  return $wb
}

function Col-Letter([int]$n) { return [string][char](64 + $n) }

# ---------------------------------------------------------------------------------------------------------
# layout
# ---------------------------------------------------------------------------------------------------------
$words = 'North', 'South', 'East', 'West', 'Centre', 'Alpha', 'Beta', 'Gamma', 'Delta', 'Q1', 'Q2', 'Q3', 'Q4', 'Jan', 'Feb', 'Mar',
  'Apples', 'Pears', 'Plums', 'Cherries', 'Widgets', 'Gadgets', 'Region', 'Total', 'Units', 'Revenue', 'Cost', 'Margin'
$longWords = 'International sales', 'Northern territories', 'Quarterly output by region', 'Customer satisfaction'
$typeTable = @(
  @{ n = 'column'; t = 51; w = 20 }, @{ n = 'column_stacked'; t = 52; w = 8 }, @{ n = 'column_percent'; t = 53; w = 4 },
  @{ n = 'bar'; t = 57; w = 14 }, @{ n = 'bar_stacked'; t = 58; w = 5 },
  @{ n = 'line'; t = 4; w = 8 }, @{ n = 'line_marker'; t = 65; w = 10 },
  @{ n = 'area'; t = 1; w = 6 }, @{ n = 'area_stacked'; t = 76; w = 3 },
  @{ n = 'scatter'; t = -4169; w = 8 }, @{ n = 'pie'; t = 5; w = 8 }, @{ n = 'doughnut'; t = -4120; w = 3 }
)
$legendTable = @(@{ n = 'none'; c = 0 }, @{ n = 'r'; c = -4152 }, @{ n = 'l'; c = -4131 }, @{ n = 't'; c = -4160 }, @{ n = 'b'; c = -4107 })

function Pick-Weighted($rng, $table) {
  $sum = 0; foreach ($e in $table) { $sum += $e.w }
  $r = $rng.Next(0, $sum)
  foreach ($e in $table) { if ($r -lt $e.w) { return $e }; $r -= $e.w }
  return $table[0]
}

function Invoke-Layout($n) {
  $rng = New-Object System.Random $Seed
  $wb = New-Scratch
  $out = New-Object System.Collections.ArrayList
  try {
    $ws = $wb.Worksheets.Item(1)
    for ($i = 0; $i -lt $n; $i++) {
      $ty = Pick-Weighted $rng $typeTable
      $pie = $ty.n -in 'pie', 'doughnut'
      $w = $rng.Next(180, 721); $h = $rng.Next(110, 451)
      $ncat = $rng.Next(3, 13); $nser = $rng.Next(1, 5)
      if ($pie) { $nser = 1 }
      $scatter = $ty.n -eq 'scatter'
      $leg = $legendTable[$rng.Next(0, 5)]
      $mag = [Math]::Pow(10, $rng.Next(-1, 6))
      $neg = ($rng.Next(0, 6) -eq 0) -and -not $pie
      $long = $rng.Next(0, 7) -eq 0
      # Only four long titles exist; asking for more distinct ones below would loop forever (the 2026-10-11 hang).
      if ($long -and $ncat -gt $longWords.Count) { $ncat = $longWords.Count }
      $cats = New-Object string[] $ncat
      $used = @{}
      for ($j = 0; $j -lt $ncat; $j++) {
        do { $c = $(if ($long) { $longWords[$rng.Next(0, $longWords.Count)] } else { $words[$rng.Next(0, $words.Count)] }) } while ($used.ContainsKey($c))
        $used[$c] = 1; $cats[$j] = $c
      }
      $series = New-Object System.Collections.ArrayList
      for ($s = 0; $s -lt $nser; $s++) {
        $v = New-Object double[] $ncat
        for ($j = 0; $j -lt $ncat; $j++) {
          $x = $rng.NextDouble() * 10
          if ($neg) { $x = ($x - 4) }
          $v[$j] = Round-To ($x * $mag) 4
        }
        [void]$series.Add($v)
      }
      $xs = $null
      if ($scatter) {
        $xs = New-Object double[] $ncat
        for ($j = 0; $j -lt $ncat; $j++) { $xs[$j] = Round-To ($rng.NextDouble() * 10 * $mag) 4 }
      }
      $names = 1..$nser | ForEach-Object { "Series$_" }
      $hasTitle = $rng.Next(0, 3) -ne 0
      $title = $(if ($hasTitle) { @('Sales', 'Trend', 'Quarterly output by region', 'A', 'Revenue per year')[$rng.Next(0, 5)] } else { $null })
      $catTitle = $(if (-not $pie -and $rng.Next(0, 5) -eq 0) { 'Region name' } else { $null })
      $valTitle = $(if (-not $pie -and $rng.Next(0, 5) -eq 0) { 'Output' } else { $null })

      [void]$ws.Cells.Clear()
      $rows = New-Object System.Collections.ArrayList
      $head = New-Object System.Collections.ArrayList
      [void]$head.Add($(if ($scatter) { 'X' } else { '' }))
      for ($s = 0; $s -lt $nser; $s++) { [void]$head.Add([string]$names[$s]) }
      [void]$rows.Add($head.ToArray())
      for ($j = 0; $j -lt $ncat; $j++) {
        $r = New-Object System.Collections.ArrayList
        [void]$r.Add($(if ($scatter) { [double]$xs[$j] } else { [string]$cats[$j] }))
        for ($s = 0; $s -lt $nser; $s++) { [void]$r.Add([double]$series[$s][$j]) }
        [void]$rows.Add($r.ToArray())
      }
      Write-Block $ws $rows.ToArray()
      foreach ($old in @($ws.ChartObjects())) { $old.Delete() | Out-Null }
      $co = $ws.ChartObjects().Add(20, 20, $w, $h)
      $ch = $co.Chart
      $ch.ChartType = $ty.t
      $ch.SetSourceData($ws.Range($ws.Cells.Item(1, 1), $ws.Cells.Item($ncat + 1, $nser + 1)))
      if (-not $scatter) { $ch.PlotBy = 2 }
      $ch.ChartType = $ty.t
      $ch.ChartArea.Font.Name = 'Calibri'
      if ($title) { $ch.HasTitle = $true; $ch.ChartTitle.Text = $title } else { $ch.HasTitle = $false }
      if ($leg.c -eq 0) { $ch.HasLegend = $false } else { $ch.HasLegend = $true; $ch.Legend.Position = $leg.c }
      if ($catTitle) { $ch.Axes(1).HasTitle = $true; $ch.Axes(1).AxisTitle.Text = $catTitle }
      if ($valTitle) { $ch.Axes(2).HasTitle = $true; $ch.Axes(2).AxisTitle.Text = $valTitle }
      $ch.Refresh()
      $pa = $ch.PlotArea
      $rec = '{{"type":"{0}","width":{1},"height":{2},"cats":{3},"series":{4},"names":{5},"xs":{6},"title":{7},"legend":"{8}","cat_title":{9},"val_title":{10},' -f `
        $ty.n, $w, $h, (StrArr $cats), ('[' + (($series | ForEach-Object { Arr $_ }) -join ',') + ']'), (StrArr $names), $(if ($xs) { Arr $xs } else { 'null' }), `
        $(if ($title) { '"' + $title + '"' } else { 'null' }), $leg.n, $(if ($catTitle) { '"' + $catTitle + '"' } else { 'null' }), $(if ($valTitle) { '"' + $valTitle + '"' } else { 'null' })
      $rec += ('"chart_area":{{"width":{0},"height":{1}}},"inside":{2},"outer":{3},"legend_rect":{4},"title_rect":{5}' -f (Num $ch.ChartArea.Width), (Num $ch.ChartArea.Height), `
        ('{{"left":{0},"top":{1},"width":{2},"height":{3}}}' -f (Num $pa.InsideLeft), (Num $pa.InsideTop), (Num $pa.InsideWidth), (Num $pa.InsideHeight)), `
        (Rect $pa), $(if ($ch.HasLegend) { Rect $ch.Legend } else { 'null' }), $(if ($ch.HasTitle) { Rect $ch.ChartTitle } else { 'null' }))
      if (-not $pie) {
        $v = $ch.Axes(2)
        $rec += (',"value_axis":{{"min":{0},"max":{1},"major":{2},"minor":{3}}}' -f (Num $v.MinimumScale), (Num $v.MaximumScale), (Num $v.MajorUnit), (Num $v.MinorUnit))
        if ($scatter) {
          $x = $ch.Axes(1)
          $rec += (',"x_axis":{{"min":{0},"max":{1},"major":{2},"minor":{3}}}' -f (Num $x.MinimumScale), (Num $x.MaximumScale), (Num $x.MajorUnit), (Num $x.MinorUnit))
        }
      }
      $rec += '}'
      [void]$out.Add($rec)
      if (($i + 1) % 100 -eq 0) { Write-Output "layout $($i + 1)/$n" }
    }
  } finally { $wb.Saved = $true; $wb.Close($false) }
  $json = "{`n  `"oracle`": `"Microsoft Excel via COM, points, chart area origin; font Calibri 10 pt, automatic layout`",`n  `"excel_build`": `"$($script:xl.Build)`",`n  `"seed`": $Seed,`n  `"cases`": [`n    " + ($out -join ",`n    ") + "`n  ]`n}`n"
  Write-Lf (Join-Path $outDir "layout-$Tag.json") $json
}

# ---------------------------------------------------------------------------------------------------------
# log
# ---------------------------------------------------------------------------------------------------------
function Invoke-Log($n, [bool]$narrow = $false) {
  $rng = New-Object System.Random ($Seed + 17)
  $wb = New-Scratch
  $out = New-Object System.Collections.ArrayList
  try {
    $ws = $wb.Worksheets.Item(1)
    $co = $ws.ChartObjects().Add(10, 10, 360, 216)
    $ch = $co.Chart
    for ($i = 0; $i -lt $n; $i++) {
      $kind = @('column', 'line', 'scatter')[$rng.Next(0, 3)]
      $base = @(10, 10, 10, 10, 2, 3, 5)[$rng.Next(0, 7)]
      $h = @(216, 150, 300, 420)[$rng.Next(0, 4)]
      $m = $rng.Next(3, 9)
      $dec = $rng.Next(1, 8)
      $lo = [Math]::Pow(10, $rng.Next(-3, 4))
      $v = New-Object double[] $m
      if ($narrow) {
        # Data inside a narrow band (up to one decade wide) at a random magnitude.
        $lo = [Math]::Pow(10, $rng.Next(-4, 13)) * (1 + $rng.NextDouble() * 8)
        $spread = [Math]::Pow(10, $rng.NextDouble())
        for ($j = 0; $j -lt $m; $j++) { $v[$j] = [double]::Parse(($lo * (1 + $rng.NextDouble() * ($spread - 1))).ToString('G6', $inv), $inv); if ($v[$j] -le 0) { $v[$j] = $lo } }
      } else {
        for ($j = 0; $j -lt $m; $j++) { $v[$j] = Round-To ($lo * [Math]::Pow(10, $rng.NextDouble() * $dec) * (1 + $rng.NextDouble())) 6; if ($v[$j] -le 0) { $v[$j] = $lo } }
      }
      [void]$ws.Cells.Clear()
      $co.Height = $h; $co.Width = 360
      $rows = New-Object System.Collections.ArrayList
      for ($j = 0; $j -lt $m; $j++) { [void]$rows.Add(@([double]$v[$j], [double]($j + 1))) }
      Write-Block $ws $rows.ToArray()
      $co.Delete() | Out-Null
      $co = $ws.ChartObjects().Add(10, 10, 360, $h)
      $ch = $co.Chart
      $ch.ChartType = $(if ($kind -eq 'column') { 51 } elseif ($kind -eq 'line') { 4 } else { -4169 })
      if ($kind -eq 'scatter') {
        $ch.SetSourceData($ws.Range($ws.Cells.Item(1, 2), $ws.Cells.Item($m, 2)))
        $ch.SeriesCollection(1).XValues = $ws.Range($ws.Cells.Item(1, 2), $ws.Cells.Item($m, 2))
        $ch.SeriesCollection(1).Values = $ws.Range($ws.Cells.Item(1, 1), $ws.Cells.Item($m, 1))
      } else {
        $ch.SetSourceData($ws.Range($ws.Cells.Item(1, 1), $ws.Cells.Item($m, 1)))
      }
      $ch.ChartArea.Font.Name = 'Calibri'
      $a = $ch.Axes(2)
      try { $a.ScaleType = -4133; $a.LogBase = $base } catch { continue }
      $ch.Refresh()
      $a = $ch.Axes(2)
      [void]$out.Add(('{{"kind":"{0}","base":{1},"height":{2},"values":{3},"inside_h":{4},"min":{5},"max":{6},"major":{7},"minor":{8},"crosses_at":{9}}}' -f `
          $kind, $base, $h, (Arr $v), (Num $ch.PlotArea.InsideHeight), (Num $a.MinimumScale), (Num $a.MaximumScale), (Num $a.MajorUnit), (Num $a.MinorUnit), (Num $a.CrossesAt)))
      if (($i + 1) % 100 -eq 0) { Write-Output "log $($i + 1)/$n" }
    }
  } finally { $wb.Saved = $true; $wb.Close($false) }
  $json = "{`n  `"oracle`": `"Microsoft Excel via COM, Axes(xlValue) with ScaleType = xlLogarithmic, chart object 360 pt wide`",`n  `"excel_build`": `"$($script:xl.Build)`",`n  `"seed`": $Seed,`n  `"cases`": [`n    " + ($out -join ",`n    ") + "`n  ]`n}`n"
  Write-Lf (Join-Path $outDir "$(if ($narrow) { 'lognarrow' } else { 'log' })-$Tag.json") $json
}

# ---------------------------------------------------------------------------------------------------------
# date
# ---------------------------------------------------------------------------------------------------------
function Invoke-Date($n) {
  $rng = New-Object System.Random ($Seed + 31)
  $wb = New-Scratch
  $out = New-Object System.Collections.ArrayList
  try {
    $ws = $wb.Worksheets.Item(1)
    for ($i = 0; $i -lt $n; $i++) {
      $pattern = @('daily', 'weekly', 'monthly', 'quarterly', 'yearly', 'irregular')[$rng.Next(0, 6)]
      $kind = @('line', 'column', 'line', 'area')[$rng.Next(0, 4)]
      $w = @(360, 240, 480, 600)[$rng.Next(0, 4)]
      $m = $rng.Next(4, 41)
      if ($pattern -eq 'yearly') { $m = $rng.Next(3, 16) }
      $start = [datetime]'2015-01-01'
      $start = $start.AddDays($rng.Next(0, 3650))
      $dates = New-Object System.Collections.ArrayList
      $d = $start
      for ($j = 0; $j -lt $m; $j++) {
        [void]$dates.Add($d)
        switch ($pattern) {
          'daily' { $d = $d.AddDays(1) }
          'weekly' { $d = $d.AddDays(7) }
          'monthly' { $d = $d.AddMonths(1) }
          'quarterly' { $d = $d.AddMonths(3) }
          'yearly' { $d = $d.AddYears(1) }
          'irregular' { $d = $d.AddDays($rng.Next(1, 40)) }
        }
      }
      $serials = $dates | ForEach-Object { [double]$_.ToOADate() }
      $vals = 1..$m | ForEach-Object { Round-To ($rng.NextDouble() * 100) 2 }
      [void]$ws.Cells.Clear()
      $rows = New-Object System.Collections.ArrayList
      for ($j = 0; $j -lt $m; $j++) { [void]$rows.Add(@([double]$serials[$j], [double]$vals[$j])) }
      Write-Block $ws $rows.ToArray()
      $ws.Range($ws.Cells.Item(1, 1), $ws.Cells.Item($m, 1)).NumberFormat = 'yyyy-mm-dd'
      foreach ($old in @($ws.ChartObjects())) { $old.Delete() | Out-Null }
      $co = $ws.ChartObjects().Add(10, 10, $w, 216)
      $ch = $co.Chart
      $ch.ChartType = $(if ($kind -eq 'column') { 51 } elseif ($kind -eq 'line') { 4 } else { 1 })
      $ch.SetSourceData($ws.Range($ws.Cells.Item(1, 2), $ws.Cells.Item($m, 2)))
      $ch.SeriesCollection(1).XValues = $ws.Range($ws.Cells.Item(1, 1), $ws.Cells.Item($m, 1))
      $ch.ChartArea.Font.Name = 'Calibri'
      $ch.HasLegend = $false; $ch.HasTitle = $false
      $ch.Refresh()
      $x = $ch.Axes(1)
      $x.CategoryType = 3
      $ch.Refresh()
      $isTime = ($x.CategoryType -eq 3)
      $rec = ('{{"kind":"{0}","pattern":"{1}","width":{2},"dates":{3},"category_type":{4},"inside_w":{5},"inside_h":{6}' -f $kind, $pattern, $w, (Arr $serials), $x.CategoryType, (Num $ch.PlotArea.InsideWidth), (Num $ch.PlotArea.InsideHeight))
      if ($isTime) {
        $rec += (',"axis":{{"min":{0},"max":{1},"major":{2},"major_scale":{3},"minor":{4},"minor_scale":{5},"base_unit":{6}}}' -f `
            (Num $x.MinimumScale), (Num $x.MaximumScale), (Num $x.MajorUnit), $x.MajorUnitScale, (Num $x.MinorUnit), $x.MinorUnitScale, $x.BaseUnit)
      }
      $rec += '}'
      [void]$out.Add($rec)
      if (($i + 1) % 100 -eq 0) { Write-Output "date $($i + 1)/$n" }
    }
  } finally { $wb.Saved = $true; $wb.Close($false) }
  $json = "{`n  `"oracle`": `"Microsoft Excel via COM, Axes(xlCategory) of a date category axis (CategoryType auto); MajorUnitScale/BaseUnit 0 = days, 1 = months, 2 = years`",`n  `"excel_build`": `"$($script:xl.Build)`",`n  `"seed`": $Seed,`n  `"cases`": [`n    " + ($out -join ",`n    ") + "`n  ]`n}`n"
  Write-Lf (Join-Path $outDir "date-$Tag.json") $json
}

# ---------------------------------------------------------------------------------------------------------
# shared helpers for the labels and dates sets
# ---------------------------------------------------------------------------------------------------------
function Js($s) {
  if ($null -eq $s) { return 'null' }
  $sb = New-Object Text.StringBuilder
  [void]$sb.Append('"')
  foreach ($c in ([string]$s).ToCharArray()) {
    $n = [int]$c
    if ($n -eq 34) { [void]$sb.Append('\"') }
    elseif ($n -eq 92) { [void]$sb.Append('\\') }
    elseif ($n -lt 32 -or $n -gt 126) { [void]$sb.Append(('\u{0:x4}' -f $n)) }
    else { [void]$sb.Append($c) }
  }
  [void]$sb.Append('"')
  return $sb.ToString()
}
function Jb($b) { if ($b) { return 'true' } else { return 'false' } }
function R12($v) { return [double]::Parse(([double]$v).ToString('G12', $inv), $inv) }

# A scratch worksheet whose cell A1 formats a number with a number format: the text Excel's formatter produces
# (the rendered tick label itself is not exposed by the object model).
function Init-FmtSheet {
  $wb = $script:xl.Workbooks.Add()
  $ws = $wb.Worksheets.Item(1)
  $ws.Columns.Item(1).ColumnWidth = 100
  $script:fmtWs = $ws
}
function Fmt-Text($v, [string]$fmt) {
  try { $c = $script:fmtWs.Cells.Item(1, 1); $c.NumberFormat = $fmt; $c.Value2 = [double]$v; return [string]$c.Text } catch { return $null }
}

# Tick values of a value axis: min + k*major (linear) or min*base^k (log).
function Value-Ticks($ax) {
  $min = [double]$ax.MinimumScale; $max = [double]$ax.MaximumScale
  $vals = New-Object System.Collections.ArrayList
  $log = $false; try { $log = ($ax.ScaleType -eq -4133) } catch {}
  if ($log) {
    $b = [double]$ax.LogBase; $v = $min; $k = 0
    while ($v -le $max * (1 + 1e-9) -and $k -lt 60) { [void]$vals.Add((R12 $v)); $v = $v * $b; $k++ }
  } else {
    $mu = [double]$ax.MajorUnit
    if ($mu -gt 0) { for ($k = 0; $k -lt 400; $k++) { $v = $min + $k * $mu; if ($v -gt $max + $mu * 1e-9) { break }; [void]$vals.Add((R12 $v)) } }
  }
  return , $vals.ToArray()
}

# {"number_format","linked","scale","min","max","major","tick_values":[],"tick_texts":[]} for a value axis.
function ValueAxis-Json($ax) {
  $tl = $ax.TickLabels
  $log = $false; try { $log = ($ax.ScaleType -eq -4133) } catch {}
  $fmt = [string]$tl.NumberFormat
  $vals = Value-Ticks $ax
  $texts = $vals | ForEach-Object { Js (Fmt-Text $_ $fmt) }
  $rec = '{"number_format":' + (Js $fmt) + ',"linked":' + (Jb ([bool]$tl.NumberFormatLinked)) + ',"scale":"' + $(if ($log) { 'log' } else { 'linear' }) + '"'
  $rec += ',"min":' + (Num $ax.MinimumScale) + ',"max":' + (Num $ax.MaximumScale) + ',"major":' + (Num $ax.MajorUnit)
  if ($log) { $rec += ',"log_base":' + (Num $ax.LogBase) }
  $rec += ',"tick_values":' + (Arr $vals) + ',"tick_texts":[' + ($texts -join ',') + ']}'
  return $rec
}

# One rectangle per point for every series: DataLabel.Left/Top/Width/Height in chart points plus its text.
function DataLabels-Json($ch) {
  $all = New-Object System.Collections.ArrayList
  $ns = $ch.SeriesCollection().Count
  for ($s = 1; $s -le $ns; $s++) {
    $sr = $ch.SeriesCollection($s)
    $vals = @($sr.Values)
    $np = $vals.Count
    $pts = New-Object System.Collections.ArrayList
    for ($p = 1; $p -le $np; $p++) {
      try {
        $dl = $sr.Points($p).DataLabel
        [void]$pts.Add(('{{"value":{0},"text":{1},"left":{2},"top":{3},"width":{4},"height":{5}}}' -f (Num $vals[$p - 1]), (Js $dl.Text), (Num $dl.Left), (Num $dl.Top), (Num $dl.Width), (Num $dl.Height)))
      } catch { [void]$pts.Add('null') }
    }
    [void]$all.Add('[' + ($pts -join ',') + ']')
  }
  return '[' + ($all -join ',') + ']'
}

# ---------------------------------------------------------------------------------------------------------
# labels
# ---------------------------------------------------------------------------------------------------------
$numFmts = 'General', '0', '0.0', '0.00', '0.000', '#,##0', '#,##0.00', '0%', '0.0%', '$#,##0', '$#,##0.00', '0.00E+00', '#,##0,"K"', '[Red]0.00', '0.0,,"M"', '#,##0;(#,##0)'
$posNames = @{ '-4108' = 'center'; '3' = 'inside_end'; '4' = 'inside_base'; '2' = 'outside_end'; '0' = 'above'; '1' = 'below'; '-4131' = 'left'; '-4152' = 'right'; '5' = 'best_fit' }
$labelTypes = @(
  @{ n = 'column'; t = 51; w = 5; pos = @(-4108, 3, 4, 2) }, @{ n = 'column_stacked'; t = 52; w = 2; pos = @(-4108, 3, 4) },
  @{ n = 'bar'; t = 57; w = 4; pos = @(-4108, 3, 4, 2) }, @{ n = 'bar_stacked'; t = 58; w = 2; pos = @(-4108, 3, 4) },
  @{ n = 'line'; t = 65; w = 5; pos = @(-4108, -4131, -4152, 0, 1) }, @{ n = 'scatter'; t = -4169; w = 4; pos = @(-4108, -4131, -4152, 0, 1) },
  @{ n = 'pie'; t = 5; w = 4; pos = @(-4108, 3, 2, 5) }
)

function Invoke-Labels($n) {
  $rng = New-Object System.Random ($Seed + 53)
  Init-FmtSheet
  $wb = New-Scratch
  $out = New-Object System.Collections.ArrayList
  try {
    $ws = $wb.Worksheets.Item(1)
    $i = 0; $attempts = 0
    while ($i -lt $n -and $attempts -lt $n * 3) {
      $attempts++
      $ty = Pick-Weighted $rng $labelTypes
      $pie = $ty.n -eq 'pie'; $scatter = $ty.n -eq 'scatter'; $stacked = $ty.n -like '*stacked'
      $w = $rng.Next(240, 641); $h = $rng.Next(160, 421)
      $ncat = $rng.Next(3, 9); $nser = $rng.Next(1, 4)
      if ($pie) { $nser = 1 }
      $log = ($rng.Next(0, 8) -eq 0) -and -not $pie -and -not $stacked
      $mag = [Math]::Pow(10, $rng.Next(-1, 6))
      $neg = ($rng.Next(0, 6) -eq 0) -and -not $pie -and -not $log
      $cellFmt = $numFmts[$rng.Next(0, $numFmts.Count)]
      $unlink = $rng.Next(0, 2) -eq 0
      $axFmt = $numFmts[$rng.Next(0, $numFmts.Count)]
      $lblUnlink = $rng.Next(0, 3) -eq 0
      $lblFmt = $numFmts[$rng.Next(0, $numFmts.Count)]
      $fs = $rng.Next(8, 15)
      $pos = $ty.pos[$rng.Next(0, $ty.pos.Count)]
      $pieMode = $rng.Next(0, 3)
      $legend = $rng.Next(0, 3); $hasTitle = $rng.Next(0, 3) -eq 0
      $cats = New-Object string[] $ncat
      $used = @{}
      for ($j = 0; $j -lt $ncat; $j++) {
        do { $c = $words[$rng.Next(0, $words.Count)] } while ($used.ContainsKey($c))
        $used[$c] = 1; $cats[$j] = $c
      }
      $series = New-Object System.Collections.ArrayList
      for ($s = 0; $s -lt $nser; $s++) {
        $v = New-Object double[] $ncat
        for ($j = 0; $j -lt $ncat; $j++) {
          $x = $(if ($log) { 0.5 + $rng.NextDouble() * 9.5 } else { $rng.NextDouble() * 10 })
          if ($neg) { $x = $x - 4 }
          $v[$j] = Round-To ($x * $mag) 4
          if ($v[$j] -eq 0 -and ($log -or $pie)) { $v[$j] = $mag }
        }
        [void]$series.Add($v)
      }
      $xs = $null
      if ($scatter) { $xs = New-Object double[] $ncat; for ($j = 0; $j -lt $ncat; $j++) { $xs[$j] = Round-To ($rng.NextDouble() * 10 * $mag) 4 } }
      try {
        [void]$ws.Cells.Clear()
        $rows = New-Object System.Collections.ArrayList
        $head = New-Object System.Collections.ArrayList
        [void]$head.Add($(if ($scatter) { 'X' } else { '' }))
        for ($s = 0; $s -lt $nser; $s++) { [void]$head.Add("Series$($s + 1)") }
        [void]$rows.Add($head.ToArray())
        for ($j = 0; $j -lt $ncat; $j++) {
          $r = New-Object System.Collections.ArrayList
          [void]$r.Add($(if ($scatter) { [double]$xs[$j] } else { [string]$cats[$j] }))
          for ($s = 0; $s -lt $nser; $s++) { [void]$r.Add([double]$series[$s][$j]) }
          [void]$rows.Add($r.ToArray())
        }
        Write-Block $ws $rows.ToArray()
        $ws.Range($ws.Cells.Item(2, 2), $ws.Cells.Item($ncat + 1, $nser + 1)).NumberFormat = $cellFmt
        foreach ($old in @($ws.ChartObjects())) { $old.Delete() | Out-Null }
        $co = $ws.ChartObjects().Add(20, 20, $w, $h)
        $ch = $co.Chart
        $ch.ChartType = $ty.t
        $ch.SetSourceData($ws.Range($ws.Cells.Item(1, 1), $ws.Cells.Item($ncat + 1, $nser + 1)))
        if (-not $scatter) { $ch.PlotBy = 2 }
        $ch.ChartType = $ty.t
        $ch.ChartArea.Font.Name = 'Calibri'
        if ($hasTitle) { $ch.HasTitle = $true; $ch.ChartTitle.Text = 'Sales' } else { $ch.HasTitle = $false }
        if ($legend -eq 0) { $ch.HasLegend = $false } else { $ch.HasLegend = $true; $ch.Legend.Position = $(if ($legend -eq 1) { -4152 } else { -4107 }) }
        if (-not $pie) {
          if ($log) { $a = $ch.Axes(2); $a.ScaleType = -4133; $a.LogBase = 10 }
          if ($unlink) { $a = $ch.Axes(2); $a.TickLabels.NumberFormatLinked = $false; $a.TickLabels.NumberFormat = $axFmt }
        }
        for ($s = 1; $s -le $nser; $s++) {
          $sr = $ch.SeriesCollection($s)
          $sr.HasDataLabels = $true
          $dls = $sr.DataLabels()
          if ($pie) {
            if ($pieMode -eq 1) { $dls.ShowValue = $false; $dls.ShowPercentage = $true }
            elseif ($pieMode -eq 2) { $dls.ShowCategoryName = $true; $dls.ShowPercentage = $true }
          }
          $dls.Position = $pos
          if ($lblUnlink) { $dls.NumberFormatLinked = $false; $dls.NumberFormat = $lblFmt }
          $dls.Font.Size = $fs
        }
        $ch.Refresh()
        $dls1 = $ch.SeriesCollection(1).DataLabels()
        $posOk = ([int]$dls1.Position -eq $pos)
        $pa = $ch.PlotArea
        $rec = '{{"type":"{0}","width":{1},"height":{2},"cats":{3},"series":{4},"xs":{5},"title":{6},"legend":{7},"cell_format":{8},' -f `
          $ty.n, $w, $h, (StrArr $cats), ('[' + (($series | ForEach-Object { Arr $_ }) -join ',') + ']'), $(if ($xs) { Arr $xs } else { 'null' }), (Jb $hasTitle), $legend, (Js $cellFmt)
        $rec += ('"inside":{0},"legend_rect":{1},"title_rect":{2},' -f ('{{"left":{0},"top":{1},"width":{2},"height":{3}}}' -f (Num $pa.InsideLeft), (Num $pa.InsideTop), (Num $pa.InsideWidth), (Num $pa.InsideHeight)), $(if ($ch.HasLegend) { Rect $ch.Legend } else { 'null' }), $(if ($ch.HasTitle) { Rect $ch.ChartTitle } else { 'null' }))
        $rec += ('"position":"{0}","position_code":{1},"position_read_back":{2},"position_accepted":{3},"label_font_size":{4},"label_number_format":{5},"label_linked":{6},"pie_content":{7},' -f `
          $posNames[[string]$pos], $pos, [int]$dls1.Position, (Jb $posOk), $fs, (Js $dls1.NumberFormat), (Jb ([bool]$dls1.NumberFormatLinked)), $(if ($pie) { '"' + @('value', 'percent', 'category_percent')[$pieMode] + '"' } else { 'null' }))
        if (-not $pie) {
          $rec += '"value_axis":' + (ValueAxis-Json $ch.Axes(2)) + ','
          if ($scatter) { $rec += '"x_axis":' + (ValueAxis-Json $ch.Axes(1)) + ',' }
          else { $cx = $ch.Axes(1); $rec += '"category_axis":{"number_format":' + (Js $cx.TickLabels.NumberFormat) + ',"linked":' + (Jb ([bool]$cx.TickLabels.NumberFormatLinked)) + '},' }
        }
        $rec += '"data_labels":' + (DataLabels-Json $ch) + '}'
        [void]$out.Add($rec)
        $i++
      } catch { Write-Output "labels case $attempts skipped: $($_.Exception.Message)" }
      if ($attempts % 50 -eq 0) { Write-Output "labels $i/$n" }
    }
  } finally { $wb.Saved = $true; $wb.Close($false) }
  $json = "{`n  `"oracle`": `"Microsoft Excel via COM, points, chart area origin; Calibri; tick_texts are Excel's number-format output for each tick value (cell .Text), the rendered tick text is not exposed; data_labels are DataLabel.Left/Top/Width/Height per series per point`",`n  `"excel_build`": `"$($script:xl.Build)`",`n  `"seed`": $Seed,`n  `"cases`": [`n    " + ($out -join ",`n    ") + "`n  ]`n}`n"
  Write-Lf (Join-Path $labelsDir "$Tag.json") $json
}

# ---------------------------------------------------------------------------------------------------------
# dates
# ---------------------------------------------------------------------------------------------------------
$dateFmts = 'yyyy-mm-dd', 'd-mmm', 'mmm-yy', 'dd/mm/yyyy', 'mmm d', 'yyyy', 'mmmm yyyy', 'm/d/yy', 'd mmm yyyy', 'ddd d'

function Add-Unit([datetime]$d, [int]$n, [int]$scale) {
  switch ($scale) { 0 { return $d.AddDays($n) } 1 { return $d.AddMonths($n) } default { return $d.AddYears($n) } }
}
function Unit-Index([double]$serial, [int]$base) {
  $d = [DateTime]::FromOADate($serial)
  switch ($base) { 0 { return [Math]::Floor($serial) } 1 { return [double]($d.Year * 12 + $d.Month - 1) } default { return [double]$d.Year } }
}

function Invoke-Dates($n) {
  $rng = New-Object System.Random ($Seed + 71)
  Init-FmtSheet
  $wb = New-Scratch
  $out = New-Object System.Collections.ArrayList
  try {
    $ws = $wb.Worksheets.Item(1)
    $i = 0; $attempts = 0
    while ($i -lt $n -and $attempts -lt $n * 3) {
      $attempts++
      $pattern = @('daily', 'weekly', 'monthly', 'quarterly', 'yearly', 'irregular')[$rng.Next(0, 6)]
      $kind = @('line', 'column')[$rng.Next(0, 2)]
      $w = @(360, 240, 480, 600)[$rng.Next(0, 4)]
      $m = $rng.Next(4, 41)
      if ($pattern -eq 'yearly') { $m = $rng.Next(3, 16) }
      $cellFmt = $dateFmts[$rng.Next(0, $dateFmts.Count)]
      $unlink = $rng.Next(0, 2) -eq 0
      $axFmt = $dateFmts[$rng.Next(0, $dateFmts.Count)]
      $forceDays = ($rng.Next(0, 7) -eq 0)
      $d = ([datetime]'2015-01-01').AddDays($rng.Next(0, 3650))
      $dates = New-Object System.Collections.ArrayList
      for ($j = 0; $j -lt $m; $j++) {
        [void]$dates.Add($d)
        switch ($pattern) {
          'daily' { $d = $d.AddDays(1) } 'weekly' { $d = $d.AddDays(7) } 'monthly' { $d = $d.AddMonths(1) }
          'quarterly' { $d = $d.AddMonths(3) } 'yearly' { $d = $d.AddYears(1) } 'irregular' { $d = $d.AddDays($rng.Next(1, 40)) }
        }
      }
      $serials = @($dates | ForEach-Object { [double]$_.ToOADate() })
      $vals = 1..$m | ForEach-Object { Round-To ($rng.NextDouble() * 100) 2 }
      try {
        [void]$ws.Cells.Clear()
        $rows = New-Object System.Collections.ArrayList
        for ($j = 0; $j -lt $m; $j++) { [void]$rows.Add(@([double]$serials[$j], [double]$vals[$j])) }
        Write-Block $ws $rows.ToArray()
        $ws.Range($ws.Cells.Item(1, 1), $ws.Cells.Item($m, 1)).NumberFormat = $cellFmt
        foreach ($old in @($ws.ChartObjects())) { $old.Delete() | Out-Null }
        $co = $ws.ChartObjects().Add(10, 10, $w, 216)
        $ch = $co.Chart
        $ch.ChartType = $(if ($kind -eq 'column') { 51 } else { 65 })
        $ch.SetSourceData($ws.Range($ws.Cells.Item(1, 2), $ws.Cells.Item($m, 2)))
        $ch.SeriesCollection(1).XValues = $ws.Range($ws.Cells.Item(1, 1), $ws.Cells.Item($m, 1))
        $ch.ChartArea.Font.Name = 'Calibri'
        $ch.HasLegend = $false; $ch.HasTitle = $false
        $x = $ch.Axes(1)
        $x.CategoryType = 3
        if ($forceDays) { $x.BaseUnit = 0 }
        if ($unlink) { $x.TickLabels.NumberFormatLinked = $false; $x.TickLabels.NumberFormat = $axFmt }
        $sr = $ch.SeriesCollection(1)
        $sr.HasDataLabels = $true
        $sr.DataLabels().Position = $(if ($kind -eq 'column') { -4108 } else { 0 })
        $ch.Refresh()
        $x = $ch.Axes(1)
        $pa = $ch.PlotArea
        $isTime = ($x.CategoryType -eq 3)
        $rec = '{{"kind":"{0}","pattern":"{1}","width":{2},"dates":{3},"cell_format":{4},"base_unit_forced_days":{5},"category_type":{6},"inside":{7}' -f `
          $kind, $pattern, $w, (Arr $serials), (Js $cellFmt), (Jb $forceDays), $x.CategoryType, ('{{"left":{0},"top":{1},"width":{2},"height":{3}}}' -f (Num $pa.InsideLeft), (Num $pa.InsideTop), (Num $pa.InsideWidth), (Num $pa.InsideHeight))
        if ($isTime) {
          $min = [double]$x.MinimumScale; $max = [double]$x.MaximumScale
          $major = [int]$x.MajorUnit; $ms = [int]$x.MajorUnitScale; $base = [int]$x.BaseUnit
          $fmt = [string]$x.TickLabels.NumberFormat
          $d0 = [DateTime]::FromOADate($min)
          $tk = New-Object System.Collections.ArrayList
          for ($k = 0; $k -lt 400; $k++) { $t = (Add-Unit $d0 ($k * $major) $ms).ToOADate(); if ($t -gt $max + 1e-9) { break }; [void]$tk.Add([double]$t) }
          $texts = $tk | ForEach-Object { Js (Fmt-Text $_ $fmt) }
          # Point centres from the data labels; position is linear in the base unit, so fit x = a + b*u.
          $px = New-Object System.Collections.ArrayList; $pu = New-Object System.Collections.ArrayList
          for ($p = 1; $p -le $m; $p++) {
            $dl = $sr.Points($p).DataLabel
            [void]$px.Add([double]($dl.Left + $dl.Width / 2)); [void]$pu.Add((Unit-Index $serials[$p - 1] $base))
          }
          $sx = 0.0; $sy = 0.0; $sxx = 0.0; $sxy = 0.0
          for ($p = 0; $p -lt $m; $p++) { $sx += $pu[$p]; $sy += $px[$p]; $sxx += $pu[$p] * $pu[$p]; $sxy += $pu[$p] * $px[$p] }
          $den = $m * $sxx - $sx * $sx
          $b = $(if ($den -ne 0) { ($m * $sxy - $sx * $sy) / $den } else { 0.0 })
          $a = ($sy - $b * $sx) / $m
          $res = 0.0; for ($p = 0; $p -lt $m; $p++) { $e = [Math]::Abs($a + $b * $pu[$p] - $px[$p]); if ($e -gt $res) { $res = $e } }
          $tx = $tk | ForEach-Object { $a + $b * (Unit-Index $_ $base) }
          $il = [double]$pa.InsideLeft
          $rec += (',"axis":{{"min":{0},"max":{1},"major":{2},"major_scale":{3},"minor":{4},"minor_scale":{5},"base_unit":{6},"base_unit_auto":{7},"between_categories":{8},"number_format":{9},"linked":{10},"left":{11},"width":{12}}}' -f `
              (Num $min), (Num $max), $major, $ms, (Num $x.MinorUnit), $x.MinorUnitScale, $base, (Jb ([bool]$x.BaseUnitIsAuto)), (Jb ([bool]$x.AxisBetweenCategories)), (Js $fmt), (Jb ([bool]$x.TickLabels.NumberFormatLinked)), (Num $x.Left), (Num $x.Width))
          $rec += ',"ticks":{"serials":' + (Arr $tk) + ',"texts":[' + ($texts -join ',') + '],"x_pt":' + (Arr $tx) + ',"x_inside_pt":' + (Arr ($tx | ForEach-Object { $_ - $il })) + '}'
          $rec += ',"points":{"x_centre_pt":' + (Arr $px) + '}'
          $rec += (',"fit":{{"intercept":{0},"pt_per_base_unit":{1},"max_residual":{2},"note":"x_pt = intercept + pt_per_base_unit * unit_index(date); unit_index counts days, months or years per base_unit; x is chart-area points of the data-label centre"}}' -f (Num $a), (Num $b), (Num $res))
        }
        $rec += '}'
        [void]$out.Add($rec)
        $i++
      } catch { Write-Output "dates case $attempts skipped: $($_.Exception.Message)" }
      if ($attempts % 50 -eq 0) { Write-Output "dates $i/$n" }
    }
  } finally { $wb.Saved = $true; $wb.Close($false) }
  $json = "{`n  `"oracle`": `"Microsoft Excel via COM, Axes(xlCategory) time scale; tick serials are min + k*major in major_scale units (0 days, 1 months, 2 years), tick_texts are Excel's number-format output, x_pt are fitted from DataLabel centres`",`n  `"excel_build`": `"$($script:xl.Build)`",`n  `"seed`": $Seed,`n  `"cases`": [`n    " + ($out -join ",`n    ") + "`n  ]`n}`n"
  Write-Lf (Join-Path $datesDir "$Tag.json") $json
}

try {
  Start-Excel
  if ($Set -in 'layout', 'all') { Invoke-Layout $(if ($Count) { $Count } else { 1500 }) }
  if ($Set -in 'log', 'all') { Invoke-Log $(if ($Count) { $Count } else { 300 }) }
  if ($Set -in 'lognarrow', 'all') { Invoke-Log $(if ($Count) { $Count } else { 300 }) $true }
  if ($Set -in 'date', 'all') { Invoke-Date $(if ($Count) { $Count } else { 300 }) }
  $lc = $(if ($Count) { $Count } elseif ($Seed -eq 1001) { 200 } else { 100 })
  if ($Set -eq 'labels') { Invoke-Labels $lc }
  if ($Set -eq 'dates') { Invoke-Dates $lc }
} finally {
  Stop-Excel
}
