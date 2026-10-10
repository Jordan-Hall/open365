<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365 spreadsheet cell styles and
displayed text.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed output (fixtures/reference/xlsx/<name>/styles.json) through
crates/xlsx/tests/styles.rs and crates/ui-egui/tests/grid_style.rs.

For every fixture it opens the workbook read-only (never saved) and records, for each cell of each worksheet's
UsedRange that has text, a fill pattern or a border (at most 3000 cells scanned per sheet): Range.Text, the font (name, size, bold, italic, underline, strike,
Font.Color), Interior (Pattern, Color, PatternColor), the four edge Borders (LineStyle, Weight, Color), the
alignment (HorizontalAlignment, VerticalAlignment, WrapText, IndentLevel, Orientation, ShrinkToFit) and the
column width in points. Colours are Excel's resolved BGR integers (theme and tint already applied). The dpi of the
run is recorded because Range.Text depends on it (#### decisions).

-MakeFixture first (re)creates fixtures/xlsx/grid-styles.xlsx and its .toml through COM; docProps/core.xml
dc:creator and cp:lastModifiedBy are then scrubbed to empty strings. Nothing is hand-edited.

  pwsh -NoProfile -File tools/office/grid-style-oracle.ps1                 # grid-styles only
  pwsh -NoProfile -File tools/office/grid-style-oracle.ps1 -MakeFixture    # rebuild the fixture first
  pwsh -NoProfile -File tools/office/grid-style-oracle.ps1 -Only '^number' # any fixture whose name matches

Process safety: starts its own hidden Excel instance, remembers its process id and only ever stops that process
(in a finally block). Other EXCEL/WINWORD processes are never touched.
#>
param(
  [string]$Only = '^grid-styles$',
  [switch]$MakeFixture
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture

Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetDpiForWindow(System.IntPtr hWnd);
'@

$script:xl = $null
$script:xlPid = 0
$script:dpi = 96

function Start-Excel {
  $app = New-Object -ComObject Excel.Application
  $app.Visible = $false
  $app.DisplayAlerts = $false
  $app.ScreenUpdating = $false
  $app.EnableEvents = $false
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  $script:xl = $app
  $script:xlPid = [int]$procId
  $script:dpi = [int][Native.Win32]::GetDpiForWindow([IntPtr]$app.Hwnd)
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

function Num($v) { return ([double]$v).ToString('R', $inv) }

function Json-Str([string]$s) {
  $sb = New-Object System.Text.StringBuilder
  [void]$sb.Append('"')
  foreach ($ch in $s.ToCharArray()) {
    $code = [int]$ch
    if ($ch -eq '"') { [void]$sb.Append('\"') }
    elseif ($ch -eq '\') { [void]$sb.Append('\\') }
    elseif ($code -lt 0x20 -or $code -gt 0x7e) { [void]$sb.AppendFormat('\u{0:x4}', $code) }
    else { [void]$sb.Append($ch) }
  }
  [void]$sb.Append('"')
  return $sb.ToString()
}

function Put($rng, $v) {
  try { $rng.Value2 = $v } catch {
    if ($v -is [double]) { $rng.Formula = $v.ToString('R', $inv) } else { $rng.Formula = [string]$v }
  }
}

function Rgb([int]$r, [int]$g, [int]$b) { return $r + 256 * $g + 65536 * $b }

function Scrub-Core([string]$path) {
  Add-Type -AssemblyName System.IO.Compression
  Add-Type -AssemblyName System.IO.Compression.FileSystem
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

# ---------------------------------------------------------------------------------------------------------------
# Fixture
# ---------------------------------------------------------------------------------------------------------------
function Write-Fixture($app, [string]$path) {
  $wb = $app.Workbooks.Add()
  while ($wb.Worksheets.Count -lt 4) { [void]$wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
  $c = $wb.Worksheets.Item(1); $c.Name = 'Colours'
  $b = $wb.Worksheets.Item(2); $b.Name = 'Borders'
  $t = $wb.Worksheets.Item(3); $t.Name = 'Text'
  $f = $wb.Worksheets.Item(4); $f.Name = 'Fonts'

  # --- Colours: theme + tint (fill and font), indexed palette, RGB, patterns.
  $tints = @(-0.9, -0.5, -0.25, -0.1, 0, 0.1, 0.25, 0.4, 0.8)
  $c.Range('A1:L60').Font.Name = 'Calibri'
  for ($ti = 1; $ti -le 12; $ti++) {
    for ($k = 0; $k -lt $tints.Count; $k++) {
      $cell = $c.Cells.Item(1 + $ti, 2 + $k)
      Put ($cell) ('x')
      $cell.Interior.ThemeColor = $ti
      $cell.Interior.TintAndShade = [double]$tints[$k]
      $cell2 = $c.Cells.Item(15 + $ti, 2 + $k)
      Put ($cell2) (12)
      $cell2.Font.ThemeColor = $ti
      $cell2.Font.TintAndShade = [double]$tints[$k]
    }
  }
  for ($n = 0; $n -lt 56; $n++) {
    $cell = $c.Cells.Item(30 + [int][Math]::Floor($n / 8), 2 + ($n % 8))
    Put ($cell) ('i')
    $cell.Interior.ColorIndex = $n + 1
    $cell.Font.ColorIndex = 56 - $n
  }
  $rgbs = @((Rgb 18 52 86), (Rgb 255 0 0), (Rgb 0 255 0), (Rgb 0 0 255), (Rgb 200 100 50), (Rgb 1 2 3), (Rgb 254 253 252), (Rgb 128 128 128))
  for ($n = 0; $n -lt $rgbs.Count; $n++) {
    $cell = $c.Cells.Item(39, 2 + $n)
    Put ($cell) ('rgb')
    $cell.Interior.Color = $rgbs[$n]
    $cell.Font.Color = $rgbs[($n + 3) % $rgbs.Count]
  }
  $patterns = @(1, -4125, -4126, -4124, 17, 18, -4128, -4166, -4121, -4162, 9, 10, 11, 12, 13, 14, 15, 16, 2, 3, 4, 5, 6, 7, 8)
  for ($n = 0; $n -lt $patterns.Count; $n++) {
    $cell = $c.Cells.Item(42 + [int][Math]::Floor($n / 9), 2 + ($n % 9))
    Put ($cell) ('p')
    $cell.Interior.Pattern = $patterns[$n]
    $cell.Interior.PatternColor = (Rgb 200 0 0)
    $cell.Interior.Color = (Rgb 255 255 150)
  }
  $c.Range('A1').ColumnWidth = 3

  # --- Borders: every style, mixed colours, adjacent cells and an inside grid.
  $b.Range('A1:Z60').Font.Name = 'Calibri'
  $styles = @(
    @(1, 1), @(1, 2), @(1, -4138), @(1, 4), @(-4115, 2), @(-4118, 2), @(-4119, 4),
    @(4, 2), @(5, 2), @(-4115, -4138), @(4, -4138), @(5, -4138), @(13, -4138)
  )
  $edges = @(7, 8, 9, 10)
  for ($n = 0; $n -lt $styles.Count; $n++) {
    $row = 2 + 2 * $n
    $cell = $b.Cells.Item($row, 2)
    Put ($cell) ($n)
    foreach ($e in $edges) {
      $bd = $cell.Borders.Item($e)
      $bd.LineStyle = $styles[$n][0]
      $bd.Weight = $styles[$n][1]
    }
    # Same style, a different colour per edge: RGB, theme with tint, palette index, automatic.
    $cell = $b.Cells.Item($row, 4)
    Put ($cell) ('c')
    foreach ($e in $edges) {
      $bd = $cell.Borders.Item($e)
      $bd.LineStyle = $styles[$n][0]
      $bd.Weight = $styles[$n][1]
    }
    $cell.Borders.Item(7).Color = (Rgb 255 0 0)
    $cell.Borders.Item(8).ThemeColor = 6
    $cell.Borders.Item(8).TintAndShade = 0.4
    $cell.Borders.Item(10).ColorIndex = 5
  }
  # Adjacent cells: a thin red right edge next to a thick blue left edge set after it.
  Put ($b.Cells.Item(30, 2)) 'L'
  Put ($b.Cells.Item(30, 3)) 'R'
  $b.Cells.Item(30, 2).Borders.Item(10).LineStyle = 1
  $b.Cells.Item(30, 2).Borders.Item(10).Weight = 2
  $b.Cells.Item(30, 2).Borders.Item(10).Color = (Rgb 255 0 0)
  $b.Cells.Item(30, 3).Borders.Item(7).LineStyle = 1
  $b.Cells.Item(30, 3).Borders.Item(7).Weight = 4
  $b.Cells.Item(30, 3).Borders.Item(7).Color = (Rgb 0 0 255)
  # Dashed above thin, and an outline with an inside grid.
  Put ($b.Cells.Item(32, 2)) 'T'
  Put ($b.Cells.Item(33, 2)) 'B'
  $b.Cells.Item(32, 2).Borders.Item(9).LineStyle = -4115
  $b.Cells.Item(32, 2).Borders.Item(9).Weight = 2
  $b.Cells.Item(33, 2).Borders.Item(8).LineStyle = 1
  $b.Cells.Item(33, 2).Borders.Item(8).Weight = 2
  $b.Cells.Item(33, 2).Borders.Item(8).Color = (Rgb 0 128 0)
  # Ties where adjacent cells disagree: same style with different colours, and two styles of
  # similar weight. The cell set last is the lower or the right one.
  $b.Cells.Item(36, 2).Borders.Item(10).LineStyle = 1
  $b.Cells.Item(36, 2).Borders.Item(10).Weight = 2
  $b.Cells.Item(36, 2).Borders.Item(10).Color = (Rgb 255 0 0)
  $b.Cells.Item(36, 3).Borders.Item(7).LineStyle = 1
  $b.Cells.Item(36, 3).Borders.Item(7).Weight = 2
  $b.Cells.Item(36, 3).Borders.Item(7).Color = (Rgb 0 0 255)
  $b.Cells.Item(38, 2).Borders.Item(9).LineStyle = 1
  $b.Cells.Item(38, 2).Borders.Item(9).Weight = 2
  $b.Cells.Item(38, 2).Borders.Item(9).Color = (Rgb 255 0 0)
  $b.Cells.Item(39, 2).Borders.Item(8).LineStyle = 1
  $b.Cells.Item(39, 2).Borders.Item(8).Weight = 2
  $b.Cells.Item(39, 2).Borders.Item(8).Color = (Rgb 0 0 255)
  $b.Cells.Item(41, 3).Borders.Item(7).LineStyle = 1
  $b.Cells.Item(41, 3).Borders.Item(7).Weight = 2
  $b.Cells.Item(41, 3).Borders.Item(7).Color = (Rgb 0 0 255)
  $b.Cells.Item(41, 2).Borders.Item(10).LineStyle = 1
  $b.Cells.Item(41, 2).Borders.Item(10).Weight = 2
  $b.Cells.Item(41, 2).Borders.Item(10).Color = (Rgb 255 0 0)
  $b.Cells.Item(43, 2).Borders.Item(10).LineStyle = -4115
  $b.Cells.Item(43, 2).Borders.Item(10).Weight = 2
  $b.Cells.Item(43, 3).Borders.Item(7).LineStyle = -4118
  $b.Cells.Item(43, 3).Borders.Item(7).Weight = 2
  $b.Cells.Item(45, 2).Borders.Item(10).LineStyle = 4
  $b.Cells.Item(45, 2).Borders.Item(10).Weight = 2
  $b.Cells.Item(45, 3).Borders.Item(7).LineStyle = -4115
  $b.Cells.Item(45, 3).Borders.Item(7).Weight = 2
  $b.Cells.Item(47, 2).Borders.Item(10).LineStyle = 1
  $b.Cells.Item(47, 2).Borders.Item(10).Weight = -4138
  $b.Cells.Item(47, 3).Borders.Item(7).LineStyle = -4119
  $b.Cells.Item(47, 3).Borders.Item(7).Weight = 4
  $grid = $b.Range('E32:G34')
  foreach ($e in 7, 8, 9, 10, 11, 12) { $grid.Borders.Item($e).LineStyle = 1; $grid.Borders.Item($e).Weight = 2 }
  $grid.Borders.Item(7).Weight = -4138
  Put $grid 1
  $b.Range('I32:J33').Interior.Color = (Rgb 220 230 241)
  $b.Range('I32:J33').Borders.Item(7).LineStyle = 1
  $b.Range('I32:J33').Borders.Item(7).Weight = 4
  $b.Range('I32:J33').Borders.Item(7).Color = (Rgb 60 60 60)

  # --- Text: alignment, overflow, wrapping, narrow numeric columns.
  $t.Range('A1:Z80').Font.Name = 'Calibri'
  $t.Range('A1:Z80').Font.Size = 11
  $widths = @(8.43, 4, 6, 10, 3, 12, 20, 8)
  for ($i = 0; $i -lt $widths.Count; $i++) { $t.Cells.Item(1, $i + 1).EntireColumn.ColumnWidth = [double]$widths[$i] }
  $txt = 'Overflowing text that is long'
  Put ($t.Range('A1')) ($txt)
  Put ($t.Range('A2')) ($txt)
  Put ($t.Range('B2')) ('x')
  Put ($t.Range('D3')) ($txt)
  $t.Range('D3').HorizontalAlignment = -4152
  Put ($t.Range('D4')) ($txt)
  $t.Range('D4').HorizontalAlignment = -4108
  Put ($t.Range('D5')) ($txt)
  $t.Range('D5').HorizontalAlignment = -4108
  Put ($t.Range('C5')) ('blocker')
  Put ($t.Range('F6')) ($txt)
  $t.Range('F6').HorizontalAlignment = -4108
  Put ($t.Range('G6')) ('r')
  Put ($t.Range('D7')) ($txt)
  $t.Range('D7').HorizontalAlignment = -4152
  Put ($t.Range('C7')) ('l')
  Put ($t.Range('A8')) ('Left with indent')
  $t.Range('A8').IndentLevel = 2
  Put ($t.Range('D8')) ('Right indent text')
  $t.Range('D8').HorizontalAlignment = -4152
  $t.Range('D8').IndentLevel = 1
  # General alignment by type
  Put ($t.Range('G9')) ('text')
  Put ($t.Range('G10')) (42.5)
  $t.Range('G11').Formula = '=TRUE'
  $t.Range('G12').Formula = '=1/0'
  $t.Range('G13').Formula = '=NA()'
  $t.Range('H9').Formula = '="formula text"'
  $t.Range('H10').Formula = '=1+1'
  # Explicit alignments, each with text and number
  $hal = @(1, -4131, -4108, -4152, 5, -4130, 7, -4117)
  for ($n = 0; $n -lt $hal.Count; $n++) {
    Put ($t.Cells.Item(16, 1 + $n)) 'ab'
    $t.Cells.Item(16, 1 + $n).HorizontalAlignment = $hal[$n]
    Put ($t.Cells.Item(17, 1 + $n)) 5
    $t.Cells.Item(17, 1 + $n).HorizontalAlignment = $hal[$n]
  }
  Put ($t.Range('A18')) ('across')
  $t.Range('A18:C18').HorizontalAlignment = 7
  Put ($t.Range('F18')) ('across')
  $t.Range('F18:H18').HorizontalAlignment = 7
  Put ($t.Range('G18')) ('stop')
  # Vertical alignment in a tall row
  $t.Rows.Item(20).RowHeight = 45
  $val = @(-4160, -4108, -4107, -4130, -4117)
  for ($n = 0; $n -lt $val.Count; $n++) {
    Put ($t.Cells.Item(20, 1 + $n)) 'v'
    $t.Cells.Item(20, 1 + $n).VerticalAlignment = $val[$n]
  }
  # Wrapping: autofit rows, wrapped text does not overflow
  Put ($t.Range('G22')) ('wrapped text that needs several lines in a narrow cell')
  $t.Range('G22').WrapText = $true
  Put ($t.Range('D22')) ('wrapped and centred text')
  $t.Range('D22').WrapText = $true
  $t.Range('D22').HorizontalAlignment = -4108
  $t.Range('D22').VerticalAlignment = -4108
  Put ($t.Range('A22')) ('one two three four five six')
  $t.Range('A22').WrapText = $true
  $t.Range('A22').HorizontalAlignment = -4152
  $t.Rows.Item(22).AutoFit() | Out-Null
  Put ($t.Range('A24')) ('Hello' + [char]10 + 'World')
  $t.Range('A24').WrapText = $true
  Put ($t.Range('F24')) ('justified wrapped text spans lines')
  $t.Range('F24').WrapText = $true
  $t.Range('F24').HorizontalAlignment = -4130
  Put ($t.Range('H24')) ('shrink this text to fit')
  $t.Range('H24').ShrinkToFit = $true
  $t.Rows.Item(24).AutoFit() | Out-Null
  # Rotation
  $rot = @(45, 90, -45, -90, -4166)
  for ($n = 0; $n -lt $rot.Count; $n++) {
    Put ($t.Cells.Item(27, 1 + $n)) 'rot'
    $t.Cells.Item(27, 1 + $n).Orientation = $rot[$n]
  }
  $t.Rows.Item(27).RowHeight = 40
  # Narrow numeric columns: B (4), C (6), E (3), F (12)
  $nums = @(1234567, 0.123456789, 3.14159265358979, 1E+25, -12345.6789, 100, 12.5, 0.00001234, 123456789012, 99999.5, -0.5, 1234.5678, 0.000123, 1E-10)
  for ($n = 0; $n -lt $nums.Count; $n++) {
    foreach ($col in 2, 3, 5, 6) { Put ($t.Cells.Item(30 + $n, $col)) ([double]$nums[$n]) }
  }
  Put ($t.Range('A30:A43')) ('General')
  # Fixed formats in narrow columns
  $fmt = @('0.00', '#,##0', '0.0%', '0.00E+00', 'dd/mm/yyyy', '"$"#,##0.00', '@')
  for ($n = 0; $n -lt $fmt.Count; $n++) {
    foreach ($col in 3, 6) {
      $cell = $t.Cells.Item(46 + $n, $col)
      $cell.NumberFormat = $fmt[$n]
      if ($fmt[$n] -eq 'dd/mm/yyyy') { Put $cell 45306 } elseif ($fmt[$n] -eq '@') { Put $cell '12345678' } else { Put $cell 1234.5678 }
    }
  }
  # Bold and larger numbers are wider
  Put ($t.Range('C54')) (12345)
  Put ($t.Range('C55')) (12345)
  $t.Range('C55').Font.Bold = $true
  Put ($t.Range('F54')) (123456)
  Put ($t.Range('F55')) (123456)
  $t.Range('F55').Font.Size = 14

  # --- Fonts
  $f.Range('A1:L30').Font.Name = 'Calibri'
  $f.Range('A1').ColumnWidth = 24
  Put ($f.Range('A1')) 'Plain'
  Put ($f.Range('A2')) 'Bold'; $f.Range('A2').Font.Bold = $true
  Put ($f.Range('A3')) 'Italic'; $f.Range('A3').Font.Italic = $true
  Put ($f.Range('A4')) 'Bold italic'; $f.Range('A4').Font.Bold = $true; $f.Range('A4').Font.Italic = $true
  Put ($f.Range('A5')) 'Single underline'; $f.Range('A5').Font.Underline = 2
  Put ($f.Range('A6')) 'Double underline'; $f.Range('A6').Font.Underline = -4119
  Put ($f.Range('A7')) 'Accounting underline'; $f.Range('A7').Font.Underline = 4
  Put ($f.Range('A8')) 'Double accounting'; $f.Range('A8').Font.Underline = 5
  Put ($f.Range('A9')) 'Strike'; $f.Range('A9').Font.Strikethrough = $true
  Put ($f.Range('A10')) 'Cambria 14'; $f.Range('A10').Font.Name = 'Cambria'; $f.Range('A10').Font.Size = 14
  Put ($f.Range('A11')) 'Calibri 20 red'; $f.Range('A11').Font.Size = 20; $f.Range('A11').Font.Color = (Rgb 192 0 0)
  Put ($f.Range('A12')) 'Everything'; $f.Range('A12').Font.Bold = $true; $f.Range('A12').Font.Italic = $true
  $f.Range('A12').Font.Underline = 2; $f.Range('A12').Font.Strikethrough = $true; $f.Range('A12').Font.ThemeColor = 5
  $f.Range('A12').Interior.ThemeColor = 4; $f.Range('A12').Interior.TintAndShade = -0.1

  $wb.Worksheets.Item(1).Activate()
  $wb.Worksheets.Item(1).Range('A1').Select()
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
  $wb.SaveAs($path, 51)
  $wb.Close($false)
  Scrub-Core $path
  $toml = [IO.Path]::ChangeExtension($path, '.toml')
  $made = "Microsoft Excel $($app.Build) (Windows)"
  $lines = @(
    'feature = "grid-styles"',
    ('made_with = ' + (Json-Str $made)),
    'author = "project member"',
    'expect_dropped = []',
    'notes = "theme colours with tints, indexed and RGB colours, fill patterns, every border style, alignments, wrapping, overflow, narrow numeric columns, fonts"'
  )
  [IO.File]::WriteAllText($toml, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
}

# ---------------------------------------------------------------------------------------------------------------
# Recording
# ---------------------------------------------------------------------------------------------------------------
function Bool-Str($v) { if ([bool]$v) { return 'true' } else { return 'false' } }

function Cell-Json($cell) {
  $addr = $cell.Address($false, $false)
  $fnt = $cell.Font
  $int = $cell.Interior
  $edge = foreach ($e in 7, 8, 9, 10) {
    $bd = $cell.Borders.Item($e)
    $ls = [int]$bd.LineStyle
    if ($ls -eq -4142) { '[-4142,0,0]' } else { '[' + $ls + ',' + [int]$bd.Weight + ',' + [int]$bd.Color + ']' }
  }
  $text = [string]$cell.Text
  if ($text -eq '' -and [int]$int.Pattern -eq -4142 -and (($edge -join '') -eq '[-4142,0,0][-4142,0,0][-4142,0,0][-4142,0,0]')) { return $null }
  $s = '      ' + (Json-Str $addr) + ': {"text": ' + (Json-Str $text)
  $s += ', "font": {"name": ' + (Json-Str ([string]$fnt.Name)) + ', "size": ' + (Num $fnt.Size)
  $s += ', "bold": ' + (Bool-Str $fnt.Bold) + ', "italic": ' + (Bool-Str $fnt.Italic)
  $s += ', "underline": ' + [int]$fnt.Underline + ', "strike": ' + (Bool-Str $fnt.Strikethrough)
  $s += ', "color": ' + [int]$fnt.Color + '}'
  $s += ', "fill": {"pattern": ' + [int]$int.Pattern
  if ([int]$int.Pattern -ne -4142) { $s += ', "color": ' + [int]$int.Color + ', "pattern_color": ' + [int]$int.PatternColor }
  $s += '}'
  $s += ', "borders_lrtb": [' + ($edge[0]) + ', ' + ($edge[3]) + ', ' + ($edge[1]) + ', ' + ($edge[2]) + ']'
  $s += ', "align": {"h": ' + [int]$cell.HorizontalAlignment + ', "v": ' + [int]$cell.VerticalAlignment
  $s += ', "wrap": ' + (Bool-Str $cell.WrapText) + ', "indent": ' + [int]$cell.IndentLevel
  $s += ', "orientation": ' + [int]$cell.Orientation + ', "shrink": ' + (Bool-Str $cell.ShrinkToFit) + '}'
  $s += ', "col_width_pt": ' + (Num $cell.Width) + '}'
  return $s
}

function Read-Styles($app, [string]$path) {
  $wb = $app.Workbooks.Open($path, 0, $true)
  try {
    $sheets = @()
    foreach ($ws in $wb.Worksheets) {
      $used = $ws.UsedRange
      $r0 = $used.Row; $c0 = $used.Column
      $rn = $used.Rows.Count; $cn = $used.Columns.Count
      $cells = @()
      $count = 0
      for ($r = $r0; $r -lt $r0 + $rn; $r++) {
        for ($c = $c0; $c -lt $c0 + $cn; $c++) {
          if ($count -ge 3000) { break }
          $one = Cell-Json $ws.Cells.Item($r, $c)
          if ($one) { $cells += $one }
          $count++
        }
      }
      $sheets += ('    {"name": ' + (Json-Str $ws.Name) + ', "cells": {' + "`n" + ($cells -join ",`n") + "`n    }}")
    }
    return $sheets
  }
  finally {
    $wb.Saved = $true
    $wb.Close($false)
  }
}

try {
  Start-Excel
  if ($MakeFixture) { Write-Fixture $script:xl (Join-Path $fixtureDir 'grid-styles.xlsx') }
  $files = Get-ChildItem -LiteralPath $fixtureDir -Filter '*.xlsx' | Sort-Object Name
  $count = 0
  foreach ($f in $files) {
    $name = [IO.Path]::GetFileNameWithoutExtension($f.Name)
    if ($Only -and $name -notmatch $Only) { continue }
    $sheets = Read-Styles $script:xl $f.FullName
    $lines = @(
      '{',
      ('  "source": "fixtures/xlsx/' + $f.Name + '",'),
      '  "oracle": "Microsoft Excel via COM: Range.Text, Font/Interior/Borders colours (BGR integers, resolved), alignment",',
      ('  "dpi": ' + $script:dpi + ','),
      ('  "excel_build": ' + (Json-Str ([string]$script:xl.Build)) + ','),
      '  "sheets": ['
    )
    $lines += ($sheets -join ",`n")
    $lines += '  ]'
    $lines += '}'
    $outDir = Join-Path $refDir $name
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    [IO.File]::WriteAllText((Join-Path $outDir 'styles.json'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
    $count++
    Write-Host "grid-style-oracle: $name"
  }
  Write-Host "grid-style-oracle: wrote $count styles.json files"
}
finally {
  Stop-Excel
}
