<#
Fixtures for the calculated-items hold-out (crates/pivot calcitems.rs): random calculated items and fields over a data seed the engine was never fitted on. Same recording as make-pivot-calc.ps1. The set is written once and only used to confirm; failures are not inspected to tune.

LOCAL DEVELOPER TOOL ONLY (not CI). Output
  fixtures/xlsx/pivot-<set>.xlsx and .toml
  fixtures/reference/xlsx/pivot-<set>/pivot.json  per PivotTable: sheet, name, case, range, location, cells [{a, v, nf, t}]
                                                   and, for GETPIVOTDATA, gpd [{f, v}] (v is a number, a string or {"e": code})
Sets: calc (calculated fields and items), getpivotdata (queries over several layouts).

Process safety: own Excel instance only (aborts when COM attached to one already running), AutomationSecurity = 3,
Quit in finally. Other EXCEL processes are never touched. Data is synthetic.
#>
param([string]$Only)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class PvCalcWin { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p); }
'@
$before = @(Get-Process EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$xl = New-Object -ComObject Excel.Application
$procId = [uint32]0
[void][PvCalcWin]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$procId)
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
$Headers = 'Region', 'Product', 'Channel', 'Units', 'Price', 'Revenue', 'Tier'
function New-Data($wb, [int]$n, [long]$seed) {
  $script:rs = $seed
  $ws = $wb.Worksheets.Item(1); $ws.Name = 'Data'
  for ($i = 0; $i -lt 7; $i++) { $ws.Cells.Item(1, ($i + 1)).Value2 = $Headers[$i] }
  $reg = 'North', 'South', 'East', 'West'
  $prod = 'Walnut', 'Apple', 'Banana', 'Cherry'
  $chn = 'Online', 'Retail', 'Partner'
  for ($i = 0; $i -lt $n; $i++) {
    $row = $i + 2
    $u = [double](1 + (Next-Rand) % 50); $p = [double](1 + (Next-Rand) % 40)
    $ws.Cells.Item($row, 1).Value2 = [string]$reg[(Next-Rand) % 4]
    $ws.Cells.Item($row, 2).Value2 = [string]$prod[(Next-Rand) % 4]
    $ws.Cells.Item($row, 3).Value2 = [string]$chn[(Next-Rand) % 3]
    $ws.Cells.Item($row, 4).Value2 = [double]$u
    $ws.Cells.Item($row, 5).Value2 = [double]$p
    $ws.Cells.Item($row, 6).Value2 = [double]($u * $p)
    $ws.Cells.Item($row, 7).Value2 = [string]('Gold', 'Silver', 'Bronze')[(Next-Rand) % 3]
  }
}
$FnCode = @{ sum = -4157; count = -4112; average = -4106; max = -4136 }
$FnName = @{ sum = 'Sum'; count = 'Count'; average = 'Average'; max = 'Max' }

function New-CalcPivot($wb, $pc, [string]$sheetName, [string]$ptName, $c) {
  $sh = $wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count))
  $sh.Name = $sheetName
  $pt = $pc.CreatePivotTable("'$sheetName'!R4C2", $ptName)
  $pt.ManualUpdate = $true
  foreach ($f in @($c.rows)) { if ($f) { $pt.PivotFields($f).Orientation = 1 } }
  foreach ($f in @($c.cols)) { if ($f) { $pt.PivotFields($f).Orientation = 2 } }
  $pt.ManualUpdate = $false
  if ($c.ContainsKey('layout')) { $pt.RowAxisLayout($(@{ compact = 0; tabular = 1; outline = 2 }[$c.layout])) }
  if ($c.ContainsKey('rowGrand')) { $pt.RowGrand = [bool]$c.rowGrand }
  if ($c.ContainsKey('colGrand')) { $pt.ColumnGrand = [bool]$c.colGrand }
  if ($c.ContainsKey('items')) {
    foreach ($ci in @($c.items)) { [void]$pt.PivotFields($ci.f).CalculatedItems().Add($ci.name, $ci.formula) }
  }
  if ($c.ContainsKey('fields')) {
    foreach ($cf in @($c.fields)) {
      $f = $pt.CalculatedFields().Add($cf.name, $cf.formula, $true)
      $f.Orientation = 4
    }
  }
  foreach ($d in @($c.data)) {
    $fn = 'sum'; if ($d.ContainsKey('fn')) { $fn = $d.fn }
    [void]$pt.AddDataField($pt.PivotFields($d.f), "$($FnName[$fn]) of $($d.f)", $FnCode[$fn])
  }
  return $pt
}

function Dump-Pivot($pt, $c, $withGpd) {
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
  $gpd = ''
  if ($withGpd -and $c.ContainsKey('queries')) {
    $col = $c0 + $nc + 2
    $row = $r0
    $items = New-Object Collections.ArrayList
    foreach ($q in @($c.queries)) {
      $f = '=GETPIVOTDATA(' + ($q -replace '\{A\}', $pt.TableRange1.Cells.Item(1, 1).Address($true, $true)) + ')'
      $cell = $ws.Cells.Item($row, $col)
      try {
        $cell.Formula = $f
        $x = $cell.Value2
        [void]$items.Add('{"f":' + (Json-Str $f) + ',"v":' + (Val-Json $x $cell) + '}')
      } catch {
        [void]$items.Add('{"f":' + (Json-Str $f) + ',"v":{"e":"#INVALID"}}')
      }
      $row++
    }
    $gpd = ",`n     `"gpd`":[`n      " + ($items -join ",`n      ") + "`n     ]"
  }
  return '    {"sheet":' + (Json-Str $ws.Name) + ',"name":' + (Json-Str $pt.Name) + ',"case":' + (Json-Str $c.desc) +
    ',"range":' + (Json-Str $tr.Address($false, $false)) + ',"location":' + (Json-Str $pt.TableRange1.Address($false, $false)) +
    ",`n     `"cells`":[`n      " + ($cells -join ",`n      ") + "`n     ]" + $gpd + '}'
}

function Write-Set($setName, $notes, [int]$rows, [long]$seed, $cases, [bool]$withGpd, [bool]$perCache = $false) {
  if ($Only -and $setName -notmatch $Only) { return }
  $path = Join-Path $fixtureDir "pivot-$setName.xlsx"
  if (Test-Path $path) { Remove-Item $path -Force }
  $wb = $xl.Workbooks.Add()
  $outs = New-Object Collections.ArrayList
  try {
    New-Data $wb $rows $seed
    $pc = $wb.PivotCaches().Create(1, "Data!R1C1:R$($rows + 1)C7")
    $k = 0
    foreach ($c in $cases) {
      $k++
      try {
        $pcUse = $pc
        if ($perCache) { $pcUse = $wb.PivotCaches().Create(1, "Data!R1C1:R$($rows + 1)C7") }
        $pt = New-CalcPivot $wb $pcUse "P$k" "PT$k" $c
        [void]$outs.Add((Dump-Pivot $pt $c $withGpd))
        Write-Host "pivot-$setName P$k $($c.desc)"
      } catch {
        Write-Host "SKIPPED pivot-$setName P$k $($c.desc): $($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
        try { $xl.DisplayAlerts = $false; $wb.Worksheets.Item("P$k").Delete() } catch {}
      }
    }
    $wb.Worksheets.Item('Data').Activate()
    $wb.RemovePersonalInformation = $true
    $wb.SaveAs($path, 51)
    $json = "{`n  `"source`": `"fixtures/xlsx/pivot-$setName.xlsx`",`n  `"oracle`": `"Microsoft Excel via COM; TableRange2 cells (Value2, NumberFormat, Text) and GETPIVOTDATA results`",`n" +
      "  `"excel_build`": `"$($xl.Build)`",`n  `"pivots`": [`n" + ($outs -join ",`n") + "`n  ]`n}`n"
    Write-Lf (Join-Path $refDir "pivot-$setName/pivot.json") $json
    $t = "feature = `"pivot`"`nmade_with = `"Microsoft Excel $($xl.Build) (Windows)`"`nauthor = `"project member`"`nexpect_dropped = []`nnotes = `"$notes`"`n"
    Write-Lf (Join-Path $fixtureDir "pivot-$setName.toml") $t
  } finally {
    try { $wb.Saved = $true; $wb.Close($false) } catch {}
  }
}

function D($f, $fn = 'sum') { return @{ f = $f; fn = $fn } }

# Random calculated items (and sometimes a calculated field) on a seed distinct from every fitted set.
function Holdout-Calc-Cases([int]$seed, [int]$count) {
  $rng = New-Object System.Random $seed
  $members = @{ Region = @('North', 'South', 'East', 'West'); Product = @('Walnut', 'Apple', 'Banana', 'Cherry'); Channel = @('Online', 'Retail', 'Partner'); Tier = @('Gold', 'Silver', 'Bronze') }
  $fnames = @('Region', 'Product', 'Channel', 'Tier')
  $layouts = 'compact', 'outline', 'tabular'
  $ops = '+', '-', '*', '/'
  $cases = @()
  for ($i = 0; $i -lt $count; $i++) {
    $itemField = $fnames[$rng.Next(0, 4)]
    $mem = @($members[$itemField] | Sort-Object { $rng.Next() })
    $nterm = $rng.Next(2, 4)
    $formula = '=' + $mem[0]
    for ($t = 1; $t -lt $nterm; $t++) {
      $formula += $ops[$rng.Next(0, 4)] + $mem[$t]
    }
    $k = $rng.Next(0, 4)
    if ($k -eq 0) { $formula = '=(' + $mem[0] + '+' + $mem[1] + ')/2' }
    elseif ($k -eq 1) { $formula += '*' + (1 + $rng.Next(1, 9) / 10.0).ToString([Globalization.CultureInfo]::InvariantCulture) }
    $others = @($fnames | Where-Object { $_ -ne $itemField } | Sort-Object { $rng.Next() })
    $rows = @($itemField); $cols = @()
    $placement = $rng.Next(0, 4)
    if ($placement -eq 1) { $rows = @($itemField, $others[0]) }
    elseif ($placement -eq 2) { $rows = @($others[0], $itemField) }
    elseif ($placement -eq 3) { $cols = @($itemField); $rows = @($others[0]) }
    if ($placement -ne 3 -and $rng.Next(0, 2) -eq 1) { $cols = @($others[1]) }
    $fn = @('sum', 'sum', 'count', 'max')[$rng.Next(0, 4)]
    $dfield = @('Units', 'Revenue')[$rng.Next(0, 2)]
    $c = @{ rows = $rows; layout = $layouts[$rng.Next(0, 3)]; data = @(D $dfield $fn)
      items = @(@{ f = $itemField; name = 'Calc' + $i; formula = $formula }) }
    if ($cols.Count) { $c.cols = $cols }
    if ($rng.Next(0, 4) -eq 0) { $c.rowGrand = $false }
    if ($rng.Next(0, 4) -eq 0) { $c.colGrand = $false }
    if ($rng.Next(0, 4) -eq 0) { $c.fields = @(@{ name = 'Cf' + $i; formula = '=Revenue-Units*Price' }) }
    $c.desc = "random $i item=$itemField $formula rows=$($rows -join '/') cols=$($cols -join '/') fn=$fn"
    $cases += $c
  }
  return $cases
}

try {
  $xl.AutomationSecurity = 3; $xl.Visible = $false; $xl.ScreenUpdating = $false; $xl.DisplayAlerts = $false; $xl.EnableEvents = $false
  Write-Set 'holdout-calc' 'hold-out: random PivotTable calculated items (and fields) over a distinct data seed' 90 8801 (Holdout-Calc-Cases 8801 40) $false $true
} finally {
  try { foreach ($w in @($xl.Workbooks)) { $w.Saved = $true; $w.Close($false) } } catch {}
  try { $xl.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl) } catch {}
}
