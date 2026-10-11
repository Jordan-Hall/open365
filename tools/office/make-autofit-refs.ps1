<#
Measures, through Open365's OWN hidden Excel instance (COM), what Excel does when a row or column is auto-fitted
implicitly (wrap text, font change, column width change) or explicitly (Rows.AutoFit / Columns.AutoFit), and what a
merge does to the styles of the covered cells. Writes:

  fixtures/reference/xlsx/autofit-implicit/autofit.json   heights/widths in points plus the saved <row>/<col> attributes
  fixtures/reference/xlsx/autofit-merge-styles/merge.json the resolved style of every cell before and after a merge

LOCAL DEVELOPER TOOL ONLY (Office automation is unsupported on servers; never run in CI). CI consumes the committed
JSON through crates/ui-egui/tests/grid_autofit_implicit.rs and crates/xlsx/tests/merge_styles.rs.

Safety: generated workbooks only; aborts if COM hands back an Excel process that was already running;
AutomationSecurity = 3 (force disable macros); Quit in finally and only this script's own pid is ever stopped.
Output: LF, ASCII, no paths, no user data.

  pwsh -NoProfile -File tools/office/make-autofit-refs.ps1 [-Phase All|Autofit|Merge]
#>
param([ValidateSet('All', 'Autofit', 'Merge')][string]$Phase = 'All')
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$inv = [Globalization.CultureInfo]::InvariantCulture
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@
$script:xl = $null; $script:xlPid = 0
$scratch = Join-Path ([IO.Path]::GetTempPath()) ('o365-autofit-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $scratch | Out-Null

function Start-Excel {
  $pre = @(Get-Process -Name EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $app = New-Object -ComObject Excel.Application
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  $script:xl = $app; $script:xlPid = [int]$procId
  if ($pre -contains [int]$procId) { $script:xlPid = 0; throw "COM attached to an Excel process that was already running (pid $procId); aborting without touching it" }
  $app.AutomationSecurity = 3
  $app.Visible = $false; $app.DisplayAlerts = $false; $app.ScreenUpdating = $false; $app.EnableEvents = $false
}
function Stop-Excel {
  $app = $script:xl; $script:xl = $null
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
function Num($v) { return ([double]$v).ToString('R', $inv) }
function New-Book($app) {
  $wb = $app.Workbooks.Add()
  $wb.Styles.Item('Normal').Font.Name = 'Calibri'
  $wb.Styles.Item('Normal').Font.Size = 11
  return $wb
}
function Save-Copy($wb, $name) {
  $p = Join-Path $scratch "$name.xlsx"
  $wb.SaveCopyAs($p)
  $dir = Join-Path $scratch $name
  [IO.Compression.ZipFile]::ExtractToDirectory($p, $dir)
  return $dir
}
function Read-Xml($dir, $rel) { $x = New-Object Xml.XmlDocument; $x.Load((Join-Path $dir $rel)); return $x }
function Write-Json($rel, $obj) {
  $out = Join-Path $root $rel
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $out) | Out-Null
  $json = ($obj | ConvertTo-Json -Depth 12) -replace "`r`n", "`n"
  [IO.File]::WriteAllText($out, $json + "`n", (New-Object Text.UTF8Encoding $false))
  Write-Host "wrote $rel"
}

# ---------------------------------------------------------------- implicit and explicit auto-fit
function Measure-Autofit($app) {
  $wb = New-Book $app
  $ws = $wb.Worksheets.Item(1)
  $long = 'The quick brown fox jumps over the lazy dog again and again'
  $ws.Columns.Item(2).ColumnWidth = 12
  $cases = New-Object System.Collections.ArrayList
  function Rec($label, $row, $step, $h) { [void]$cases.Add([ordered]@{ label = $label; row = $row; step = $step; height_pt = [double](Num $h) }) }
  $ht = { param($r) $ws.Rows.Item($r).Height }

  # 1: wrap on, wrap off
  $ws.Cells.Item(1, 2).Value2 = $long
  Rec 'wrap toggle' 1 'start' (& $ht 1)
  $ws.Cells.Item(1, 2).WrapText = $true; Rec 'wrap toggle' 1 'wrap on' (& $ht 1)
  $ws.Cells.Item(1, 2).WrapText = $false; Rec 'wrap toggle' 1 'wrap off' (& $ht 1)
  # 2: font size and name changes on a wrapped cell
  $ws.Cells.Item(2, 2).Value2 = $long; $ws.Cells.Item(2, 2).WrapText = $true
  Rec 'font change' 2 'wrapped' (& $ht 2)
  $ws.Cells.Item(2, 2).Font.Size = 20; Rec 'font change' 2 'size 20' (& $ht 2)
  $ws.Cells.Item(2, 2).Font.Size = 8; Rec 'font change' 2 'size 8' (& $ht 2)
  $ws.Cells.Item(2, 2).Font.Name = 'Cambria'; $ws.Cells.Item(2, 2).Font.Size = 14; Rec 'font change' 2 'cambria 14' (& $ht 2)
  # 3: size change on an unwrapped cell, then ClearFormats
  $ws.Cells.Item(3, 2).Value2 = 'x'; $ws.Cells.Item(3, 2).Font.Size = 24; Rec 'size only' 3 'size 24' (& $ht 3)
  $ws.Cells.Item(3, 2).ClearFormats() | Out-Null; Rec 'size only' 3 'clear formats' (& $ht 3)
  # 4: a custom height row stays put, explicit AutoFit releases it
  $ws.Cells.Item(4, 2).Value2 = $long
  $ws.Rows.Item(4).RowHeight = 40; Rec 'custom height' 4 'row height 40' (& $ht 4)
  $ws.Cells.Item(4, 2).WrapText = $true; Rec 'custom height' 4 'wrap on' (& $ht 4)
  $ws.Rows.Item(4).AutoFit() | Out-Null; Rec 'custom height' 4 'rows autofit' (& $ht 4)
  $ws.Cells.Item(4, 2).Font.Size = 20; Rec 'custom height' 4 'size 20 after autofit' (& $ht 4)
  # 5: a wrapped row follows the column width. Heights read straight after a width change are stale, so each
  # width gets its own row/column and is read back from the saved XML (the 'saved_rows' list below).
  $widthCases = @(@(13, 12, 12), @(14, 14, 24), @(15, 16, 6), @(16, 18, 40))
  foreach ($wc in $widthCases) {
    $ws.Cells.Item($wc[0], $wc[1]).Value2 = $long; $ws.Cells.Item($wc[0], $wc[1]).WrapText = $true
    $ws.Columns.Item($wc[1]).ColumnWidth = $wc[2]
  }
  # 6: wrapped merged cell does not drive the height
  $ws.Cells.Item(6, 6).Value2 = $long; $ws.Range("F6:G6").Merge() | Out-Null; $ws.Cells.Item(6, 6).WrapText = $true; Rec 'merged wrap' 6 'merge then wrap' (& $ht 6)
  # 7: tallest font among cells wins
  $ws.Cells.Item(7, 2).Value2 = 'a'; $ws.Cells.Item(7, 3).Value2 = 'b'; $ws.Cells.Item(7, 3).Font.Size = 28; Rec 'tallest cell' 7 'size 28 beside 11' (& $ht 7)
  # 8: newline in a value
  $ws.Cells.Item(8, 2).Value2 = "one`ntwo`nthree"; Rec 'newline' 8 'value with newlines, wrap off' (& $ht 8)
  $ws.Cells.Item(8, 2).WrapText = $true; Rec 'newline' 8 'wrap on' (& $ht 8)
  # 9: indent and rotation do not matter; bold wraps differently
  $ws.Cells.Item(9, 2).Value2 = $long; $ws.Cells.Item(9, 2).WrapText = $true; $ws.Cells.Item(9, 2).Font.Bold = $true; Rec 'bold wrap' 9 'bold' (& $ht 9)
  # 10: height set by hand equal to default stays custom
  $ws.Rows.Item(10).RowHeight = 15; $ws.Cells.Item(10, 2).Value2 = 'x'; $ws.Cells.Item(10, 2).Font.Size = 24; Rec 'custom equal default' 10 'size 24' (& $ht 10)

  # columns: explicit autofit and manual width
  $ws.Cells.Item(12, 8).Value2 = 'A reasonably long piece of text'; $ws.Columns.Item(8).AutoFit() | Out-Null
  $ws.Columns.Item(9).ColumnWidth = 20
  $ws.Cells.Item(12, 10).Value2 = 12345; $ws.Columns.Item(10).AutoFit() | Out-Null
  $colw = [ordered]@{ autofit_text = $ws.Columns.Item(8).ColumnWidth; manual = $ws.Columns.Item(9).ColumnWidth; autofit_number = $ws.Columns.Item(10).ColumnWidth }
  $dir = Save-Copy $wb 'autofit'
  $x = Read-Xml $dir 'xl/worksheets/sheet1.xml'
  $ns = New-Object Xml.XmlNamespaceManager($x.NameTable); $ns.AddNamespace('m', 'http://schemas.openxmlformats.org/spreadsheetml/2006/main')
  $rows = @()
  foreach ($r in $x.SelectNodes('//m:sheetData/m:row', $ns)) {
    $rows += [ordered]@{ r = [int]$r.r; ht = $r.GetAttribute('ht'); customHeight = $r.GetAttribute('customHeight') }
  }
  $cols = @()
  foreach ($c in $x.SelectNodes('//m:cols/m:col', $ns)) {
    $cols += [ordered]@{ min = [int]$c.min; max = [int]$c.max; width = $c.GetAttribute('width'); bestFit = $c.GetAttribute('bestFit'); customWidth = $c.GetAttribute('customWidth') }
  }
  $fmt = $x.SelectSingleNode('//m:sheetFormatPr', $ns)
  $wb.Saved = $true; $wb.Close($false)
  return [ordered]@{
    oracle = 'Microsoft Excel via COM (own hidden instance): Rows(r).Height after each step; the saved sheet XML rows/cols'
    excel_build = [string]$app.Build
    default_font = 'Calibri 11'
    column_b_width_chars = 12
    text = $long
    cases = $cases.ToArray()
    column_widths_chars = $colw
    width_cases = @($widthCases | ForEach-Object { [ordered]@{ row = $_[0]; col = $_[1]; width_chars = $_[2] } })
    saved_rows = $rows
    saved_cols = $cols
    sheet_format = [ordered]@{ defaultRowHeight = $fmt.GetAttribute('defaultRowHeight'); customHeight = $fmt.GetAttribute('customHeight') }
  }
}

# ---------------------------------------------------------------- merge: covered-cell styles
function Resolve-Styles($dir) {
  $st = Read-Xml $dir 'xl/styles.xml'
  $ns = New-Object Xml.XmlNamespaceManager($st.NameTable); $ns.AddNamespace('m', 'http://schemas.openxmlformats.org/spreadsheetml/2006/main')
  $xfs = @($st.SelectNodes('//m:cellXfs/m:xf', $ns))
  $fills = @($st.SelectNodes('//m:fills/m:fill', $ns))
  $borders = @($st.SelectNodes('//m:borders/m:border', $ns))
  $fonts = @($st.SelectNodes('//m:fonts/m:font', $ns))
  $nf = @{}
  foreach ($n in $st.SelectNodes('//m:numFmts/m:numFmt', $ns)) { $nf[[int]$n.numFmtId] = $n.formatCode }
  $color = { param($e) if (-not $e) { return $null }; if ($e.rgb) { return "rgb:$($e.rgb)" }; if ($e.theme) { return "theme:$($e.theme)" }; if ($e.indexed) { return "indexed:$($e.indexed)" }; return 'auto' }
  $sh = Read-Xml $dir 'xl/worksheets/sheet1.xml'
  $ns2 = New-Object Xml.XmlNamespaceManager($sh.NameTable); $ns2.AddNamespace('m', 'http://schemas.openxmlformats.org/spreadsheetml/2006/main')
  $cells = [ordered]@{}
  foreach ($c in $sh.SelectNodes('//m:sheetData/m:row/m:c', $ns2)) {
    $s = if ($c.s) { [int]$c.s } else { 0 }
    $xf = $xfs[$s]
    $fill = $fills[[int]$xf.fillId].patternFill
    $b = $borders[[int]$xf.borderId]
    $f = $fonts[[int]$xf.fontId]
    $side = { param($n) if ($n -and $n.style) { return "$($n.style) $(& $color $n.color)" } else { return $null } }
    $al = $xf.alignment
    $cells[$c.r] = [ordered]@{
      xf = $s
      numFmt = if ($nf.ContainsKey([int]$xf.numFmtId)) { $nf[[int]$xf.numFmtId] } else { "builtin:$($xf.numFmtId)" }
      fill = if ($fill.patternType -eq 'solid') { "solid $(& $color $fill.fgColor)" } else { [string]$fill.patternType }
      left = & $side $b.left; right = & $side $b.right; top = & $side $b.top; bottom = & $side $b.bottom
      diagonal = & $side $b.diagonal
      font = "$($f.name.val) $($f.sz.val)$(if ($f.b) {' bold'})$(if ($f.i) {' italic'})$(if ($f.u) {' underline'}) $(& $color $f.color)"
      halign = if ($al) { [string]$al.horizontal } else { '' }
      valign = if ($al) { [string]$al.vertical } else { '' }
      wrap = [bool]($al -and $al.wrapText)
      indent = if ($al) { [string]$al.indent } else { '' }
    }
  }
  $merges = @($sh.SelectNodes('//m:mergeCells/m:mergeCell', $ns2) | ForEach-Object { $_.ref })
  return [ordered]@{ cells = $cells; merges = $merges }
}

function Style-Block($ws, $addr, [int]$seed) {
  # Give every cell of $addr its own look so that what survives a merge can be told apart.
  $fills = @(0xFFC0C0, 0xC0FFC0, 0xC0C0FF, 0xFFFFC0, 0xC0FFFF, 0xFFC0FF, 0xE0E0E0, 0xA0D0F0, 0xF0D0A0)
  $i = 0
  foreach ($c in $ws.Range($addr).Cells) {
    $c.Interior.Color = $fills[($i + $seed) % $fills.Count]
    $c.Font.Name = @('Calibri', 'Cambria', 'Arial')[$i % 3]
    $c.Font.Size = 9 + $i
    $c.Font.Bold = ($i % 2 -eq 0)
    $c.Font.Color = 0x0000FF + ($i * 0x001000)
    $c.NumberFormat = @('0.00', '#,##0', 'yyyy-mm-dd', '@', '0%')[$i % 5]
    $c.HorizontalAlignment = @(-4131, -4152, -4108)[$i % 3]
    $c.VerticalAlignment = @(-4160, -4108, -4107)[$i % 3]
    $c.WrapText = ($i % 2 -eq 1)
    foreach ($e in 7, 8, 9, 10) { $c.Borders.Item($e).LineStyle = 1; $c.Borders.Item($e).Weight = 2; $c.Borders.Item($e).Color = 0x3F3F3F + $i * 0x101010 }
    $i++
  }
}

function Measure-Merge($app) {
  $out = New-Object System.Collections.ArrayList
  $scen = @(
    @{ name = 'merge cells 2x3, every cell styled'; range = 'B2:D3'; how = 'merge'; style = 'B2:D3' },
    @{ name = 'merge cells 2x3, only top-left styled'; range = 'B2:D3'; how = 'merge'; style = 'B2' },
    @{ name = 'merge and centre 1x3'; range = 'B2:D2'; how = 'center'; style = 'B2:D2' },
    @{ name = 'merge across 2x3'; range = 'B2:D3'; how = 'across'; style = 'B2:D3' },
    @{ name = 'outline border only then merge'; range = 'B2:D3'; how = 'merge'; style = 'outline' },
    @{ name = 'bottom-right cell styled only'; range = 'B2:C3'; how = 'merge'; style = 'C3' },
    @{ name = 'unmerge after merge'; range = 'B2:C3'; how = 'unmerge'; style = 'B2:C3' }
  )
  foreach ($s in $scen) {
    $wb = New-Book $app
    $ws = $wb.Worksheets.Item(1)
    $r = $ws.Range($s.range)
    # neighbours with their own border so edge sharing shows
    $ws.Range('A1:F5').Value2 = 7
    if ($s.style -eq 'outline') {
      foreach ($e in 7, 8, 9, 10) { $r.Borders.Item($e).LineStyle = 1; $r.Borders.Item($e).Weight = -4138 }
      $r.Interior.Color = 0xFFC0C0
    } else {
      $st = $s.style
      Style-Block $ws $st 0
    }
    $before = Resolve-Styles (Save-Copy $wb ('b' + $out.Count))
    if ($s.how -eq 'merge') { $r.Merge() | Out-Null }
    elseif ($s.how -eq 'center') { $r.Merge() | Out-Null; $r.HorizontalAlignment = -4108 }
    elseif ($s.how -eq 'across') { $r.Merge($true) | Out-Null }
    elseif ($s.how -eq 'unmerge') { $r.Merge() | Out-Null; $r.UnMerge() | Out-Null }
    $after = Resolve-Styles (Save-Copy $wb ('a' + $out.Count))
    [void]$out.Add([ordered]@{ name = $s.name; range = $s.range; how = $s.how; before = $before; after = $after })
    $wb.Saved = $true; $wb.Close($false)
  }
  return [ordered]@{
    oracle = 'Microsoft Excel via COM (own hidden instance): sheet1.xml cell s indexes resolved through styles.xml, saved with SaveCopyAs before and after the merge'
    excel_build = [string]$app.Build
    note = 'cells B2.. carry per-cell fill, font, number format, alignment and thin borders; A1:F5 hold the number 7 (default style) so neighbours exist'
    scenarios = $out.ToArray()
  }
}

try {
  Start-Excel
  if ($Phase -in 'All', 'Autofit') { Write-Json 'fixtures/reference/xlsx/autofit-implicit/autofit.json' (Measure-Autofit $script:xl) }
  if ($Phase -in 'All', 'Merge') { Write-Json 'fixtures/reference/xlsx/autofit-merge-styles/merge.json' (Measure-Merge $script:xl) }
}
finally {
  Stop-Excel
  if (Test-Path -LiteralPath $scratch) { Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue }
}
