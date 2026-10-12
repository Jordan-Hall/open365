<#
Fixtures for PivotTable table styles (crates/pivot style.rs). Builds PivotTables in Excel with the built-in
PivotStyleLight/Medium/Dark styles, in many layouts and with the style flags (row/column headers, row/column stripes,
last column) switched, and records for every cell of TableRange2 the format Excel shows (Range.DisplayFormat: fill,
font bold/italic/colour and the four borders). The definitions of the styles themselves come from
make-pivot-styledefs.ps1.

LOCAL DEVELOPER TOOL ONLY (not CI). Output
  fixtures/xlsx/pivot-<set>.xlsx  and .toml
  fixtures/reference/xlsx/pivot-<set>/pivot.json   {sheet, name, case, style, flags, range, cells:[{a,f,b,i,c,bd}]}
    f  fill colour (BGR integer) or -1 for no fill; b,i bold/italic; c font colour (BGR);
    bd [left, top, bottom, right], each "" (none) or "<line style>:<weight>:<colour>"
Set: tablestyles (worksheet table styles named as pivot styles).
After a run: pwsh -NoProfile -File tools/office/grid-geometry-oracle.ps1 -Only '^pivot-' and office-oracle.ps1 -Mode reference
for each new workbook, as pivot-oracle.ps1 describes.

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
public static class PvTblWin { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p); }
'@
$before = @(Get-Process EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$xl = New-Object -ComObject Excel.Application
$procId = [uint32]0
[void][PvTblWin]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$procId)
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

$script:rs = [long]1
function Next-Rand { $script:rs = ($script:rs * 1103515245 + 12345) % 2147483648; return [long]($script:rs / 65536) }
$Headers = 'Region', 'Product', 'Channel', 'Units', 'Revenue', 'Tier', 'Size'

function New-Data($wb, [int]$n, [long]$seed) {
  $script:rs = $seed
  $ws = $wb.Worksheets.Item(1); $ws.Name = 'Data'
  for ($i = 0; $i -lt 7; $i++) { $ws.Cells.Item(1, ($i + 1)).Value2 = $Headers[$i] }
  $reg = 'North', 'South', 'East', 'West'
  $prod = 'Walnut', 'Apple', 'Banana', 'Cherry'
  $chn = 'Online', 'Retail', 'Partner'
  for ($i = 0; $i -lt $n; $i++) {
    $row = $i + 2
    $ws.Cells.Item($row, 1).Value2 = [string]$reg[(Next-Rand) % 4]
    $ws.Cells.Item($row, 2).Value2 = [string]$prod[(Next-Rand) % 4]
    $ws.Cells.Item($row, 3).Value2 = [string]$chn[(Next-Rand) % 3]
    $ws.Cells.Item($row, 4).Value2 = [double](1 + (Next-Rand) % 50)
    $ws.Cells.Item($row, 5).Value2 = [double]([Math]::Round(1 + ((Next-Rand) % 99000) / 100.0, 2))
    $ws.Cells.Item($row, 6).Value2 = [string]('Gold', 'Silver', 'Bronze')[(Next-Rand) % 3]
    $ws.Cells.Item($row, 7).Value2 = [string]('S', 'L')[(Next-Rand) % 2]
  }
}

$FnCode = @{ sum = -4157; count = -4112; average = -4106; max = -4136 }
$FnName = @{ sum = 'Sum'; count = 'Count'; average = 'Average'; max = 'Max' }

function New-StyledPivot($wb, $pc, [string]$sheetName, [string]$ptName, $c) {
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
  if ($c.ContainsKey('rowGrand')) { $pt.RowGrand = [bool]$c.rowGrand }
  if ($c.ContainsKey('colGrand')) { $pt.ColumnGrand = [bool]$c.colGrand }
  foreach ($d in @($c.data)) {
    $fn = 'sum'; if ($d.ContainsKey('fn')) { $fn = $d.fn }
    [void]$pt.AddDataField($pt.PivotFields($d.f), "$($FnName[$fn]) of $($d.f)", $FnCode[$fn])
  }
  if ($c.ContainsKey('dataOnRows') -and $c.dataOnRows) { $pt.DataPivotField.Orientation = 1 }
  foreach ($f in @($c.rows) + @($c.cols)) {
    if (-not $f) { continue }
    $pf = $pt.PivotFields($f)
    if ($c.ContainsKey('sub') -and $c.sub.ContainsKey($f)) {
      $m = $c.sub[$f]
      if ($m -eq 'none') { $fl = New-Object 'object[]' 12; for ($i = 0; $i -lt 12; $i++) { $fl[$i] = $false }; $pf.Subtotals = $fl }
      elseif ($m -eq 'bottom') { $pf.LayoutSubtotalLocation = 2 }
      elseif ($m -eq 'top') { $pf.LayoutSubtotalLocation = 1 }
    }
    if ($c.ContainsKey('blank') -and $c.blank -contains $f) { $pf.LayoutBlankLine = $true }
  }
  $pt.TableStyle2 = $c.style
  $pt.ShowTableStyleRowHeaders = [bool]$c.rh
  $pt.ShowTableStyleColumnHeaders = [bool]$c.ch
  $pt.ShowTableStyleRowStripes = [bool]$c.rs
  $pt.ShowTableStyleColumnStripes = [bool]$c.cs
  $pt.ShowTableStyleLastColumn = [bool]$c.lc
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
    [void]$cells.Add('{"a":"' + $addr + '","f":' + $fill + ',"b":' + $bo + ',"i":' + $it + ',"c":' + [int]$d.Font.Color + ',"bd":[' + ($bd -join ',') + ']}')
  }
  $flags = "rh=$([int][bool]$c.rh) ch=$([int][bool]$c.ch) rs=$([int][bool]$c.rs) cs=$([int][bool]$c.cs) lc=$([int][bool]$c.lc)"
  return '    {"sheet":' + (Json-Str $ws.Name) + ',"name":' + (Json-Str $pt.Name) + ',"case":' + (Json-Str $c.desc) +
    ',"style":' + (Json-Str $c.style) + ',"flags":' + (Json-Str $flags) + ',"range":' + (Json-Str $tr.Address($false, $false)) +
    ",`n     `"cells`":[`n      " + ($cells -join ",`n      ") + "`n     ]}"
}

function Write-Set($setName, $notes, [int]$rows, [long]$seed, $cases) {
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
        $pt = New-StyledPivot $wb $pc "P$k" "PT$k" $c
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

function D($f, $fn = 'sum') { return @{ f = $f; fn = $fn } }
function S($style, $rh = 1, $ch = 1, $rs = 0, $cs = 0, $lc = 0) { return @{ style = $style; rh = $rh; ch = $ch; rs = $rs; cs = $cs; lc = $lc } }
function Case($desc, $s, $h) { $h.desc = $desc; foreach ($k in $s.Keys) { $h[$k] = $s[$k] }; return $h }


# Worksheet table styles (TableStyleLight/Medium/Dark) on PivotTables: a pivot can name them like its own styles.
function Style-CasesTable {
  $cases = @()
  $styles = 'TableStyleLight1', 'TableStyleLight9', 'TableStyleLight15', 'TableStyleLight21', 'TableStyleMedium2', 'TableStyleMedium4', 'TableStyleMedium9', 'TableStyleMedium15', 'TableStyleMedium28', 'TableStyleDark1', 'TableStyleDark6', 'TableStyleDark11'
  $k = 0
  foreach ($st in $styles) {
    $k++
    $fl = @(@(1, 1, 0, 0, 0), @(1, 1, 1, 0, 1), @(0, 1, 0, 1, 0), @(1, 0, 1, 1, 1))[$k % 4]
    $s = S $st $fl[0] $fl[1] $fl[2] $fl[3] $fl[4]
    $cases += Case "$st compact rows=Region/Channel cols=Tier" $s @{ rows = @('Region', 'Channel'); cols = @('Tier'); data = @(D 'Units') }
    $cases += Case "$st tabular rows=Region/Product" (S $st 1 1 1 0 0) @{ layout = 'tabular'; rows = @('Region', 'Product'); data = @(D 'Units') }
    $cases += Case "$st outline rows=Region/Channel pages=Size" (S $st 1 1 0 1 1) @{ layout = 'outline'; rows = @('Region', 'Channel'); pages = @('Size'); data = @(D 'Revenue' 'max') }
  }
  return $cases
}

try {
  $xl.AutomationSecurity = 3; $xl.Visible = $false; $xl.ScreenUpdating = $false; $xl.DisplayAlerts = $false; $xl.EnableEvents = $false
  Write-Set 'tablestyles' 'PivotTables with built-in worksheet table styles (TableStyleLight/Medium/Dark): display format per cell' 60 29 (Style-CasesTable)
} finally {
  try { foreach ($w in @($xl.Workbooks)) { $w.Saved = $true; $w.Close($false) } } catch {}
  try { $xl.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl) } catch {}
  Start-Sleep -Seconds 2
  if (Get-Process -Id $xlPid -ErrorAction SilentlyContinue) { Stop-Process -Id $xlPid -Force }
}
