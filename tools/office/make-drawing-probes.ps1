<#
Builds the Open365 drawing probe corpus (fixtures/xlsx/drawing-*.xlsx) through Excel COM. Excel writes every
file; nothing is hand-edited. The data is synthetic (a generated gradient picture, a rectangle, a column chart).

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this
script is never run in CI. CI consumes the committed fixtures and the references that drawing-oracle.ps1
writes (fixtures/reference/xlsx/<name>/drawing.json).

Fixtures
  drawing-basic    default grid plus custom widths and heights; a picture, a rectangle and a chart, all
                   move-and-size, anchored at fractional offsets inside their cells.
  drawing-editas   a picture, a rectangle and a chart per Placement (xlMoveAndSize, xlMove, xlFreeFloating),
                   then columns widened and rows made taller above and left of them, so the saved anchors
                   differ per editAs mode.
  drawing-hidden   hidden and resized rows and columns: objects created over cells that are hidden or
                   resized afterwards (each Placement), and objects created after the hiding.
  drawing-anchors  drawing-basic with its anchors rewritten as oneCellAnchor and absoluteAnchor (zip patch of
                   xl/drawings/drawing1.xml; Excel then opens the result for the oracle).

  pwsh -NoProfile -File tools/office/make-drawing-probes.ps1 [-Only '^drawing-basic']

Process safety: starts its own hidden Excel instance, records its process id and only ever stops that
process (in a finally block). Other EXCEL and WINWORD processes are never touched.
#>
param([string]$Only)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@

$script:xl = $null
$script:xlPid = 0
$script:basicPos = @()

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

function Write-Manifest($name, $notes) {
  $t = "feature = `"drawing`"`nmade_with = `"Microsoft Excel $($script:xl.Build) (Windows)`"`nauthor = `"project member`"`nexpect_dropped = []`nnotes = `"$notes`"`n"
  [IO.File]::WriteAllText((Join-Path $fixtureDir "$name.toml"), $t, (New-Object Text.UTF8Encoding($false)))
}

# A 96x64 gradient PNG with a frame, generated here (no external image).
$png = Join-Path ([IO.Path]::GetTempPath()) 'open365-drawing-probe.png'
function Make-Png {
  $bmp = New-Object System.Drawing.Bitmap 96, 64
  for ($y = 0; $y -lt 64; $y++) { for ($x = 0; $x -lt 96; $x++) {
    $edge = ($x -lt 2 -or $y -lt 2 -or $x -ge 94 -or $y -ge 62)
    if ($edge) { $c = [Drawing.Color]::FromArgb(20, 30, 60) } else { $c = [Drawing.Color]::FromArgb(40 + $x, 90 + $y * 2, 200 - $x) }
    $bmp.SetPixel($x, $y, $c)
  } }
  $bmp.Save($png, [Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
}

function Fill-Data($ws) {
  $ws.Cells.Item(1, 1).Value2 = 'Item'; $ws.Cells.Item(1, 2).Value2 = 'A'; $ws.Cells.Item(1, 3).Value2 = 'B'
  $a = 12, 19, 7, 15; $b = 8, 14, 11, 9
  for ($i = 0; $i -lt 4; $i++) {
    $ws.Cells.Item($i + 2, 1).Value2 = "r$($i + 1)"
    $ws.Cells.Item($i + 2, 2).Value2 = [double]$a[$i]; $ws.Cells.Item($i + 2, 3).Value2 = [double]$b[$i]
  }
}

# Adds one object at (l, t, w, h) points. kind: pic | rect | chart. placement: 1 move+size, 2 move, 3 free.
function Add-Obj($ws, $kind, $name, $l, $t, $w, $h, $placement) {
  switch ($kind) {
    'pic' { $s = $ws.Shapes.AddPicture($png, 0, -1, $l, $t, $w, $h) }
    'rect' { $s = $ws.Shapes.AddShape(1, $l, $t, $w, $h) }
    'chart' {
      $co = $ws.ChartObjects().Add($l, $t, $w, $h)
      $co.Chart.ChartType = 51
      $co.Chart.SetSourceData($ws.Range('A1:C5'))
      $co.Chart.ChartArea.Font.Name = 'Calibri'
      $s = $ws.Shapes.Item($ws.Shapes.Count)
    }
  }
  $s.Name = $name
  $s.Placement = $placement
  return $s
}

function New-Probe($name, $notes, [scriptblock]$build) {
  if ($Only -and $name -notmatch $Only) { return }
  $path = Join-Path $fixtureDir "$name.xlsx"
  if (Test-Path $path) { Remove-Item $path -Force }
  $wb = $script:xl.Workbooks.Add()
  & $build $wb
  $wb.RemovePersonalInformation = $true
  $wb.SaveAs($path, 51)
  Write-Manifest $name $notes
  $wb.Close($false)
  [void][Runtime.InteropServices.Marshal]::ReleaseComObject($wb)
  Write-Output "wrote $name"
}

function New-Sheet($wb, $name, [bool]$first) {
  if ($first) { $ws = $wb.Worksheets.Item(1) }
  else { $ws = $wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
  $ws.Name = $name
  Fill-Data $ws
  return $ws
}

function Basic($wb) {
  $ws = New-Sheet $wb 'Basic' $true
  $ws.Columns.Item(3).ColumnWidth = 18; $ws.Columns.Item(5).ColumnWidth = 4
  $ws.Rows.Item(3).RowHeight = 30; $ws.Rows.Item(6).RowHeight = 9.75
  $b2 = $ws.Range('B2'); $e3 = $ws.Range('E3'); $b12 = $ws.Range('B12'); $h26 = $ws.Range('H26')
  $s = Add-Obj $ws 'pic' 'Pic 1' ($b2.Left + 7.5) ($b2.Top + 4.25) 90 60 1
  $script:basicPos += , @($s.Left, $s.Top, $s.Width, $s.Height)
  $s = Add-Obj $ws 'rect' 'Rect 1' ($e3.Left + 3.2) ($e3.Top + 9.9) 100.5 40.25 1
  $script:basicPos += , @($s.Left, $s.Top, $s.Width, $s.Height)
  $s = Add-Obj $ws 'chart' 'Chart 1' $b12.Left $b12.Top ($h26.Left - $b12.Left) ($h26.Top - $b12.Top) 1
  $script:basicPos += , @($s.Left, $s.Top, $s.Width, $s.Height)
  # A rotated rectangle: shows whether the anchor holds the unrotated frame or the rotated bounds.
  $j3 = $ws.Range('J3')
  $s = Add-Obj $ws 'rect' 'Rect rot' ($j3.Left + 5) ($j3.Top + 5) 120 40 1
  $s.Rotation = 30
  $script:basicPos += , @($s.Left, $s.Top, $s.Width, $s.Height)
}

function EditAs($wb) {
  $ws = New-Sheet $wb 'EditAs' $true
  $y = 60
  foreach ($p in 1, 2, 3) {
    $c = $ws.Range('D1').Offset(0, ($p - 1) * 4)
    [void](Add-Obj $ws 'pic' "Pic p$p" ($c.Left + 5) $y 90 60 $p)
    [void](Add-Obj $ws 'rect' "Rect p$p" ($c.Left + 5) ($y + 80) 100 40 $p)
    [void](Add-Obj $ws 'chart' "Chart p$p" ($c.Left + 5) ($y + 140) 180 110 $p)
  }
  # Grow cells above and to the left of every object, as a user would, after the objects exist.
  $ws.Columns.Item(1).ColumnWidth = 20
  $ws.Columns.Item(2).ColumnWidth = 15
  $ws.Rows.Item(1).RowHeight = 40
  $ws.Rows.Item(2).RowHeight = 25.5
}

function Hidden($wb) {
  $ws = New-Sheet $wb 'AfterHide' $true
  # Objects first (one per Placement), then hide and resize cells beneath and before them.
  foreach ($p in 1, 2, 3) {
    $c = $ws.Range('B2').Offset(($p - 1) * 8, 0)
    [void](Add-Obj $ws 'rect' "Over p$p" ($c.Left + 4) ($c.Top + 3) 260 110 $p)
  }
  $ws.Columns.Item('C').Hidden = $true
  $ws.Rows.Item(4).Hidden = $true
  $ws.Rows.Item(12).Hidden = $true
  $ws.Rows.Item(13).Hidden = $true
  $ws.Columns.Item('E').ColumnWidth = 3
  $ws.Rows.Item(20).RowHeight = 33.75
  $ws2 = New-Sheet $wb 'BeforeHide' $false
  # Hide and resize first, then place objects by the resulting coordinates, including inside hidden cells.
  $ws2.Columns.Item('C').Hidden = $true
  $ws2.Columns.Item('F').ColumnWidth = 2.5
  $ws2.Rows.Item(4).Hidden = $true
  $ws2.Rows.Item(5).RowHeight = 45
  $ws2.Rows.Item(9).Hidden = $true
  $a = $ws2.Range('B2'); $b = $ws2.Range('G8')
  [void](Add-Obj $ws2 'rect' 'Span' ($a.Left + 2) ($a.Top + 2) ($b.Left - $a.Left) ($b.Top - $a.Top) 1)
  $d = $ws2.Range('D10')
  [void](Add-Obj $ws2 'pic' 'Pic after' ($d.Left + 3) ($d.Top + 3) 90 60 1)
  $c3 = $ws2.Range('C3')
  [void](Add-Obj $ws2 'rect' 'In hidden col' $c3.Left ($c3.Top + 1) 40 20 1)
  $r9 = $ws2.Range('B9')
  [void](Add-Obj $ws2 'rect' 'In hidden row' ($r9.Left + 1) $r9.Top 40 20 1)
}

function Make-Anchors {
  $name = 'drawing-anchors'
  if ($Only -and $name -notmatch $Only) { return }
  $src = Join-Path $fixtureDir 'drawing-basic.xlsx'
  $dst = Join-Path $fixtureDir "$name.xlsx"
  Copy-Item $src $dst -Force
  $zip = [IO.Compression.ZipFile]::Open($dst, 'Update')
  try {
    $entry = $zip.GetEntry('xl/drawings/drawing1.xml')
    $sr = New-Object IO.StreamReader($entry.Open()); $text = $sr.ReadToEnd(); $sr.Close()
    $xdr = 'http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing'
    $doc = New-Object Xml.XmlDocument; $doc.PreserveWhitespace = $true; $doc.LoadXml($text)
    $anchors = @($doc.DocumentElement.ChildNodes | Where-Object { $_.LocalName -eq 'twoCellAnchor' })
    $i = 0
    foreach ($a in $anchors) {
      $i++
      $xe = $a.SelectSingleNode(".//*[local-name()='ext' and @cx]")
      $cx = $xe.GetAttribute('cx'); $cy = $xe.GetAttribute('cy')
      if ($i -eq 1) {
        $n = $doc.CreateElement('xdr', 'oneCellAnchor', $xdr)
        $n.AppendChild($a.SelectSingleNode("*[local-name()='from']")) | Out-Null
      } else {
        $n = $doc.CreateElement('xdr', 'absoluteAnchor', $xdr)
        $pos = $doc.CreateElement('xdr', 'pos', $xdr)
        $p = $script:basicPos[$i - 1]
        $pos.SetAttribute('x', [string][int64][math]::Round($p[0] * 12700))
        $pos.SetAttribute('y', [string][int64][math]::Round($p[1] * 12700))
        $n.AppendChild($pos) | Out-Null
      }
      $e = $doc.CreateElement('xdr', 'ext', $xdr); $e.SetAttribute('cx', $cx); $e.SetAttribute('cy', $cy)
      $n.AppendChild($e) | Out-Null
      foreach ($child in @($a.ChildNodes)) {
        if ($child.LocalName -notin 'from', 'to') { $n.AppendChild($child) | Out-Null }
      }
      $doc.DocumentElement.ReplaceChild($n, $a) | Out-Null
    }
    $entry.Delete()
    $new = $zip.CreateEntry('xl/drawings/drawing1.xml')
    $sw = New-Object IO.StreamWriter($new.Open(), (New-Object Text.UTF8Encoding($false)))
    $sw.Write($doc.OuterXml); $sw.Close()
  } finally { $zip.Dispose() }
  Write-Manifest $name 'drawing-basic with oneCellAnchor (picture) and absoluteAnchor (rectangle, chart)'
  Write-Output "wrote $name"
}

try {
  Make-Png
  Start-Excel
  New-Probe 'drawing-basic' 'picture, rectangle and chart, move and size, fractional offsets, custom widths and heights' { param($wb) Basic $wb }
  New-Probe 'drawing-editas' 'picture, rectangle and chart per Placement, cells above and left grown after creation' { param($wb) EditAs $wb }
  New-Probe 'drawing-hidden' 'hidden and resized rows and columns, before and after the objects exist' { param($wb) Hidden $wb }
  if ($script:basicPos.Count -eq 0 -and (-not $Only -or 'drawing-anchors' -match $Only)) {
    # -Only skipped drawing-basic: read the object rectangles back from the saved file.
    $wb = $script:xl.Workbooks.Open((Join-Path $fixtureDir 'drawing-basic.xlsx'), 0, $true)
    foreach ($s in $wb.Worksheets.Item(1).Shapes) { $script:basicPos += , @($s.Left, $s.Top, $s.Width, $s.Height) }
    $wb.Close($false)
  }
  Make-Anchors
} finally {
  Stop-Excel
  Remove-Item $png -ErrorAction SilentlyContinue
}
