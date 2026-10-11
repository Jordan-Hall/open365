<#
Fixtures for PivotTable refresh (crates/pivot refresh): what Excel writes when it refreshes a PivotTable cache.
Per set (a workbook with several PivotTables, one cache each) it saves
  fixtures/xlsx/pivot-refresh-<set>.xlsx                    before: freshly built, caches and PivotTables as Excel wrote them
  fixtures/reference/xlsx/pivot-refresh-<set>/after.xlsx    after: the source edited (edits.json) and every cache refreshed by Excel
  fixtures/reference/xlsx/pivot-refresh-<set>/edits.json    [{"a": "A5", "v": "Central" | 12.5 | null}] applied to sheet Data
  fixtures/reference/xlsx/pivot-refresh-<set>/pivot.json    per PivotTable: sheet, name, case, range, cells [{a, v, nf, t}] after the refresh
The test (crates/pivot/tests/refresh.rs) opens the before file, applies the edits, refreshes with refresh_sheet and
compares the pivot parts with the after file. After a run regenerate the references the crates/xlsx tests require:
  pwsh -NoProfile -File tools/office/grid-geometry-oracle.ps1 -Only '^pivot-'
  pwsh -NoProfile -File tools/office/office-oracle.ps1 -Mode reference -Path <each fixtures/xlsx/pivot-refresh-*.xlsx>

LOCAL DEVELOPER TOOL ONLY (not CI). Process safety: own Excel instance only (aborts when COM attached to one already
running), AutomationSecurity = 3, Quit in finally. Other EXCEL processes are never touched. Data is synthetic.
#>
param([string]$Only)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class PvRefWin { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p); }
'@
$before = @(Get-Process EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$xl = New-Object -ComObject Excel.Application
$procId = [uint32]0
[void][PvRefWin]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$procId)
if ($before -contains [int]$procId) { Write-Host 'attached to a pre-existing Excel instance; aborting without touching it'; exit 1 }

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
  if ($c.ContainsKey('desc_sort')) {
    foreach ($f in @($c.desc_sort)) { $pt.PivotFields($f).AutoSort(2, $f) }
  }
  return $pt
}

function Dump-Pivot($pt, $c) {
  $tr = $pt.TableRange2
  $r0 = $tr.Row; $c0 = $tr.Column; $nr = $tr.Rows.Count; $nc = $tr.Columns.Count
  $vals = $tr.Value2
  $ws = $pt.Parent
  $cells = New-Object Collections.ArrayList
  for ($r = 1; $r -le $nr; $r++) {
    for ($cc = 1; $cc -le $nc; $cc++) {
      $x = $vals[$r, $cc]
      if ($null -eq $x) { continue }
      $cell = $ws.Cells.Item(($r0 + $r - 1), ($c0 + $cc - 1))
      $addr = (Col-Letters ($c0 + $cc - 1)) + ($r0 + $r - 1)
      [void]$cells.Add('{"a":"' + $addr + '","v":' + (Val-Json $x $cell) + ',"nf":' + (Json-Str ([string]$cell.NumberFormat)) + ',"t":' + (Json-Str ([string]$cell.Text)) + '}')
    }
  }
  return '    {"sheet":' + (Json-Str $ws.Name) + ',"name":' + (Json-Str $pt.Name) + ',"case":' + (Json-Str $c.desc) +
    ',"range":' + (Json-Str $tr.Address($false, $false)) +
    ",`n     `"cells`":[`n      " + ($cells -join ",`n      ") + "`n     ]}"
}

function Edit-Json($e) {
  $v = $e.v
  if ($null -eq $v) { $vj = 'null' } elseif ($v -is [string]) { $vj = Json-Str $v } else { $vj = ([double]$v).ToString('R', $inv) }
  return '{"a":' + (Json-Str $e.a) + ',"v":' + $vj + '}'
}

function Write-RefSet($setName, $notes, [int]$rows, [long]$seed, $cases, $edits) {
  if ($Only -and $setName -notmatch $Only) { return }
  $path = Join-Path $fixtureDir "pivot-refresh-$setName.xlsx"
  $afterPath = Join-Path $refDir "pivot-refresh-$setName/after.xlsx"
  New-Item -ItemType Directory -Force (Split-Path -Parent $afterPath) | Out-Null
  foreach ($p in $path, $afterPath) { if (Test-Path $p) { Remove-Item $p -Force } }
  $wb = $xl.Workbooks.Add()
  $outs = New-Object Collections.ArrayList
  try {
    New-Data $wb $rows $seed
    $k = 0
    $pts = @()
    foreach ($c in $cases) {
      $k++
      $pc = $wb.PivotCaches().Create(1, "Data!R1C1:R$($rows + 1)C10")
      $pts += New-RefPivot $wb $pc "P$k" "PT$k" $c
      Write-Host "pivot-refresh-$setName P$k $($c.desc)"
    }
    $wb.Worksheets.Item('Data').Activate()
    $wb.RemovePersonalInformation = $true
    $wb.SaveAs($path, 51)
    # A script block makes the edits from the data (to remove an item from every row it is in).
    if ($edits -is [scriptblock]) { $edits = @(& $edits $wb.Worksheets.Item('Data') $rows) }
    # Edit the source the way edits.json says, refresh every cache, save the result.
    $ws = $wb.Worksheets.Item('Data')
    foreach ($e in $edits) { $m = [regex]::Match($e.a, '^([A-Z]+)([0-9]+)$'); $cn = 0; foreach ($ch in $m.Groups[1].Value.ToCharArray()) { $cn = $cn * 26 + ([int]$ch - 64) }; $cell = $ws.Cells.Item([int]$m.Groups[2].Value, $cn); if ($null -eq $e.v) { [void]$cell.ClearContents() } elseif ($e.v -is [string]) { $cell.Value2 = [string]$e.v } else { $cell.Value2 = [double]$e.v } }
    $k = 0
    foreach ($c in $cases) {
      $k++
      $pt = $wb.Worksheets.Item("P$k").PivotTables("PT$k")
      $pt.PivotCache().Refresh() | Out-Null
      [void]$outs.Add((Dump-Pivot $pt $c))
    }
    $wb.SaveAs($afterPath, 51)
    $json = "{`n  `"source`": `"fixtures/xlsx/pivot-refresh-$setName.xlsx`",`n  `"oracle`": `"Microsoft Excel via COM; TableRange2 cells after editing the source and refreshing`",`n" +
      "  `"excel_build`": `"$($xl.Build)`",`n  `"pivots`": [`n" + ($outs -join ",`n") + "`n  ]`n}`n"
    Write-Lf (Join-Path $refDir "pivot-refresh-$setName/pivot.json") $json
    Write-Lf (Join-Path $refDir "pivot-refresh-$setName/edits.json") ("[`n" + (($edits | ForEach-Object { '  ' + (Edit-Json $_) }) -join ",`n") + "`n]`n")
    $t = "feature = `"pivot`"`nmade_with = `"Microsoft Excel $($xl.Build) (Windows)`"`nauthor = `"project member`"`nexpect_dropped = []`nnotes = `"$notes`"`n"
    Write-Lf (Join-Path $fixtureDir "pivot-refresh-$setName.toml") $t
  } finally {
    try { $wb.Saved = $true; $wb.Close($false) } catch {}
  }
}

function D($f, $fn = 'sum') { return @{ f = $f; fn = $fn } }
function Refresh-Cases {
  $cases = @()
  $cases += @{ desc = 'one row field'; rows = @('Region'); data = @(D 'Revenue') }
  $cases += @{ desc = 'rows and columns'; rows = @('Region', 'Product'); cols = @('Channel'); data = @(D 'Units') }
  $cases += @{ desc = 'page field'; rows = @('Product'); pages = @('Tier'); data = @(D 'Revenue') }
  $cases += @{ desc = 'numeric row field'; rows = @('Units'); data = @(D 'Revenue') }
  $cases += @{ desc = 'date row field'; rows = @('Date'); data = @(D 'Units') }
  $cases += @{ desc = 'tier by size max'; rows = @('Tier'); cols = @('Size'); data = @(D 'Score' 'max') }
  $cases += @{ desc = 'descending sort'; layout = 'tabular'; rows = @('Product', 'Tier'); desc_sort = @('Product'); data = @(D 'Units' 'count') }
  return $cases
}

function E($a, $v) { return @{ a = $a; v = $v } }
# The edits: none; values only; new, removed, renamed and blank items.
$EditsNone = @()
$EditsValues = @((E 'G2' 1234.5), (E 'G5' 99.25), (E 'F7' 12.75), (E 'H9' 100), (E 'H10' 0), (E 'G20' 4321.0))
$EditsItems = @(
  (E 'A2' 'Central'), (E 'A3' 'Central'), (E 'A4' $null), (E 'A7' 'north'),
  (E 'B2' 'Apple'), (E 'B3' 'Zucchini'), (E 'B8' 'Walnut'),
  (E 'C5' 'Kiosk'), (E 'I3' 'Platinum'), (E 'I4' 'platinum'), (E 'J6' 'XL'),
  (E 'E3' 77), (E 'E4' 1000.5), (E 'E6' $null), (E 'D3' 46000), (E 'D4' 44000), (E 'D9' $null)
)

# Removes whole items: every cell of a column with the old value gets the new one.
$EditsRemoved = {
  param($ws, $n)
  $out = @()
  $map = @(@(1, 'West', 'East'), @(2, 'Fig', 'Cherry'), @(3, 'Partner', 'Online'), @(5, 10, 11), @(9, 'Bronze', 'Gold'), @(10, 'L', 'S'))
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
  Write-RefSet 'none' 'PivotTable cache refreshed by Excel with the source unchanged' 60 31 (Refresh-Cases) $EditsNone
  Write-RefSet 'values' 'PivotTable cache refreshed by Excel after changing source values only' 60 31 (Refresh-Cases) $EditsValues
  Write-RefSet 'items' 'PivotTable cache refreshed by Excel after adding, renaming and blanking source items' 60 31 (Refresh-Cases) $EditsItems
  Write-RefSet 'removed' 'PivotTable cache refreshed by Excel after replacing every cell of some items (text and number items leave the source)' 60 31 (Refresh-Cases) $EditsRemoved
} finally {
  try { foreach ($w in @($xl.Workbooks)) { $w.Saved = $true; $w.Close($false) } } catch {}
  try { $xl.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl) } catch {}
}
