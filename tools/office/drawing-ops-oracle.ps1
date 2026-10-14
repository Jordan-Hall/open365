<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for what happens to worksheet drawing
objects when a user inserts a picture or edits the cells beneath objects.

LOCAL DEVELOPER TOOL ONLY (Microsoft does not support unattended server-side Office automation). CI consumes
the committed output through crates/xlsx/tests/drawing_ops_oracle.rs.

  pwsh -NoProfile -File tools/office/drawing-ops-oracle.ps1 -Make     # (re)creates fixtures/xlsx/drawing-ops.xlsx
  pwsh -NoProfile -File tools/office/drawing-ops-oracle.ps1           # writes the two references below
  pwsh -NoProfile -File tools/office/drawing-ops-oracle.ps1 -Only ops|insert

Outputs
  fixtures/reference/xlsx/drawing-ops/ops.json      per scenario (insert/delete/hide/resize rows, columns, cells),
                                                    every shape: Left, Top, Width, Height, Placement, TopLeftCell,
                                                    BottomRightCell, or "deleted" when the shape no longer exists.
  fixtures/reference/xlsx/drawing-insert/insert.json  Shapes.AddPicture at a cell for PNG, JPEG (300 dpi), GIF, BMP
                                                    and SVG: size in points, Placement and the anchor element Excel
                                                    writes; AddShape and AddTextbox: the Excel-written element.

Method notes
  - Shapes.AddPicture(file, LinkToFile:=0, SaveWithDocument:=-1, Left, Top, -1, -1) places the picture at the
    top-left corner of the active cell at its natural size. The ribbon command scales a picture larger than the
    window to fit; COM does not.
  - Shapes.AddShape / AddTextbox need an explicit size through COM; the click-to-insert default size cannot be
    read here (a click needs mouse input). The element Excel writes (style, text body, insets) is recorded.

Process safety: starts its own hidden Excel instance, records its process id and only ever stops that process
(in a finally block). Other EXCEL and WINWORD processes are never touched.
Output is stable: LF line endings, ASCII only, no absolute paths and no user information.
#>
param([switch]$Make, [string]$Only)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetDpiForWindow(System.IntPtr hWnd);
'@

$script:xl = $null
$script:xlPid = 0
$script:dpi = 96
$tmp = Join-Path ([IO.Path]::GetTempPath()) 'open365-drawing-ops'

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
  $script:dpi = [int][Native.Win32]::GetDpiForWindow([IntPtr]$app.Hwnd)
  Write-Output "Excel PID $procId (started by this script)"
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

function Num($v) { return ([double]$v).ToString('R', $inv) }
function Esc($s) { return ([string]$s).Replace('\', '\\').Replace('"', '\"').Replace("`r", '').Replace("`n", '\n') }
function Cell-Addr($r) { if ($null -eq $r) { return '' }; return [string]$r.Address($false, $false) }
function Write-Json($dir, $file, $text) {
  New-Item -ItemType Directory -Force $dir | Out-Null
  [IO.File]::WriteAllText((Join-Path $dir $file), $text.Replace("`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
}

# ---- fixture: three rectangles, one per Placement, spanning several rows and columns ------------------------
function Make-Fixture {
  $path = Join-Path $fixtureDir 'drawing-ops.xlsx'
  if (Test-Path $path) { Remove-Item $path -Force }
  $wb = $script:xl.Workbooks.Add()
  $ws = $wb.Worksheets.Item(1)
  $ws.Name = 'Ops'
  foreach ($r in 1..14) { $ws.Cells.Item($r, 1).Value2 = "r$r" }
  $ws.Cells.Item(1, 2).Value2 = 'B'
  $starts = @('C5', 'H5', 'M5')
  $p = 1
  foreach ($a in $starts) {
    $c = $ws.Range($a)
    $s = $ws.Shapes.AddShape(1, $c.Left + 4, $c.Top + 3, 150, 70)
    $s.Name = "Box p$p"
    $s.Placement = $p
    $p++
  }
  $r4 = $ws.Range('C20:E21')
  $s = $ws.Shapes.AddShape(1, $r4.Left, $r4.Top, $r4.Width, $r4.Height)
  $s.Name = 'Box p4'
  $s.Placement = 1
  $c = $ws.Range('R5')
  $png = Join-Path $tmp 'ops.png'
  $bmp = New-Object System.Drawing.Bitmap 64, 48
  for ($y = 0; $y -lt 48; $y++) { for ($x = 0; $x -lt 64; $x++) { $bmp.SetPixel($x, $y, [Drawing.Color]::FromArgb(40 + $x, 90 + $y, 200 - $x)) } }
  $bmp.Save($png, [Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
  $pic = $ws.Shapes.AddPicture($png, 0, -1, $c.Left + 2, $c.Top + 2, -1, -1)
  $pic.Name = 'Pic p1'
  $wb.RemovePersonalInformation = $true
  $wb.SaveAs($path, 51)
  $t = "feature = `"drawing`"`nmade_with = `"Microsoft Excel $($script:xl.Build) (Windows)`"`nauthor = `"project member`"`nexpect_dropped = []`nnotes = `"three rectangles (one per Placement) and a picture over rows 5 to 8, for insert, delete, hide and resize scenarios`"`n"
  [IO.File]::WriteAllText((Join-Path $fixtureDir 'drawing-ops.toml'), $t, (New-Object Text.UTF8Encoding($false)))
  $wb.Close($false)
  Write-Output 'wrote drawing-ops'
}

# ---- scenarios -----------------------------------------------------------------------------------------------
$scenarios = [ordered]@{
  'insert-rows-above'    = { param($ws) $ws.Rows.Item('3:4').Insert() | Out-Null }
  'insert-row-inside'    = { param($ws) $ws.Rows.Item('6:6').Insert() | Out-Null }
  'insert-row-at-top'    = { param($ws) $ws.Rows.Item('5:5').Insert() | Out-Null }
  'insert-rows-below'    = { param($ws) $ws.Rows.Item('12:13').Insert() | Out-Null }
  'delete-rows-above'    = { param($ws) $ws.Rows.Item('2:3').Delete() | Out-Null }
  'delete-row-inside'    = { param($ws) $ws.Rows.Item('6:6').Delete() | Out-Null }
  'delete-rows-partial'  = { param($ws) $ws.Rows.Item('3:5').Delete() | Out-Null }
  'delete-rows-over'     = { param($ws) $ws.Rows.Item('4:9').Delete() | Out-Null }
  'insert-cols-left'     = { param($ws) $ws.Columns.Item('B:B').Insert() | Out-Null }
  'insert-col-inside'    = { param($ws) $ws.Columns.Item('E:E').Insert() | Out-Null }
  'insert-col-between'   = { param($ws) $ws.Columns.Item('G:G').Insert() | Out-Null }
  'delete-col-left'      = { param($ws) $ws.Columns.Item('A:A').Delete() | Out-Null }
  'delete-cols-inside'   = { param($ws) $ws.Columns.Item('D:E').Delete() | Out-Null }
  'delete-cols-over'     = { param($ws) $ws.Columns.Item('H:K').Delete() | Out-Null }
  'hide-rows-inside'     = { param($ws) $ws.Rows.Item('6:6').Hidden = $true }
  'hide-rows-over'       = { param($ws) $ws.Rows.Item('5:8').Hidden = $true }
  'hide-rows-above'      = { param($ws) $ws.Rows.Item('2:3').Hidden = $true }
  'hide-col-inside'      = { param($ws) $ws.Columns.Item('D:D').Hidden = $true }
  'hide-cols-over'       = { param($ws) $ws.Columns.Item('C:F').Hidden = $true }
  'hide-cols-left'       = { param($ws) $ws.Columns.Item('A:A').Hidden = $true }
  'row-height-inside'    = { param($ws) $ws.Rows.Item(6).RowHeight = 40 }
  'row-height-above'     = { param($ws) $ws.Rows.Item(2).RowHeight = 40 }
  'col-width-inside'     = { param($ws) $ws.Columns.Item('D').ColumnWidth = 20 }
  'col-width-left'       = { param($ws) $ws.Columns.Item('A').ColumnWidth = 20 }
  'hide-then-unhide'     = { param($ws) $ws.Rows.Item('6:6').Hidden = $true; $ws.Rows.Item('6:6').Hidden = $false }
  'cells-insert-down'    = { param($ws) $ws.Range('C6:F6').Insert(-4121) | Out-Null }
  'cells-insert-down-b'  = { param($ws) $ws.Range('H3:K4').Insert(-4121) | Out-Null }
  'cells-insert-right'   = { param($ws) $ws.Range('C5:D8').Insert(-4161) | Out-Null }
  'cells-delete-up'      = { param($ws) $ws.Range('H3:K4').Delete(-4162) | Out-Null }
  'cells-delete-left'    = { param($ws) $ws.Range('A5:B8').Delete(-4159) | Out-Null }
  'insert-row-at-bottom-edge' = { param($ws) $ws.Rows.Item('22:22').Insert() | Out-Null }
  'insert-row-at-top-edge'  = { param($ws) $ws.Rows.Item('20:20').Insert() | Out-Null }
  'insert-row-mid-edge'     = { param($ws) $ws.Rows.Item('21:21').Insert() | Out-Null }
  'delete-row-below-edge'   = { param($ws) $ws.Rows.Item('22:22').Delete() | Out-Null }
  'delete-row-last-inside'  = { param($ws) $ws.Rows.Item('21:21').Delete() | Out-Null }
  'delete-rows-exact'       = { param($ws) $ws.Rows.Item('20:21').Delete() | Out-Null }
  'insert-col-bottom-edge'  = { param($ws) $ws.Columns.Item('F:F').Insert() | Out-Null }
  'insert-col-left-edge'    = { param($ws) $ws.Columns.Item('C:C').Insert() | Out-Null }
  'delete-cols-exact'       = { param($ws) $ws.Columns.Item('C:E').Delete() | Out-Null }
  'cells-down-at-from-row'  = { param($ws) $ws.Range('C5:F5').Insert(-4121) | Out-Null }
  'cells-down-narrow-band'  = { param($ws) $ws.Range('D6:E6').Insert(-4121) | Out-Null }
  'cells-down-edge-band'    = { param($ws) $ws.Range('D20:E20').Insert(-4121) | Out-Null }
  'cells-right-wide-band'   = { param($ws) $ws.Range('C5:D12').Insert(-4161) | Out-Null }
  'cells-right-at-inside'   = { param($ws) $ws.Range('D5:D8').Insert(-4161) | Out-Null }
  'cells-right-at-from'     = { param($ws) $ws.Range('C5:C8').Insert(-4161) | Out-Null }
  'cells-right-exact-rows'  = { param($ws) $ws.Range('C5:D10').Insert(-4161) | Out-Null }
  'cells-right-fromrow-only' = { param($ws) $ws.Range('C5:D5').Insert(-4161) | Out-Null }
  'cells-left-wide-band'    = { param($ws) $ws.Range('C5:D12').Delete(-4159) | Out-Null }
  'cells-up-inside'         = { param($ws) $ws.Range('C6:F6').Delete(-4162) | Out-Null }
  'cells-up-at-from'        = { param($ws) $ws.Range('C5:F5').Delete(-4162) | Out-Null }
  'cells-up-narrow'         = { param($ws) $ws.Range('D6:E6').Delete(-4162) | Out-Null }
  'row-height-bottom-edge'  = { param($ws) $ws.Rows.Item(22).RowHeight = 40 }
  'hide-row-last'           = { param($ws) $ws.Rows.Item(21).Hidden = $true }
  'hide-cols-p4'            = { param($ws) $ws.Columns.Item('D:D').Hidden = $true }
  'row-insert-then-delete' = { param($ws) $ws.Rows.Item('6:6').Insert() | Out-Null; $ws.Rows.Item('6:6').Delete() | Out-Null }
}

function Record-Shapes($ws) {
  $items = @()
  foreach ($n in 'Box p1', 'Box p2', 'Box p3', 'Box p4', 'Pic p1') {
    $s = $null
    try { $s = $ws.Shapes.Item($n) } catch { $s = $null }
    if ($null -eq $s) {
      $items += ('        {{"name": "{0}", "deleted": true}}' -f $n)
    } else {
      $items += ('        {{"name": "{0}", "placement": {1}, "left": {2}, "top": {3}, "width": {4}, "height": {5}, "topLeftCell": "{6}", "bottomRightCell": "{7}"}}' -f
        $n, [int]$s.Placement, (Num $s.Left), (Num $s.Top), (Num $s.Width), (Num $s.Height), (Cell-Addr $s.TopLeftCell), (Cell-Addr $s.BottomRightCell))
    }
  }
  return ($items -join ",`n")
}

function Run-Ops {
  $path = Join-Path $fixtureDir 'drawing-ops.xlsx'
  if (-not (Test-Path $path)) { throw 'run with -Make first' }
  $out = @()
  $wb = $script:xl.Workbooks.Open($path, 0, $true)
  $ws = $wb.Worksheets.Item(1)
  $out += ('    {"scenario": "initial", "shapes": [' + "`n" + (Record-Shapes $ws) + "`n    ]}")
  $wb.Close($false)
  foreach ($k in $scenarios.Keys) {
    $wb = $script:xl.Workbooks.Open($path, 0, $true)
    $ws = $wb.Worksheets.Item(1)
    $ws.Activate()
    try {
      & $scenarios[$k] $ws
      $out += ('    {"scenario": "' + $k + '", "shapes": [' + "`n" + (Record-Shapes $ws) + "`n    ]}")
    } catch {
      $out += ('    {"scenario": "' + $k + '", "error": "' + (Esc $_.Exception.Message) + '", "shapes": []}')
    }
    $wb.Saved = $true
    $wb.Close($false)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($wb)
    Write-Output "scenario $k"
  }
  $json = "{`n  `"dpi`": $($script:dpi),`n  `"scenarios`": [`n" + ($out -join ",`n") + "`n  ]`n}`n"
  Write-Json (Join-Path $refDir 'drawing-ops') 'ops.json' $json
}

# ---- Insert Picture / shapes: the element Excel writes ---------------------------------------------------------
function Anchor-Xml($wb) {
  $copy = Join-Path $tmp ('copy' + [guid]::NewGuid().ToString('N') + '.xlsx')
  $wb.SaveCopyAs($copy)
  $zip = [IO.Compression.ZipFile]::OpenRead($copy)
  try {
    $entry = $zip.GetEntry('xl/drawings/drawing1.xml')
    $sr = New-Object IO.StreamReader($entry.Open()); $text = $sr.ReadToEnd(); $sr.Close()
  } finally { $zip.Dispose() }
  Remove-Item $copy -Force
  $text = [regex]::Replace($text, '^<\?xml[^>]*\?>\s*', '')
  $text = [regex]::Replace($text, 'descr="[^"]*"', 'descr=""')
  return $text
}

function Make-Images {
  New-Item -ItemType Directory -Force $tmp | Out-Null
  $files = [ordered]@{}
  $bmp = New-Object System.Drawing.Bitmap 96, 64
  $g = [Drawing.Graphics]::FromImage($bmp); $g.Clear([Drawing.Color]::SteelBlue); $g.Dispose()
  $bmp.SetResolution(96, 96); $bmp.Save((Join-Path $tmp 'p96.png'), [Drawing.Imaging.ImageFormat]::Png)
  $files['png-96dpi-96x64'] = 'p96.png'
  $bmp.SetResolution(192, 192); $bmp.Save((Join-Path $tmp 'p192.png'), [Drawing.Imaging.ImageFormat]::Png)
  $files['png-192dpi-96x64'] = 'p192.png'
  $bmp.SetResolution(96, 96); $bmp.Save((Join-Path $tmp 'p.gif'), [Drawing.Imaging.ImageFormat]::Gif)
  $files['gif-96x64'] = 'p.gif'
  $bmp.Save((Join-Path $tmp 'p.bmp'), [Drawing.Imaging.ImageFormat]::Bmp)
  $files['bmp-96x64'] = 'p.bmp'
  $bmp.Dispose()
  $big = New-Object System.Drawing.Bitmap 600, 400
  $g = [Drawing.Graphics]::FromImage($big); $g.Clear([Drawing.Color]::Tomato); $g.Dispose()
  $big.SetResolution(300, 300); $big.Save((Join-Path $tmp 'j300.jpg'), [Drawing.Imaging.ImageFormat]::Jpeg)
  $files['jpeg-300dpi-600x400'] = 'j300.jpg'
  $big.SetResolution(72, 72); $big.Save((Join-Path $tmp 'j72.jpg'), [Drawing.Imaging.ImageFormat]::Jpeg)
  $files['jpeg-72dpi-600x400'] = 'j72.jpg'
  $big.Dispose()
  $huge = New-Object System.Drawing.Bitmap 3000, 2000
  $huge.SetResolution(96, 96); $huge.Save((Join-Path $tmp 'huge.png'), [Drawing.Imaging.ImageFormat]::Png); $huge.Dispose()
  $files['png-96dpi-3000x2000'] = 'huge.png'
  $svg = '<svg xmlns="http://www.w3.org/2000/svg" width="48" height="32" viewBox="0 0 48 32"><rect width="48" height="32" fill="#2a6"/></svg>'
  [IO.File]::WriteAllText((Join-Path $tmp 'p.svg'), $svg, (New-Object Text.UTF8Encoding($false)))
  $files['svg-48x32'] = 'p.svg'
  return $files
}

function Run-Insert {
  $files = Make-Images
  $items = @()
  foreach ($k in $files.Keys) {
    $wb = $script:xl.Workbooks.Add()
    $ws = $wb.Worksheets.Item(1)
    $ws.Rows.Item(3).RowHeight = 30
    $c = $ws.Range('D5')
    try {
      $s = $ws.Shapes.AddPicture((Join-Path $tmp $files[$k]), 0, -1, $c.Left, $c.Top, -1, -1)
      $xml = Anchor-Xml $wb
      $items += ('    {{"case": "{0}", "type": {1}, "placement": {2}, "left": {3}, "top": {4}, "width": {5}, "height": {6}, "topLeftCell": "{7}", "bottomRightCell": "{8}", "xml": "{9}"}}' -f
        $k, [int]$s.Type, [int]$s.Placement, (Num $s.Left), (Num $s.Top), (Num $s.Width), (Num $s.Height), (Cell-Addr $s.TopLeftCell), (Cell-Addr $s.BottomRightCell), (Esc $xml))
    } catch {
      $items += ('    {{"case": "{0}", "error": "{1}"}}' -f $k, (Esc $_.Exception.Message))
    }
    $wb.Saved = $true; $wb.Close($false)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($wb)
    Write-Output "insert $k"
  }
  # Shapes: msoShapeRectangle (1), msoShapeOval (9) and a text box with text.
  foreach ($case in 'rectangle', 'oval', 'textbox', 'textbox-text', 'line') {
    $wb = $script:xl.Workbooks.Add()
    $ws = $wb.Worksheets.Item(1)
    $c = $ws.Range('D5')
    switch ($case) {
      'rectangle' { $s = $ws.Shapes.AddShape(1, $c.Left, $c.Top, 100, 60) }
      'oval' { $s = $ws.Shapes.AddShape(9, $c.Left, $c.Top, 100, 60) }
      'textbox' { $s = $ws.Shapes.AddTextbox(1, $c.Left, $c.Top, 100, 60) }
      'textbox-text' { $s = $ws.Shapes.AddTextbox(1, $c.Left, $c.Top, 100, 60); $s.TextFrame2.TextRange.Text = 'Hello' }
      'line' { $s = $ws.Shapes.AddLine($c.Left, $c.Top, $c.Left + 100, $c.Top + 60) }
    }
    $xml = Anchor-Xml $wb
    $items += ('    {{"case": "{0}", "type": {1}, "placement": {2}, "left": {3}, "top": {4}, "width": {5}, "height": {6}, "topLeftCell": "{7}", "bottomRightCell": "{8}", "xml": "{9}"}}' -f
      $case, [int]$s.Type, [int]$s.Placement, (Num $s.Left), (Num $s.Top), (Num $s.Width), (Num $s.Height), (Cell-Addr $s.TopLeftCell), (Cell-Addr $s.BottomRightCell), (Esc $xml))
    $wb.Saved = $true; $wb.Close($false)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($wb)
    Write-Output "insert $case"
  }
  $json = "{`n  `"dpi`": $($script:dpi),`n  `"cases`": [`n" + ($items -join ",`n") + "`n  ]`n}`n"
  Write-Json (Join-Path $refDir 'drawing-insert') 'insert.json' $json
}

try {
  New-Item -ItemType Directory -Force $tmp | Out-Null
  Start-Excel
  if ($Make) { Make-Fixture }
  else {
    if (-not $Only -or $Only -eq 'ops') { Run-Ops }
    if (-not $Only -or $Only -eq 'insert') { Run-Insert }
  }
} finally {
  Stop-Excel
  Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
