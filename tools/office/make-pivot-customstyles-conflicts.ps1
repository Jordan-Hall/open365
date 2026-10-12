<#
Oracle for custom PivotTable style NAME CONFLICTS: what Excel shows when a workbook's styles.xml holds a custom table style
whose name is also a built-in style's (same case or other case), when two custom styles share a name, and when the pivot
names a table-only style. The cases are made by editing the styles.xml and pivotTable xml of a base workbook Excel wrote
(Excel's COM object model cannot make such files), opening the result in Excel and reading Range.DisplayFormat of every cell
of TableRange2 and PivotTable.TableStyle2. Excel is also asked to save the opened file, to record how it names the styles.
Writes fixtures/reference/xlsx/pivot-customstyles-conflicts/{case-N.xlsx, case-N-saved.xlsx, pivot.json}.
pivot.json: {pivots:[{case, style (name in the file), shown (TableStyle2 as Excel reports it), cells:[{a,f,b,i,c,bd}]}]}

LOCAL DEVELOPER TOOL ONLY (not CI). Own Excel instance only (aborts when COM attached to one already running); prints the
PID; AutomationSecurity = 3; Quit in finally; kills only that PID if it lingers. Data is synthetic.
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$outDir = Join-Path $root 'fixtures/reference/xlsx/pivot-customstyles-conflicts'
New-Item -ItemType Directory -Force $outDir | Out-Null
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class PvCsWin { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p); }
'@
$before = @(Get-Process EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$xl = New-Object -ComObject Excel.Application
$procId = [uint32]0
[void][PvCsWin]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$procId)
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
function Col-Letters([int]$c) { $s = ''; while ($c -gt 0) { $m = ($c - 1) % 26; $s = [string][char](65 + $m) + $s; $c = [int](($c - $m - 1) / 26) }; return $s }

function Read-Zip($path, $name) {
  $z = [IO.Compression.ZipFile]::OpenRead($path)
  try { $e = $z.GetEntry($name); $r = New-Object IO.StreamReader($e.Open()); return $r.ReadToEnd() } finally { $z.Dispose() }
}
function Write-Zip($src, $dst, $edits) {
  Copy-Item $src $dst -Force
  $z = [IO.Compression.ZipFile]::Open($dst, 'Update')
  try {
    foreach ($k in $edits.Keys) {
      $old = $z.GetEntry($k); $old.Delete()
      $new = $z.CreateEntry($k)
      $w = New-Object IO.StreamWriter($new.Open(), (New-Object Text.UTF8Encoding($false)))
      $w.Write($edits[$k]); $w.Close()
    }
  } finally { $z.Dispose() }
}

$fill = @{ red = 'FFFF0000'; blue = 'FF0000FF'; green = 'FF00B050' }
function Dxf($c) { return '<dxf><font><b/></font><fill><patternFill patternType="solid"><fgColor rgb="' + $fill[$c] + '"/><bgColor rgb="' + $fill[$c] + '"/></patternFill></fill></dxf>' }

# case: styles = list of @{name; colour; table; pivot}; use = name the pivot uses.
$cases = @(
  @{ desc = 'control: a unique custom style'; use = 'MyStyle'; styles = @(@{ name = 'MyStyle'; c = 'red' }) },
  @{ desc = 'custom style named like a built-in pivot style (same case)'; use = 'PivotStyleLight16'; styles = @(@{ name = 'PivotStyleLight16'; c = 'red' }) },
  @{ desc = 'custom style named like a built-in pivot style (other case)'; use = 'PivotStyleLight16'; styles = @(@{ name = 'pivotstylelight16'; c = 'red' }) },
  @{ desc = 'custom style named like another built-in (medium)'; use = 'PivotStyleMedium9'; styles = @(@{ name = 'PivotStyleMedium9'; c = 'green' }) },
  @{ desc = 'two custom styles with one name: red then blue'; use = 'MyStyle'; styles = @(@{ name = 'MyStyle'; c = 'red' }, @{ name = 'MyStyle'; c = 'blue' }) },
  @{ desc = 'two custom styles, names differing in case'; use = 'MyStyle'; styles = @(@{ name = 'mystyle'; c = 'red' }, @{ name = 'MyStyle'; c = 'blue' }) },
  @{ desc = 'a table-only custom style used by the pivot'; use = 'MyTbl'; styles = @(@{ name = 'MyTbl'; c = 'red'; pivot = 0; table = 1 }) },
  @{ desc = 'custom style named like a built-in table style'; use = 'TableStyleMedium2'; styles = @(@{ name = 'TableStyleMedium2'; c = 'red' }) }
)

try {
  $xl.AutomationSecurity = 3; $xl.Visible = $false; $xl.ScreenUpdating = $false; $xl.DisplayAlerts = $false; $xl.EnableEvents = $false
  $base = Join-Path $outDir 'base.xlsx'
  if (Test-Path $base) { Remove-Item $base -Force }
  $wb = $xl.Workbooks.Add()
  try {
    $ws = $wb.Worksheets.Item(1); $ws.Name = 'Data'
    $h = 'Region', 'Product', 'Channel', 'Units', 'Revenue'
    for ($i = 0; $i -lt 5; $i++) { $ws.Cells.Item(1, $i + 1).Value2 = $h[$i] }
    $reg = 'North', 'South', 'East'; $prod = 'Apple', 'Banana'; $chn = 'Online', 'Retail'
    for ($r = 0; $r -lt 24; $r++) {
      $ws.Cells.Item($r + 2, 1).Value2 = $reg[$r % 3]; $ws.Cells.Item($r + 2, 2).Value2 = $prod[$r % 2]
      $ws.Cells.Item($r + 2, 3).Value2 = $chn[[int][Math]::Floor($r / 3) % 2]
      $ws.Cells.Item($r + 2, 4).Value2 = [double](1 + $r); $ws.Cells.Item($r + 2, 5).Value2 = [double](10 * ($r + 1) + 0.5)
    }
    $pc = $wb.PivotCaches().Create(1, 'Data!R1C1:R25C5')
    $sh = $wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item(1)); $sh.Name = 'P1'
    $pt = $pc.CreatePivotTable("'P1'!R4C2", 'PT1')
    $pt.PivotFields('Region').Orientation = 1
    $pt.PivotFields('Channel').Orientation = 2
    [void]$pt.AddDataField($pt.PivotFields('Revenue'), 'Sum of Revenue', -4157)
    $pt.TableStyle2 = 'PivotStyleLight16'
    $wb.RemovePersonalInformation = $true
    $wb.SaveAs($base, 51)
  } finally { $wb.Saved = $true; $wb.Close($false) }

  $styles0 = Read-Zip $base 'xl/styles.xml'
  $pt0 = Read-Zip $base 'xl/pivotTables/pivotTable1.xml'
  $outs = New-Object Collections.ArrayList
  $k = 0
  foreach ($c in $cases) {
    $k++
    $path = Join-Path $outDir "case-$k.xlsx"; $saved = Join-Path $outDir "case-$k-saved.xlsx"
    foreach ($p in $path, $saved) { if (Test-Path $p) { Remove-Item $p -Force } }
    $dxfCount = [int]([regex]::Match($styles0, '<dxfs count="(\d+)"').Groups[1].Value)
    $dxfXml = ''; $tsXml = ''; $n = 0
    foreach ($s in $c.styles) {
      $dxfXml += (Dxf $s.c)
      $piv = 1; $tab = 0
      if ($s.ContainsKey('pivot')) { $piv = $s.pivot }
      if ($s.ContainsKey('table')) { $tab = $s.table }
      $attrs = ''
      if ($piv -eq 0) { $attrs += ' pivot="0"' }
      if ($tab -eq 1) { $attrs += ' table="1"' } else { $attrs += ' table="0"' }
      $tsXml += '<tableStyle name="' + $s.name + '"' + $attrs + ' count="1"><tableStyleElement type="wholeTable" dxfId="' + ($dxfCount + $n) + '"/></tableStyle>'
      $n++
    }
    $st = $styles0
    if ($st -match '<dxfs count="\d+"\s*/>') { $st = [regex]::Replace($st, '<dxfs count="\d+"\s*/>', "<dxfs count=`"$($dxfCount + $n)`">$dxfXml</dxfs>") }
    else { $st = [regex]::Replace($st, '<dxfs count="\d+">', "<dxfs count=`"$($dxfCount + $n)`">"); $st = $st.Replace('</dxfs>', $dxfXml + '</dxfs>') }
    $ts = [regex]::Match($st, '<tableStyles[^>]*?(/>|>.*?</tableStyles>)').Value
    $defaults = [regex]::Match($ts, 'defaultTableStyle="[^"]*"\s+defaultPivotStyle="[^"]*"').Value
    $st = $st.Replace($ts, "<tableStyles count=`"$n`" $defaults>$tsXml</tableStyles>")
    $ptx = $pt0.Replace('name="PivotStyleLight16"', 'name="' + $c.use + '"')
    Write-Zip $base $path @{ 'xl/styles.xml' = $st; 'xl/pivotTables/pivotTable1.xml' = $ptx }
    $w = $xl.Workbooks.Open($path)
    try {
      $pt = $w.Worksheets.Item('P1').PivotTables('PT1')
      $tr = $pt.TableRange2
      $cells = New-Object Collections.ArrayList
      foreach ($cell in $tr.Cells) {
        $d = $cell.DisplayFormat
        $f = -1; if ($d.Interior.Pattern -ne -4142) { $f = [int]$d.Interior.Color }
        $bd = @(); foreach ($e in 7, 8, 9, 10) { $b = $d.Borders.Item($e); if ($b.LineStyle -eq -4142) { $bd += '""' } else { $bd += (Json-Str ("$($b.LineStyle):$($b.Weight):$([int]$b.Color)")) } }
        [void]$cells.Add('{"a":"' + (Col-Letters $cell.Column) + $cell.Row + '","f":' + $f + ',"b":' + [int][bool]$d.Font.Bold + ',"i":' + [int][bool]$d.Font.Italic + ',"c":' + [int]$d.Font.Color + ',"bd":[' + ($bd -join ',') + ']}')
      }
      [void]$outs.Add('    {"case":' + (Json-Str $c.desc) + ',"file":' + (Json-Str "case-$k.xlsx") + ',"style":' + (Json-Str $c.use) + ',"shown":' + (Json-Str ([string]$pt.TableStyle2.Name)) + ',"sheet":"P1",' + "`n     `"cells`":[`n      " + ($cells -join ",`n      ") + "`n     ]}")
      Write-Host "case $k [$($c.desc)] shown=$($pt.TableStyle2.Name)"
      $w.RemovePersonalInformation = $true
      $w.SaveAs($saved, 51)
    } finally { $w.Saved = $true; $w.Close($false) }
  }
  $json = "{`n  `"oracle`": `"Microsoft Excel via COM; DisplayFormat of TableRange2 cells for hand-edited custom table style names`",`n  `"excel_build`": `"$($xl.Build)`",`n  `"pivots`": [`n" + ($outs -join ",`n") + "`n  ]`n}`n"
  [IO.File]::WriteAllText((Join-Path $outDir 'pivot.json'), ($json -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
} finally {
  try { foreach ($w in @($xl.Workbooks)) { $w.Saved = $true; $w.Close($false) } } catch {}
  try { $xl.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl) } catch {}
  Start-Sleep -Seconds 2
  if (Get-Process -Id $xlPid -ErrorAction SilentlyContinue) { Stop-Process -Id $xlPid -Force }
}
