<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365 PivotTables: it builds PivotTables
over synthetic source data in many layouts and aggregations, saves the workbooks as fixtures and records the
refreshed output grid (address, value, number format, display text) as reference JSON.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed output through crates/pivot/tests.

Output
  fixtures/xlsx/pivot-<set>.xlsx          Excel-written workbook: sheet "Data" plus one sheet per PivotTable
  fixtures/xlsx/pivot-<set>.toml          manifest
  fixtures/reference/xlsx/pivot-<set>/pivot.json
                                          per PivotTable: sheet, name, case description, TableRange2 (page fields
                                          included) and every non-empty cell {a, v, nf, t}. v is a number, a string
                                          or {"e": "#DIV/0!"}; nf is Range.NumberFormat; t is Range.Text.
  After a run regenerate the references the crates/xlsx tests require:
    pwsh -NoProfile -File tools/office/grid-geometry-oracle.ps1 -Only '^pivot-'
    pwsh -NoProfile -File tools/office/office-oracle.ps1 -Mode reference -Path <each fixtures/xlsx/pivot-*.xlsx>

Sets
  In-sample (the rules in crates/pivot were fitted on these): layouts, aggregates, subtotals, showas, grouping.
  Hold-out (random layouts over a different data seed, never looked at while fitting): holdout-a, holdout-b.
  -Only <regex> limits the sets written. -Show prints each grid to the console as well (exploration).

Process safety: starts its own Excel instance with AutomationSecurity = 3 (disable macros), remembers its process id
and only ever stops that process (in a finally block). Other EXCEL processes are never touched. Regional and Office
security settings are never changed: captions are those of the Excel UI language the machine runs (en-GB here), which
the reference records implicitly through the stored text.

Output is stable: LF line endings, ASCII only, no absolute paths and no user information. Data is synthetic.
#>
param(
  [string]$Only,
  [switch]$Show
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture

Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class PvWin {
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
  [void][PvWin]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
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

function Write-Lf($path, $text) {
  New-Item -ItemType Directory -Force (Split-Path -Parent $path) | Out-Null
  [IO.File]::WriteAllText($path, ($text -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
}

function Json-Str([string]$s) {
  $sb = New-Object Text.StringBuilder
  [void]$sb.Append('"')
  foreach ($ch in $s.ToCharArray()) {
    $c = [int]$ch
    if ($ch -eq '"') { [void]$sb.Append('\"') }
    elseif ($ch -eq '\') { [void]$sb.Append('\\') }
    elseif ($c -lt 32 -or $c -gt 126) { [void]$sb.Append(('\u{0:x4}' -f $c)) }
    else { [void]$sb.Append($ch) }
  }
  [void]$sb.Append('"')
  return $sb.ToString()
}

# ---------------------------------------------------------------------------------------------------------
# Synthetic source data: deterministic, no personal data.
# ---------------------------------------------------------------------------------------------------------
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
  return $ws
}

# ---------------------------------------------------------------------------------------------------------
# PivotTable construction from a case description.
# ---------------------------------------------------------------------------------------------------------
$FnCode = @{ sum = -4157; count = -4112; average = -4106; max = -4136; min = -4139; product = -4149
  countnums = -4113; stddev = -4155; stddevp = -4156; var = -4164; varp = -4165 }
$FnName = @{ sum = 'Sum'; count = 'Count'; average = 'Average'; max = 'Max'; min = 'Min'; product = 'Product'
  countnums = 'Count Numbers'; stddev = 'StdDev'; stddevp = 'StdDevp'; var = 'Var'; varp = 'Varp' }
$SubIdx = @{ auto = 1; sum = 2; count = 3; average = 4; max = 5; min = 6; product = 7; countnums = 8
  stddev = 9; stddevp = 10; var = 11; varp = 12 }
$ShowAs = @{ none = -4143; diff = 2; pctof = 3; pctdiff = 4; running = 5; pctrow = 6; pctcol = 7; pcttotal = 8
  index = 9; pctparentrow = 10; pctparentcol = 11; pctparent = 12; pctrunning = 13; rankasc = 14; rankdesc = 15 }


function Field-Spec($f) { if ($f -is [string]) { return @{ f = $f } } else { return $f } }

function Set-Subtotals($pf, $sub) {
  # $sub: 'none' | 'auto' | array of function keys
  $flags = New-Object 'object[]' 12
  for ($i = 0; $i -lt 12; $i++) { $flags[$i] = $false }
  if ($sub -eq 'auto') { $flags[0] = $true }
  elseif ($sub -ne 'none') { foreach ($k in @($sub)) { $flags[$SubIdx[$k] - 1] = $true } }
  $pf.Subtotals = $flags
}

function Add-Fields($pt, $list, [int]$orientation) {
  foreach ($raw in @($list)) {
    $s = Field-Spec $raw
    $pf = $pt.PivotFields($s.f)
    $pf.Orientation = $orientation
  }
}

function Apply-FieldOptions($pt, $list) {
  if ($null -eq $list) { return }
  foreach ($raw in @($list)) {
    $s = Field-Spec $raw
    $pf = $pt.PivotFields($s.f)
    if ($s.ContainsKey('sub')) { Set-Subtotals $pf $s.sub }
    if ($s.ContainsKey('top')) { $pf.LayoutSubtotalLocation = $(if ($s.top) { 1 } else { 2 }) }
    if ($s.ContainsKey('blank')) { $pf.LayoutBlankLine = [bool]$s.blank }
    if ($s.ContainsKey('repeat')) { $pf.RepeatLabels = [bool]$s.repeat }
    if ($s.ContainsKey('nf')) { $pf.NumberFormat = $s.nf }
    if ($s.ContainsKey('compact')) { $pf.LayoutForm = $(if ($s.compact) { 0 } else { 1 }) }
    if ($s.ContainsKey('caption')) { $pf.Caption = $s.caption }
    if ($s.ContainsKey('hide')) { foreach ($it in @($s.hide)) { $pf.PivotItems($it).Visible = $false } }
    if ($s.ContainsKey('showall')) { $pf.ShowAllItems = [bool]$s.showall }
    if ($s.ContainsKey('sort')) {
      $so = $s.sort
      if ($so -eq 'asc') { $pf.AutoSort(1, $s.f) }
      elseif ($so -eq 'desc') { $pf.AutoSort(2, $s.f) }
      elseif ($so -is [hashtable]) { $pf.AutoSort($(if ($so.dir -eq 'desc') { 2 } else { 1 }), $so.by) }
    }
    if ($s.ContainsKey('page')) { $pf.CurrentPage = $s.page }
  }
}

function Apply-Grouping($pt, $c) {
  if (-not $c.ContainsKey('group')) { return }
  foreach ($g in @($c.group)) {
    $pf = $pt.PivotFields($g.f)
    $cell = $pf.DataRange.Cells.Item(1, 1)
    if ($g.kind -eq 'date') {
      $per = New-Object 'object[]' 7
      for ($i = 0; $i -lt 7; $i++) { $per[$i] = $false }
      foreach ($p in @($g.by)) { $per[@{ seconds = 0; minutes = 1; hours = 2; days = 3; months = 4; quarters = 5; years = 6 }[$p]] = $true }
      $by = [Type]::Missing
      if ($g.ContainsKey('step')) { $by = [double]$g.step }
      $start = $true; $end = $true
      if ($g.ContainsKey('start')) { $start = [double]$g.start }
      if ($g.ContainsKey('end')) { $end = [double]$g.end }
      [void]$cell.Group($start, $end, $by, $per)
    } else {
      [void]$cell.Group([double]$g.start, [double]$g.end, [double]$g.step)
    }
  }
}

function New-Pivot($wb, $pc, [string]$sheetName, [string]$ptName, $c) {
  $sh = $wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count))
  $sh.Name = $sheetName
  $top = 4 + $(if ($c.ContainsKey('pages')) { @($c.pages).Count + 2 } else { 0 })
  $left = 2
  if ($c.ContainsKey('left')) { $left = $c.left }
  $pt = $pc.CreatePivotTable("'$sheetName'!R${top}C${left}", $ptName)
  $pt.ManualUpdate = $true
  if ($c.ContainsKey('rows')) { Add-Fields $pt $c.rows 1 }
  if ($c.ContainsKey('cols')) { Add-Fields $pt $c.cols 2 }
  if ($c.ContainsKey('pages')) { Add-Fields $pt $c.pages 3 }
  $pt.ManualUpdate = $false
  if ($c.ContainsKey('layout')) {
    $pt.RowAxisLayout($(@{ compact = 0; tabular = 1; outline = 2 }[$c.layout]))
  }
  if ($c.ContainsKey('repeatall')) { $pt.RepeatAllLabels($(if ($c.repeatall) { 2 } else { 1 })) }
  if ($c.ContainsKey('rowGrand')) { $pt.RowGrand = [bool]$c.rowGrand }
  if ($c.ContainsKey('colGrand')) { $pt.ColumnGrand = [bool]$c.colGrand }
  if ($c.ContainsKey('rowsCaption')) { $pt.CompactLayoutRowHeader = $c.rowsCaption }
  if ($c.ContainsKey('colsCaption')) { $pt.CompactLayoutColumnHeader = $c.colsCaption }
  if ($c.ContainsKey('nullstr')) { $pt.NullString = $c.nullstr; $pt.DisplayNullString = $true }
  if ($c.ContainsKey('errstr')) { $pt.ErrorString = $c.errstr; $pt.DisplayErrorString = $true }
  if ($c.ContainsKey('showHeaders')) { $pt.DisplayFieldCaptions = [bool]$c.showHeaders }
  Apply-Grouping $pt $c
  $dfs = @()
  foreach ($d in @($c.data)) {
    $fnKey = 'sum'; if ($d.ContainsKey('fn')) { $fnKey = $d.fn }
    $cap = $d.name
    if (-not $cap) { $cap = "$($FnName[$fnKey]) of $($d.f)" }
    $df = $pt.AddDataField($pt.PivotFields($d.f), $cap, $FnCode[$fnKey])
    if ($d.ContainsKey('nf')) { $df.NumberFormat = $d.nf }
    if ($d.ContainsKey('show')) {
      $df.Calculation = $ShowAs[$d.show]
      if ($d.ContainsKey('base')) { $df.BaseField = $d.base }
      if ($d.ContainsKey('item')) { $df.BaseItem = $d.item }
    }
    $dfs += $df
  }
  if ($c.ContainsKey('dataOnRows') -and $c.dataOnRows) { $pt.DataPivotField.Orientation = 1 }
  Apply-FieldOptions $pt $c.rows
  Apply-FieldOptions $pt $c.cols
  Apply-FieldOptions $pt $c.pages
  return $pt
}

# ---------------------------------------------------------------------------------------------------------
# Dump the refreshed grid.
# ---------------------------------------------------------------------------------------------------------
$ErrNames = @{ -2146826281 = '#DIV/0!'; -2146826246 = '#N/A'; -2146826259 = '#NAME?'; -2146826288 = '#NULL!'
  -2146826252 = '#NUM!'; -2146826265 = '#REF!'; -2146826273 = '#VALUE!' }

function Col-Letters([int]$c) {
  $s = ''
  while ($c -gt 0) { $m = ($c - 1) % 26; $s = [string][char](65 + $m) + $s; $c = [int](($c - $m - 1) / 26) }
  return $s
}

function Dump-Pivot($pt, [string]$desc) {
  $tr = $pt.TableRange2
  $r0 = $tr.Row; $c0 = $tr.Column
  $nr = $tr.Rows.Count; $nc = $tr.Columns.Count
  $vals = $tr.Value2
  $ws = $pt.Parent
  $cells = New-Object Collections.ArrayList
  for ($r = 1; $r -le $nr; $r++) {
    for ($c = 1; $c -le $nc; $c++) {
      $x = $vals[$r, $c]
      if ($null -eq $x) { continue }
      $cell = $ws.Cells.Item(($r0 + $r - 1), ($c0 + $c - 1))
      $addr = (Col-Letters ($c0 + $c - 1)) + ($r0 + $r - 1)
      $nf = [string]$cell.NumberFormat
      $text = [string]$cell.Text
      if ($x -is [string]) { $v = Json-Str $x }
      elseif ($x -is [int]) { $en = $ErrNames[[int]$x]; if (-not $en) { $en = $text }; $v = '{"e":' + (Json-Str $en) + '}' }
      else { $v = ([double]$x).ToString('R', $inv) }
      [void]$cells.Add('{"a":' + (Json-Str $addr) + ',"v":' + $v + ',"nf":' + (Json-Str $nf) + ',"t":' + (Json-Str $text) + '}')
    }
  }
  $range = $tr.Address($false, $false)
  $loc = $pt.TableRange1.Address($false, $false)
  if ($Show) {
    Write-Host "== $($pt.Name) $desc range=$range"
    for ($r = 1; $r -le $nr; $r++) {
      $line = @()
      for ($c = 1; $c -le $nc; $c++) { $x = $vals[$r, $c]; if ($x -is [double]) { $x = [Math]::Round($x, 4) }; $line += [string]$x }
      Write-Host ($line -join ' | ')
    }
  }
  return '    {"sheet":' + (Json-Str $ws.Name) + ',"name":' + (Json-Str $pt.Name) + ',"case":' + (Json-Str $desc) +
    ',"range":' + (Json-Str $range) + ',"location":' + (Json-Str $loc) + ",`n     `"cells`":[`n      " + ($cells -join ",`n      ") + "`n     ]}"
}

function Write-Set($setName, $notes, [int]$rows, [long]$seed, $cases) {
  if ($Only -and $setName -notmatch $Only) { return }
  $path = Join-Path $fixtureDir "pivot-$setName.xlsx"
  if (Test-Path $path) { Remove-Item $path -Force }
  $wb = $script:xl.Workbooks.Add()
  $outs = New-Object Collections.ArrayList
  try {
    [void](New-Data $wb $rows $seed)
    $pc = $wb.PivotCaches().Create(1, "Data!R1C1:R$($rows + 1)C10")
    $k = 0
    foreach ($c in $cases) {
      $k++
      try {
        $pt = New-Pivot $wb $pc ("P$k") ("PT$k") $c
        [void]$outs.Add((Dump-Pivot $pt $c.desc))
        Write-Host "pivot-$setName P$k $($c.desc)"
      } catch {
        Write-Host "SKIPPED pivot-$setName P$k $($c.desc): $($_.Exception.Message)"
        try { $script:xl.DisplayAlerts = $false; $wb.Worksheets.Item("P$k").Delete() } catch {}
      }
    }
    $wb.Worksheets.Item('Data').Activate()
    $wb.RemovePersonalInformation = $true
    $wb.SaveAs($path, 51)
    $json = "{`n  `"source`": `"fixtures/xlsx/pivot-$setName.xlsx`",`n  `"oracle`": `"Microsoft Excel via COM; TableRange2 cells (Value2, NumberFormat, Text)`",`n" +
      "  `"excel_build`": `"$($script:xl.Build)`",`n  `"pivots`": [`n" + ($outs -join ",`n") + "`n  ]`n}`n"
    Write-Lf (Join-Path $refDir "pivot-$setName/pivot.json") $json
    $t = "feature = `"pivot`"`nmade_with = `"Microsoft Excel $($script:xl.Build) (Windows)`"`nauthor = `"project member`"`nexpect_dropped = []`nnotes = `"$notes`"`n"
    Write-Lf (Join-Path $fixtureDir "pivot-$setName.toml") $t
  } finally {
    try { $wb.Saved = $true; $wb.Close($false) } catch {}
  }
}

# ---------------------------------------------------------------------------------------------------------
# Cases
# ---------------------------------------------------------------------------------------------------------
function D($f, $fn = 'sum', $extra = @{}) { $h = @{ f = $f; fn = $fn }; foreach ($k in $extra.Keys) { $h[$k] = $extra[$k] }; return $h }

function Layout-Cases {
  $cases = @()
  foreach ($lay in 'compact', 'outline', 'tabular') {
    $cases += @{ desc = "$lay, one row field"; layout = $lay; rows = @('Region'); data = @(D 'Revenue') }
    $cases += @{ desc = "$lay, two row fields"; layout = $lay; rows = @('Region', 'Product'); data = @(D 'Revenue') }
    $cases += @{ desc = "$lay, three row fields"; layout = $lay; rows = @('Region', 'Channel', 'Product'); data = @(D 'Units') }
    $cases += @{ desc = "$lay, rows and one column field"; layout = $lay; rows = @('Region'); cols = @('Channel'); data = @(D 'Revenue') }
    $cases += @{ desc = "$lay, two rows two cols"; layout = $lay; rows = @('Region', 'Product'); cols = @('Channel', 'Tier'); data = @(D 'Units') }
    $cases += @{ desc = "$lay, two data fields on columns"; layout = $lay; rows = @('Region', 'Product'); data = @((D 'Revenue'), (D 'Units' 'count')) }
    $cases += @{ desc = "$lay, two data fields, column field"; layout = $lay; rows = @('Region'); cols = @('Channel'); data = @((D 'Revenue'), (D 'Units' 'average')) }
    $cases += @{ desc = "$lay, data on rows"; layout = $lay; rows = @('Region'); data = @((D 'Revenue'), (D 'Units' 'max')); dataOnRows = $true }
    $cases += @{ desc = "$lay, page field"; layout = $lay; rows = @('Region'); cols = @('Channel'); pages = @('Product'); data = @(D 'Revenue') }
  }
  $cases += @{ desc = 'columns only'; cols = @('Channel'); data = @(D 'Revenue') }
  $cases += @{ desc = 'no row or column fields'; data = @(D 'Revenue') }
  $cases += @{ desc = 'no data field'; rows = @('Region'); cols = @('Channel'); data = @() }
  return $cases
}

function Aggregate-Cases {
  $cases = @()
  foreach ($fn in 'sum', 'count', 'average', 'max', 'min', 'product', 'countnums', 'stddev', 'stddevp', 'var', 'varp') {
    $cases += @{ desc = "$fn of Units by Region and Channel"; rows = @('Region'); cols = @('Channel'); data = @(D 'Units' $fn) }
    $cases += @{ desc = "$fn of Score by Product"; rows = @('Product'); data = @(D 'Score' $fn @{ nf = '0.000' }) }
  }
  $cases += @{ desc = 'count of text field'; rows = @('Region'); data = @((D 'Channel' 'count'), (D 'Channel' 'countnums' @{ name = 'CountNums Channel' })) }
  $cases += @{ desc = 'same field twice'; rows = @('Region'); data = @((D 'Revenue' 'sum'), (D 'Revenue' 'average'), (D 'Revenue' 'max')) }
  $cases += @{ desc = 'custom captions and formats'; rows = @('Product'); cols = @('Channel')
    data = @((D 'Revenue' 'sum' @{ name = 'Total revenue'; nf = '$#,##0' }), (D 'Price' 'average' @{ name = 'Mean price'; nf = '0.0' })) }
  $cases += @{ desc = 'three data fields rows two levels'; layout = 'outline'; rows = @('Region', 'Channel'); data = @((D 'Units'), (D 'Price' 'min'), (D 'Score' 'max')) }
  return $cases
}

function Subtotal-Cases {
  $cases = @()
  $cases += @{ desc = 'no grand totals'; rows = @('Region'); cols = @('Channel'); rowGrand = $false; colGrand = $false; data = @(D 'Revenue') }
  $cases += @{ desc = 'row grand only'; rows = @('Region'); cols = @('Channel'); rowGrand = $true; colGrand = $false; data = @(D 'Revenue') }
  $cases += @{ desc = 'column grand only'; rows = @('Region'); cols = @('Channel'); rowGrand = $false; colGrand = $true; data = @(D 'Revenue') }
  $cases += @{ desc = 'tabular no subtotals'; layout = 'tabular'; rows = @(@{ f = 'Region'; sub = 'none' }, 'Product'); data = @(D 'Revenue') }
  $cases += @{ desc = 'outline no subtotals'; layout = 'outline'; rows = @(@{ f = 'Region'; sub = 'none' }, 'Product'); data = @(D 'Revenue') }
  $cases += @{ desc = 'compact no subtotals'; layout = 'compact'; rows = @(@{ f = 'Region'; sub = 'none' }, 'Product'); data = @(D 'Revenue') }
  $cases += @{ desc = 'outline subtotals at bottom'; layout = 'outline'; rows = @(@{ f = 'Region'; top = $false }, 'Product'); data = @(D 'Revenue') }
  $cases += @{ desc = 'compact subtotals at bottom'; layout = 'compact'; rows = @(@{ f = 'Region'; top = $false }, 'Product'); data = @(D 'Revenue') }
  $cases += @{ desc = 'tabular subtotals at top'; layout = 'tabular'; rows = @(@{ f = 'Region'; top = $true }, 'Product'); data = @(D 'Revenue') }
  $cases += @{ desc = 'custom subtotals sum and count'; layout = 'tabular'; rows = @(@{ f = 'Region'; sub = @('sum', 'count') }, 'Product'); data = @(D 'Revenue') }
  $cases += @{ desc = 'custom subtotals average max min'; layout = 'outline'; rows = @(@{ f = 'Region'; sub = @('average', 'max', 'min') }, 'Product'); data = @(D 'Revenue') }
  $cases += @{ desc = 'custom subtotals on columns'; rows = @('Region'); cols = @(@{ f = 'Channel'; sub = @('sum', 'average') }, 'Product'); data = @(D 'Units') }
  $cases += @{ desc = 'all subtotal functions'; layout = 'tabular'; rows = @(@{ f = 'Region'; sub = @('sum', 'count', 'average', 'max', 'min', 'product', 'countnums', 'stddev', 'stddevp', 'var', 'varp') }, 'Product'); data = @(D 'Score') }
  $cases += @{ desc = 'no subtotals on columns'; rows = @('Region'); cols = @(@{ f = 'Channel'; sub = 'none' }, 'Product'); data = @(D 'Units') }
  $cases += @{ desc = 'sort descending by label'; rows = @(@{ f = 'Region'; sort = 'desc' }); cols = @(@{ f = 'Channel'; sort = 'desc' }); data = @(D 'Revenue') }
  $cases += @{ desc = 'sort products ascending explicitly'; rows = @(@{ f = 'Product'; sort = 'asc' }); data = @(D 'Revenue') }
  $cases += @{ desc = 'sort by value descending'; rows = @(@{ f = 'Product'; sort = @{ by = 'Sum of Revenue'; dir = 'desc' } }); data = @(D 'Revenue') }
  $cases += @{ desc = 'sort by value ascending, two fields'; rows = @(@{ f = 'Region'; sort = @{ by = 'Sum of Units'; dir = 'asc' } }, @{ f = 'Product'; sort = @{ by = 'Sum of Units'; dir = 'desc' } }); data = @(D 'Units') }
  $cases += @{ desc = 'blank row after items'; layout = 'outline'; rows = @(@{ f = 'Region'; blank = $true }, 'Product'); data = @(D 'Revenue') }
  $cases += @{ desc = 'blank row after items compact'; layout = 'compact'; rows = @(@{ f = 'Region'; blank = $true }, 'Channel'); data = @(D 'Revenue') }
  $cases += @{ desc = 'repeat all labels tabular'; layout = 'tabular'; repeatall = $true; rows = @('Region', 'Channel', 'Product'); data = @(D 'Units') }
  $cases += @{ desc = 'repeat all labels outline'; layout = 'outline'; repeatall = $true; rows = @('Region', 'Product'); data = @(D 'Units') }
  $cases += @{ desc = 'hidden items'; rows = @(@{ f = 'Region'; hide = @('East') }); cols = @(@{ f = 'Channel'; hide = @('Partner') }); data = @(D 'Revenue') }
  $cases += @{ desc = 'page field filtered to one item'; rows = @('Region'); pages = @(@{ f = 'Channel'; page = 'Retail' }); data = @(D 'Revenue') }
  $cases += @{ desc = 'two page fields'; rows = @('Product'); pages = @('Channel', 'Region'); data = @(D 'Units') }
  $cases += @{ desc = 'custom header captions'; rows = @('Region'); cols = @('Channel'); rowsCaption = 'Areas'; colsCaption = 'Routes'; data = @(D 'Units') }
  $cases += @{ desc = 'field number format on numeric rows'; rows = @(@{ f = 'Score'; nf = '000' }); data = @(D 'Units' 'count') }
  $cases += @{ desc = 'dates as row labels'; rows = @('Date'); data = @(D 'Units') }
  return $cases
}

function ShowAs-Cases {
  $cases = @()
  foreach ($s in 'pcttotal', 'pctrow', 'pctcol', 'index') {
    $cases += @{ desc = "show $s with row and column fields"; rows = @('Region'); cols = @('Channel'); data = @(D 'Revenue' 'sum' @{ show = $s }) }
    $cases += @{ desc = "show $s two row fields tabular"; layout = 'tabular'; rows = @('Region', 'Product'); data = @(D 'Units' 'sum' @{ show = $s }) }
  }
  foreach ($s in 'pctparentrow', 'pctparentcol') {
    $cases += @{ desc = "show $s"; rows = @('Region', 'Product'); cols = @('Channel'); data = @(D 'Units' 'sum' @{ show = $s }) }
$cases += @{ desc = 'show pctparent of Region'; rows = @('Region', 'Product'); cols = @('Channel'); data = @(D 'Units' 'sum' @{ show = 'pctparent'; base = 'Region' }) }
  }
  $cases += @{ desc = 'running total along Product'; rows = @('Product'); cols = @('Channel'); data = @(D 'Units' 'sum' @{ show = 'running'; base = 'Product' }) }
  $cases += @{ desc = 'percent running total along Product'; rows = @('Product'); data = @(D 'Units' 'sum' @{ show = 'pctrunning'; base = 'Product' }) }
  $cases += @{ desc = 'difference from previous'; rows = @('Product'); cols = @('Channel'); data = @(D 'Units' 'sum' @{ show = 'diff'; base = 'Product'; item = '(previous)' }) }
  $cases += @{ desc = 'difference from a named item'; rows = @('Product'); data = @(D 'Units' 'sum' @{ show = 'diff'; base = 'Product'; item = 'Banana' }) }
  $cases += @{ desc = 'percent of a named item'; rows = @('Product'); data = @(D 'Units' 'sum' @{ show = 'pctof'; base = 'Product'; item = 'Banana' }) }
  $cases += @{ desc = 'percent difference from previous'; rows = @('Product'); data = @(D 'Units' 'sum' @{ show = 'pctdiff'; base = 'Product'; item = '(previous)' }) }
  $cases += @{ desc = 'rank ascending'; rows = @('Product'); data = @(D 'Units' 'sum' @{ show = 'rankasc'; base = 'Product' }) }
  $cases += @{ desc = 'rank descending'; rows = @('Product'); data = @(D 'Units' 'sum' @{ show = 'rankdesc'; base = 'Product' }) }
  $cases += @{ desc = 'two data fields one shown as percent'; rows = @('Region'); cols = @('Channel'); data = @((D 'Revenue'), (D 'Revenue' 'sum' @{ show = 'pcttotal'; name = 'Share' })) }
  return $cases
}

function Grouping-Cases {
  $cases = @()
  $cases += @{ desc = 'date by months'; rows = @('Date'); group = @(@{ f = 'Date'; kind = 'date'; by = @('months') }); data = @(D 'Revenue') }
  $cases += @{ desc = 'date by years and months'; rows = @('Date'); group = @(@{ f = 'Date'; kind = 'date'; by = @('months', 'years') }); data = @(D 'Revenue') }
  $cases += @{ desc = 'date by quarters'; rows = @('Date'); group = @(@{ f = 'Date'; kind = 'date'; by = @('quarters') }); data = @(D 'Units') }
  $cases += @{ desc = 'date by years'; rows = @('Date'); group = @(@{ f = 'Date'; kind = 'date'; by = @('years') }); cols = @('Channel'); data = @(D 'Units') }
  $cases += @{ desc = 'date by years quarters months'; layout = 'tabular'; rows = @('Date'); group = @(@{ f = 'Date'; kind = 'date'; by = @('months', 'quarters', 'years') }); data = @(D 'Units') }
  $cases += @{ desc = 'date by days step 30'; rows = @('Date'); group = @(@{ f = 'Date'; kind = 'date'; by = @('days'); step = 30 }); data = @(D 'Units') }
  $cases += @{ desc = 'date months on columns'; rows = @('Region'); cols = @('Date'); group = @(@{ f = 'Date'; kind = 'date'; by = @('months') }); data = @(D 'Units') }
  $cases += @{ desc = 'number group by 20'; rows = @('Score'); group = @(@{ f = 'Score'; kind = 'number'; start = 0; end = 100; step = 20 }); data = @(D 'Units') }
  $cases += @{ desc = 'number group by 25 auto bounds'; rows = @('Score'); cols = @('Channel'); group = @(@{ f = 'Score'; kind = 'number'; start = 0; end = 100; step = 25 }); data = @(D 'Revenue') }
  $cases += @{ desc = 'number group by 10 on units'; layout = 'tabular'; rows = @('Units'); group = @(@{ f = 'Units'; kind = 'number'; start = 1; end = 50; step = 10 }); data = @(D 'Score' 'average') }
  return $cases
}

function Holdout-Cases([long]$seed, [int]$count) {
  $rng = New-Object System.Random ([int]$seed)
  $rowPool = 'Region', 'Product', 'Channel', 'Tier', 'Size'
  $dataPool = 'Units', 'Price', 'Revenue', 'Score'
  $fns = 'sum', 'count', 'average', 'max', 'min', 'product', 'countnums', 'stddev', 'stddevp', 'var', 'varp'
  $layouts = 'compact', 'outline', 'tabular'
  $cases = @()
  for ($i = 0; $i -lt $count; $i++) {
    $pool = @($rowPool | Sort-Object { $rng.Next() })
    $nr = $rng.Next(0, 4); $nc = $rng.Next(0, 3); if ($nr + $nc -gt 3) { $nc = 3 - $nr }
    $np = 0; if ($nr + $nc -lt 3 -and $rng.Next(0, 3) -eq 0) { $np = 1 }
    $rows = @(); $cols = @(); $pages = @()
    $k = 0
    for ($j = 0; $j -lt $nr; $j++) { $rows += $pool[$k]; $k++ }
    for ($j = 0; $j -lt $nc; $j++) { $cols += $pool[$k]; $k++ }
    for ($j = 0; $j -lt $np; $j++) { $pages += $pool[$k]; $k++ }
    $nd = $rng.Next(1, 4)
    $data = @(); $used = @{}
    $desc = ''
    for ($j = 0; $j -lt $nd; $j++) {
      $f = $dataPool[$rng.Next(0, 4)]; $fn = $fns[$rng.Next(0, $fns.Count)]
      if ($used.ContainsKey("$f/$fn")) { continue }
      $used["$f/$fn"] = 1
      $data += (D $f $fn)
    }
    $c = @{ data = $data; layout = $layouts[$rng.Next(0, 3)] }
    if ($rows.Count) { $rr = @(); foreach ($f in $rows) { $h = @{ f = $f }; if ($rng.Next(0, 4) -eq 0) { $h.sub = 'none' }; if ($rng.Next(0, 5) -eq 0) { $h.sort = 'desc' }; $rr += $h }; $c.rows = $rr }
    if ($cols.Count) { $cc = @(); foreach ($f in $cols) { $h = @{ f = $f }; if ($rng.Next(0, 4) -eq 0) { $h.sub = 'none' }; $cc += $h }; $c.cols = $cc }
    if ($pages.Count) { $c.pages = $pages }
    if ($rng.Next(0, 4) -eq 0) { $c.rowGrand = $false }
    if ($rng.Next(0, 4) -eq 0) { $c.colGrand = $false }
    if ($rng.Next(0, 6) -eq 0 -and $c.layout -ne 'compact') { $c.repeatall = $true }
    $c.desc = "random $i layout=$($c.layout) rows=$($rows -join '/') cols=$($cols -join '/') pages=$($pages -join '/') data=" + (($data | ForEach-Object { "$($_.f):$($_.fn)" }) -join ',')
    $cases += $c
  }
  return $cases
}

# ---------------------------------------------------------------------------------------------------------
Start-Excel
try {
  Write-Set 'layouts' 'PivotTable layouts: compact, outline and tabular over rows, columns, pages and data fields' 80 1 (Layout-Cases)
  Write-Set 'aggregates' 'PivotTable aggregation functions, captions and number formats' 80 2 (Aggregate-Cases)
  Write-Set 'subtotals' 'PivotTable subtotals, grand totals, sorting, filters, blank rows and repeated labels' 80 3 (Subtotal-Cases)
  Write-Set 'showas' 'PivotTable Show Values As calculations' 80 4 (ShowAs-Cases)
  Write-Set 'grouping' 'PivotTable date and number grouping' 120 5 (Grouping-Cases)
  Write-Set 'holdout-a' 'hold-out: random PivotTable layouts over different source data' 70 7001 (Holdout-Cases 7001 14)
  Write-Set 'holdout-b' 'hold-out: random PivotTable layouts over different source data' 90 7002 (Holdout-Cases 7002 14)
} finally {
  Stop-Excel
}
