<#
Uses the locally installed Microsoft Word / Excel 16.0 as the oracle for the application chrome: ribbon tabs, groups,
controls, title bar, Quick Access Toolbar, search box, status bar, formula bar and sheet tabs. Output feeds
crates/ui-shell (fixtures/reference/ui/*.json) and its geometry test.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed JSON.

What it records (clean-room: structure and numbers only, no artwork):
  * app facts: version, build, window DPI and scale, window size.
  * the UI Automation tree of the ribbon (every tab selected in turn): tab / group / control names, UIA control
    types, automation ids, class names, enabled state, and bounding rectangles in physical pixels relative to the
    window's top-left corner.
  * the same for the title area (Quick Access Toolbar, document title, search box, caption buttons), the status bar
    and, for Excel, the formula bar, Name Box and sheet tabs.
  * CommandBars.GetLabelMso / GetEnabledMso for every control whose automation id is an idMso (label and enabled
    only; screen-tip text is NOT stored, only whether one exists).
  * a screenshot of the window per tab (PrintWindow, so other windows cannot overlap it). Screenshots contain
    Microsoft artwork: they are written ONLY to -ShotDir (default: a folder under the temp dir) and never to the repo.

Measured at the machine's own display scale only. The script never changes Windows regional, display or Office
security settings, so other scales are derived (px / scale), not measured; the JSON says so (`measuredScale`).

Process safety: starts its OWN visible instance, sets AutomationSecurity = 3 (msoAutomationSecurityForceDisable),
takes the process id from the window handle and, in `finally`, calls Quit and only then stops that exact process id
if it is still running and is still the expected executable. Other WINWORD/EXCEL processes are never touched.

  pwsh -NoProfile -File tools/office/ribbon-oracle.ps1 [-App Word|Excel|Both] [-ShotDir <dir>]
#>
param(
  [ValidateSet('Word', 'Excel', 'Both')][string]$App = 'Both',
  # Also record the ribbon at these window widths (physical px) into <app>-responsive.json (group reduction).
  [string]$ResponsiveWidths = '',
  # Also record the title row, tab row, status bar (and for Excel the formula strip / sheet tabs) at these window
  # widths into <app>-chrome-responsive.json (which items hide, how tabs compress, which status items drop out).
  [string]$ChromeWidths = '',
  # Do not rewrite <app>-ribbon.json (the main record goes to -ShotDir instead); use with -ChromeWidths.
  [switch]$ChromeOnly,
  # Milliseconds to wait after each resize before reading the chrome (-ChromeWidths).
  [int]$ChromeSettleMs = 700,
  # -ChromeWidths also accepts ranges: 1900..425:25 (from..to:step).
  [string]$ShotDir = (Join-Path ([IO.Path]::GetTempPath()) 'open365-ribbon-shots'),
  [int]$WindowWidth = 1600,
  [int]$WindowHeight = 1000
)
$ErrorActionPreference = 'Stop'
$ResponsiveList = @($ResponsiveWidths -split ',' | Where-Object { $_ } | ForEach-Object { [int]$_ })
$ChromeList = @($ChromeWidths -split ',' | Where-Object { $_ } | ForEach-Object {
    if ($_ -match '^(\d+)\.\.(\d+):(\d+)$') {
      $a = [int]$Matches[1]; $b = [int]$Matches[2]; $st = [int]$Matches[3]
      if ($a -ge $b) { for ($x = $a; $x -ge $b; $x -= $st) { $x } } else { for ($x = $a; $x -le $b; $x += $st) { $x } }
    } else { [int]$_ }
  })
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$outDir = Join-Path $root 'fixtures/reference/ui'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
New-Item -ItemType Directory -Force -Path $ShotDir | Out-Null

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type -Namespace Native -Name Ui -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint p);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(System.IntPtr c);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern uint GetDpiForWindow(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool SetWindowPos(System.IntPtr h, System.IntPtr a, int x, int y, int w, int hh, uint f);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool PrintWindow(System.IntPtr h, System.IntPtr dc, uint flags);
[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool GetWindowRect(System.IntPtr h, out RECT r);
[System.Runtime.InteropServices.StructLayout(System.Runtime.InteropServices.LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
'@
# Physical pixels: UIA rectangles are only comparable across machines when the process is per-monitor DPI aware.
[void][Native.Ui]::SetProcessDpiAwarenessContext([IntPtr](-4))

$UIA = [Windows.Automation.AutomationElement]
$walker = [Windows.Automation.TreeWalker]::ControlViewWalker

function Get-TypeName($el) { $el.Current.ControlType.ProgrammaticName -replace '^ControlType\.', '' }

# Rectangles are relative to the window origin so the JSON does not depend on where the window sits.
function Get-Rect($el, $ox, $oy) {
  $r = $el.Current.BoundingRectangle
  if ([double]::IsInfinity($r.X) -or $r.IsEmpty) { return @(0, 0, 0, 0) }
  return @([int]($r.X - $ox), [int]($r.Y - $oy), [int]$r.Width, [int]$r.Height)
}

$script:MsoCache = @{}
function Get-Mso($office, $id) {
  if ([string]::IsNullOrEmpty($id)) { return $null }
  if ($script:MsoCache.ContainsKey($id)) { return $script:MsoCache[$id] }
  $info = $null
  try {
    $cb = $office.CommandBars
    $label = $cb.GetLabelMso($id)
    $enabled = $cb.GetEnabledMso($id)
    $hasTip = $false
    try { $hasTip = -not [string]::IsNullOrEmpty($cb.GetScreentipMso($id)) } catch {}
    $info = [ordered]@{ label = $label; enabled = [bool]$enabled; hasScreentip = $hasTip }
  } catch {}
  $script:MsoCache[$id] = $info
  return $info
}

$script:Scrub = @()
function Scrub-Text([string]$s) {
  if ($s) { $s = [regex]::Replace($s, '[\u200e\u200f\u202a-\u202e]', '') }
  foreach ($t in $script:Scrub) { if ($t -and $s) { $s = $s.Replace($t, '<user>') } }
  if ($s -match '^(Document|Book)\d+\s+-\s+(Word|Excel)$') { $s = $s -replace '^(Document|Book)\d+', '<document>' }
  return $s
}

# Walks the UIA subtree. `skipClasses` elements are recorded but not descended into (the document canvas).
function Dump-Element($el, [int]$ox, [int]$oy, [int]$depth, [int]$maxDepth, $office, $skipClasses, [ref]$budget) {
  if ($budget.Value -le 0) { return $null }
  $budget.Value--
  $c = $el.Current
  $id = $c.AutomationId
  if ($id -eq 'MeControlWidget' -and $c.Name -and $script:Scrub -notcontains $c.Name) { $script:Scrub += $c.Name }
  $node = [ordered]@{
    type = Get-TypeName $el
    name = if ($id -in 'MeControlWidget', 'PersonaPicture') { '<user>' } else { Scrub-Text $c.Name }
    id = $id
    cls = $c.ClassName
    rect = Get-Rect $el $ox $oy
    enabled = [bool]$c.IsEnabled
    offscreen = [bool]$c.IsOffscreen
  }
  # Alt key-tip sequence ("Alt, H, B"), exposed by UI Automation.
  if ($c.AccessKey) { $node.accessKey = $c.AccessKey }
  $mso = Get-Mso $office $id
  if ($mso) { $node.mso = $mso }
  $kids = New-Object System.Collections.ArrayList
  if ($depth -lt $maxDepth -and ($skipClasses -notcontains $c.ClassName)) {
    $child = $walker.GetFirstChild($el)
    while ($child) {
      $d = Dump-Element $child $ox $oy ($depth + 1) $maxDepth $office $skipClasses $budget
      if ($d) { [void]$kids.Add($d) }
      $child = $walker.GetNextSibling($child)
    }
  }
  if ($kids.Count -gt 0) { $node.children = $kids.ToArray() }
  return $node
}

function Flatten-Nodes($node) {
  $copy = [ordered]@{}
  foreach ($k in $node.Keys) { if ($k -ne 'children') { $copy[$k] = $node[$k] } }
  $copy
  if ($node.Contains('children')) { foreach ($c in $node.children) { Flatten-Nodes $c } }
}

function Find-All($el, [scriptblock]$pred, [int]$maxDepth) {
  $found = New-Object System.Collections.ArrayList
  $stack = New-Object System.Collections.Stack
  $stack.Push(@($el, 0))
  while ($stack.Count -gt 0) {
    $item = $stack.Pop(); $e = $item[0]; $d = $item[1]
    if (& $pred $e) { [void]$found.Add($e) }
    if ($d -lt $maxDepth) {
      $ch = $walker.GetFirstChild($e)
      while ($ch) { $stack.Push(@($ch, ($d + 1))); $ch = $walker.GetNextSibling($ch) }
    }
  }
  return $found
}

function Save-Shot($hwnd, [string]$path, [int]$w, [int]$h) {
  $bmp = New-Object System.Drawing.Bitmap $w, $h
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $dc = $g.GetHdc()
  [void][Native.Ui]::PrintWindow($hwnd, $dc, 2)
  $g.ReleaseHdc($dc); $g.Dispose()
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
}

# Colour runs along a line of a saved screenshot: @(start, length, 'rrggbb'). Only numbers are kept; the image stays out of the repo.
function Scan-Runs([string]$path, [int]$x0, [int]$y0, [int]$dx, [int]$dy, [int]$n, [int]$maxRuns = 40) {
  $bmp = New-Object System.Drawing.Bitmap $path
  $runs = New-Object System.Collections.ArrayList
  $cur = $null; $start = 0
  for ($i = 0; $i -lt $n; $i++) {
    $x = $x0 + $dx * $i; $y = $y0 + $dy * $i
    if ($x -ge $bmp.Width -or $y -ge $bmp.Height) { break }
    $c = $bmp.GetPixel($x, $y); $hex = '{0:x2}{1:x2}{2:x2}' -f $c.R, $c.G, $c.B
    if ($hex -ne $cur) {
      if ($null -ne $cur -and $runs.Count -lt $maxRuns) { [void]$runs.Add(@($start, ($i - $start), $cur)) }
      $cur = $hex; $start = $i
    }
  }
  if ($null -ne $cur -and $runs.Count -lt $maxRuns) { [void]$runs.Add(@($start, ($n - $start), $cur)) }
  $bmp.Dispose()
  return $runs.ToArray()
}

function Invoke-Oracle([string]$Which) {
  $exe = if ($Which -eq 'Word') { 'WINWORD' } else { 'EXCEL' }
  # Office processes that were already running: if COM hands us one of them we abort (never touch them).
  $preIds = @(Get-Process -Name $exe -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $office = New-Object -ComObject "$Which.Application"
  $procId = [uint32]0
  $mine = $false
  $script:RibbonWasCollapsed = $false
  try {
    $office.Visible = $true
    $office.AutomationSecurity = 3
    $office.DisplayAlerts = 0
    if ($Which -eq 'Word') { [void]$office.Documents.Add(); $h = $office.ActiveWindow.Hwnd } else { [void]$office.Workbooks.Add(); $h = $office.Hwnd }
    $hwnd = [IntPtr][int64]$h
    [void][Native.Ui]::GetWindowThreadProcessId($hwnd, [ref]$procId)
    if ($preIds -contains [int]$procId) { throw "COM attached to an Office process that was already running (pid $procId); aborting without touching it" }
    $mine = $true
    # Default window size/position as Office created it (before we resize), and the minimum size it accepts.
    $wrect = New-Object Native.Ui+RECT
    [void][Native.Ui]::GetWindowRect($hwnd, [ref]$wrect)
    $defaultWindow = @($wrect.Left, $wrect.Top, ($wrect.Right - $wrect.Left), ($wrect.Bottom - $wrect.Top))
    [void][Native.Ui]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, 200, 200, 0x14)
    Start-Sleep -Milliseconds 800
    [void][Native.Ui]::GetWindowRect($hwnd, [ref]$wrect)
    $minWindow = @(($wrect.Right - $wrect.Left), ($wrect.Bottom - $wrect.Top))
    # SWP_NOZORDER | SWP_NOACTIVATE: size only, never steal focus.
    [void][Native.Ui]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, $WindowWidth, $WindowHeight, 0x14)
    Start-Sleep -Seconds 3
    # The tab content is only in the UIA tree while the ribbon is pinned open. If this user keeps it collapsed,
    # expand it for the measurement and put it back in `finally` (a view preference of our own instance only).
    try {
      if ($office.CommandBars.GetPressedMso('MinimizeRibbon')) {
        $office.CommandBars.ExecuteMso('MinimizeRibbon'); $script:RibbonWasCollapsed = $true; Start-Sleep -Milliseconds 800
      }
    } catch {}
    $dpi = [Native.Ui]::GetDpiForWindow($hwnd)
    $scale = $dpi / 96.0
    try { $script:Scrub = @($office.UserName, $office.UserInitials) | Where-Object { $_ -and $_.Length -gt 2 } } catch {}
    try { $script:Scrub += @($env:USERNAME) } catch {}

    $win = $UIA::FromHandle($hwnd)
    $wr = $win.Current.BoundingRectangle
    $ox = [int]$wr.X; $oy = [int]$wr.Y
    $winRect = Get-Rect $win $ox $oy

    # Locate the pieces. The ribbon host is the pane that owns the "Ribbon Tabs" tab strip.
    $tabStrip = (Find-All $win { param($e) $e.Current.ControlType -eq [Windows.Automation.ControlType]::Tab -and $e.Current.Name -eq 'Ribbon Tabs' } 14) | Select-Object -First 1
    if (-not $tabStrip) { throw "No ribbon tab strip found in $Which" }
    $ribbonHost = $tabStrip.Current.Name  # name only for the log
    $ribbonPane = $walker.GetParent($tabStrip)
    $tabItems = @(Find-All $tabStrip { param($e) $e.Current.ControlType -eq [Windows.Automation.ControlType]::TabItem } 2)
    [array]::Reverse($tabItems)

    $skip = @('_WwG', '_WwB', 'XLDESK', 'EXCEL7', 'EXCEL6')
    $result = [ordered]@{
      schema = 1
      tool = 'tools/office/ribbon-oracle.ps1'
      app = $Which
      version = "$($office.Version) build $($office.Build)"
      measuredScale = $scale
      dpi = $dpi
      note = 'Rectangles are physical pixels at measuredScale, relative to the window top-left. Other scales are derived (px / measuredScale), not measured.'
      window = $winRect
      defaultWindow = $defaultWindow
      minWindow = $minWindow
      uiaOnlyScrubbed = 'user and document names are replaced; no screenshots or artwork are stored here'
    }

    # Everything in the ribbon pane except the lower ribbon: title bar, QAT, search, tab strip, caption buttons.
    $titleNodes = New-Object System.Collections.ArrayList
    $budget = [ref]400
    $ch = $walker.GetFirstChild($ribbonPane)
    $lowerRibbon = $null
    while ($ch) {
      if ($ch.Current.Name -eq 'Lower Ribbon') { $lowerRibbon = $ch }
      else { $d = Dump-Element $ch $ox $oy 0 3 $office $skip $budget; if ($d) { [void]$titleNodes.Add($d) } }
      $ch = $walker.GetNextSibling($ch)
    }
    $result.chrome = [ordered]@{ ribbonPane = (Get-Rect $ribbonPane $ox $oy); elements = $titleNodes.ToArray() }

    # Every tab in turn: select, wait, dump the lower ribbon.
    $tabs = New-Object System.Collections.ArrayList
    $first = $true
    foreach ($tab in $tabItems) {
      $tname = $tab.Current.Name
      try {
        $sp = $tab.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern)
        $sp.Select()
      } catch { try { $tab.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke() } catch {} }
      Start-Sleep -Milliseconds 900
      $lr = $walker.GetFirstChild($ribbonPane)
      $lower = $null
      while ($lr) { if ($lr.Current.Name -eq 'Lower Ribbon') { $lower = $lr }; $lr = $walker.GetNextSibling($lr) }
      $b = [ref]2500
      $content = if ($lower) { Dump-Element $lower $ox $oy 0 9 $office $skip $b } else { $null }
      [void]$tabs.Add([ordered]@{
          name = $tname; id = $tab.Current.AutomationId; accessKey = $tab.Current.AccessKey; rect = (Get-Rect $tab $ox $oy)
          lowerRibbon = $content
        })
      $shot = Join-Path $ShotDir ("{0}-{1}.png" -f $Which.ToLower(), ($tname -replace '[^A-Za-z0-9]', ''))
      Save-Shot $hwnd $shot $WindowWidth $WindowHeight
    }
    $result.tabs = $tabs.ToArray()

    # Status bar, formula bar, name box, sheet tabs: every top-level pane other than the ribbon dock / document canvas.
    $others = New-Object System.Collections.ArrayList
    $top = $walker.GetFirstChild($win)
    while ($top) {
      $isRibbonDock = ($top.Current.ClassName -eq 'MsoCommandBarDock') -and ((Get-Rect $top $ox $oy)[1] -lt ($WindowHeight / 4))
      if (-not $isRibbonDock -and $top.Current.ClassName -ne 'DropShadow') {
        $b = [ref]800
        # The Excel workbook window (XLDESK) holds the sheet tabs; its children are shallow, the cells are not UIA children.
        if ($top.Current.ClassName -eq 'XLDESK') { $d = Dump-Element $top $ox $oy 0 5 $office @('EXCEL7', 'EXCEL6') $b }
        else { $d = Dump-Element $top $ox $oy 0 7 $office $skip $b }
        if ($d) { [void]$others.Add($d) }
      }
      $top = $walker.GetNextSibling($top)
    }
    $result.panes = $others.ToArray()

    # Sizes the UIA tree does not carry: grid metrics (Excel) or page/zoom metrics (Word). Read-only.
    $m = [ordered]@{}
    try {
      if ($Which -eq 'Excel') {
        $aw = $office.ActiveWindow; $sh = $office.ActiveSheet
        $m.formulaBarHeightLines = $office.FormulaBarHeight
        $m.zoomPercent = $aw.Zoom
        $m.standardHeightPt = $sh.StandardHeight
        $m.standardWidthChars = $sh.StandardWidth
        $a1 = $sh.Range('A1'); $b2 = $sh.Range('B2')
        $m.a1Pt = @($a1.Left, $a1.Top, $a1.Width, $a1.Height)
        $m.b2Pt = @($b2.Left, $b2.Top)
        $m.a1Screen = @($aw.PointsToScreenPixelsX($a1.Left), $aw.PointsToScreenPixelsY($a1.Top))
        $m.b2Screen = @($aw.PointsToScreenPixelsX($b2.Left), $aw.PointsToScreenPixelsY($b2.Top))
        $m.windowOrigin = @($ox, $oy)
        $m.note = 'Screen pixels are absolute; subtract windowOrigin for window-relative. Header size = a1Screen - the XLDESK pane origin.'
      } else {
        $aw = $office.ActiveWindow
        $m.zoomPercent = $aw.View.Zoom.Percentage
        $m.displayRuler = [bool]$aw.ActivePane.DisplayRulers
        $m.viewType = $aw.View.Type
        $pg = $office.ActiveDocument.PageSetup
        $m.pagePt = @($pg.PageWidth, $pg.PageHeight, $pg.LeftMargin, $pg.TopMargin)
        $l = 0; $t = 0; $w = 0; $hh = 0
        $aw.GetPoint([ref]$l, [ref]$t, [ref]$w, [ref]$hh, $office.ActiveDocument.Range(0, 0))
        $m.firstCharScreen = @($l, $t)
        $m.windowOrigin = @($ox, $oy)
        $m.note = 'Page origin = firstCharScreen - margin*zoom*scale; page gap/shadow are not exposed by the object model.'
      }
    } catch { $m.error = $_.Exception.Message }
    try {
      $homeShot = Join-Path $ShotDir ("{0}-Home.png" -f $Which.ToLower())
      if (Test-Path $homeShot) {
        # Runs are relative to the scan origin, which is recorded with each scan (window-relative physical px).
        if ($Which -eq 'Excel') {
          $m.scanRowY500 = @{ origin = @(0, 500); dir = 'x'; runs = (Scan-Runs $homeShot 0 500 1 0 400) }
          $m.scanColX300 = @{ origin = @(300, 300); dir = 'y'; runs = (Scan-Runs $homeShot 300 300 0 1 150) }
          $m.scanColX1500 = @{ origin = @(1500, 300); dir = 'y'; runs = (Scan-Runs $homeShot 1500 300 0 1 700) }
          $m.scanRowY248 = @{ origin = @(0, 248); dir = 'x'; runs = (Scan-Runs $homeShot 0 248 1 0 1600 120) }
          $m.scanRowY300 = @{ origin = @(0, 300); dir = 'x'; runs = (Scan-Runs $homeShot 0 300 1 0 1600 40) }
          $m.scanColX900 = @{ origin = @(900, 215); dir = 'y'; runs = (Scan-Runs $homeShot 900 215 0 1 120 40) }
          $m.scanColX60 = @{ origin = @(60, 215); dir = 'y'; runs = (Scan-Runs $homeShot 60 215 0 1 120 40) }
          $m.scanColX1000Bottom = @{ origin = @(1000, 900); dir = 'y'; runs = (Scan-Runs $homeShot 1000 900 0 1 100 40) }
          $m.scanRowY940 = @{ origin = @(0, 940); dir = 'x'; runs = (Scan-Runs $homeShot 0 940 1 0 1600 60) }
        } else {
          $m.scanRowY500 = @{ origin = @(0, 500); dir = 'x'; runs = (Scan-Runs $homeShot 0 500 1 0 1600) }
          $m.scanColX700 = @{ origin = @(700, 200); dir = 'y'; runs = (Scan-Runs $homeShot 700 200 0 1 800) }
        }
      }
    } catch { $m.scanError = $_.Exception.Message }
    $result.metrics = $m

    # Responsive: the ribbon at narrower windows (group reduction). Compact: group name + rect + control name/rect.
    if ($ResponsiveList.Count -gt 0) {
      $resp = New-Object System.Collections.ArrayList
      foreach ($rw in $ResponsiveList) {
        [void][Native.Ui]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, $rw, $WindowHeight, 0x14)
        Start-Sleep -Milliseconds 1500
        $rt = New-Object System.Collections.ArrayList
        foreach ($tab in $tabItems) {
          try { $tab.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select() } catch {}
          Start-Sleep -Milliseconds 700
          $lr = $walker.GetFirstChild($ribbonPane); $lower = $null
          while ($lr) { if ($lr.Current.Name -eq 'Lower Ribbon') { $lower = $lr }; $lr = $walker.GetNextSibling($lr) }
          $b = [ref]2500
          $content = if ($lower) { Dump-Element $lower $ox $oy 0 9 $office $skip $b } else { $null }
          [void]$rt.Add([ordered]@{ name = $tab.Current.Name; rect = (Get-Rect $tab $ox $oy); lowerRibbon = $content })
        }
        [void]$resp.Add([ordered]@{ width = $rw; tabs = $rt.ToArray() })
      }
      [void][Native.Ui]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, $WindowWidth, $WindowHeight, 0x14)
      $rj = [ordered]@{ schema = 1; app = $Which; measuredScale = $scale; widths = $resp.ToArray() } | ConvertTo-Json -Depth 40 -Compress
      foreach ($t in $script:Scrub) { if ($t) { $rj = $rj.Replace($t, '<user>') } }
      [IO.File]::WriteAllText((Join-Path $outDir ("{0}-responsive.json" -f $Which.ToLower())), $rj, (New-Object Text.UTF8Encoding($false)))
    }

    # Chrome at other window widths: the title row, tab row, status bar and (Excel) formula strip / sheet tabs, one
    # compact record per width: window (outer w, h), pane origin, then [name, id, x, y, w, h, offscreen] with x/y
    # relative to the ribbon pane's top-left corner (so the 9 px invisible resize frame does not matter).
    if ($ChromeList.Count -gt 0) {
      $homeTab = $tabItems | Where-Object { $_.Current.AutomationId -eq 'TabHome' } | Select-Object -First 1
      try { $homeTab.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select() } catch {}
      $statusEl = (Find-All $win { param($e) $e.Current.ControlType -eq [Windows.Automation.ControlType]::StatusBar } 14) | Select-Object -First 1
      $extraEls = @()
      if ($Which -eq 'Excel') {
        foreach ($cls in @('ComboBox', 'ExcelBookTabControl')) {
          $e1 = (Find-All $win { param($e) $e.Current.ClassName -eq $cls } 14) | Select-Object -First 1
          if ($e1) { $extraEls += $e1 }
        }
        $e1 = (Find-All $win { param($e) $e.Current.AutomationId -eq 'FormulaBar' } 14) | Select-Object -First 1
        if ($e1) { $extraEls += $e1 }
      }
      function Add-Rec($list, $el, [int]$px, [int]$py) {
        $c = $el.Current; $r = $c.BoundingRectangle
        if ([double]::IsInfinity($r.X) -or $r.IsEmpty) { $rect = @(0, 0, 0, 0) } else { $rect = @([int]($r.X - $px), [int]($r.Y - $py), [int]$r.Width, [int]$r.Height) }
        $nm = Scrub-Text $c.Name
        if ($c.AutomationId -in 'MeControlWidget', 'PersonaPicture') { $nm = '<user>' }
        [void]$list.Add(@($nm, $c.AutomationId, $rect[0], $rect[1], $rect[2], $rect[3], [int][bool]$c.IsOffscreen))
      }
      $cw = New-Object System.Collections.ArrayList
      foreach ($rw in $ChromeList) {
        [void][Native.Ui]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, $rw, $WindowHeight, 0x14)
        Start-Sleep -Milliseconds $ChromeSettleMs
        $pr = $ribbonPane.Current.BoundingRectangle
        $px = [int]$pr.X; $py = [int]$pr.Y
        $top = New-Object System.Collections.ArrayList
        $ch1 = $walker.GetFirstChild($ribbonPane)
        while ($ch1) {
          $c1 = $ch1.Current
          if ($c1.Name -ne 'Lower Ribbon') {
            if ($c1.ClassName -eq 'NetUIPanViewer' -and $c1.ControlType -eq [Windows.Automation.ControlType]::Tab) {
              Add-Rec $top $ch1 $px $py
              $t1 = $walker.GetFirstChild($ch1)
              while ($t1) { Add-Rec $top $t1 $px $py; $t1 = $walker.GetNextSibling($t1) }
            } elseif ($c1.ClassName -in 'NetUIElement', 'NetUIOfficeCaption', 'NetUISearchBoxAnchor') {
              Add-Rec $top $ch1 $px $py
              if ($c1.ClassName -eq 'NetUIElement') {
                $t1 = $walker.GetFirstChild($ch1)
                while ($t1) { Add-Rec $top $t1 $px $py; $t1 = $walker.GetNextSibling($t1) }
              }
            } else { Add-Rec $top $ch1 $px $py }
          }
          $ch1 = $walker.GetNextSibling($ch1)
        }
        $stat = New-Object System.Collections.ArrayList
        if ($statusEl) {
          Add-Rec $stat $statusEl $px $py
          $t1 = $walker.GetFirstChild($statusEl)
          while ($t1) { Add-Rec $stat $t1 $px $py; $t1 = $walker.GetNextSibling($t1) }
        }
        $extra = New-Object System.Collections.ArrayList
        foreach ($e1 in $extraEls) {
          Add-Rec $extra $e1 $px $py
          if ($e1.Current.ClassName -eq 'ExcelBookTabControl') {
            $t1 = $walker.GetFirstChild($e1)
            while ($t1) { Add-Rec $extra $t1 $px $py; $t1 = $walker.GetNextSibling($t1) }
          }
        }
        $gr = New-Object Native.Ui+RECT
        [void][Native.Ui]::GetWindowRect($hwnd, [ref]$gr)
        [void]$cw.Add([ordered]@{ w = $rw; win = @(($gr.Right - $gr.Left), ($gr.Bottom - $gr.Top)); pane = @($px, $py, [int]$pr.Width, [int]$pr.Height); top = $top.ToArray(); status = $stat.ToArray(); extra = $extra.ToArray() })
      }
      [void][Native.Ui]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, $WindowWidth, $WindowHeight, 0x14)
      $cj = [ordered]@{ schema = 2; app = $Which; measuredScale = $scale; note = 'w = requested outer window width in physical px (client = win[0] - 18). Records are [name, id, x, y, w, h, offscreen] with x, y relative to the ribbon pane origin.'; widths = $cw.ToArray() } | ConvertTo-Json -Depth 8 -Compress
      foreach ($t in $script:Scrub) { if ($t) { $cj = $cj.Replace($t, '<user>') } }
      [IO.File]::WriteAllText((Join-Path $outDir ("{0}-chrome-responsive.json" -f $Which.ToLower())), $cj, (New-Object Text.UTF8Encoding($false)))
    }

    # Backstage: open the File tab, record the navigation list and layout, then go back.
    try {
      $fileTab = (Find-All $ribbonPane { param($e) $e.Current.AutomationId -eq 'FileTabButton' } 6) | Select-Object -First 1
      if ($fileTab) {
        $fileTab.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
        Start-Sleep -Milliseconds 1500
        $win2 = $UIA::FromHandle($hwnd)
        $b = [ref]600
        $bs = Dump-Element $win2 $ox $oy 0 10 $office @('_WwG', '_WwB', 'XLDESK', 'EXCEL7') $b
        # Keep the navigation column and the title strip only: the content pane lists the user's recent files.
        $result.backstage = @(Flatten-Nodes $bs | Where-Object { $_.rect[0] -lt 215 -or $_.rect[1] -lt 80 })
        Save-Shot $hwnd (Join-Path $ShotDir ("{0}-backstage.png" -f $Which.ToLower())) $WindowWidth $WindowHeight
        $back = (Find-All $win2 { param($e) $e.Current.Name -eq 'Back' -and $e.Current.ControlType -eq [Windows.Automation.ControlType]::Button } 12) | Select-Object -First 1
        if ($back) { $back.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke(); Start-Sleep -Milliseconds 800 }
      }
    } catch { $result.backstageError = $_.Exception.Message }

    $path = Join-Path $(if ($ChromeOnly) { $ShotDir } else { $outDir }) ("{0}-ribbon.json" -f $Which.ToLower())
    $json = $result | ConvertTo-Json -Depth 40 -Compress
    foreach ($t in $script:Scrub) { if ($t) { $json = $json.Replace($t, '<user>') } }
    [IO.File]::WriteAllText($path, $json, (New-Object Text.UTF8Encoding($false)))
    Write-Host ("{0}: wrote {1} ({2} tabs, {3} bytes, dpi {4}, pid {5})" -f $Which, $path, $tabs.Count, $json.Length, $dpi, $procId)
  } finally {
    try { if ($mine -and $script:RibbonWasCollapsed) { $office.CommandBars.ExecuteMso('MinimizeRibbon') } } catch {}
    if ($mine) {
      try { foreach ($d in @($office.Documents)) { $d.Saved = $true } } catch {}
      try { foreach ($w in @($office.Workbooks)) { $w.Saved = $true } } catch {}
    }
    if ($mine) { try { $office.Quit() } catch {} }
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($office) } catch {}
    [GC]::Collect(); [GC]::WaitForPendingFinalizers()
    if ($mine -and $procId -gt 0) {
      $deadline = (Get-Date).AddSeconds(10)
      while ((Get-Date) -lt $deadline -and (Get-Process -Id $procId -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 250 }
      $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
      # Only the process id taken from our own window handle.
      if ($p -and $p.ProcessName -eq $exe) { Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue }
    }
  }
}

if ($App -in 'Word', 'Both') { Invoke-Oracle 'Word' }
if ($App -in 'Excel', 'Both') { Invoke-Oracle 'Excel' }
