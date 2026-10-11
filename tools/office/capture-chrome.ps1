<#
Captures the application chrome of the locally installed Microsoft Excel / Word as pixels, for the Open365 preview
pixel comparison and the chrome metric measurements (menu rows, arrows, icons, gallery cells, text fit).

LOCAL DEVELOPER TOOL ONLY (Office is not supported for unattended automation; never run in CI).

Output goes to -OutDir (a scratch folder; default is a folder under the temp dir). It is never the repository:
the images contain Microsoft artwork. Only numbers derived from them may be committed.

Files written per capture <name>:
  <name>.png   viewable image
  <name>.raw   8-byte header (width, height as u32 little endian) then width*height RGBA bytes, row major
and one manifest <app>-capture.json: capture names, window size, scale, theme sample, popup rectangles (window relative,
physical px), opener rectangles.

Safety (same rules as ribbon-oracle.ps1):
  * starts its OWN visible instance; if COM hands back an Office process that was already running the script aborts
    without touching it;
  * AutomationSecurity = 3 (msoAutomationSecurityForceDisable), DisplayAlerts off;
  * `finally`: Quit, then stop only the exact process id taken from our own window handle, and only when it is still
    the expected executable;
  * never changes ClearType, DPI, theme or any Office option. Whatever the app draws now is what is captured.

  pwsh -NoProfile -File tools/office/capture-chrome.ps1 [-App Excel|Word|Both] [-OutDir <dir>] [-Popups]

-Popups (Excel): opens the Home tab menus, galleries and combo lists through UI Automation (ExpandCollapse) and
captures each popup window with PrintWindow. -TextProbes (Excel): adds a worksheet text box with black text on white
(grayscale antialiasing in the drawing layer) and a cell range with the same text (ClearType), for the text fit.
#>
param(
  [ValidateSet('Word', 'Excel', 'Both')][string]$App = 'Both',
  [string]$OutDir = (Join-Path ([IO.Path]::GetTempPath()) 'open365-capture-chrome'),
  [int]$WindowWidth = 1600,
  [int]$WindowHeight = 1000,
  [switch]$Popups,
  [switch]$TextProbes,
  [string]$PopupIds = ''
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type -Namespace Native -Name Cap -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint p);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(System.IntPtr c);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern uint GetDpiForWindow(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool SetWindowPos(System.IntPtr h, System.IntPtr a, int x, int y, int w, int hh, uint f);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool PrintWindow(System.IntPtr h, System.IntPtr dc, uint flags);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool PostMessage(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);
'@
[void][Native.Cap]::SetProcessDpiAwarenessContext([IntPtr](-4))

$UIA = [Windows.Automation.AutomationElement]
$rawWalker = [Windows.Automation.TreeWalker]::RawViewWalker

# PrintWindow (PW_RENDERFULLCONTENT) into a bitmap; writes <name>.png and <name>.raw (RGBA with a size header).
function Save-Capture($hwnd, [string]$name, [int]$w, [int]$h) {
  $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $dc = $g.GetHdc()
  [void][Native.Cap]::PrintWindow($hwnd, $dc, 2)
  $g.ReleaseHdc($dc); $g.Dispose()
  $bmp.Save((Join-Path $OutDir "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  $rect = New-Object System.Drawing.Rectangle 0, 0, $w, $h
  $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $bytes = New-Object byte[] ($w * $h * 4)
  [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
  $bmp.UnlockBits($data); $bmp.Dispose()
  for ($i = 0; $i -lt $bytes.Length; $i += 4) { $b = $bytes[$i]; $bytes[$i] = $bytes[$i + 2]; $bytes[$i + 2] = $b; $bytes[$i + 3] = 255 }
  $fs = [IO.File]::Create((Join-Path $OutDir "$name.raw"))
  $hdr = [BitConverter]::GetBytes([uint32]$w) + [BitConverter]::GetBytes([uint32]$h)
  $fs.Write($hdr, 0, 8); $fs.Write($bytes, 0, $bytes.Length); $fs.Close()
}

function Get-ProcTops([int]$procId, $host_) {
  $h = @{}
  $cond = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ProcessIdProperty), $procId
  foreach ($e in $UIA::RootElement.FindAll([Windows.Automation.TreeScope]::Children, $cond)) {
    if ($e.Current.ClassName -eq 'MSO_BORDEREFFECT_WINDOW_CLASS' -and -not $e.Current.IsOffscreen) { $h[($e.GetRuntimeId() -join '.')] = $e }
  }
  if ($host_) {
    $ch = $rawWalker.GetFirstChild($host_)
    while ($ch) {
      if ($ch.Current.ClassName -eq 'Net UI Tool Window' -and -not $ch.Current.IsOffscreen) { $h[($ch.GetRuntimeId() -join '.')] = $ch }
      $ch = $rawWalker.GetNextSibling($ch)
    }
  }
  return $h
}

function Find-First($root, [scriptblock]$pred, [int]$maxDepth = 12) {
  $stack = New-Object System.Collections.Stack
  $stack.Push(@($root, 0))
  while ($stack.Count -gt 0) {
    $it = $stack.Pop(); $e = $it[0]; $d = $it[1]
    if (& $pred $e) { return $e }
    if ($d -lt $maxDepth) {
      try {
        $ch = $rawWalker.GetFirstChild($e)
        while ($ch) { $stack.Push(@($ch, ($d + 1))); $ch = $rawWalker.GetNextSibling($ch) }
      } catch {}
    }
  }
  return $null
}

function Invoke-Capture([string]$Which) {
  $exe = if ($Which -eq 'Word') { 'WINWORD' } else { 'EXCEL' }
  $preIds = @(Get-Process -Name $exe -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $office = New-Object -ComObject "$Which.Application"
  $procId = [uint32]0
  $mine = $false
  $key = $Which.ToLower()
  try {
    $office.Visible = $true
    $office.AutomationSecurity = 3
    $office.DisplayAlerts = 0
    if ($Which -eq 'Word') { [void]$office.Documents.Add(); $h = $office.ActiveWindow.Hwnd } else { [void]$office.Workbooks.Add(); $h = $office.Hwnd }
    $hwnd = [IntPtr][int64]$h
    [void][Native.Cap]::GetWindowThreadProcessId($hwnd, [ref]$procId)
    if ($preIds -contains [int]$procId) { throw "COM attached to an Office process that was already running (pid $procId); aborting without touching it" }
    $mine = $true
    # SWP_NOZORDER | SWP_NOACTIVATE
    [void][Native.Cap]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, $WindowWidth, $WindowHeight, 0x14)
    Start-Sleep -Seconds 3
    $dpi = [Native.Cap]::GetDpiForWindow($hwnd)
    $scale = $dpi / 96.0
    $win = $UIA::FromHandle($hwnd)
    $manifest = [ordered]@{ app = $Which; scale = $scale; window = @($WindowWidth, $WindowHeight); captures = @(); popups = @() }

    # Home tab selected (it is the default tab; select it anyway so the capture is repeatable).
    $homeTab = Find-First $win { param($e) $e.Current.AutomationId -eq 'TabHome' }
    try { if ($homeTab) { $homeTab.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select() } } catch {}
    Start-Sleep -Milliseconds 900
    Save-Capture $hwnd "$key-home" $WindowWidth $WindowHeight
    $manifest.captures += "$key-home"

    if ($TextProbes -and $Which -eq 'Excel') {
      # A text box (drawing layer) and cells carrying the same black-on-white text, so the grayscale and the ClearType
      # rendering of the same glyphs can be compared. Our own workbook only; it is never saved.
      $ws = $office.ActiveSheet
      $ws.Range('B2:B6').Value2 = 'The quick brown fox jumps over the lazy dog 0123456789'
      $shape = $ws.Shapes.AddTextbox(1, 400, 30, 420, 120)
      $shape.TextFrame2.TextRange.Text = "The quick brown fox jumps over the lazy dog 0123456789`rHamburgefonstiv, Rhythm and Blues"
      $shape.TextFrame2.TextRange.Font.Size = 11
      $shape.TextFrame2.TextRange.Font.Name = 'Segoe UI'
      $shape.TextFrame2.TextRange.Font.Fill.ForeColor.RGB = 0
      $shape.Fill.ForeColor.RGB = 0xFFFFFF
      Start-Sleep -Milliseconds 900
      Save-Capture $hwnd "$key-textprobe" $WindowWidth $WindowHeight
      $manifest.captures += "$key-textprobe"
      $shape.Delete()
    }

    if ($Popups -and $Which -eq 'Excel') {
      $targets = @('FontColorPicker_Dropdown', 'CellFillColorPicker_Dropdown', 'Font', 'FontSize', 'NumberFormatGallery',
        'BordersGallery_Dropdown', 'AccountingFormatMenu_Dropdown', 'FillMenu', 'ClearMenu', 'SortAndFilterMenu',
        'ConditionalFormattingMenu', 'FormatCellsMenu')
      if ($PopupIds) { $targets = @($PopupIds -split ',') }
      $baseline = Get-ProcTops $procId $win
      foreach ($tid in $targets) {
        $el = Find-First $win { param($e) $e.Current.AutomationId -eq $tid }
        if (-not $el) { $manifest.popups += [ordered]@{ id = $tid; missing = $true }; continue }
        $or = $el.Current.BoundingRectangle
        $wr = $win.Current.BoundingRectangle
        $rec = [ordered]@{ id = $tid; opener = @([int]($or.X - $wr.X), [int]($or.Y - $wr.Y), [int]$or.Width, [int]$or.Height); windows = @() }
        try {
          $o = $null
          if (-not $el.TryGetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$o)) { $rec.noExpand = $true; $manifest.popups += $rec; continue }
          $o.Expand()
          $shown = $false
          for ($i = 0; $i -lt 16 -and -not $shown; $i++) {
            Start-Sleep -Milliseconds 250
            $shown = @((Get-ProcTops $procId $win).Values | Where-Object { $_.Current.ClassName -eq 'Net UI Tool Window' }).Count -gt 0
          }
          Start-Sleep -Milliseconds 700
          $now = Get-ProcTops $procId $win
          $k = 0
          foreach ($pk in $now.Keys) {
            if ($baseline.ContainsKey($pk)) { continue }
            $pe = $now[$pk]; $c = $pe.Current; $r = $c.BoundingRectangle
            if ([double]::IsInfinity($r.X) -or $r.IsEmpty -or $c.ClassName -ne 'Net UI Tool Window') { continue }
            $nh = [IntPtr][int64]$c.NativeWindowHandle
            if ($nh -eq [IntPtr]::Zero) { continue }
            $nm = "$key-popup-$tid-$k"
            Save-Capture $nh $nm ([int]$r.Width) ([int]$r.Height)
            $rec.windows += [ordered]@{ capture = $nm; rect = @([int]($r.X - $wr.X), [int]($r.Y - $wr.Y), [int]$r.Width, [int]$r.Height) }
            $k++
          }
        } catch { $rec.error = $_.Exception.Message }
        try { $o.Collapse() } catch {}
        Start-Sleep -Milliseconds 500
        for ($i = 0; $i -lt 6; $i++) {
          $now = Get-ProcTops $procId $win
          $extra = @($now.Keys | Where-Object { -not $baseline.ContainsKey($_) })
          if ($extra.Count -eq 0) { break }
          foreach ($pk in $extra) {
            $nh = [int64]$now[$pk].Current.NativeWindowHandle
            if ($nh -ne 0) { [void][Native.Cap]::PostMessage([IntPtr]$nh, 0x100, [IntPtr]0x1B, [IntPtr]0); [void][Native.Cap]::PostMessage([IntPtr]$nh, 0x101, [IntPtr]0x1B, [IntPtr]0) }
          }
          Start-Sleep -Milliseconds 400
        }
        $manifest.popups += $rec
      }
    }
    $json = $manifest | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText((Join-Path $OutDir "$key-capture.json"), $json, (New-Object Text.UTF8Encoding($false)))
    Write-Host ("{0}: {1} captures, {2} popup records, scale {3}" -f $Which, $manifest.captures.Count, $manifest.popups.Count, $scale)
  } finally {
    if ($mine) {
      try { foreach ($d in @($office.Documents)) { $d.Saved = $true } } catch {}
      try { foreach ($w in @($office.Workbooks)) { $w.Saved = $true } } catch {}
      try { $office.Quit() } catch {}
    }
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($office) } catch {}
    [GC]::Collect(); [GC]::WaitForPendingFinalizers()
    if ($mine -and $procId -gt 0) {
      $deadline = (Get-Date).AddSeconds(10)
      while ((Get-Date) -lt $deadline -and (Get-Process -Id $procId -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 250 }
      $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
      if ($p -and $p.ProcessName -eq $exe) { Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue }
    }
  }
}

if ($App -in 'Excel', 'Both') { Invoke-Capture 'Excel' }
if ($App -in 'Word', 'Both') { Invoke-Capture 'Word' }
