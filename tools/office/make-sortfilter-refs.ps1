<#
Excel oracle for Sort, AutoFilter and Data Validation (Microsoft Excel 16.0 through COM).

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed outputs through crates/xlsx/tests/{sort,filter,validation}.rs:

  fixtures/xlsx/sortfilter-sort.xlsx          fixtures/reference/xlsx/sortfilter-sort/scenarios.json
  fixtures/xlsx/sortfilter-filter.xlsx        fixtures/reference/xlsx/sortfilter-filter/scenarios.json
  fixtures/xlsx/sortfilter-validation.xlsx    fixtures/reference/xlsx/sortfilter-validation/scenarios.json

Each fixture holds one sheet per scenario in its "before" state (all content is made up here, no personal data).
The script then applies the scenario's operation to that same open workbook through the Excel object model (the
Sort object, Range.AutoFilter, Validation.Add: the calls a recorded UI macro makes), reads the result and records
it in scenarios.json next to the scenario description:

  sort        the cell values / formulas / fill / font colour of the sorted range after Range.Sort
  filter      the hidden rows and the <autoFilter> element Excel saved for the sheet; "today" is Excel's TODAY()
  validation  Validation.Value (does the cell's value pass its own rule) for each candidate value, and the
              <dataValidations> element Excel saved

  pwsh -NoProfile -File tools/office/make-sortfilter-refs.ps1 [-Group sort|filter|validation|all]

The regional settings are those of the machine (Excel's International(...) values are recorded in the JSON); no
Windows or Office setting is changed. AutomationSecurity = 3 (macros never run). The script starts its own hidden
Excel, remembers its process id and quits it in the finally block; other Office processes are never touched. The
fixture workbooks are written only by this script; docProps/core.xml creator fields are scrubbed.
#>
param(
  [ValidateSet('sort', 'filter', 'validation', 'all')][string]$Group = 'all'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refRoot = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('open365-sortfilter-' + [guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $tmp)

Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$script:xl = $null
$script:xlPid = 0

function Start-Excel {
  $app = New-Object -ComObject Excel.Application
  $app.Visible = $false
  $app.DisplayAlerts = $false
  $app.EnableEvents = $false
  $app.AutomationSecurity = 3
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
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
    # Only the process this script started; the id was taken from our own window handle.
    if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $script:xlPid -Force -ErrorAction SilentlyContinue }
  }
  $script:xlPid = 0
}

# PowerShell's COM binder caches the first argument type of a call site, so a loop that assigns numbers, text and
# booleans to Value2 fails; InvokeMember has no such cache.
function SetV($rng, $v) {
  [void][System.__ComObject].InvokeMember('Value2', [Reflection.BindingFlags]::SetProperty, $null, $rng, @($v))
}
function SetP($obj, [string]$name, $v) {
  [void][System.__ComObject].InvokeMember($name, [Reflection.BindingFlags]::SetProperty, $null, $obj, @($v))
}

function Bgr([string]$hex) {
  $r = [Convert]::ToInt32($hex.Substring(0, 2), 16); $g = [Convert]::ToInt32($hex.Substring(2, 2), 16); $b = [Convert]::ToInt32($hex.Substring(4, 2), 16)
  return $r + 256 * $g + 65536 * $b
}
function HexOf([int]$bgr) { return ('{0:X2}{1:X2}{2:X2}' -f ($bgr -band 255), (($bgr -shr 8) -band 255), (($bgr -shr 16) -band 255)) }

function Col-Letter([int]$n) { $s = ''; while ($n -gt 0) { $m = ($n - 1) % 26; $s = [char](65 + $m) + $s; $n = [int][Math]::Floor(($n - 1) / 26) }; return $s }

$errCodes = @{ -2146826288 = '#NULL!'; -2146826281 = '#DIV/0!'; -2146826273 = '#VALUE!'; -2146826265 = '#REF!'; -2146826259 = '#NAME?'; -2146826252 = '#NUM!'; -2146826246 = '#N/A' }

function Scrub-Core([string]$path) {
  $zip = [IO.Compression.ZipFile]::Open($path, 'Update')
  try {
    $entry = $zip.GetEntry('docProps/core.xml')
    if ($entry) {
      $reader = New-Object IO.StreamReader($entry.Open(), (New-Object Text.UTF8Encoding $false))
      $xml = $reader.ReadToEnd(); $reader.Close()
      $xml = [regex]::Replace($xml, '<dc:creator>.*?</dc:creator>', '<dc:creator></dc:creator>')
      $xml = [regex]::Replace($xml, '<cp:lastModifiedBy>.*?</cp:lastModifiedBy>', '<cp:lastModifiedBy></cp:lastModifiedBy>')
      $entry.Delete()
      $new = $zip.CreateEntry('docProps/core.xml')
      $w = New-Object IO.StreamWriter($new.Open(), (New-Object Text.UTF8Encoding $false))
      $w.Write($xml); $w.Close()
    }
  }
  finally { $zip.Dispose() }
}

function Read-ZipText([string]$path, [string]$entryName) {
  $zip = [IO.Compression.ZipFile]::OpenRead($path)
  try {
    $e = $zip.GetEntry($entryName)
    if (-not $e) { return $null }
    $r = New-Object IO.StreamReader($e.Open(), (New-Object Text.UTF8Encoding $false))
    $t = $r.ReadToEnd(); $r.Close(); return $t
  }
  finally { $zip.Dispose() }
}

# Sheet part names by sheet position, from the saved workbook and its relationships.
function Sheet-Parts([string]$path) {
  $wbx = [xml](Read-ZipText $path 'xl/workbook.xml')
  $rels = [xml](Read-ZipText $path 'xl/_rels/workbook.xml.rels')
  $map = @{}
  foreach ($r in $rels.Relationships.Relationship) { $map[$r.Id] = $r.Target }
  $out = @{}
  foreach ($s in $wbx.workbook.sheets.sheet) {
    $rid = $s.GetAttribute('id', 'http://schemas.openxmlformats.org/officeDocument/2006/relationships')
    $t = $map[$rid]; if ($t.StartsWith('/')) { $t = $t.TrimStart('/') } else { $t = 'xl/' + $t }
    $out[$s.name] = $t
  }
  return $out
}

function Extract-Element([string]$xml, [string]$name) {
  $m = [regex]::Match($xml, "<$name(\s[^>]*)?(/>|>.*?</$name>)", 'Singleline')
  if ($m.Success) { return $m.Value } else { return $null }
}

# ---- cell spec -> workbook ---------------------------------------------------------------------------------

function Put-Cell($ws, [int]$r, [int]$c, $spec) {
  if ($null -eq $spec) { return }
  $rng = $ws.Cells.Item($r, $c)
  if ($spec -is [hashtable]) {
    if ($spec.ContainsKey('nf')) { SetP $rng 'NumberFormat' $spec.nf }
    if ($spec.ContainsKey('f')) { SetP $rng 'Formula' $spec.f }
    elseif ($spec.ContainsKey('text')) { SetV $rng ("'" + $spec.text) }
    elseif ($spec.ContainsKey('v')) { SetV $rng $spec.v }
    if ($spec.ContainsKey('fill')) { SetP $rng.Interior 'Color' (Bgr $spec.fill) }
    if ($spec.ContainsKey('font')) { SetP $rng.Font 'Color' (Bgr $spec.font) }
    if ($spec.ContainsKey('bold')) { SetP $rng.Font 'Bold' $true }
    return
  }
  SetV $rng $spec
}

function Fill-Sheet($ws, $rows) {
  for ($i = 0; $i -lt $rows.Count; $i++) {
    $row = $rows[$i]
    for ($j = 0; $j -lt $row.Count; $j++) { Put-Cell $ws ($i + 1) ($j + 1) $row[$j] }
  }
}

function Read-Rect($ws, [string]$addr, [bool]$colors) {
  $rng = $ws.Range($addr)
  $v = $rng.Value2; $f = $rng.Formula
  $nr = $rng.Rows.Count; $nc = $rng.Columns.Count
  $vals = @(); $forms = @(); $fills = @(); $fonts = @()
  for ($i = 1; $i -le $nr; $i++) {
    $rv = @(); $rf = @(); $rfill = @(); $rfont = @()
    for ($j = 1; $j -le $nc; $j++) {
      $x = if ($nr -eq 1 -and $nc -eq 1) { $v } else { $v[$i, $j] }
      $fx = if ($nr -eq 1 -and $nc -eq 1) { $f } else { $f[$i, $j] }
      if ($null -eq $x) { $rv += , $null }
      elseif ($x -is [int]) { $rv += , @{ e = $errCodes[$x] } }
      else { $rv += , $x }
      if ($fx -is [string] -and $fx.StartsWith('=')) { $rf += , $fx } else { $rf += , $null }
      if ($colors) {
        $cell = $rng.Cells.Item($i, $j)
        if ($cell.Interior.ColorIndex -eq -4142) { $rfill += , $null } else { $rfill += , (HexOf ([int]$cell.Interior.Color)) }
        $rfont += , (HexOf ([int]$cell.Font.Color))
      }
    }
    $vals += , $rv; $forms += , $rf
    if ($colors) { $fills += , $rfill; $fonts += , $rfont }
  }
  $o = [ordered]@{ values = $vals; formulas = $forms }
  if ($colors) { $o.fills = $fills; $o.fonts = $fonts }
  return $o
}

function Write-Json([string]$path, $obj) {
  $dir = Split-Path -Parent $path
  if (-not (Test-Path $dir)) { [void](New-Item -ItemType Directory -Path $dir) }
  $json = $obj | ConvertTo-Json -Depth 20
  [IO.File]::WriteAllText($path, $json + "`n", (New-Object Text.UTF8Encoding $false))
}

function Intl-Info($app) {
  return [ordered]@{
    excel = $app.Version
    culture = (Get-Culture).Name
    decimal = $app.International(3)
    list = $app.International(5)
    country = $app.International(1)
    dateOrder = $app.International(32)
  }
}

function New-Fixture($app, $scenarios, [string]$file) {
  $wb = $app.Workbooks.Add()
  while ($wb.Worksheets.Count -lt $scenarios.Count) { [void]$wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
  for ($i = 0; $i -lt $scenarios.Count; $i++) {
    $ws = $wb.Worksheets.Item($i + 1)
    $ws.Name = $scenarios[$i].id
    Fill-Sheet $ws $scenarios[$i].cells
  }
  $path = Join-Path $fixtureDir $file
  if (Test-Path $path) { Remove-Item $path -Force }
  $wb.SaveAs($path, 51)
  return $wb
}

# =========================================================================================================
# SORT
# =========================================================================================================

function Sort-Scenarios {
  $mixed = @(3, 'b', 'A', 'a', $true, $false, 'B', $null, '-x', 'x', @{ text = 'y' }, 1.5, 'ab', 'a b', 'a-b', @{ f = '=1/0' }, @{ f = '=""' }, @{ f = '=NA()' }, 0, -2, 'Z', 'z', 'é', 'e', 'f')
  $col = { param($list) $rows = @(); foreach ($x in $list) { $rows += , @($x) }; return , $rows }
  $words = @('banana', 'Apple', 'cherry', 'apple', 'Banana', 'APPLE', 'cherry', 'Cherry', 'date', 'Date')
  $s = @()
  $s += @{ id = 'mixed-asc'; cells = (& $col $mixed); range = 'A1:A25'; header = 'no'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'mixed-desc'; cells = (& $col $mixed); range = 'A1:A25'; header = 'no'; keys = @(@{ col = 'A'; order = 'desc' }) }
  $s += @{ id = 'case-insens'; cells = (& $col $words); range = 'A1:A10'; header = 'no'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'case-sens-asc'; cells = (& $col $words); range = 'A1:A10'; header = 'no'; match_case = $true; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'case-sens-desc'; cells = (& $col $words); range = 'A1:A10'; header = 'no'; match_case = $true; keys = @(@{ col = 'A'; order = 'desc' }) }
  $punct = @('co-op', 'coop', 'co op', "o'neil", 'oneil', 'o neil', 'Cote', 'côte', 'coté', 'cote', 'a1', 'a10', 'a2', 'a-1', '10', '9', '_x', '~y', '#z', 'ß', 'ss', 'st', 'æ', 'ae', 'af')
  $s += @{ id = 'punct-accents'; cells = (& $col $punct); range = 'A1:A25'; header = 'no'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'numbers-text'; cells = (& $col @(@{ text = '10' }, @{ text = '9' }, 10, 9, @{ text = '1e3' }, 1000, -5, @{ text = '-5' }, 0.5, @{ f = '=""' }, 'abc')); range = 'A1:A11'; header = 'no'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $table = @(
    @('Name', 'Dept', 'Score', 'Note'),
    @('Ann', 'Ops', 70, 'a'), @('Bob', 'Dev', 85, 'b'), @('Cy', 'Ops', 85, 'c'), @('Di', 'Dev', 70, 'd'),
    @('Ed', 'Ops', 60, 'e'), @('Flo', 'Dev', 85, 'f'), @('Gus', 'Ops', 70, 'g'), @('Hal', 'Dev', 60, 'h'))
  $s += @{ id = 'header-yes'; cells = $table; range = 'A1:D9'; header = 'yes'; keys = @(@{ col = 'C'; order = 'asc' }) }
  $s += @{ id = 'header-no'; cells = $table; range = 'A1:D9'; header = 'no'; keys = @(@{ col = 'C'; order = 'asc' }) }
  $s += @{ id = 'header-guess-text-over-numbers'; cells = $table; range = 'A1:D9'; header = 'guess'; keys = @(@{ col = 'C'; order = 'desc' }) }
  $s += @{ id = 'header-guess-all-text'; cells = (& $col @('Name', 'zed', 'amy', 'bob')); range = 'A1:A4'; header = 'guess'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'header-guess-numbers'; cells = (& $col @(5, 3, 9, 1)); range = 'A1:A4'; header = 'guess'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'header-guess-two-cols'; cells = @(@('Item', 'Qty'), @('pear', 3), @('fig', 1), @('kiwi', 2)); range = 'A1:B4'; header = 'guess'; keys = @(@{ col = 'B'; order = 'asc' }) }
  $s += @{ id = 'multi-key'; cells = $table; range = 'A1:D9'; header = 'yes'; keys = @(@{ col = 'B'; order = 'asc' }, @{ col = 'C'; order = 'desc' }) }
  $s += @{ id = 'multi-key-desc-first'; cells = $table; range = 'A1:D9'; header = 'yes'; keys = @(@{ col = 'C'; order = 'desc' }, @{ col = 'B'; order = 'desc' }, @{ col = 'A'; order = 'desc' }) }
  $lr = @(@('k', 'b', 'a', 'c', 'B'), @(1, 2, 3, 4, 5), @('x', 'y', 'z', 'w', 'v'))
  $s += @{ id = 'left-to-right'; cells = $lr; range = 'A1:E3'; header = 'no'; orientation = 'rows'; keys = @(@{ col = 'A'; order = 'asc'; row = 1 }) }
  $s += @{ id = 'left-to-right-header'; cells = $lr; range = 'A1:E3'; header = 'yes'; orientation = 'rows'; keys = @(@{ col = 'A'; order = 'desc'; row = 2 }) }
  $pri = @('Priority', 'Low', 'High', 'Medium', 'high', 'Urgent', 'LOW', 'Medium', 'None', $null, 'High')
  $s += @{ id = 'custom-list'; cells = (& $col $pri); range = 'A1:A11'; header = 'yes'; keys = @(@{ col = 'A'; order = 'asc'; custom = 'High,Medium,Low' }) }
  $s += @{ id = 'custom-list-desc'; cells = (& $col $pri); range = 'A1:A11'; header = 'yes'; keys = @(@{ col = 'A'; order = 'desc'; custom = 'High,Medium,Low' }) }
  $days = @('Day', 'Wed', 'mon', 'Sun', 'Fri', 'Tue', 'Sat', 'Thu', 'Mon', 'Funday')
  $s += @{ id = 'custom-list-days'; cells = (& $col $days); range = 'A1:A10'; header = 'yes'; keys = @(@{ col = 'A'; order = 'asc'; custom = 'Mon,Tue,Wed,Thu,Fri,Sat,Sun' }) }
  $cc = @(
    @('Item', 'Val'),
    @(@{ v = 'a'; fill = 'FF0000' }, 5), @(@{ v = 'b'; fill = '0000FF' }, 4), @('c', 3), @(@{ v = 'd'; fill = 'FF0000' }, 2),
    @(@{ v = 'e'; fill = '00FF00' }, 1), @(@{ v = 'f'; fill = '0000FF' }, 9), @('g', 8), @(@{ v = 'h'; fill = 'FF0000' }, 7))
  $s += @{ id = 'cell-color-top'; cells = $cc; range = 'A1:B9'; header = 'yes'; colors = $true; keys = @(@{ col = 'A'; order = 'asc'; on = 'cellcolor'; color = '0000FF' }) }
  $s += @{ id = 'cell-color-bottom'; cells = $cc; range = 'A1:B9'; header = 'yes'; colors = $true; keys = @(@{ col = 'A'; order = 'desc'; on = 'cellcolor'; color = '0000FF' }) }
  $s += @{ id = 'cell-color-then-value'; cells = $cc; range = 'A1:B9'; header = 'yes'; colors = $true; keys = @(@{ col = 'A'; order = 'asc'; on = 'cellcolor'; color = 'FF0000' }, @{ col = 'B'; order = 'asc' }) }
  $s += @{ id = 'cell-color-two-levels'; cells = $cc; range = 'A1:B9'; header = 'yes'; colors = $true; keys = @(@{ col = 'A'; order = 'asc'; on = 'cellcolor'; color = '00FF00' }, @{ col = 'A'; order = 'asc'; on = 'cellcolor'; color = 'FF0000' }) }
  $fc = @(
    @('Item', 'Val'),
    @(@{ v = 'a'; font = 'FF0000' }, 5), @(@{ v = 'b'; font = '0000FF' }, 4), @('c', 3), @(@{ v = 'd'; font = 'FF0000' }, 2),
    @(@{ v = 'e'; font = '00FF00' }, 1), @(@{ v = 'f'; font = '0000FF' }, 9))
  $s += @{ id = 'font-color-top'; cells = $fc; range = 'A1:B7'; header = 'yes'; colors = $true; keys = @(@{ col = 'A'; order = 'asc'; on = 'fontcolor'; color = '0000FF' }) }
  $s += @{ id = 'font-color-bottom'; cells = $fc; range = 'A1:B7'; header = 'yes'; colors = $true; keys = @(@{ col = 'A'; order = 'desc'; on = 'fontcolor'; color = 'FF0000' }) }
  $fm = @(
    @('n', 'double', 'prev', 'abs'),
    @(3, @{ f = '=A2*2' }, @{ f = '=A1' }, @{ f = '=$A$2' }),
    @(1, @{ f = '=A3*2' }, @{ f = '=A2' }, @{ f = '=$A$2' }),
    @(2, @{ f = '=A4*2' }, @{ f = '=A3' }, @{ f = '=$A$2' }))
  $s += @{ id = 'formulas-move'; cells = $fm; range = 'A1:D4'; header = 'yes'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $gaps = @(@('k', 'v'), @(3, 'c'), @($null, 'blank1'), @(1, 'a'), @($null, 'blank2'), @(2, 'b'))
  $s += @{ id = 'blank-keys-asc'; cells = $gaps; range = 'A1:B6'; header = 'yes'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'blank-keys-desc'; cells = $gaps; range = 'A1:B6'; header = 'yes'; keys = @(@{ col = 'A'; order = 'desc' }) }
  $dates = @(@{ v = 45000; nf = 'yyyy-mm-dd' }, @{ v = 44000; nf = 'yyyy-mm-dd' }, @{ v = 45500.5; nf = 'yyyy-mm-dd hh:mm' }, 45000.25, 'text', 2958465, 0)
  $s += @{ id = 'dates-numbers'; cells = (& $col $dates); range = 'A1:A7'; header = 'no'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $uni = @('zebra', 'Äpfel', 'apfel', 'Zebra', 'åsa', 'ärlig', 'ölig', 'öl', 'ove', 'Ωmega', 'alpha', 'Привет', 'абв', 'Бэ', '日本', 'ñandú', 'nube', 'ch', 'cz', 'd', 'll', 'lz', 'i', 'ı', 'İ')
  $s += @{ id = 'unicode-en'; cells = (& $col $uni); range = 'A1:A25'; header = 'no'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'custom-list-mixed'; cells = (& $col @('h', 'Low', 5, $true, 'High', 'zzz', @{ f = '=NA()' }, $null, 'Medium', -1, 'aaa', $false)); range = 'A1:A12'; header = 'no'; keys = @(@{ col = 'A'; order = 'asc'; custom = 'High,Medium,Low' }) }
  $s += @{ id = 'custom-list-mixed-desc'; cells = (& $col @('h', 'Low', 5, $true, 'High', 'zzz', @{ f = '=NA()' }, $null, 'Medium', -1, 'aaa', $false)); range = 'A1:A12'; header = 'no'; keys = @(@{ col = 'A'; order = 'desc'; custom = 'High,Medium,Low' }) }
  $s += @{ id = 'float-fuzz'; cells = (& $col @(@{ f = '=0.1+0.2' }, 0.3, @{ f = '=0.3+1E-15' }, @{ f = '=0.3-1E-16' }, 0.3000000000000001)); range = 'A1:A5'; header = 'no'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'float-fuzz-2'; cells = (& $col @(0.3000000000000001, @{ f = '=0.1+0.2' }, 0.3, @{ f = '=0.3-1E-16' }, @{ f = '=0.3+1E-15' })); range = 'A1:A5'; header = 'no'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $g = @{ api = 'range'; header = 'guess' }
  $s += @{ id = 'g-text-numbers'; cells = @(@('Item', 'Qty'), @('pear', 3), @('fig', 1), @('kiwi', 2)); range = 'A1:B4'; api = 'range'; header = 'guess'; keys = @(@{ col = 'B'; order = 'asc' }) }
  $s += @{ id = 'g-all-text'; cells = (& $col @('zz', 'b', 'a', 'c')); range = 'A1:A4'; api = 'range'; header = 'guess'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'g-all-numbers'; cells = (& $col @(9, 5, 3, 7)); range = 'A1:A4'; api = 'range'; header = 'guess'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'g-bold-text'; cells = @(@(@{ v = 'zzA'; bold = $true }, @{ v = 'zzB'; bold = $true }), @('x', 'q'), @('y', 'p'), @('w', 'r')); range = 'A1:B4'; api = 'range'; header = 'guess'; keys = @(@{ col = 'B'; order = 'asc' }) }
  $s += @{ id = 'g-text-num-same'; cells = @(@('zzA', 9), @('x', 3), @('y', 1), @('w', 2)); range = 'A1:B4'; api = 'range'; header = 'guess'; keys = @(@{ col = 'B'; order = 'asc' }) }
  $s += @{ id = 'g-blank-second-row'; cells = @(@('zzA', 'zzB'), @('x', $null), @('y', 1), @('w', 2)); range = 'A1:B4'; api = 'range'; header = 'guess'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'g-bool'; cells = @(@('zzA', 'zzB'), @($true, $false), @($false, $true), @($true, $true)); range = 'A1:B4'; api = 'range'; header = 'guess'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'g-numfmt-differs'; cells = @(@(@{ v = 'zzA'; nf = '0.00' }, @{ v = 'zzB'; nf = '0.00' }), @('x', 'q'), @('y', 'p'), @('w', 'r')); range = 'A1:B4'; api = 'range'; header = 'guess'; keys = @(@{ col = 'B'; order = 'asc' }) }
  $s += @{ id = 'g-numeric-text-first-row'; cells = @(@(@{ text = '5' }, @{ text = '6' }), @(1, 2), @(3, 4)); range = 'A1:B3'; api = 'range'; header = 'guess'; keys = @(@{ col = 'A'; order = 'desc' }) }
  $s += @{ id = 'g-text-over-numeric-text'; cells = (& $col @('zzA', @{ text = '1' }, @{ text = '3' }, @{ text = '2' })); range = 'A1:A4'; api = 'range'; header = 'guess'; keys = @(@{ col = 'A'; order = 'asc' }) }
  $s += @{ id = 'g-two-keys'; cells = @(@('Name', 'Dept', 'Score'), @('Ann', 'Ops', 70), @('Bob', 'Dev', 85), @('Cy', 'Ops', 85), @('Di', 'Dev', 70)); range = 'A1:C5'; api = 'range'; header = 'guess'; keys = @(@{ col = 'B'; order = 'asc' }, @{ col = 'C'; order = 'desc' }) }
  $s += @{ id = 'g-yes-range-api'; cells = @(@('zzA', 'zzB'), @('x', 'q'), @('y', 'p'), @('w', 'r')); range = 'A1:B4'; api = 'range'; header = 'yes'; keys = @(@{ col = 'B'; order = 'asc' }) }
  return , $s
}

function Apply-Sort($app, $ws, $sc) {
  if ($sc.api -eq 'range') {
    $rng = $ws.Range($sc.range)
    $miss = [Type]::Missing
    $kr = @(); $ko = @()
    foreach ($k in $sc.keys) { $kr += , $ws.Range($rng.Columns.Item(([int][char]$k.col - 64)).Address($false, $false)); $ko += , $(if ($k.order -eq 'desc') { 2 } else { 1 }) }
    while ($kr.Count -lt 3) { $kr += , $miss; $ko += , 1 }
    $hdr = switch ($sc.header) { 'yes' { 1 } 'no' { 2 } default { 0 } }
    [void]$rng.Sort($kr[0], $ko[0], $kr[1], $miss, $ko[1], $kr[2], $ko[2], $hdr, $miss, [bool]$sc.match_case, 1)
    return
  }
  $sort = $ws.Sort
  $sort.SortFields.Clear()
  $rng = $ws.Range($sc.range)
  $orient = if ($sc.orientation -eq 'rows') { 2 } else { 1 }
  foreach ($k in $sc.keys) {
    $on = switch ($k.on) { 'cellcolor' { 1 } 'fontcolor' { 2 } default { 0 } }
    $order = if ($k.order -eq 'desc') { 2 } else { 1 }
    if ($sc.orientation -eq 'rows') { $addr = $ws.Range($sc.range).Rows.Item(([int]$k.row)).Address($false, $false) } else { $keyCol = $rng.Columns.Item(([int][char]$k.col - 64)); $addr = $keyCol.Address($false, $false) }
    $kr = $ws.Range($addr)
    if ($k.custom) { $f = $sort.SortFields.Add2($kr, $on, $order, $k.custom) } else { $f = $sort.SortFields.Add2($kr, $on, $order) }
    if ($on -ne 0) { $f.SortOnValue.Color = (Bgr $k.color) }
  }
  $sort.SetRange($rng)
  $sort.Header = switch ($sc.header) { 'yes' { 1 } 'no' { 2 } default { 0 } }
  $sort.MatchCase = [bool]$sc.match_case
  $sort.Orientation = $orient
  $sort.Apply()
}

function Run-Sort($app) {
  $scenarios = Sort-Scenarios
  $wb = New-Fixture $app $scenarios 'sortfilter-sort.xlsx'
  $out = @()
  for ($i = 0; $i -lt $scenarios.Count; $i++) {
    $sc = $scenarios[$i]; $ws = $wb.Worksheets.Item($i + 1)
    $before = Read-Rect $ws $sc.range ([bool]$sc.colors)
    Apply-Sort $app $ws $sc
    $after = Read-Rect $ws $sc.range ([bool]$sc.colors)
    $entry = [ordered]@{ id = $sc.id; range = $sc.range; header = $sc.header; orientation = if ($sc.orientation) { $sc.orientation } else { 'columns' }; match_case = [bool]$sc.match_case; keys = $sc.keys; before = $before; after = $after }
    $out += , $entry
    Write-Host "sort $($sc.id)"
  }
  $wb.Saved = $true; $wb.Close($false)
  Scrub-Core (Join-Path $fixtureDir 'sortfilter-sort.xlsx')
  Write-Json (Join-Path $refRoot 'sortfilter-sort/scenarios.json') ([ordered]@{ intl = (Intl-Info $app); scenarios = $out })
}

# =========================================================================================================
# FILTER
# =========================================================================================================

function Filter-Base([double]$today) {
  $d = { param($off) @{ v = ($today + $off); nf = 'yyyy-mm-dd' } }
  $names = @('Ann', 'bob', 'Cy', 'Di', 'Ed', 'Flo', 'Gus', 'Hal', 'Ivy', 'Jo', 'Kim', 'Lee', 'Max', 'ann', 'Zed', $null, 'Ben', 'Bea', 'Abe', 'A*c')
  $qty = @(10, 20, 20, 5, 15, 30, 25, 40, 35, 10, 50, 45, 5, 1, $null, 60, 12.5, 7, 100, 3)
  $off = @(0, -1, 1, -7, 7, -30, 30, -90, 90, -365, 365, 3, -3, -400, 400, $null, 14, -14, 60, -60)
  $cat = @('red', 'blue', 'red', 'green', 'blue', 'red', 'green', 'blue', 'red', 'green', 'blue', 'red', 'green', 'blue', 'red', 'green', 'none', 'none', 'red', 'blue')
  $fills = @{ red = 'FF0000'; blue = '0000FF'; green = '00FF00' }
  $rows = @(, @('Name', 'Qty', 'Date', 'Cat', 'Flag'))
  for ($i = 0; $i -lt 20; $i++) {
    $c = @{ v = $cat[$i] }
    if ($fills.ContainsKey($cat[$i])) { $c.fill = $fills[$cat[$i]] }
    if ($cat[$i] -eq 'red') { $c.font = '0000FF' }
    if ($cat[$i] -eq 'green') { $c.font = '000000' }
    $q = $qty[$i]; if ($null -ne $q) { $q = @{ v = $q; nf = '0.00' } }
    $dt = if ($null -eq $off[$i]) { $null } else { & $d $off[$i] }
    $flag = if ($i % 3 -eq 0) { $true } elseif ($i % 3 -eq 1) { $false } else { 'x' }
    $rows += , @($names[$i], $q, $dt, $c, $flag)
  }
  return , $rows
}

function Filter-Scenarios([double]$today) {
  $base = Filter-Base $today
  $r = 'A1:E21'
  $s = @()
  $add = { param($id, $fields) $script:fs += , @{ id = $id; cells = $base; range = $r; fields = $fields } }
  $script:fs = @()
  & $add 'values-text' @(@{ field = 1; values = @('Ann', 'Cy', 'Zed') })
  & $add 'values-text-case' @(@{ field = 1; values = @('ann') })
  & $add 'values-blank' @(@{ field = 1; values = @('Ann', '') })
  & $add 'values-number-display' @(@{ field = 2; values = @('10.00', '20.00', '12.50') })
  & $add 'values-category' @(@{ field = 4; values = @('red') })
  & $add 'values-bool' @(@{ field = 5; values = @('TRUE') })
  & $add 'custom-gt' @(@{ field = 2; op = 'custom'; c1 = '>20' })
  & $add 'custom-ge' @(@{ field = 2; op = 'custom'; c1 = '>=20' })
  & $add 'custom-lt' @(@{ field = 2; op = 'custom'; c1 = '<10' })
  & $add 'custom-le' @(@{ field = 2; op = 'custom'; c1 = '<=10' })
  & $add 'custom-eq' @(@{ field = 2; op = 'custom'; c1 = '=20' })
  & $add 'custom-ne' @(@{ field = 2; op = 'custom'; c1 = '<>20' })
  & $add 'custom-and-between' @(@{ field = 2; op = 'custom'; c1 = '>=10'; c2 = '<=30'; logic = 'and' })
  & $add 'custom-or' @(@{ field = 2; op = 'custom'; c1 = '<5'; c2 = '>50'; logic = 'or' })
  & $add 'custom-text-begins' @(@{ field = 1; op = 'custom'; c1 = 'a*' })
  & $add 'custom-text-ends' @(@{ field = 1; op = 'custom'; c1 = '*e' })
  & $add 'custom-text-contains' @(@{ field = 1; op = 'custom'; c1 = '*e*' })
  & $add 'custom-text-not-contains' @(@{ field = 1; op = 'custom'; c1 = '<>*e*' })
  & $add 'custom-text-qmark' @(@{ field = 1; op = 'custom'; c1 = '???' })
  & $add 'custom-text-tilde' @(@{ field = 1; op = 'custom'; c1 = 'A~*c' })
  & $add 'custom-text-gt' @(@{ field = 1; op = 'custom'; c1 = '>Cy' })
  & $add 'custom-text-eq-ci' @(@{ field = 1; op = 'custom'; c1 = '=ANN' })
  & $add 'custom-eq-blank' @(@{ field = 1; op = 'custom'; c1 = '=' })
  & $add 'custom-ne-blank' @(@{ field = 1; op = 'custom'; c1 = '<>' })
  & $add 'custom-ne-text' @(@{ field = 1; op = 'custom'; c1 = '<>Ann' })
  & $add 'custom-number-on-text-col' @(@{ field = 1; op = 'custom'; c1 = '>5' })
  & $add 'custom-bool' @(@{ field = 5; op = 'custom'; c1 = '=TRUE' })
  & $add 'top10-items' @(@{ field = 2; op = 'top10items'; c1 = 3 })
  & $add 'top10-items-ties' @(@{ field = 2; op = 'top10items'; c1 = 6 })
  & $add 'bottom10-items' @(@{ field = 2; op = 'bottom10items'; c1 = 4 })
  & $add 'top10-percent' @(@{ field = 2; op = 'top10percent'; c1 = 25 })
  & $add 'top10-percent-10' @(@{ field = 2; op = 'top10percent'; c1 = 10 })
  & $add 'bottom10-percent' @(@{ field = 2; op = 'bottom10percent'; c1 = 20 })
  & $add 'top10-items-1' @(@{ field = 2; op = 'top10items'; c1 = 1 })
  & $add 'above-average' @(@{ field = 2; op = 'dynamic'; c1 = 'aboveAverage' })
  & $add 'below-average' @(@{ field = 2; op = 'dynamic'; c1 = 'belowAverage' })
  foreach ($p in 'today', 'yesterday', 'tomorrow', 'thisWeek', 'lastWeek', 'nextWeek', 'thisMonth', 'lastMonth', 'nextMonth', 'thisQuarter', 'lastQuarter', 'nextQuarter', 'thisYear', 'lastYear', 'nextYear', 'yearToDate') {
    & $add ("dynamic-$p") @(@{ field = 3; op = 'dynamic'; c1 = $p })
  }
  foreach ($p in 'Q1', 'Q2', 'Q3', 'Q4', 'M1', 'M3', 'M6', 'M12') { & $add ("dynamic-$p") @(@{ field = 3; op = 'dynamic'; c1 = $p }) }
  & $add 'cell-color-red' @(@{ field = 4; op = 'cellcolor'; c1 = 'FF0000' })
  & $add 'cell-color-blue' @(@{ field = 4; op = 'cellcolor'; c1 = '0000FF' })
  & $add 'cell-color-none' @(@{ field = 4; op = 'cellcolor'; c1 = 'none' })
  & $add 'font-color-blue' @(@{ field = 4; op = 'fontcolor'; c1 = '0000FF' })
  & $add 'font-color-black' @(@{ field = 4; op = 'fontcolor'; c1 = '000000' })
  & $add 'font-color-auto' @(@{ field = 4; op = 'fontcolor'; c1 = 'none' })
  & $add 'multi-column' @(@{ field = 4; values = @('red', 'blue') }, @{ field = 2; op = 'custom'; c1 = '>=10' })
  & $add 'multi-three' @(@{ field = 4; values = @('red', 'blue', 'green') }, @{ field = 2; op = 'custom'; c1 = '>=10' }, @{ field = 1; op = 'custom'; c1 = '*a*' })
  & $add 'date-custom-after' @(@{ field = 3; op = 'custom'; c1 = ('>' + [string]($today - 5)) })
  & $add 'custom-eq-display' @(@{ field = 2; op = 'custom'; c1 = '=20.00' })
  & $add 'custom-text-ge' @(@{ field = 1; op = 'custom'; c1 = '>=Di' })
  & $add 'custom-text-lt-or-blank' @(@{ field = 1; op = 'custom'; c1 = '<b'; c2 = '='; logic = 'or' })
  & $add 'custom-ne-and-ne' @(@{ field = 1; op = 'custom'; c1 = '<>Ann'; c2 = '<>Cy'; logic = 'and' })
  & $add 'values-star-literal' @(@{ field = 1; values = @('A*c') })
  # Dates with times, filtered by date groups (Criteria2 = level, date, level, date, ... in the machine's date order).
  $dt = @(@('d'), @(@{ v = 45000; nf = 'yyyy-mm-dd hh:mm' }), @(@{ v = 45001; nf = 'yyyy-mm-dd hh:mm' }), @(@{ v = 45031; nf = 'yyyy-mm-dd hh:mm' }), @(@{ v = 45365; nf = 'yyyy-mm-dd hh:mm' }), @(@{ v = 45366; nf = 'yyyy-mm-dd hh:mm' }), @(@{ v = 45000.5; nf = 'yyyy-mm-dd hh:mm' }), @(@{ v = 45000.75; nf = 'yyyy-mm-dd hh:mm' }), @(@{ v = 45000.5006; nf = 'yyyy-mm-dd hh:mm' }), @('text'), @($null))
  $gd = { param($id, $groups) $script:fs += , @{ id = $id; cells = $dt; range = 'A1:A11'; fields = @(@{ field = 1; op = 'dategroups'; groups = $groups }) } }
  & $gd 'dategroup-day' @(2, '15/03/2023')
  & $gd 'dategroup-month' @(1, '01/03/2023')
  & $gd 'dategroup-year' @(0, '01/01/2024')
  & $gd 'dategroup-multi' @(2, '15/03/2023', 2, '16/03/2023', 1, '01/04/2023')
  & $gd 'dategroup-hour' @(3, '15/03/2023 12:00:00')
  & $gd 'dategroup-minute' @(4, '15/03/2023 12:00:00')
  & $add 'custom-eq-or' @(@{ field = 2; op = 'custom'; c1 = '=20'; c2 = '=30'; logic = 'or' })
  & $add 'custom-ne-number-on-text' @(@{ field = 1; op = 'custom'; c1 = '<>20' })
  # Blank kinds and mixed types in one column (own data).
  $bk = { param($id, $f) $script:fs += , @{ id = $id; cells = $bkdata; range = 'A1:A9'; fields = @($f) } }
  $bkdata = @(@('h'), @('x'), @{ f = '=""' }, @{ text = ' ' }, @($null), @{ f = '=""&""' }, @('y'), @{ f = '=A8' }, @{ text = '' })
  & $bk 'blankkinds-values-blank' @{ field = 1; values = @('') }
  & $bk 'blankkinds-eq-empty' @{ field = 1; op = 'custom'; c1 = '=' }
  & $bk 'blankkinds-ne-empty' @{ field = 1; op = 'custom'; c1 = '<>' }
  & $bk 'blankkinds-eq-star' @{ field = 1; op = 'custom'; c1 = '=*' }
  & $bk 'blankkinds-ne-star' @{ field = 1; op = 'custom'; c1 = '<>*' }
  & $bk 'blankkinds-values-x' @{ field = 1; values = @('x') }
  & $bk 'blankkinds-values-space' @{ field = 1; values = @(' ') }
  $mx = @(@('h'), @(5), @{ text = '5' }, @(10), @{ text = '10' }, @('abc'), @($true), @($null), @(5.5), @{ f = '=1/0' })
  $mixed = { param($id, $f) $script:fs += , @{ id = $id; cells = $mx; range = 'A1:A10'; fields = @($f) } }
  & $mixed 'mixed-ge5' @{ field = 1; op = 'custom'; c1 = '>=5' }
  & $mixed 'mixed-eq5' @{ field = 1; op = 'custom'; c1 = '=5' }
  & $mixed 'mixed-ne5' @{ field = 1; op = 'custom'; c1 = '<>5' }
  & $mixed 'mixed-lt-abd' @{ field = 1; op = 'custom'; c1 = '<abd' }
  & $mixed 'mixed-values-5' @{ field = 1; values = @('5') }
  & $mixed 'mixed-values-err' @{ field = 1; values = @('#DIV/0!') }
  & $mixed 'mixed-top3' @{ field = 1; op = 'top10items'; c1 = 3 }
  & $mixed 'mixed-above-avg' @{ field = 1; op = 'dynamic'; c1 = 'aboveAverage' }
  return , $script:fs}

function Apply-Filter($app, $ws, $sc) {
  $rng = $ws.Range($sc.range)
  if ($ws.AutoFilterMode) { $ws.AutoFilterMode = $false }
  foreach ($f in $sc.fields) {
    $field = [int]$f.field
    switch ($f.op) {
      'custom' {
        if ($f.c2) { $op = if ($f.logic -eq 'or') { 2 } else { 1 }; [void]$rng.AutoFilter($field, $f.c1, $op, $f.c2) }
        else { [void]$rng.AutoFilter($field, $f.c1) }
      }
      'top10items' { [void]$rng.AutoFilter($field, [string]$f.c1, 3) }
      'bottom10items' { [void]$rng.AutoFilter($field, [string]$f.c1, 4) }
      'top10percent' { [void]$rng.AutoFilter($field, [string]$f.c1, 5) }
      'bottom10percent' { [void]$rng.AutoFilter($field, [string]$f.c1, 6) }
      'dynamic' {
        $codes = @{ today = 1; yesterday = 2; tomorrow = 3; thisWeek = 4; lastWeek = 5; nextWeek = 6; thisMonth = 7; lastMonth = 8; nextMonth = 9; thisQuarter = 10; lastQuarter = 11; nextQuarter = 12; thisYear = 13; lastYear = 14; nextYear = 15; yearToDate = 16; Q1 = 17; Q2 = 18; Q3 = 19; Q4 = 20; M1 = 21; M2 = 22; M3 = 23; M4 = 24; M5 = 25; M6 = 26; M7 = 27; M8 = 28; M9 = 29; M10 = 30; M11 = 31; M12 = 32; aboveAverage = 33; belowAverage = 34 }
        [void]$rng.AutoFilter($field, $codes[$f.c1], 11)
      }
      'cellcolor' {
        if ($f.c1 -eq 'none') { [void]$rng.AutoFilter($field, [Type]::Missing, 12) } else { [void]$rng.AutoFilter($field, (Bgr $f.c1), 8) }
      }
      'fontcolor' {
        if ($f.c1 -eq 'none') { [void]$rng.AutoFilter($field, [Type]::Missing, 13) } else { [void]$rng.AutoFilter($field, (Bgr $f.c1), 9) }
      }
      'dategroups' { [void]$rng.AutoFilter($field, [Type]::Missing, 7, [object[]]$f.groups) }
      default { [void]$rng.AutoFilter($field, [object[]]$f.values, 7) }
    }
  }
}

function Run-Filter($app) {
  $today = [double]$app.Evaluate('TODAY()')
  $scenarios = Filter-Scenarios $today
  $wb = New-Fixture $app $scenarios 'sortfilter-filter.xlsx'
  $res = @()
  for ($i = 0; $i -lt $scenarios.Count; $i++) {
    $sc = $scenarios[$i]; $ws = $wb.Worksheets.Item($i + 1)
    try { Apply-Filter $app $ws $sc } catch { Write-Warning "filter $($sc.id): $($_.Exception.Message)"; $res += , $null; continue }
    $hidden = @()
    $rr = $ws.Range($sc.range); $r0 = $rr.Row; $r1 = $rr.Row + $rr.Rows.Count - 1; for ($r = $r0 + 1; $r -le $r1; $r++) { if ($ws.Rows.Item($r).Hidden) { $hidden += $r } }
    $res += , $hidden
    Write-Host "filter $($sc.id) hidden=$($hidden -join ',')"
  }
  $copy = Join-Path $tmp 'filter-copy.xlsx'
  $wb.SaveCopyAs($copy)
  $parts = Sheet-Parts $copy
  $dxfs = Extract-Element (Read-ZipText $copy 'xl/styles.xml') 'dxfs'
  $out = @()
  for ($i = 0; $i -lt $scenarios.Count; $i++) {
    $sc = $scenarios[$i]
    if ($null -eq $res[$i]) { continue }
    $xml = Read-ZipText $copy $parts[$sc.id]
    $entry = [ordered]@{ id = $sc.id; range = $sc.range; today = $today; fields = $sc.fields; hidden_rows = [object[]]$res[$i]; auto_filter = (Extract-Element $xml 'autoFilter') }
    $out += , $entry
  }
  $wb.Saved = $true; $wb.Close($false)
  Scrub-Core (Join-Path $fixtureDir 'sortfilter-filter.xlsx')
  Write-Json (Join-Path $refRoot 'sortfilter-filter/scenarios.json') ([ordered]@{ intl = (Intl-Info $app); today = $today; dxfs = $dxfs; scenarios = $out })
}

# =========================================================================================================
# VALIDATION
# =========================================================================================================

function Validation-Scenarios {
  $n = { param($v) @{ t = 'n'; v = $v } }
  $t = { param($v) @{ t = 's'; v = $v } }
  $b = { param($v) @{ t = 'b'; v = $v } }
  $z = @{ t = 'z' }
  $common = @(& $n 0), (& $n 1), (& $n 5), (& $n 10), (& $n 11), (& $n 5.5), (& $n -3), (& $n 10.0000000001), (& $n 4.9999999999999), (& $t 'abc'), (& $t '5'), (& $b $true), $z
  $s = @()
  $ops = @(
    @('between', 1, '=5', '=10'), @('notBetween', 2, '=5', '=10'), @('equal', 3, '=5', $null), @('notEqual', 4, '=5', $null),
    @('greaterThan', 5, '=5', $null), @('lessThan', 6, '=5', $null), @('greaterThanOrEqual', 7, '=5', $null), @('lessThanOrEqual', 8, '=5', $null))
  foreach ($o in $ops) {
    $s += @{ id = "whole-$($o[0])"; cell = 'A1'; range = 'A1'; vtype = 1; op = $o[1]; f1 = $o[2]; f2 = $o[3]; cells = @(); candidates = $common }
    $s += @{ id = "decimal-$($o[0])"; cell = 'A1'; range = 'A1'; vtype = 2; op = $o[1]; f1 = $o[2]; f2 = $o[3]; cells = @(); candidates = $common }
  }
  $s += @{ id = 'whole-noblank'; cell = 'A1'; range = 'A1'; vtype = 1; op = 1; f1 = '=1'; f2 = '=10'; ignore_blank = $false; cells = @(); candidates = @($z, (& $n 5), (& $n 20)) }
  $s += @{ id = 'whole-blank-ok'; cell = 'A1'; range = 'A1'; vtype = 1; op = 1; f1 = '=1'; f2 = '=10'; ignore_blank = $true; cells = @(); candidates = @($z, (& $n 5), (& $n 20)) }
  $s += @{ id = 'whole-cellref-bounds'; cell = 'A1'; range = 'A1'; vtype = 1; op = 1; f1 = '=$C$1'; f2 = '=$C$2'; cells = @(@($null, $null, 2), @($null, $null, 4)); candidates = @((& $n 1), (& $n 2), (& $n 3), (& $n 4), (& $n 5)) }
  $s += @{ id = 'whole-formula-bound'; cell = 'A1'; range = 'A1'; vtype = 1; op = 5; f1 = '=SUM($C$1:$C$2)'; f2 = $null; cells = @(@($null, $null, 2), @($null, $null, 4)); candidates = @((& $n 5), (& $n 6), (& $n 7)) }
  $dayN = 45000
  $s += @{ id = 'date-between'; cell = 'A1'; range = 'A1'; vtype = 4; op = 1; f1 = "=$dayN"; f2 = "=$($dayN + 30)"; cells = @(); candidates = @((& $n ($dayN - 1)), (& $n $dayN), (& $n ($dayN + 15)), (& $n ($dayN + 15.5)), (& $n ($dayN + 30)), (& $n ($dayN + 31)), (& $t 'x'), $z) }
  $s += @{ id = 'date-greater'; cell = 'A1'; range = 'A1'; vtype = 4; op = 5; f1 = "=$dayN"; f2 = $null; cells = @(); candidates = @((& $n $dayN), (& $n ($dayN + 0.5)), (& $n ($dayN + 1)), (& $n -1)) }
  $s += @{ id = 'date-date-fn'; cell = 'A1'; range = 'A1'; vtype = 4; op = 7; f1 = '=DATE(2023,3,15)'; f2 = $null; cells = @(); candidates = @((& $n 45000), (& $n 45001), (& $n 44999), (& $n 45000.9)) }
  $s += @{ id = 'time-between'; cell = 'A1'; range = 'A1'; vtype = 5; op = 1; f1 = '=0.25'; f2 = '=0.75'; cells = @(); candidates = @((& $n 0.2), (& $n 0.25), (& $n 0.5), (& $n 0.75), (& $n 0.8), (& $n 1.5), (& $n 3.5), (& $t 'x'), $z) }
  $s += @{ id = 'time-less'; cell = 'A1'; range = 'A1'; vtype = 5; op = 6; f1 = '=0.5'; f2 = $null; cells = @(); candidates = @((& $n 0.25), (& $n 0.5), (& $n 0.75), (& $n 1.25), (& $n 1.75)) }
  $tl = @((& $t ''), (& $t 'a'), (& $t 'abc'), (& $t 'abcde'), (& $t 'abcdef'), (& $n 12345), (& $n 123456), (& $n 1.5), (& $n 0), (& $b $true), (& $t 'é€'), (& $t "line1`nline2"), $z)
  foreach ($o in $ops) { $s += @{ id = "textlength-$($o[0])"; cell = 'A1'; range = 'A1'; vtype = 6; op = $o[1]; f1 = '=3'; f2 = if ($o[0] -like '*etween') { '=5' } else { $null }; cells = @(); candidates = $tl } }
  $s += @{ id = 'list-literal'; cell = 'A1'; range = 'A1'; vtype = 3; op = 1; f1 = 'Low,Medium,High'; f2 = $null; cells = @(); candidates = @((& $t 'Low'), (& $t 'low'), (& $t 'MEDIUM'), (& $t 'High'), (& $t 'Highest'), (& $t ' High'), (& $t ''), (& $n 5), $z) }
  $s += @{ id = 'list-literal-numbers'; cell = 'A1'; range = 'A1'; vtype = 3; op = 1; f1 = '1,2,3.5,-4'; f2 = $null; cells = @(); candidates = @((& $n 1), (& $n 3.5), (& $n -4), (& $n 4), (& $t '1'), (& $t '3.5'), (& $n 2), $z) }
  $s += @{ id = 'list-range'; cell = 'A1'; range = 'A1'; vtype = 3; op = 1; f1 = '=$C$1:$C$4'; f2 = $null; cells = @(@($null, $null, 'Red'), @($null, $null, 'green'), @($null, $null, 7), @($null, $null, $null)); candidates = @((& $t 'Red'), (& $t 'red'), (& $t 'GREEN'), (& $n 7), (& $t '7'), (& $t 'blue'), $z) }
  $s += @{ id = 'list-range-noblank'; cell = 'A1'; range = 'A1'; vtype = 3; op = 1; f1 = '=$C$1:$C$3'; f2 = $null; ignore_blank = $false; cells = @(@($null, $null, 'Red'), @($null, $null, 'green'), @($null, $null, 7)); candidates = @($z, (& $t 'Red')) }
  $s += @{ id = 'list-range-with-blank-cell'; cell = 'A1'; range = 'A1'; vtype = 3; op = 1; f1 = '=$C$1:$C$3'; f2 = $null; ignore_blank = $true; cells = @(@($null, $null, 'Red'), @($null, $null, $null), @($null, $null, 'x')); candidates = @($z, (& $t 'Red'), (& $t 'x'), (& $t 'y')) }
  $s += @{ id = 'list-row-range'; cell = 'A1'; range = 'A1'; vtype = 3; op = 1; f1 = '=$C$1:$E$1'; f2 = $null; cells = @(, @($null, $null, 'a', 'b', 'c')); candidates = @((& $t 'a'), (& $t 'c'), (& $t 'd')) }
  $s += @{ id = 'list-formula-offset'; cell = 'A1'; range = 'A1'; vtype = 3; op = 1; f1 = '=OFFSET($C$1,0,0,2,1)'; f2 = $null; cells = @(@($null, $null, 'p'), @($null, $null, 'q'), @($null, $null, 'r')); candidates = @((& $t 'p'), (& $t 'q'), (& $t 'r')) }
  $s += @{ id = 'list-dates'; cell = 'A1'; range = 'A1'; vtype = 3; op = 1; f1 = '=$C$1:$C$2'; f2 = $null; cells = @(@($null, $null, @{ v = 45000; nf = 'yyyy-mm-dd' }), @($null, $null, @{ v = 45001; nf = 'yyyy-mm-dd' })); candidates = @((& $n 45000), (& $n 45001), (& $n 45002), (& $t '45000')) }
  $s += @{ id = 'custom-gt-other'; cell = 'B2'; range = 'B2:B5'; vtype = 7; op = 1; f1 = '=B2>A2'; f2 = $null; cells = @(@(1, $null), @(5, $null), @(3, $null), @(9, $null), @(2, $null)); candidates = @(@{ c = 'B2'; t = 'n'; v = 2 }, @{ c = 'B2'; t = 'n'; v = 1 }, @{ c = 'B3'; t = 'n'; v = 6 }, @{ c = 'B3'; t = 'n'; v = 3 }, @{ c = 'B4'; t = 'n'; v = 4 }, @{ c = 'B4'; t = 'n'; v = 2 }, @{ c = 'B5'; t = 'n'; v = 10 }, @{ c = 'B5'; t = 'n'; v = 9 }, @{ c = 'B5'; t = 's'; v = 'zz' }) }
  $s += @{ id = 'custom-isnumber'; cell = 'A1'; range = 'A1'; vtype = 7; op = 1; f1 = '=ISNUMBER(A1)'; f2 = $null; cells = @(); candidates = @((& $n 5), (& $t 'x'), (& $b $true), $z, (& $n 0)) }
  $s += @{ id = 'custom-and-or'; cell = 'A1'; range = 'A1'; vtype = 7; op = 1; f1 = '=AND(A1>=1,A1<=10,MOD(A1,2)=0)'; f2 = $null; cells = @(); candidates = @((& $n 2), (& $n 3), (& $n 10), (& $n 12), (& $n 0), (& $t 'x')) }
  $s += @{ id = 'custom-number-result'; cell = 'A1'; range = 'A1'; vtype = 7; op = 1; f1 = '=A1-5'; f2 = $null; cells = @(); candidates = @((& $n 5), (& $n 6), (& $n 4), (& $t 'x')) }
  $s += @{ id = 'custom-text-result'; cell = 'A1'; range = 'A1'; vtype = 7; op = 1; f1 = '=LEFT(A1,1)'; f2 = $null; cells = @(); candidates = @((& $t 'abc'), (& $t '1x'), (& $n 12)) }
  $s += @{ id = 'custom-error-result'; cell = 'A1'; range = 'A1'; vtype = 7; op = 1; f1 = '=1/(A1-3)'; f2 = $null; cells = @(); candidates = @((& $n 3), (& $n 4), (& $n 0)) }
  $s += @{ id = 'custom-countif-unique'; cell = 'B1'; range = 'B1:B5'; vtype = 7; op = 1; f1 = '=COUNTIF($B$1:$B$5,B1)=1'; f2 = $null; cells = @(@($null, 'a'), @($null, 'b'), @($null, 'a'), @($null, 'c'), @($null, 'd')); candidates = @(@{ c = 'B1'; t = 's'; v = 'a' }, @{ c = 'B2'; t = 's'; v = 'b' }, @{ c = 'B4'; t = 's'; v = 'c' }, @{ c = 'B5'; t = 's'; v = 'a' }) }
  $s += @{ id = 'input-only'; cell = 'A1'; range = 'A1'; vtype = 0; op = $null; f1 = $null; f2 = $null; input_title = 'Enter'; input_msg = 'Type a number'; cells = @(); candidates = @((& $n 5), (& $t 'x'), $z) }
  $s += @{ id = 'messages-stop'; cell = 'A1'; range = 'A1'; vtype = 1; op = 1; f1 = '=1'; f2 = '=10'; alert = 1; input_title = 'Title in'; input_msg = 'Msg in'; error_title = 'Title err'; error_msg = 'Msg err & <more>'; cells = @(); candidates = @((& $n 5), (& $n 50)) }
  $s += @{ id = 'messages-warning'; cell = 'A1'; range = 'A1'; vtype = 2; op = 5; f1 = '=0'; f2 = $null; alert = 2; error_title = 'Warn'; error_msg = 'Careful'; show_input = $false; cells = @(); candidates = @((& $n 5), (& $n -5)) }
  $s += @{ id = 'messages-info'; cell = 'A1'; range = 'A1'; vtype = 6; op = 8; f1 = '=3'; f2 = $null; alert = 3; show_error = $false; in_cell_dropdown = $false; cells = @(); candidates = @((& $t 'ab'), (& $t 'abcd')) }
  $s += @{ id = 'list-no-dropdown'; cell = 'A1'; range = 'A1'; vtype = 3; op = 1; f1 = 'x,y'; f2 = $null; in_cell_dropdown = $false; cells = @(); candidates = @((& $t 'x'), (& $t 'z')) }
  $s += @{ id = 'multi-area'; cell = 'A1'; range = 'A1:A3,C1:C3'; vtype = 1; op = 1; f1 = '=1'; f2 = '=3'; cells = @(); candidates = @(@{ c = 'A1'; t = 'n'; v = 2 }, @{ c = 'A3'; t = 'n'; v = 4 }, @{ c = 'C2'; t = 'n'; v = 2 }, @{ c = 'C3'; t = 'n'; v = 9 }) }
  return , $s
}

function Run-Validation($app) {
  $scenarios = Validation-Scenarios
  $wb = New-Fixture $app $scenarios 'sortfilter-validation.xlsx'
  $res = @()
  for ($i = 0; $i -lt $scenarios.Count; $i++) {
    $sc = $scenarios[$i]; $ws = $wb.Worksheets.Item($i + 1)
    $rng = $ws.Range($sc.range)
    $v = $rng.Validation
    $v.Delete()
    $alert = if ($sc.alert) { [int]$sc.alert } else { 1 }
    $f1 = if ($sc.f1) { $sc.f1 } else { [Type]::Missing }
    $f2 = if ($sc.f2) { $sc.f2 } else { [Type]::Missing }
    if ($sc.vtype -eq 0) { [void]$v.Add(0, $alert) }
    elseif ($sc.vtype -eq 3 -or $sc.vtype -eq 7) { [void]$v.Add($sc.vtype, $alert, 1, $f1) }
    else { [void]$v.Add($sc.vtype, $alert, $sc.op, $f1, $f2) }
    $v = $rng.Cells.Item(1, 1).Validation
    if ($sc.ContainsKey('ignore_blank')) { $v.IgnoreBlank = [bool]$sc.ignore_blank }
    if ($sc.ContainsKey('in_cell_dropdown')) { $v.InCellDropdown = [bool]$sc.in_cell_dropdown }
    if ($sc.input_title) { $v.InputTitle = $sc.input_title }
    if ($sc.input_msg) { $v.InputMessage = $sc.input_msg }
    if ($sc.error_title) { $v.ErrorTitle = $sc.error_title }
    if ($sc.error_msg) { $v.ErrorMessage = $sc.error_msg }
    if ($sc.ContainsKey('show_input')) { $v.ShowInput = [bool]$sc.show_input }
    if ($sc.ContainsKey('show_error')) { $v.ShowError = [bool]$sc.show_error }
    $results = @()
    foreach ($c in $sc.candidates) {
      $addr = if ($c.c) { $c.c } else { $sc.cell }
      $cell = $ws.Range($addr)
      $old = $cell.Formula
      switch ($c.t) {
        'n' { SetV $cell ([double]$c.v) }
        's' { if ($c.v -eq '') { SetP $cell 'Formula' '=""' } else { SetV $cell ("'" + $c.v) } }
        'b' { SetV $cell ([bool]$c.v) }
        default { [void]$cell.ClearContents() }
      }
      $ok = [bool]$cell.Validation.Value
      $results += , $ok
      if ($old) { SetP $cell 'Formula' $old } else { [void]$cell.ClearContents() }
    }
    $res += , $results
    Write-Host "validation $($sc.id) -> $($results -join ',')"
  }
  $copy = Join-Path $tmp 'validation-copy.xlsx'
  $wb.SaveCopyAs($copy)
  $parts = Sheet-Parts $copy
  $out = @()
  for ($i = 0; $i -lt $scenarios.Count; $i++) {
    $sc = $scenarios[$i]
    $xml = Read-ZipText $copy $parts[$sc.id]
    $cands = @()
    for ($k = 0; $k -lt $sc.candidates.Count; $k++) {
      $c = $sc.candidates[$k]
      $addr = if ($c.c) { $c.c } else { $sc.cell }
      $cands += , ([ordered]@{ cell = $addr; type = $c.t; value = $c.v; valid = $res[$i][$k] })
    }
    $out += , ([ordered]@{ id = $sc.id; range = $sc.range; data_validations = (Extract-Element $xml 'dataValidations'); candidates = $cands })
  }
  $wb.Saved = $true; $wb.Close($false)
  Scrub-Core (Join-Path $fixtureDir 'sortfilter-validation.xlsx')
  Write-Json (Join-Path $refRoot 'sortfilter-validation/scenarios.json') ([ordered]@{ intl = (Intl-Info $app); scenarios = $out })
}

try {
  Start-Excel
  if ($Group -eq 'sort' -or $Group -eq 'all') { Run-Sort $script:xl }
  if ($Group -eq 'filter' -or $Group -eq 'all') { Run-Filter $script:xl }
  if ($Group -eq 'validation' -or $Group -eq 'all') { Run-Validation $script:xl }
}
finally {
  Stop-Excel
  foreach ($f in Get-ChildItem -LiteralPath $tmp -File -ErrorAction SilentlyContinue) { Remove-Item -LiteralPath $f.FullName -Force }
  Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
}
