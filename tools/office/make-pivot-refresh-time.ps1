<#
Oracle for PivotTable refresh of time-of-day date groups (hours, minutes, seconds) and number groups over fractional
values. Same layout and method as make-pivot-refresh-ext.ps1; the Data sheet holds dates with a time of day. Per set it writes into fixtures/reference/xlsx/pivot-refresh-<set>/:
  before.xlsx  freshly built (formats applied / groups / calculated items added), caches as Excel wrote them
  after.xlsx   source edited (edits.json) and every cache refreshed by Excel
  edits.json   [{"a":"A5","v":"Central"|12.5|null}] applied to sheet Data
  pivot.json   per PivotTable: sheet, case, range, cells [{a,v,t,nf,f,b,i,c,bd}] after the refresh
               (f fill BGR or -1, b/i bold/italic, c font colour, bd [left, top, bottom, right] "<style>:<weight>:<bgr>" or "")
LOCAL DEVELOPER TOOL ONLY (not CI). Own Excel instance only (aborts when COM attached to one already running); prints the
PID of that instance; AutomationSecurity = 3; Quit in finally. Other EXCEL processes are never touched. Data is synthetic.
Usage: pwsh -NoProfile -File make-pivot-refresh-time.ps1 [-Only regex]
#>
param([string]$Only)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class PvTimeWin { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p); }
'@
$before = @(Get-Process EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$xl = New-Object -ComObject Excel.Application
$procId = [uint32]0
[void][PvTimeWin]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$procId)
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
$script:noTime = $false
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
    $tod = ((Next-Rand) % 86400) / 86400.0; if ($script:noTime) { $tod = 0 }
    $ws.Cells.Item($row, 4).Value2 = [double](45292 + (Next-Rand) % 731 + $tod)
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
  $ws.Range($ws.Cells.Item(2, 4), $ws.Cells.Item(($n + 1), 4)).NumberFormat = 'dd/mm/yyyy hh:mm:ss'
  $ws.Range($ws.Cells.Item(2, 6), $ws.Cells.Item(($n + 1), 6)).NumberFormat = '0.00'
  $ws.Range($ws.Cells.Item(2, 7), $ws.Cells.Item(($n + 1), 7)).NumberFormat = '#,##0.00'
}
$FnCode = @{ sum = -4157; count = -4112; average = -4106; max = -4136; min = -4139 }
$FnName = @{ sum = 'Sum'; count = 'Count'; average = 'Average'; max = 'Max'; min = 'Min' }

function Apply-Group($pt, $g) {
  $pf = $pt.PivotFields($g.f)
  $cell = $pf.DataRange.Cells.Item(1, 1)
  if ($g.kind -eq 'date') {
    $per = New-Object 'object[]' 7
    for ($i = 0; $i -lt 7; $i++) { $per[$i] = $false }
    foreach ($p in @($g.by)) { $per[@{ seconds = 0; minutes = 1; hours = 2; days = 3; months = 4; quarters = 5; years = 6 }[$p]] = $true }
    $by = [Type]::Missing; if ($g.ContainsKey('step')) { $by = [double]$g.step }
    $start = $true; $end = $true
    if ($g.ContainsKey('start')) { $start = [double]$g.start }
    if ($g.ContainsKey('end')) { $end = [double]$g.end }
    [void]$cell.Group($start, $end, $by, $per)
  } else {
    $start = $true; $end = $true
    if ($g.ContainsKey('start')) { $start = [double]$g.start }
    if ($g.ContainsKey('end')) { $end = [double]$g.end }
    [void]$cell.Group($start, $end, [double]$g.step)
  }
}

function Apply-Fmt($pt, $f) {
  $r = $null
  switch ($f.where) {
    'label' { $r = $pt.PivotFields($f.f).PivotItems($f.item).LabelRange }
    'itemdata' { $r = $pt.PivotFields($f.f).PivotItems($f.item).DataRange }
    'fieldlabel' { $r = $pt.PivotFields($f.f).LabelRange }
    'fielddata' { $r = $pt.PivotFields($f.f).DataRange }
    'databody' { $r = $pt.DataBodyRange }
    'rowrange' { $r = $pt.RowRange }
    'table1' { $r = $pt.TableRange1 }
    'cell' { $r = $pt.DataBodyRange.Cells.Item($f.r, $f.c) }
  }
  if ($f.ContainsKey('fill')) { $r.Interior.Color = [int]$f.fill }
  if ($f.ContainsKey('bold')) { $r.Font.Bold = $true }
  if ($f.ContainsKey('around')) { [void]$r.BorderAround(1, $f.weight, 0, [int]$f.around) }
  foreach ($e in @($f.edges)) { if ($e) { $b = $r.Borders.Item($e); $b.LineStyle = 1; $b.Weight = $f.weight; $b.Color = [int]$f.color } }
}

function New-RefPivot($wb, $pc, [string]$sheetName, [string]$ptName, $c) {
  $sh = $wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count))
  $sh.Name = $sheetName
  $top = 4 + $(if ($c.ContainsKey('pages')) { @($c.pages).Count + 2 } else { 0 })
  $pt = $pc.CreatePivotTable("'$sheetName'!R${top}C2", $ptName)
  $pt.ManualUpdate = $true
  foreach ($f in @($c.rows)) { if ($f) { $pt.PivotFields($f).Orientation = 1 } }
  foreach ($f in @($c.cols)) { if ($f) { $pt.PivotFields($f).Orientation = 2 } }
  foreach ($f in @($c.pages)) { if ($f) { $pt.PivotFields($f).Orientation = 3 } }
  $pt.ManualUpdate = $false
  if ($c.ContainsKey('layout')) { $pt.RowAxisLayout($(@{ compact = 0; tabular = 1; outline = 2 }[$c.layout])) }
  foreach ($d in @($c.data)) {
    $fn = 'sum'; if ($d.ContainsKey('fn')) { $fn = $d.fn }
    [void]$pt.AddDataField($pt.PivotFields($d.f), "$($FnName[$fn]) of $($d.f)", $FnCode[$fn])
  }
  foreach ($g in @($c.group)) { if ($g) { Apply-Group $pt $g } }
  foreach ($ci in @($c.calc)) { if ($ci) { [void]$pt.PivotFields($ci.f).CalculatedItems().Add($ci.name, $ci.formula) } }
  foreach ($f in @($c.formats)) { if ($f) { Apply-Fmt $pt $f } }
  return $pt
}

function Dump-Pivot($pt, $c) {
  $tr = $pt.TableRange2
  $ws = $pt.Parent
  $cells = New-Object Collections.ArrayList
  foreach ($cell in $tr.Cells) {
    $x = $cell.Value2
    if ($null -eq $x) { continue }
    $d = $cell.DisplayFormat
    $fill = -1; if ($d.Interior.Pattern -ne -4142) { $fill = [int]$d.Interior.Color }
    $bd = @()
    foreach ($e in 7, 8, 9, 10) {
      $b = $d.Borders.Item($e); $ls = $b.LineStyle
      if ($ls -eq -4142) { $bd += '""' } else { $bd += (Json-Str ("${ls}:$($b.Weight):$([int]$b.Color)")) }
    }
    $addr = (Col-Letters $cell.Column) + $cell.Row
    [void]$cells.Add('{"a":"' + $addr + '","v":' + (Val-Json $x $cell) + ',"nf":' + (Json-Str ([string]$cell.NumberFormat)) + ',"t":' + (Json-Str ([string]$cell.Text)) +
      ',"f":' + $fill + ',"b":' + [int][bool]$d.Font.Bold + ',"i":' + [int][bool]$d.Font.Italic + ',"c":' + [int]$d.Font.Color + ',"bd":[' + ($bd -join ',') + ']}')
  }
  return '    {"sheet":' + (Json-Str $ws.Name) + ',"name":' + (Json-Str $pt.Name) + ',"case":' + (Json-Str $c.desc) +
    ',"range":' + (Json-Str $tr.Address($false, $false)) + ",`n     `"cells`":[`n      " + ($cells -join ",`n      ") + "`n     ]}"
}
function Edit-Json($e) {
  $v = $e.v
  if ($null -eq $v) { $vj = 'null' } elseif ($v -is [string]) { $vj = Json-Str $v } else { $vj = ([double]$v).ToString('R', $inv) }
  return '{"a":' + (Json-Str $e.a) + ',"v":' + $vj + '}'
}

function Write-RefSet($setName, [int]$rows, [long]$seed, $cases, $edits) {
  if ($Only -and $setName -notmatch $Only) { return }
  $dir = Join-Path $refDir "pivot-refresh-$setName"
  $path = Join-Path $dir 'before.xlsx'; $afterPath = Join-Path $dir 'after.xlsx'
  New-Item -ItemType Directory -Force $dir | Out-Null
  foreach ($p in $path, $afterPath) { if (Test-Path $p) { Remove-Item $p -Force } }
  $wb = $xl.Workbooks.Add()
  $outs = New-Object Collections.ArrayList
  try {
    New-Data $wb $rows $seed
    $k = 0
    foreach ($c in $cases) {
      $k++
      $pc = $wb.PivotCaches().Create(1, "Data!R1C1:R$($rows + 1)C10")
      try { [void](New-RefPivot $wb $pc "P$k" "PT$k" $c); Write-Host "pivot-refresh-$setName P$k $($c.desc)" }
      catch { Write-Host "FAILED $setName P$k $($c.desc): $($_.Exception.Message)"; throw }
    }
    $wb.Worksheets.Item('Data').Activate()
    $wb.RemovePersonalInformation = $true
    $wb.SaveAs($path, 51)
    if ($edits -is [scriptblock]) { $edits = @(& $edits $wb.Worksheets.Item('Data') $rows) }
    $ws = $wb.Worksheets.Item('Data')
    foreach ($e in $edits) { $m = [regex]::Match($e.a, '^([A-Z]+)([0-9]+)$'); $cn = 0; foreach ($ch in $m.Groups[1].Value.ToCharArray()) { $cn = $cn * 26 + ([int]$ch - 64) }; $cell = $ws.Cells.Item([int]$m.Groups[2].Value, $cn); if ($null -eq $e.v) { [void]$cell.ClearContents() } elseif ($e.v -is [string]) { $cell.Value2 = [string]$e.v } else { $cell.Value2 = [double]$e.v } }
    $k = 0
    foreach ($c in $cases) {
      $k++
      $pt = $wb.Worksheets.Item("P$k").PivotTables("PT$k")
      $pt.PivotCache().Refresh() | Out-Null
      [void]$outs.Add((Dump-Pivot $pt $c))
    }
    $wb.RemovePersonalInformation = $true
    $wb.SaveAs($afterPath, 51)
    $json = "{`n  `"oracle`": `"Microsoft Excel via COM; TableRange2 cells and DisplayFormat after editing the source and refreshing`",`n  `"excel_build`": `"$($xl.Build)`",`n  `"pivots`": [`n" + ($outs -join ",`n") + "`n  ]`n}`n"
    Write-Lf (Join-Path $dir 'pivot.json') $json
    Write-Lf (Join-Path $dir 'edits.json') ("[`n" + (($edits | ForEach-Object { '  ' + (Edit-Json $_) }) -join ",`n") + "`n]`n")
  } finally {
    try { $wb.Saved = $true; $wb.Close($false) } catch {}
  }
}

function EdgeCases {
  return @(
    @{ desc = 'date by hours'; rows = @('Date'); data = @(D 'Revenue'); group = @(@{ f = 'Date'; kind = 'date'; by = @('hours') }) },
    @{ desc = 'date by months'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('months') }) },
    @{ desc = 'date by days step 7'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('days'); step = 7 }) }
  )
}
function D($f, $fn = 'sum') { return @{ f = $f; fn = $fn } }
function E($a, $v) { return @{ a = $a; v = $v } }
$Red = 255; $Blue = 16711680; $Green = 65280; $Yellow = 65535
function TimeCases {
  return @(
    @{ desc = 'date by hours'; rows = @('Date'); data = @(D 'Revenue'); group = @(@{ f = 'Date'; kind = 'date'; by = @('hours') }) },
    @{ desc = 'date by minutes'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('minutes') }) },
    @{ desc = 'date by seconds'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('seconds') }) },
    @{ desc = 'date by days and hours'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('days', 'hours') }) },
    @{ desc = 'date by hours and minutes'; rows = @('Date'); data = @(D 'Revenue'); group = @(@{ f = 'Date'; kind = 'date'; by = @('hours', 'minutes') }) },
    @{ desc = 'date by months and hours'; rows = @('Date'); cols = @('Channel'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('months', 'hours') }) },
    @{ desc = 'date by hours explicit bounds'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('hours'); start = 45400.25; end = 45900.5 }) },
    @{ desc = 'date by days step 7 with hours'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('days'); step = 7 }) },
    @{ desc = 'date by days alone'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('days') }) },
    @{ desc = 'date by months and days'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('months', 'days') }) }
  )
}
function FractionCases {
  return @(
    @{ desc = 'price by 10 auto bounds'; rows = @('Price'); data = @(D 'Units'); group = @(@{ f = 'Price'; kind = 'number'; step = 10 }) },
    @{ desc = 'price by 12.5 explicit'; rows = @('Price'); data = @(D 'Revenue'); group = @(@{ f = 'Price'; kind = 'number'; start = 0; end = 100; step = 12.5 }) },
    @{ desc = 'price by 0.5 auto'; rows = @('Price'); cols = @('Channel'); data = @(D 'Units'); group = @(@{ f = 'Price'; kind = 'number'; step = 20.5 }) },
    @{ desc = 'revenue by 500 auto'; layout = 'tabular'; rows = @('Revenue'); data = @(D 'Units'); group = @(@{ f = 'Revenue'; kind = 'number'; step = 500 }) },
    @{ desc = 'revenue by 250 explicit'; rows = @('Revenue'); data = @(D 'Units' 'count'); group = @(@{ f = 'Revenue'; kind = 'number'; start = 0; end = 4500; step = 250 }) },
    @{ desc = 'score by 7.5'; rows = @('Score'); data = @(D 'Units'); group = @(@{ f = 'Score'; kind = 'number'; step = 7.5 }) },
    @{ desc = 'score explicit fractional bounds step 10'; rows = @('Score'); data = @(D 'Units'); group = @(@{ f = 'Score'; kind = 'number'; start = 0.5; end = 100.5; step = 10 }) },
    @{ desc = 'score explicit step 5 end not on a step'; rows = @('Score'); data = @(D 'Units'); group = @(@{ f = 'Score'; kind = 'number'; start = 0; end = 98; step = 5 }) }
  )
}
$EditsTime = @((E 'D3' 46100.75), (E 'D4' 45000.25), (E 'D9' 45600.5), (E 'D12' 45700.125), (E 'E3' 77), (E 'E5' 1000), (E 'G2' 999.5))
# The latest date becomes a whole date while other dates keep a time of day.
$EditsMaxWhole = {
  param($ws, $n)
  $mx = 0; $at = 0; $mn = 1e9; $atMin = 0
  for ($r = 2; $r -le ($n + 1); $r++) { $x = [double]$ws.Cells.Item($r, 4).Value2; if ($x -gt $mx) { $mx = $x; $at = $r }; if ($x -lt $mn) { $mn = $x; $atMin = $r } }
  return @((E "D$at" ([Math]::Floor($mx))), (E "D$atMin" ([Math]::Floor($mn) + 0.5)), (E 'E3' 77))
}
# Whole dates only, then one date with a time of day in the middle (the latest stays a whole date).
$EditsOneTime = @((E 'D5' 45500.5), (E 'E3' 77))
$EditsFraction = @((E 'F3' 150.5), (E 'F4' 0.25), (E 'F9' 33.3), (E 'H3' 250.5), (E 'H4' (-5.5)), (E 'E3' 77), (E 'G2' 4999.5), (E 'G5' 0.5))

try {
  $xl.AutomationSecurity = 3; $xl.Visible = $false; $xl.ScreenUpdating = $false; $xl.DisplayAlerts = $false; $xl.EnableEvents = $false
  Write-RefSet 'grouped-time' 120 5 (TimeCases) $EditsTime
  Write-RefSet 'grouped-fraction' 120 5 (FractionCases) $EditsFraction
  Write-RefSet 'grouped-time-maxwhole' 120 5 (EdgeCases) $EditsMaxWhole
  $script:noTime = $true
  Write-RefSet 'grouped-time-onetime' 120 5 (EdgeCases) $EditsOneTime
  $script:noTime = $false
} finally {
  try { foreach ($w in @($xl.Workbooks)) { $w.Saved = $true; $w.Close($false) } } catch {}
  try { $xl.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl) } catch {}
  Start-Sleep -Seconds 2
  if (Get-Process -Id $xlPid -ErrorAction SilentlyContinue) { Stop-Process -Id $xlPid -Force }
}
