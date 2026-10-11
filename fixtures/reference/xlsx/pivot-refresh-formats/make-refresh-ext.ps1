<#
Oracle for PivotTable refresh beyond make-pivot-refresh.ps1: user formats (borders, fills), grouped fields and calculated
items. Per set it writes into fixtures/reference/xlsx/pivot-refresh-<set>/ (the parent of this script's folder):
  before.xlsx  freshly built (formats applied / groups / calculated items added), caches as Excel wrote them
  after.xlsx   source edited (edits.json) and every cache refreshed by Excel
  edits.json   [{"a":"A5","v":"Central"|12.5|null}] applied to sheet Data
  pivot.json   per PivotTable: sheet, case, range, cells [{a,v,t,nf,f,b,i,c,bd}] after the refresh
               (f fill BGR or -1, b/i bold/italic, c font colour, bd [left, top, bottom, right] "<style>:<weight>:<bgr>" or "")
LOCAL DEVELOPER TOOL ONLY (not CI). Own Excel instance only (aborts when COM attached to one already running); prints the
PID of that instance; AutomationSecurity = 3; Quit in finally. Other EXCEL processes are never touched. Data is synthetic.
Usage: pwsh -NoProfile -File make-refresh-ext.ps1 [-Only regex]
#>
param([string]$Only)
$ErrorActionPreference = 'Stop'
$refDir = Split-Path -Parent $PSScriptRoot
$inv = [Globalization.CultureInfo]::InvariantCulture
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class PvExtWin { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p); }
'@
$before = @(Get-Process EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$xl = New-Object -ComObject Excel.Application
$procId = [uint32]0
[void][PvExtWin]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$procId)
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

function D($f, $fn = 'sum') { return @{ f = $f; fn = $fn } }
function E($a, $v) { return @{ a = $a; v = $v } }
$Red = 255; $Blue = 16711680; $Green = 65280; $Yellow = 65535
function FormatCases {
  return @(
    @{ desc = 'borders on an item label, the data body and a cell'; rows = @('Region'); data = @(D 'Revenue'); formats = @(
        @{ where = 'label'; f = 'Region'; item = 'West'; edges = @(9); weight = 4; color = $Red },
        @{ where = 'label'; f = 'Region'; item = 'North'; edges = @(7, 10); weight = 2; color = $Blue },
        @{ where = 'databody'; edges = @(7, 8, 9, 10); weight = 2; color = $Green },
        @{ where = 'cell'; r = 2; c = 1; around = $Yellow; weight = 4 }) },
    @{ desc = 'rows and columns, item data outline'; rows = @('Region', 'Product'); cols = @('Channel'); data = @(D 'Units'); formats = @(
        @{ where = 'itemdata'; f = 'Region'; item = 'East'; around = $Red; weight = 2 },
        @{ where = 'label'; f = 'Product'; item = 'Fig'; fill = $Yellow; edges = @(8, 9); weight = 2; color = $Blue },
        @{ where = 'fieldlabel'; f = 'Channel'; edges = @(9); weight = -4138; color = $Green }) },
    @{ desc = 'page field and whole table outline'; rows = @('Product'); pages = @('Tier'); data = @(D 'Revenue'); formats = @(
        @{ where = 'table1'; around = $Blue; weight = -4138 },
        @{ where = 'itemdata'; f = 'Product'; item = 'Walnut'; edges = @(8); weight = 2; color = $Red }) },
    @{ desc = 'tabular, borders on item data of a removed item'; layout = 'tabular'; rows = @('Product', 'Tier'); data = @(D 'Units' 'count'); formats = @(
        @{ where = 'itemdata'; f = 'Product'; item = 'Fig'; around = $Red; weight = 4 },
        @{ where = 'label'; f = 'Tier'; item = 'Bronze'; edges = @(7, 9); weight = 2; color = $Green; fill = $Blue },
        @{ where = 'fielddata'; f = 'Tier'; edges = @(10); weight = 2; color = $Blue }) },
    @{ desc = 'outline layout, row range and field labels'; layout = 'outline'; rows = @('Tier', 'Region'); cols = @('Size'); data = @(D 'Score' 'max'); formats = @(
        @{ where = 'rowrange'; edges = @(10); weight = 2; color = $Red },
        @{ where = 'label'; f = 'Region'; item = 'West'; around = $Green; weight = 2; bold = 1 }) }
  )
}
function GroupCases {
  return @(
    @{ desc = 'date by months auto bounds'; rows = @('Date'); data = @(D 'Revenue'); group = @(@{ f = 'Date'; kind = 'date'; by = @('months') }) },
    @{ desc = 'date by years and months'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('months', 'years') }) },
    @{ desc = 'date by quarters columns'; rows = @('Region'); cols = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('quarters') }) },
    @{ desc = 'date by months explicit bounds'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('months'); start = 45400; end = 45900 }) },
    @{ desc = 'date by days step 30'; rows = @('Date'); data = @(D 'Units'); group = @(@{ f = 'Date'; kind = 'date'; by = @('days'); step = 30 }) },
    @{ desc = 'number group by 20 explicit'; rows = @('Score'); data = @(D 'Units'); group = @(@{ f = 'Score'; kind = 'number'; start = 0; end = 100; step = 20 }) },
    @{ desc = 'number group by 25 auto bounds'; rows = @('Score'); cols = @('Channel'); data = @(D 'Revenue'); group = @(@{ f = 'Score'; kind = 'number'; step = 25 }) },
    @{ desc = 'number group units by 10'; layout = 'tabular'; rows = @('Units'); data = @(D 'Score' 'average'); group = @(@{ f = 'Units'; kind = 'number'; start = 1; end = 50; step = 10 }) }
  )
}
function CalcCases {
  return @(
    @{ desc = 'one calculated item'; rows = @('Product'); data = @(D 'Revenue'); calc = @(@{ f = 'Product'; name = 'Combo'; formula = '=Walnut+Cherry' }) },
    @{ desc = 'two calculated items, columns'; rows = @('Product'); cols = @('Channel'); data = @(D 'Units'); calc = @(@{ f = 'Product'; name = 'Combo'; formula = '=Walnut+Cherry' }, @{ f = 'Product'; name = 'Gap'; formula = '=Banana-Cherry' }) },
    @{ desc = 'calculated item on columns'; rows = @('Region'); cols = @('Tier'); data = @(D 'Revenue'); calc = @(@{ f = 'Tier'; name = 'Top'; formula = '=Gold+Silver' }) },
    @{ desc = 'calculated item with a second row field'; layout = 'tabular'; rows = @('Region', 'Product'); data = @(D 'Units'); calc = @(@{ f = 'Product'; name = 'Combo'; formula = '=Walnut+Banana' }) }
  )
}

$EditsItems = @(
  (E 'A2' 'Central'), (E 'A3' 'Central'), (E 'A4' $null), (E 'A7' 'north'),
  (E 'B2' 'Apple'), (E 'B3' 'Zucchini'), (E 'B8' 'Walnut'),
  (E 'C5' 'Kiosk'), (E 'I3' 'Platinum'), (E 'I4' 'platinum'), (E 'J6' 'XL'),
  (E 'E3' 77), (E 'E4' 1000.5), (E 'E6' $null), (E 'D3' 46000), (E 'D4' 44000), (E 'D9' $null)
)
$EditsGroup = @(
  (E 'D3' 46100), (E 'D4' 45000), (E 'D9' 45600), (E 'D12' 45700),
  (E 'H3' 250), (E 'H4' (-5)), (E 'H9' 33), (E 'H10' $null), (E 'E3' 77), (E 'E5' 1000), (E 'E6' $null), (E 'G2' 999.5), (E 'C4' 'Kiosk')
)
$EditsRemoved = {
  param($ws, $n)
  $out = @()
  $map = @(@(1, 'West', 'East'), @(2, 'Fig', 'Date'), @(3, 'Partner', 'Online'), @(9, 'Bronze', 'Gold'))
  foreach ($m in $map) {
    for ($r = 2; $r -le ($n + 1); $r++) {
      $x = $ws.Cells.Item($r, $m[0]).Value2
      if ($null -ne $x -and "$x" -eq "$($m[1])") { $out += (E ((Col-Letters $m[0]) + $r) $m[2]) }
    }
  }
  return $out
}

try {
  $xl.AutomationSecurity = 3; $xl.Visible = $false; $xl.ScreenUpdating = $false; $xl.DisplayAlerts = $false; $xl.EnableEvents = $false
  Write-RefSet 'formats' 60 31 (FormatCases) $EditsItems
  Write-RefSet 'formats-removed' 60 31 (FormatCases) $EditsRemoved
  Write-RefSet 'grouped' 120 5 (GroupCases) $EditsGroup
  Write-RefSet 'calcitems' 60 31 (CalcCases) $EditsItems
  Write-RefSet 'calcitems-values' 60 31 (CalcCases) @((E 'G2' 1234.5), (E 'G5' 99.25), (E 'E7' 12), (E 'E9' 100))
} finally {
  try { foreach ($w in @($xl.Workbooks)) { $w.Saved = $true; $w.Close($false) } } catch {}
  try { $xl.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl) } catch {}
  Start-Sleep -Seconds 2
  if (Get-Process -Id $xlPid -ErrorAction SilentlyContinue) { Stop-Process -Id $xlPid -Force }
}
