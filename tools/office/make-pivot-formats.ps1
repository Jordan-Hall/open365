<#
Fixtures for PivotTable user formats (crates/pivot formats.rs): formats the user applies to pivot areas (item labels, item
data, whole field label/data ranges, the data body, header ranges, single cells, the whole table) on top of a table style.
Per cell of TableRange2 the reference records Range.DisplayFormat (fill, bold/italic/colour, four borders) and Range.NumberFormat.
The formats are random operations drawn from a seeded generator over random layouts.

LOCAL DEVELOPER TOOL ONLY (not CI). Output
  fixtures/xlsx/pivot-<set>.xlsx  and .toml
  fixtures/reference/xlsx/pivot-<set>/pivot.json   {sheet, name, case, style, flags, range, cells:[{a,f,b,i,c,bd,nf}]}
    f fill colour (BGR integer) or -1; b,i bold/italic; c font colour (BGR); nf number format code;
    bd [left, top, bottom, right], each "" (none) or "<line style>:<weight>:<colour>"
Sets: formats (in-sample), holdout-formats (fresh seed, written once).

Process safety: own Excel instance only (aborts when COM attached to one already running), AutomationSecurity = 3,
Quit in finally. Other EXCEL processes are never touched. Data is synthetic.
#>
param([string]$Only)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class PvFmtWin { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p); }
'@
$before = @(Get-Process EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$xl = New-Object -ComObject Excel.Application
$procId = [uint32]0
[void][PvFmtWin]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$procId)
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

$script:rs = [long]1
function Next-Rand { $script:rs = ($script:rs * 1103515245 + 12345) % 2147483648; return [long]($script:rs / 65536) }
function Pick($arr) { $a = @($arr); return $a[(Next-Rand) % $a.Count] }
$Headers = 'Region', 'Product', 'Channel', 'Units', 'Revenue', 'Tier', 'Size'
$Items = @{ Region = 'North', 'South', 'East', 'West'; Product = 'Walnut', 'Apple', 'Banana', 'Cherry'; Channel = 'Online', 'Retail', 'Partner'; Tier = 'Gold', 'Silver', 'Bronze' }

function New-Data($wb, [int]$n, [long]$seed) {
  $script:rs = $seed
  $ws = $wb.Worksheets.Item(1); $ws.Name = 'Data'
  for ($i = 0; $i -lt 7; $i++) { $ws.Cells.Item(1, ($i + 1)).Value2 = $Headers[$i] }
  $reg = $Items.Region; $prod = $Items.Product; $chn = $Items.Channel
  for ($i = 0; $i -lt $n; $i++) {
    $row = $i + 2
    $ws.Cells.Item($row, 1).Value2 = [string]$reg[(Next-Rand) % 4]
    $ws.Cells.Item($row, 2).Value2 = [string]$prod[(Next-Rand) % 4]
    $ws.Cells.Item($row, 3).Value2 = [string]$chn[(Next-Rand) % 3]
    $ws.Cells.Item($row, 4).Value2 = [double](1 + (Next-Rand) % 50)
    $ws.Cells.Item($row, 5).Value2 = [double]([Math]::Round(1 + ((Next-Rand) % 99000) / 100.0, 2))
    $ws.Cells.Item($row, 6).Value2 = [string]$Items.Tier[(Next-Rand) % 3]
    $ws.Cells.Item($row, 7).Value2 = [string]('S', 'L')[(Next-Rand) % 2]
  }
}

$Colors = 255, 65280, 16711680, 65535, 16711935, 8421504, 49407, 12632256
$NumFmts = '0.00', '#,##0', '0%'

$script:propMode = 'any'
$script:maxOps = 4
function Apply-Props($rng, [bool]$isData) {
  $nprops = 1 + (Next-Rand) % 2
  if ($script:propMode -ne 'any') { $nprops = 1 }
  for ($k = 0; $k -lt $nprops; $k++) {
    $which = (Next-Rand) % 6
    if ($script:propMode -eq 'fill') { $which = 0 } elseif ($script:propMode -eq 'border') { $which = 4 }
    switch ($which) {
      0 { $rng.Interior.Color = [int](Pick $Colors) }
      1 { $rng.Font.Bold = $true }
      2 { $rng.Font.Italic = $true }
      3 { $rng.Font.Color = [int](Pick $Colors) }
      4 { $e = Pick (7, 8, 9, 10); $b = $rng.Borders.Item($e); $b.LineStyle = 1; $b.Color = [int](Pick $Colors) }
      5 { if ($isData) { $rng.NumberFormat = [string](Pick $NumFmts) } else { $rng.Font.Bold = $true } }
    }
  }
}

function Apply-Format($pt, $c) {
  $rowF = @($c.rows); $colF = @($c.cols)
  $all = @($rowF) + @($colF)
  switch ((Next-Rand) % 11) {
    0 { $f = Pick $rowF; $it = Pick $Items[$f]; $r = $pt.PivotFields($f).PivotItems($it).LabelRange; Apply-Props $r $false; return "label $f/$it" }
    1 { $f = Pick $rowF; $it = Pick $Items[$f]; $r = $pt.PivotFields($f).PivotItems($it).DataRange; Apply-Props $r $true; return "itemdata $f/$it" }
    2 { if ($colF.Count) { $f = Pick $colF; $it = Pick $Items[$f]; $r = $pt.PivotFields($f).PivotItems($it).DataRange; Apply-Props $r $true; return "colitemdata $f/$it" }
        else { $f = Pick $rowF; $r = $pt.PivotFields($f).LabelRange; Apply-Props $r $false; return "fieldlabel $f" } }
    3 { $f = Pick $rowF; $r = $pt.PivotFields($f).LabelRange; Apply-Props $r $false; return "fieldlabel $f" }
    4 { $f = Pick $rowF; $r = $pt.PivotFields($f).DataRange; Apply-Props $r $false; return "fielddata $f" }
    5 { Apply-Props $pt.DataBodyRange $true; return "databody" }
    6 { Apply-Props $pt.RowRange $false; return "rowrange" }
    7 { Apply-Props $pt.ColumnRange $false; return "colrange" }
    8 { $db = $pt.DataBodyRange; $r = $db.Cells.Item(1 + (Next-Rand) % $db.Rows.Count, 1 + (Next-Rand) % $db.Columns.Count); Apply-Props $r $true; return "cell $($r.Address($false,$false))" }
    9 { Apply-Props $pt.TableRange1 $false; return "table1" }
    10 { $db = $pt.DataBodyRange; $r = $db.Rows.Item($db.Rows.Count); Apply-Props $r $true; return "lastrow" }
  }
}

$FnCode = @{ sum = -4157; count = -4112; average = -4106; max = -4136 }
$FnName = @{ sum = 'Sum'; count = 'Count'; average = 'Average'; max = 'Max' }

function New-FormattedPivot($wb, $pc, [string]$sheetName, [string]$ptName, $c) {
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
  if ($c.ContainsKey('sub')) {
    foreach ($f in $c.sub) { $fl = New-Object 'object[]' 12; for ($i = 0; $i -lt 12; $i++) { $fl[$i] = $false }; $pt.PivotFields($f).Subtotals = $fl }
  }
  if ($c.ContainsKey('blank')) { foreach ($f in $c.blank) { $pt.PivotFields($f).LayoutBlankLine = $true } }
  $pt.TableStyle2 = $c.style
  $pt.ShowTableStyleRowStripes = [bool]$c.rs
  $pt.ShowTableStyleColumnStripes = [bool]$c.cs
  $pt.ShowTableStyleLastColumn = [bool]$c.lc
  $ops = @()
  $nops = 1 + (Next-Rand) % $script:maxOps
  for ($k = 0; $k -lt $nops; $k++) {
    try { $ops += (Apply-Format $pt $c) } catch { $ops += "failed($($_.Exception.Message.Split("`n")[0]))" }
  }
  $c.desc = "$($c.desc) | " + ($ops -join '; ')
  return $pt
}

function Dump-Styled($pt, $c) {
  $tr = $pt.TableRange2
  $ws = $pt.Parent
  $cells = New-Object Collections.ArrayList
  foreach ($cell in $tr.Cells) {
    $d = $cell.DisplayFormat
    $fill = -1; if ($d.Interior.Pattern -ne -4142) { $fill = [int]$d.Interior.Color }
    $bd = @()
    $bo4 = $d.Borders
    foreach ($e in 7, 8, 9, 10) {
      $b = $bo4.Item($e)
      $ls = $b.LineStyle
      if ($ls -eq -4142) { $bd += '""' } else { $bd += (Json-Str ("${ls}:$($b.Weight):$([int]$b.Color)")) }
    }
    $addr = (Col-Letters $cell.Column) + $cell.Row
    $bo = [int][bool]$d.Font.Bold; $it = [int][bool]$d.Font.Italic
    [void]$cells.Add('{"a":"' + $addr + '","f":' + $fill + ',"b":' + $bo + ',"i":' + $it + ',"c":' + [int]$d.Font.Color + ',"bd":[' + ($bd -join ',') + '],"nf":' + (Json-Str ([string]$cell.NumberFormat)) + '}')
  }
  $flags = "rs=$([int][bool]$c.rs) cs=$([int][bool]$c.cs) lc=$([int][bool]$c.lc)"
  return '    {"sheet":' + (Json-Str $ws.Name) + ',"name":' + (Json-Str $pt.Name) + ',"case":' + (Json-Str $c.desc) +
    ',"style":' + (Json-Str $c.style) + ',"flags":' + (Json-Str $flags) + ',"range":' + (Json-Str $tr.Address($false, $false)) +
    ",`n     `"cells`":[`n      " + ($cells -join ",`n      ") + "`n     ]}"
}

function Write-Set($setName, $notes, [int]$rows, [long]$seed, [long]$caseSeed, [int]$count, [string]$mode = 'any', [int]$maxOps = 4) {
  if ($Only -and $setName -notmatch $Only) { return }
  $script:propMode = $mode; $script:maxOps = $maxOps
  $path = Join-Path $fixtureDir "pivot-$setName.xlsx"
  if (Test-Path $path) { Remove-Item $path -Force }
  $wb = $xl.Workbooks.Add()
  $outs = New-Object Collections.ArrayList
  try {
    New-Data $wb $rows $seed
    $pc = $wb.PivotCaches().Create(1, "Data!R1C1:R$($rows + 1)C7")
    $script:rs = $caseSeed
    $styles = 'PivotStyleLight16', 'PivotStyleMedium9', 'PivotStyleDark2', 'PivotStyleLight1', 'PivotStyleMedium5', 'PivotStyleMedium12', 'PivotStyleLight21'
    for ($k = 1; $k -le $count; $k++) {
      $nr = 1 + (Next-Rand) % 3
      $rowsF = @('Region', 'Channel', 'Product') | Select-Object -First $nr
      $c = @{ rows = $rowsF; data = @(@{ f = 'Units'; fn = 'sum' }); style = (Pick $styles); rs = (Next-Rand) % 2; cs = (Next-Rand) % 2; lc = (Next-Rand) % 3 -eq 0 }
      $c.layout = Pick ('compact', 'tabular', 'outline')
      if ((Next-Rand) % 2 -eq 0) { $c.cols = @('Tier') } else { $c.cols = @() }
      if ((Next-Rand) % 4 -eq 0) { $c.pages = @('Size') }
      if ((Next-Rand) % 4 -eq 0 -and $nr -gt 1) { $c.sub = @($rowsF[0]) }
      if ((Next-Rand) % 5 -eq 0) { $c.blank = @($rowsF[0]) }
      $c.desc = "$($c.layout) rows=$($rowsF -join '/') cols=$($c.cols -join '/')"
      try {
        $pt = New-FormattedPivot $wb $pc "P$k" "PT$k" $c
        [void]$outs.Add((Dump-Styled $pt $c))
        Write-Host "pivot-$setName P$k $($c.desc)"
      } catch {
        Write-Host "SKIPPED pivot-$setName P$k $($c.desc): $($_.Exception.Message)"
        try { $xl.DisplayAlerts = $false; $wb.Worksheets.Item("P$k").Delete() } catch {}
      }
    }
    $wb.Worksheets.Item('Data').Activate()
    $wb.RemovePersonalInformation = $true
    $wb.SaveAs($path, 51)
    $json = "{`n  `"source`": `"fixtures/xlsx/pivot-$setName.xlsx`",`n  `"oracle`": `"Microsoft Excel via COM; Range.DisplayFormat of TableRange2 cells`",`n" +
      "  `"excel_build`": `"$($xl.Build)`",`n  `"pivots`": [`n" + ($outs -join ",`n") + "`n  ]`n}`n"
    Write-Lf (Join-Path $refDir "pivot-$setName/pivot.json") $json
    $t = "feature = `"pivot`"`nmade_with = `"Microsoft Excel $($xl.Build) (Windows)`"`nauthor = `"project member`"`nexpect_dropped = []`nnotes = `"$notes`"`n"
    Write-Lf (Join-Path $fixtureDir "pivot-$setName.toml") $t
  } finally {
    try { $wb.Saved = $true; $wb.Close($false) } catch {}
  }
}

try {
  $xl.AutomationSecurity = 3; $xl.Visible = $false; $xl.ScreenUpdating = $false; $xl.DisplayAlerts = $false; $xl.EnableEvents = $false
  Write-Set 'formats' 'PivotTable user formats on pivot areas over a table style, random operations (display format per cell)' 60 31 7 60
  Write-Set 'formats-areas' 'PivotTable user formats, one fill per pivot: which cells an area selects (display format per cell)' 60 41 11 70 'fill' 1
  Write-Set 'formats-borders' 'PivotTable user formats, one border per pivot: how an area border shows (display format per cell)' 60 43 13 50 'border' 1
  Write-Set 'holdout-formats' 'PivotTable user formats, hold-out: fresh seeds, written once (display format per cell)' 60 97 1234 40
} finally {
  try { foreach ($w in @($xl.Workbooks)) { $w.Saved = $true; $w.Close($false) } } catch {}
  try { $xl.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl) } catch {}
}
