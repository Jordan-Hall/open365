<#
Controlled probes of how Excel lays user border formats over a PivotTable (area outline rules, shared edges, conflicts with
the table style). Writes fixtures/reference/xlsx/pivot-formats-probe/:
  probe.xlsx   the pivots, formats as Excel stored them (formats/format/pivotArea + dxfs)
  probe.json   per pivot: case, ops, range and for EVERY cell of TableRange2 (+1 row and column, empty cells too): address,
               value, disp = Range.DisplayFormat borders [left, top, bottom, right] "<style>:<weight>:<bgr>" or "",
               xf = the cell's own borders (Range.Borders) in the same notation
LOCAL DEVELOPER TOOL ONLY (not CI). Own Excel instance only (aborts when COM attached to one already running); prints the
PID of that instance; AutomationSecurity = 3; Quit in finally. Other EXCEL processes are never touched. Data is synthetic.
Usage: pwsh -NoProfile -File make-pivot-formats-probe.ps1 [-Only regex-on-case-description] [-Tag name]  (writes <Tag>.xlsx and <Tag>.json)
#>
param([string]$Only, [string]$Tag = 'probe')
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class PvProbeWin { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p); }
'@
$before = @(Get-Process EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$xl = New-Object -ComObject Excel.Application
$procId = [uint32]0
[void][PvProbeWin]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$procId)
if ($before -contains [int]$procId) { Write-Host 'attached to a pre-existing Excel instance; aborting without touching it'; exit 1 }
Write-Host "EXCEL PID $procId"
$xlPid = [int]$procId

function Json-Str([string]$s) {
  $sb = New-Object Text.StringBuilder; [void]$sb.Append('"')
  foreach ($ch in $s.ToCharArray()) {
    $c = [int]$ch
    if ($ch -eq '"') { [void]$sb.Append('\"') } elseif ($ch -eq '\') { [void]$sb.Append('\\') }
    elseif ($c -lt 32 -or $c -gt 126) { [void]$sb.Append(('\u{0:x4}' -f $c)) } else { [void]$sb.Append($ch) }
  }
  [void]$sb.Append('"'); return $sb.ToString()
}
function Write-Lf($path, $text) {
  New-Item -ItemType Directory -Force (Split-Path -Parent $path) | Out-Null
  [IO.File]::WriteAllText($path, ($text -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
}
function Col-Letters([int]$c) { $s = ''; while ($c -gt 0) { $m = ($c - 1) % 26; $s = [string][char](65 + $m) + $s; $c = [int](($c - $m - 1) / 26) }; return $s }
$ErrNames = @{ -2146826281 = '#DIV/0!'; -2146826246 = '#N/A'; -2146826259 = '#NAME?'; -2146826288 = '#NULL!'
  -2146826252 = '#NUM!'; -2146826265 = '#REF!'; -2146826273 = '#VALUE!' }
function Val-Json($x, $cell) {
  if ($x -is [string]) { return (Json-Str $x) }
  if ($x -is [int]) { $en = $ErrNames[[int]$x]; if (-not $en) { $en = [string]$cell.Text }; return ('{"e":' + (Json-Str $en) + '}') }
  return ([double]$x).ToString('R', $inv)
}
$script:rs = [long]1
function Next-Rand { $script:rs = ($script:rs * 1103515245 + 12345) % 2147483648; return [long]($script:rs / 65536) }
$Headers = 'Region', 'Product', 'Channel', 'Date', 'Units', 'Price', 'Revenue', 'Score', 'Tier', 'Size'
function New-Data($wb, [int]$n, [long]$seed) {
  $script:rs = $seed
  $ws = $wb.Worksheets.Item(1); $ws.Name = 'Data'
  for ($i = 0; $i -lt 10; $i++) { $ws.Cells.Item(1, ($i + 1)).Value2 = $Headers[$i] }
  $reg = 'North', 'South', 'East', 'West'
  $prod = 'Walnut', 'apple', 'Banana', 'Cherry', 'Fig', 'Date'
  $chn = 'Online', 'Retail', 'Partner'
  for ($i = 0; $i -lt $n; $i++) {
    $row = $i + 2
    $r = Next-Rand
    if (($r % 20) -ne 0) { $ws.Cells.Item($row, 1).Value2 = [string]$reg[$r % 4] }
    $ws.Cells.Item($row, 2).Value2 = [string]$prod[(Next-Rand) % 6]
    $ws.Cells.Item($row, 3).Value2 = [string]$chn[(Next-Rand) % 3]
    $ws.Cells.Item($row, 4).Value2 = [double](45292 + (Next-Rand) % 731)
    $hasU = ((Next-Rand) % 30) -ne 0
    $u = [double](1 + (Next-Rand) % 50)
    if ($hasU) { $ws.Cells.Item($row, 5).Value2 = [double]$u }
    $p = [double]([Math]::Round(1 + ((Next-Rand) % 9900) / 100.0, 2))
    $ws.Cells.Item($row, 6).Value2 = [double]$p
    if ($hasU) { $ws.Cells.Item($row, 7).Value2 = [double]([Math]::Round($u * $p, 2)) }
    $ws.Cells.Item($row, 8).Value2 = [double]((Next-Rand) % 101)
    $ws.Cells.Item($row, 9).Value2 = [string]('Gold', 'Silver', 'Bronze')[(Next-Rand) % 3]
    $ws.Cells.Item($row, 10).Value2 = [string]('S', 'L')[(Next-Rand) % 2]
  }
  $ws.Range($ws.Cells.Item(2, 4), $ws.Cells.Item(($n + 1), 4)).NumberFormat = 'dd/mm/yyyy'
  $ws.Range($ws.Cells.Item(2, 6), $ws.Cells.Item(($n + 1), 6)).NumberFormat = '0.00'
  $ws.Range($ws.Cells.Item(2, 7), $ws.Cells.Item(($n + 1), 7)).NumberFormat = '#,##0.00'
}
$FnCode = @{ sum = -4157; count = -4112; average = -4106; max = -4136; min = -4139 }
$FnName = @{ sum = 'Sum'; count = 'Count'; average = 'Average'; max = 'Max'; min = 'Min' }


$FnCode = @{ sum = -4157; count = -4112 }
$Weights = @{ thin = 2; medium = -4138; thick = 4; hair = 1 }
function Edge-Range($pt, $o) {
  switch ($o.where) {
    'databody' { return ,$pt.DataBodyRange }
    'rowrange' { return ,$pt.RowRange }
    'table1' { return ,$pt.TableRange1 }
    'itemlabel' { return ,$pt.PivotFields($o.f).PivotItems($o.item).LabelRange }
    'itemdata' { return ,$pt.PivotFields($o.f).PivotItems($o.item).DataRange }
    'fieldlabel' { return ,$pt.PivotFields($o.f).LabelRange }
    'fielddata' { return ,$pt.PivotFields($o.f).DataRange }
    'cell' { return ,$pt.DataBodyRange.Cells.Item($o.r, $o.c) }
  }
}
function Apply-Op($pt, $o) {
  $r = (Edge-Range $pt $o)
  $w = $Weights[$o.weight]; if (-not $w) { $w = 2 }
  if ($o.edge -eq 'around') { [void]$r.BorderAround(1, $w, 0, [int]$o.color); return }
  foreach ($e in @($o.edge)) { $b = $r.Borders.Item([int]$e); $b.LineStyle = 1; $b.Weight = $w; $b.Color = [int]$o.color }
}
function BD($b) { $ls = $b.LineStyle; if ($ls -eq -4142) { return '""' } return (Json-Str ("${ls}:$($b.Weight):$([int]$b.Color)")) }
function Dump-Probe($pt, $c, [string]$tag) {
  $tr = $pt.TableRange2; $ws = $pt.Parent; $cells = New-Object Collections.ArrayList
  $r1 = $tr.Row; $c1 = $tr.Column; $nr = $tr.Rows.Count + 1; $nc = $tr.Columns.Count + 1
  for ($i = 0; $i -lt $nr; $i++) { for ($j = 0; $j -lt $nc; $j++) {
    $cell = $ws.Cells.Item($r1 + $i, $c1 + $j)
    $d = $cell.DisplayFormat
    $disp = @(); $xf = @()
    foreach ($e in 7, 8, 9, 10) { $disp += (BD $d.Borders.Item($e)); $xf += (BD $cell.Borders.Item($e)) }
    $v = $cell.Value2; $vj = 'null'; if ($null -ne $v) { $vj = Json-Str ([string]$v) }
    [void]$cells.Add('{"a":"' + (Col-Letters ($c1 + $j)) + ($r1 + $i) + '","v":' + $vj + ',"disp' + $tag + '":[' + ($disp -join ',') + '],"xf' + $tag + '":[' + ($xf -join ',') + ']}')
  } }
  $ops = ($c.ops | ForEach-Object { Json-Str (($_.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join ' ') }) -join ','
  return '    {"sheet":' + (Json-Str $ws.Name) + ',"case":' + (Json-Str $c.desc) + ',"style":' + (Json-Str $c.style) + ',"ops":[' + $ops + '],"range":' + (Json-Str $tr.Address($false, $false)) + ",`n     `"cells`":[`n      " + ($cells -join ",`n      ") + "`n     ]}"
}
function New-Probe($wb, $pc, [string]$name, $c) {
  $sh = $wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)); $sh.Name = $name
  $pt = $pc.CreatePivotTable("'$name'!R4C2", "PT$name")
  $pt.ManualUpdate = $true
  foreach ($f in @($c.rows)) { $pt.PivotFields($f).Orientation = 1 }
  foreach ($f in @($c.cols)) { if ($f) { $pt.PivotFields($f).Orientation = 2 } }
  $pt.ManualUpdate = $false
  if ($c.layout) { $pt.RowAxisLayout($(@{ compact = 0; tabular = 1; outline = 2 }[$c.layout])) }
  [void]$pt.AddDataField($pt.PivotFields('Units'), 'Sum of Units', -4157)
  $pt.TableStyle2 = $c.style
  foreach ($o in @($c.ops)) { Apply-Op $pt $o }
  return $pt
}
function O($where, $edge, $weight, $color, $f = '', $item = '') { return @{ where = $where; edge = $edge; weight = $weight; color = $color; f = $f; item = $item } }
$Red = 255; $Blue = 16711680; $Green = 65280
$L = 7; $T = 8; $B = 9; $R = 10
function Cases {
  $c = @()
  foreach ($style in 'PivotStyleLight16', 'PivotStyleMedium5') {
    $s = $style.Substring(10)
    $c += @{ desc = "$s databody top thin"; style = $style; rows = @('Region'); ops = @((O 'databody' $T 'thin' $Red)) }
    $c += @{ desc = "$s databody top thick"; style = $style; rows = @('Region'); ops = @((O 'databody' $T 'thick' $Red)) }
    $c += @{ desc = "$s databody left thin"; style = $style; rows = @('Region'); ops = @((O 'databody' $L 'thin' $Red)) }
    $c += @{ desc = "$s databody bottom thin"; style = $style; rows = @('Region'); ops = @((O 'databody' $B 'thin' $Red)) }
    $c += @{ desc = "$s databody right thick"; style = $style; rows = @('Region'); ops = @((O 'databody' $R 'thick' $Red)) }
    $c += @{ desc = "$s interior top of North data"; style = $style; rows = @('Region'); ops = @((O 'itemdata' $T 'thin' $Red 'Region' 'North')) }
    $c += @{ desc = "$s interior bottom of North data thick"; style = $style; rows = @('Region'); ops = @((O 'itemdata' $B 'thick' $Red 'Region' 'North')) }
    $c += @{ desc = "$s North label right and data left"; style = $style; rows = @('Region'); ops = @((O 'itemlabel' $R 'thin' $Red 'Region' 'North'), (O 'itemdata' $L 'thin' $Blue 'Region' 'North')) }
    $c += @{ desc = "$s edge East bottom thin red then North top thick blue"; style = $style; rows = @('Region'); ops = @((O 'itemdata' $B 'thin' $Red 'Region' 'East'), (O 'itemdata' $T 'thick' $Blue 'Region' 'North')) }
    $c += @{ desc = "$s edge East bottom thick red then North top thin blue"; style = $style; rows = @('Region'); ops = @((O 'itemdata' $B 'thick' $Red 'Region' 'East'), (O 'itemdata' $T 'thin' $Blue 'Region' 'North')) }
    $c += @{ desc = "$s edge North top thin blue then East bottom thin red"; style = $style; rows = @('Region'); ops = @((O 'itemdata' $T 'thin' $Blue 'Region' 'North'), (O 'itemdata' $B 'thin' $Red 'Region' 'East')) }
    $c += @{ desc = "$s edge East bottom thin red then North top thin blue"; style = $style; rows = @('Region'); ops = @((O 'itemdata' $B 'thin' $Red 'Region' 'East'), (O 'itemdata' $T 'thin' $Blue 'Region' 'North')) }
    $c += @{ desc = "$s rowrange right"; style = $style; rows = @('Region'); ops = @((O 'rowrange' $R 'thin' $Green)) }
    $c += @{ desc = "$s table1 around medium"; style = $style; rows = @('Region'); ops = @((O 'table1' 'around' 'medium' $Green)) }
    $c += @{ desc = "$s North data around"; style = $style; rows = @('Region'); ops = @((O 'itemdata' 'around' 'thin' $Blue 'Region' 'North')) }
    $c += @{ desc = "$s fieldlabel bottom thick"; style = $style; rows = @('Region'); ops = @((O 'fieldlabel' $B 'thick' $Green 'Region')) }
    $c += @{ desc = "$s databody inside horizontal"; style = $style; rows = @('Region'); ops = @((O 'databody' 12 'thin' $Red)) }
    $c += @{ desc = "$s databody inside vertical, 2 cols"; style = $style; rows = @('Region'); cols = @('Channel'); ops = @((O 'databody' 11 'thin' $Red)) }
  }
  $c += @{ desc = 'replica databody four edges'; style = 'PivotStyleLight16'; rows = @('Region'); ops = @((O 'databody' $L 'thin' $Green), (O 'databody' $T 'thin' $Green), (O 'databody' $B 'thin' $Green), (O 'databody' $R 'thin' $Green)) }
  $c += @{ desc = 'replica databody four edges and a cell'; style = 'PivotStyleLight16'; rows = @('Region'); ops = @((O 'databody' $L 'thin' $Green), (O 'databody' $T 'thin' $Green), (O 'databody' $B 'thin' $Green), (O 'databody' $R 'thin' $Green), @{ where = 'cell'; r = 2; c = 1; edge = 'around'; weight = 'thick'; color = 0 }) }
  $c += @{ desc = 'databody top thin green only'; style = 'PivotStyleLight16'; rows = @('Region'); ops = @((O 'databody' $T 'thin' $Green)) }
  $c += @{ desc = 'outline rowrange right'; style = 'PivotStyleLight16'; layout = 'outline'; rows = @('Tier', 'Region'); ops = @((O 'rowrange' $R 'thin' $Red)) }
  $c += @{ desc = 'outline Tier Bronze label bottom'; style = 'PivotStyleLight16'; layout = 'outline'; rows = @('Tier', 'Region'); ops = @((O 'itemlabel' $B 'thin' $Red 'Tier' 'Bronze')) }
  $c += @{ desc = 'outline Region field data right'; style = 'PivotStyleLight16'; layout = 'outline'; rows = @('Tier', 'Region'); ops = @((O 'fielddata' $R 'thin' $Red 'Region')) }
  $c += @{ desc = 'outline Tier field data right'; style = 'PivotStyleLight16'; layout = 'outline'; rows = @('Tier', 'Region'); ops = @((O 'fielddata' $R 'thin' $Red 'Tier')) }
  $c += @{ desc = 'tabular Tier Bronze data around'; style = 'PivotStyleLight16'; layout = 'tabular'; rows = @('Tier', 'Region'); ops = @((O 'itemdata' 'around' 'thin' $Red 'Tier' 'Bronze')) }
  $c += @{ desc = 'tabular Tier Bronze label left'; style = 'PivotStyleLight16'; layout = 'tabular'; rows = @('Tier', 'Region'); ops = @((O 'itemlabel' $L 'thin' $Red 'Tier' 'Bronze')) }
  $c += @{ desc = 'outline databody around, column field'; style = 'PivotStyleLight16'; layout = 'outline'; rows = @('Tier', 'Region'); cols = @('Size'); ops = @((O 'databody' 'around' 'thin' $Red)) }
  return $c
}
try {
  $xl.AutomationSecurity = 3; $xl.Visible = $false; $xl.ScreenUpdating = $false; $xl.DisplayAlerts = $false; $xl.EnableEvents = $false
  $dir = Join-Path $refDir 'pivot-formats-probe'; New-Item -ItemType Directory -Force $dir | Out-Null
  $path = Join-Path $dir "$Tag.xlsx"; if (Test-Path $path) { Remove-Item $path -Force }
  $wb = $xl.Workbooks.Add(); $outs = New-Object Collections.ArrayList
  try {
    New-Data $wb 60 31
    $k = 0
    foreach ($c in (Cases)) {
      if ($Only -and $c.desc -notmatch $Only) { continue }
      $k++
      $pc = $wb.PivotCaches().Create(1, 'Data!R1C1:R61C10')
      try { $pt = New-Probe $wb $pc "P$k" $c; [void]$outs.Add((Dump-Probe $pt $c '')); $pt.PivotCache().Refresh() | Out-Null; $c2 = @{ desc = ($c.desc + ' [refreshed]'); style = $c.style; ops = $c.ops }; [void]$outs.Add((Dump-Probe $pt $c2 '')); Write-Host "P$k $($c.desc)" }
      catch { Write-Host "FAILED P$k $($c.desc): $($_.Exception.Message)"; throw }
    }
    $wb.Worksheets.Item('Data').Activate()
    $wb.RemovePersonalInformation = $true
    $wb.SaveAs($path, 51)
    $json = "{`n  `"oracle`": `"Microsoft Excel via COM; DisplayFormat and own borders of every cell of TableRange2 (+1 row/column)`",`n  `"excel_build`": `"$($xl.Build)`",`n  `"pivots`": [`n" + ($outs -join ",`n") + "`n  ]`n}`n"
    Write-Lf (Join-Path $dir "$Tag.json") $json
  } finally { try { $wb.Saved = $true; $wb.Close($false) } catch {} }
} finally {
  try { foreach ($w in @($xl.Workbooks)) { $w.Saved = $true; $w.Close($false) } } catch {}
  try { $xl.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl) } catch {}
  Start-Sleep -Seconds 2
  if (Get-Process -Id $xlPid -ErrorAction SilentlyContinue) { Stop-Process -Id $xlPid -Force }
}
