<#
Fixtures for PivotTable calculated fields, calculated items and GETPIVOTDATA (crates/pivot calc.rs, getpivotdata.rs).
Builds PivotTables in Excel with calculated fields and items, records the refreshed grid (like pivot-oracle.ps1) and
the result of GETPIVOTDATA formulas written next to the PivotTable.

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
function Calc-Cases {
  $cases = @()
  $cases += @{ desc = 'field margin'; rows = @('Region'); fields = @(@{ name = 'Margin'; formula = '=Revenue-Units*Price' }); data = @(D 'Units') }
  $cases += @{ desc = 'field ratio with columns'; rows = @('Region'); cols = @('Channel'); fields = @(@{ name = 'Avg'; formula = '=Revenue/Units' }); data = @() }
  $cases += @{ desc = 'field if'; rows = @('Product'); fields = @(@{ name = 'Big'; formula = '=IF(Units>300,Revenue,0)' }); data = @(D 'Units') }
  $cases += @{ desc = 'field constants and power'; rows = @('Region'); fields = @(@{ name = 'Scaled'; formula = '=Revenue*1.1+Units^2' }); data = @(D 'Units') }
  $cases += @{ desc = 'field two fields tabular'; layout = 'tabular'; rows = @('Region', 'Product'); fields = @(@{ name = 'Margin'; formula = '=Revenue-Units*Price' }, @{ name = 'Per unit'; formula = '=Revenue/Units' }); data = @(D 'Units') }
  $cases += @{ desc = 'field functions'; rows = @('Region'); fields = @(@{ name = 'Rounded'; formula = '=ROUND(Revenue/Units,1)+ABS(Units-Price)' }); data = @() }
  $cases += @{ desc = 'item sum'; rows = @('Region'); cols = @('Product'); items = @(@{ f = 'Product'; name = 'AppleBanana'; formula = '=Apple+Banana' }); data = @(D 'Units') }
  $cases += @{ desc = 'item difference on rows'; rows = @('Product'); items = @(@{ f = 'Product'; name = 'Diff'; formula = '=Apple-Banana' }); data = @(D 'Revenue') }
  $cases += @{ desc = 'item average two'; rows = @('Product'); cols = @('Channel'); items = @(@{ f = 'Product'; name = 'Mean'; formula = '=(Apple+Cherry)/2' }); data = @(D 'Units') }
  $cases += @{ desc = 'item on region with channel columns'; rows = @('Region'); cols = @('Channel'); items = @(@{ f = 'Region'; name = 'Half'; formula = '=North*0.5' }); data = @(D 'Revenue') }
  $cases += @{ desc = 'item with average data field'; rows = @('Product'); items = @(@{ f = 'Product'; name = 'Both'; formula = '=Apple+Banana' }); data = @(D 'Units' 'average') }
  $cases += @{ desc = 'item with count data field'; rows = @('Product'); items = @(@{ f = 'Product'; name = 'Both'; formula = '=Apple+Banana' }); data = @(D 'Units' 'count') }
  $cases += @{ desc = 'item with max data field'; rows = @('Product'); items = @(@{ f = 'Product'; name = 'Both'; formula = '=Apple+Banana' }); data = @(D 'Units' 'max') }
  $cases += @{ desc = 'item and field together'; rows = @('Product'); items = @(@{ f = 'Product'; name = 'AB'; formula = '=Apple+Banana' }); fields = @(@{ name = 'Margin'; formula = '=Revenue-Units*Price' }); data = @(D 'Units') }
  $cases += @{ desc = 'item with two row fields'; rows = @('Region', 'Product'); items = @(@{ f = 'Product'; name = 'AB'; formula = '=Apple+Banana' }); data = @(D 'Units') }
  $cases += @{ desc = 'item no grand totals'; rows = @('Product'); cols = @('Channel'); rowGrand = $false; colGrand = $false; items = @(@{ f = 'Product'; name = 'AB'; formula = '=Apple+Banana' }); data = @(D 'Units') }
  return $cases
}

function Gpd-Cases {
  $cases = @()
  $q1 = @('"Sum of Units",{A}', '"Sum of Units",{A},"Region","East"', '"Sum of Units",{A},"Region","Nowhere"', '"Units",{A}', '"sum of units",{A},"region","east"',
    '"Sum of Revenue",{A}', '"Sum of Units",{A},"Product","Apple"', '"Sum of Units",{A},"Region","East","Product","Apple"', '"Sum of Units",{A},"Product","Apple","Region","East"',
    '{A}', '"Sum of Units",{A},"Region"', '"Sum of Units",{A},"Region","East","Region","East"', '"Sum of Units",{A},"Channel","Online"')
  $cases += @{ desc = 'one row field'; rows = @('Region'); data = @((D 'Units')); queries = $q1 }
  $q2 = @('"Sum of Units",{A}', '"Sum of Units",{A},"Region","East"', '"Sum of Units",{A},"Channel","Online"', '"Sum of Units",{A},"Region","East","Channel","Online"',
    '"Sum of Units",{A},"Channel","Online","Region","East"', '"Sum of Units",{A},"Region","East","Channel","Nope"', '"Sum of Revenue",{A},"Region","West"', '"Sum of Units",{A},"Product","Apple"')
  $cases += @{ desc = 'rows and columns'; rows = @('Region'); cols = @('Channel'); data = @((D 'Units'), (D 'Revenue')); queries = $q2 }
  $q3 = @('"Sum of Units",{A},"Region","East"', '"Sum of Units",{A},"Region","East","Product","Apple"', '"Sum of Units",{A},"Product","Apple"',
    '"Sum of Units",{A},"Region","East","Product","Apple","Channel","Online"', '"Sum of Units",{A},"Region","East","Channel","Online"', '"Sum of Units",{A},"Region","East","Product","Apple","Channel","Online","Tier","Gold"')
  $cases += @{ desc = 'three levels tabular'; layout = 'tabular'; rows = @('Region', 'Product'); cols = @('Channel'); data = @((D 'Units')); queries = $q3 }
  $q4 = @('"Sum of Units",{A}', '"Sum of Units",{A},"Region","East"', '"Sum of Units",{A},"Region","East","Product","Apple"')
  $cases += @{ desc = 'no grand totals'; rows = @('Region', 'Product'); rowGrand = $false; colGrand = $false; data = @((D 'Units')); queries = $q4 }
  $q5 = @('"Sum of Units",{A},"Units",5', '"Sum of Units",{A},"Region","East","Units",5')
  $cases += @{ desc = 'numeric item'; rows = @('Tier', 'Region'); data = @((D 'Units')); queries = @('"Sum of Units",{A},"Tier","Gold"', '"Sum of Units",{A},"Tier","Gold","Region","East"', '"Sum of Units",{A},"Tier",5') }
  $cases += @{ desc = 'two data fields on rows'; rows = @('Region'); data = @((D 'Units'), (D 'Price' 'average')); queries = @('"Sum of Units",{A},"Region","East"', '"Average of Price",{A},"Region","East"', '"Average of Price",{A}', '"Units",{A}', '"Price",{A}') }
  return $cases
}

try {
  $xl.AutomationSecurity = 3; $xl.Visible = $false; $xl.ScreenUpdating = $false; $xl.DisplayAlerts = $false; $xl.EnableEvents = $false
  Write-Set 'calc' 'PivotTable calculated fields and calculated items' 90 21 (Calc-Cases) $false $true
  Write-Set 'getpivotdata' 'GETPIVOTDATA queries over PivotTable layouts' 90 22 (Gpd-Cases) $true $true
} finally {
  try { foreach ($w in @($xl.Workbooks)) { $w.Saved = $true; $w.Close($false) } } catch {}
  try { $xl.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl) } catch {}
}
