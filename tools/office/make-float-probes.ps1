<#
Generates the floating picture / shape placement probes (fixtures/docx/layout-float-*.docx + .toml, the directory every parity test reads) by driving the locally
installed Microsoft Word over COM. Each probe is one Letter sheet of generic text with a handful of floating pictures
or shapes whose wrap type, distances, anchors and positions are set through the object model. The reference JSON
(each float's final rectangle and the lines around it) comes from tools/office/word-layout-oracle.ps1:

  pwsh -NoProfile -File tools/office/make-float-probes.ps1
  pwsh -NoProfile -File tools/office/word-layout-oracle.ps1 -Path fixtures/docx/layout-float-wrap-square.docx   # one per probe, or the whole of fixtures/docx

LOCAL DEVELOPER TOOL ONLY (Microsoft does not support unattended server-side Office automation).
Starts its own Word instance (AutomationSecurity = force-disable macros), never attaches to or kills another one,
quits it in a finally block. Document author and properties are generic. All text and pictures are original.
Calibri 11, Letter, 72 pt margins on every probe, so the geometry is the same on every machine and locale.
Constants: WdWrapType square 0, tight 1, through 2, none/front 3, topBottom 4, behind 5; WdWrapSideType both 0, left 1,
right 2, largest 3; horizontal relative-to margin 0, page 1, column 2, character 3, leftMargin 4, rightMargin 5, inside 6,
outside 7; vertical relative-to margin 0, page 1, paragraph 2, line 3, topMargin 4, bottomMargin 5, inside 6, outside 7;
alignment constants left -999998, center -999995, right -999996, top -999999, bottom -999997, inside -999994, outside -999993.
#>
param([string[]]$Only = @())   # fixture names to (re)create; default: all float probes
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$outDir = Join-Path $root 'fixtures\docx'
New-Item -ItemType Directory -Force $outDir | Out-Null
$created = New-Object System.Collections.Generic.List[string]
$skipped = New-Object System.Collections.Generic.List[string]
function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }

$LEFT = -999998; $CENTER = -999995; $RIGHT = -999996; $TOP = -999999; $BOTTOM = -999997; $INSIDE = -999994; $OUTSIDE = -999993

$T1 = 'The morning ferry crosses the estuary twice before the tide turns, carrying commuters, bicycles and crates of vegetables from the farms on the far shore. Nobody on board hurries; they read, drink tea from flasks and watch the herons stand in the shallows. When the ferry docks, a short queue forms along the jetty and the deckhand coils the rope with practised ease.'
$T2 = 'A well-made map leaves out far more than it shows. The cartographer decides which roads matter, which rivers deserve a name and where a label may sit without hiding a village. Every choice trades detail for clarity, and the reader trusts the result because the trades were made carefully and applied the same way across the whole sheet.'
$T3 = 'In the workshop the lamps stay lit late. Shavings gather under the bench, a kettle ticks as it cools, and the half-finished chair waits for its last joint. The maker steps back, squints along the rail and decides that tomorrow will do for the glue, since good work is not improved by being rushed past the point of care.'

$wordBefore = Get-Pids 'WINWORD'
$word = $null; $oldUser = $null; $oldInit = $null; $oldLocal = $null; $wordPids = @()
$imgDir = Join-Path ([IO.Path]::GetTempPath()) ("o365_float_{0}" -f $PID)
try {
  Add-Type -AssemblyName System.Drawing
  New-Item -ItemType Directory -Force $imgDir | Out-Null
  $word = New-Object -ComObject Word.Application
  $wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $wordBefore })
  if ($wordPids.Count -eq 0) { throw 'COM attached to a Word instance this script did not start; refusing to touch it' }
  $word.Visible = $false; $word.DisplayAlerts = 0
  $word.AutomationSecurity = 3   # msoAutomationSecurityForceDisable
  $oldUser = $word.UserName; $oldInit = $word.UserInitials; $oldLocal = $word.Options.UseLocalUserInfo
  $word.Options.UseLocalUserInfo = $true
  $word.UserName = 'project member'; $word.UserInitials = 'PM'

  function New-Png($name, [int[]]$rgb) {
    $path = Join-Path $script:imgDir "$name.png"
    $bmp = New-Object System.Drawing.Bitmap 64, 64
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::FromArgb($rgb[0], $rgb[1], $rgb[2]))
    $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::White), 3
    $g.DrawRectangle($pen, 1, 1, 61, 61); $g.DrawLine($pen, 0, 0, 63, 63)
    $pen.Dispose(); $g.Dispose(); $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    $path
  }
  $RED = New-Png 'red' @(200, 40, 40); $BLUE = New-Png 'blue' @(40, 70, 200); $GREEN = New-Png 'green' @(40, 160, 70)

  # Letter, 72 pt margins, Calibri 11, single spacing, 6 pt after. Body: one paragraph per entry of $texts.
  function Init-Doc([string[]]$texts) {
    $d = $script:doc
    $n = $d.Styles.Item('Normal')
    $n.Font.Name = 'Calibri'; $n.Font.Size = 11
    $n.ParagraphFormat.SpaceBefore = 0; $n.ParagraphFormat.SpaceAfter = 6; $n.ParagraphFormat.LineSpacingRule = 0
    $ps = $d.PageSetup
    $ps.PageWidth = 612; $ps.PageHeight = 792
    $ps.TopMargin = 72; $ps.BottomMargin = 72; $ps.LeftMargin = 72; $ps.RightMargin = 72
    $d.Content.Text = ($texts -join "`r")
  }
  function Body($count) { $o = @(); for ($i = 0; $i -lt $count; $i++) { $o += (@($T1, $T2, $T3)[$i % 3]) }; $o }

  # Floating picture anchored at the start of paragraph $pi. $o: hashtable of
  # w h wrap side hrel x vrel y dist(@T,B,L,R) overlap(bool) incell(bool) png.
  function Add-F($pi, $o) {
    $r = $script:doc.Paragraphs.Item($pi).Range; $r.Collapse(1)
    $png = if ($o.png) { $o.png } else { $script:RED }
    $sh = $script:doc.InlineShapes.AddPicture($png, $false, $true, $r).ConvertToShape()
    $sh.LockAspectRatio = 0; $sh.Width = [single]$o.w; $sh.Height = [single]$o.h
    Set-Float $sh $o
    $sh
  }
  function Set-Float($sh, $o) {
    $wrap = if ($null -ne $o.wrap) { $o.wrap } else { 0 }
    $sh.WrapFormat.Type = $wrap
    if ($wrap -in 0, 1, 2 -and $null -ne $o.side) { $sh.WrapFormat.Side = $o.side }
    if ($o.dist) { $sh.WrapFormat.DistanceTop = $o.dist[0]; $sh.WrapFormat.DistanceBottom = $o.dist[1]; $sh.WrapFormat.DistanceLeft = $o.dist[2]; $sh.WrapFormat.DistanceRight = $o.dist[3] }
    if ($null -ne $o.overlap) { $sh.WrapFormat.AllowOverlap = [int]([bool]$o.overlap) * -1 }
    if ($null -ne $o.incell) { $sh.LayoutInCell = [int]([bool]$o.incell) * -1 }
    $sh.RelativeHorizontalPosition = $(if ($null -ne $o.hrel) { $o.hrel } else { 0 })
    $sh.RelativeVerticalPosition = $(if ($null -ne $o.vrel) { $o.vrel } else { 2 })
    $sh.Left = [single]$(if ($null -ne $o.x) { $o.x } else { 0 }); $sh.Top = [single]$(if ($null -ne $o.y) { $o.y } else { 0 })
  }
  # Solid-colour autoshape (no outline) anchored at paragraph $pi; $type is an MsoAutoShapeType (1 rectangle, 7 isosceles triangle, 9 oval).
  function Add-S($pi, $type, $rgb, $o) {
    $r = $script:doc.Paragraphs.Item($pi).Range; $r.Collapse(1)
    $sh = $script:doc.Shapes.AddShape($type, 0, 0, [single]$o.w, [single]$o.h, $r)
    $sh.Fill.ForeColor.RGB = $rgb[0] + 256 * $rgb[1] + 65536 * $rgb[2]
    $sh.Line.Visible = 0
    Set-Float $sh $o
    $sh
  }

  function F-Fixture($name, $notes, [scriptblock]$build, $compat = 0) {
    $doc = $null; $path = Join-Path $script:outDir "$name.docx"
    if ($script:Only.Count -gt 0 -and $name -notin $script:Only) { return }
    try {
      if (Test-Path $path) { Remove-Item $path -Force }
      $doc = $script:word.Documents.Add()
      $script:doc = $doc
      if ($compat) { $doc.SetCompatibilityMode($compat) }
      & $build
      $doc.RemovePersonalInformation = $true
      $doc.SaveAs2($path, 16)
      Set-Content -Path (Join-Path $script:outDir ("$name.toml")) -Value ("feature = `"layout-float`"
made_with = `"$($script:word.Name) $($script:word.Build) (Windows)`"
author = `"project member`"
expect_dropped = []
notes = `"$notes`"
") -Encoding utf8 -NoNewline
      $script:created.Add($name)
    } catch {
      $script:skipped.Add("$name : $($_.Exception.Message) @ $($_.InvocationInfo.PositionMessage -replace '\s+', ' ')")
      Remove-Item $path, (Join-Path $script:outDir "$name.toml") -Force -ErrorAction SilentlyContinue
    } finally {
      if ($doc) { try { $doc.Close(0) } catch {}; Release $doc }
      $script:doc = $null
    }
  }

  # ---- wrap types ----------------------------------------------------------------------------------------------
  F-Fixture 'layout-float-wrap-square' 'square wrapping, both sides: float aligned left, right and centred in the margin box' {
    Init-Doc (Body 9)
    [void](Add-F 1 @{ w = 100; h = 60; wrap = 0; side = 0; hrel = 0; x = $LEFT;   vrel = 2; y = 0 })
    [void](Add-F 4 @{ w = 110; h = 70; wrap = 0; side = 0; hrel = 0; x = $RIGHT;  vrel = 2; y = 0; png = $BLUE })
    [void](Add-F 7 @{ w = 120; h = 50; wrap = 0; side = 0; hrel = 0; x = $CENTER; vrel = 2; y = 0; png = $GREEN })
  }
  F-Fixture 'layout-float-wrap-sides' 'square wrapping with wrap side left, right and largest, float at an offset inside the text column' {
    Init-Doc (Body 9)
    [void](Add-F 1 @{ w = 100; h = 60; wrap = 0; side = 1; hrel = 0; x = 150; vrel = 2; y = 0 })
    [void](Add-F 4 @{ w = 110; h = 70; wrap = 0; side = 2; hrel = 0; x = 150; vrel = 2; y = 0; png = $BLUE })
    [void](Add-F 7 @{ w = 120; h = 50; wrap = 0; side = 3; hrel = 0; x = 100; vrel = 2; y = 0; png = $GREEN })
  }
  F-Fixture 'layout-float-wrap-tight' 'tight and through wrapping: rectangular pictures, then a triangle and an oval autoshape' {
    Init-Doc (Body 12)
    [void](Add-F 1 @{ w = 100; h = 60; wrap = 1; side = 0; hrel = 0; x = $LEFT;  vrel = 2; y = 0 })
    [void](Add-F 3 @{ w = 110; h = 70; wrap = 2; side = 0; hrel = 0; x = $RIGHT; vrel = 2; y = 0; png = $BLUE })
    [void](Add-S 6 7 @(200, 120, 20) @{ w = 140; h = 90; wrap = 1; side = 0; hrel = 0; x = 120; vrel = 2; y = 0 })
    [void](Add-S 9 9 @(120, 40, 160) @{ w = 130; h = 100; wrap = 1; side = 0; hrel = 0; x = $RIGHT; vrel = 2; y = 0 })
    [void](Add-S 11 7 @(20, 140, 160) @{ w = 150; h = 80; wrap = 2; side = 0; hrel = 0; x = 140; vrel = 2; y = 0 })
  }
  F-Fixture 'layout-float-wrap-topbottom' 'top and bottom wrapping: aligned, offset and with distances' {
    Init-Doc (Body 9)
    [void](Add-F 2 @{ w = 100; h = 60; wrap = 4; hrel = 0; x = $LEFT; vrel = 2; y = 0 })
    [void](Add-F 5 @{ w = 120; h = 50; wrap = 4; hrel = 0; x = 200; vrel = 2; y = 0; dist = @(10, 20, 0, 0); png = $BLUE })
    [void](Add-F 8 @{ w = 90; h = 70; wrap = 4; hrel = 0; x = 100; vrel = 2; y = 30; dist = @(0, 0, 0, 0); png = $GREEN })
  }
  F-Fixture 'layout-float-wrap-behind-front' 'behind text and in front of text: body lines are not displaced' {
    Init-Doc (Body 6)
    [void](Add-F 1 @{ w = 100; h = 60; wrap = 5; hrel = 0; x = 120; vrel = 2; y = 0 })
    [void](Add-F 3 @{ w = 110; h = 70; wrap = 3; hrel = 0; x = 250; vrel = 2; y = 0; png = $BLUE })
    [void](Add-F 5 @{ w = 120; h = 50; wrap = 5; hrel = 0; x = $LEFT; vrel = 2; y = 0; png = $GREEN })
  }
  F-Fixture 'layout-float-dist' 'square wrapping with wrap distances distT/distB/distL/distR of 0, 6, 12 and 24 points, float in mid column' {
    Init-Doc (Body 12)
    [void](Add-F 1 @{ w = 100; h = 60; wrap = 0; side = 0; hrel = 0; x = 150; vrel = 2; y = 0; dist = @(0, 0, 0, 0) })
    [void](Add-F 4 @{ w = 100; h = 61; wrap = 0; side = 0; hrel = 0; x = 150; vrel = 2; y = 0; dist = @(6, 6, 6, 6); png = $BLUE })
    [void](Add-F 7 @{ w = 100; h = 62; wrap = 0; side = 0; hrel = 0; x = 150; vrel = 2; y = 0; dist = @(12, 12, 12, 12); png = $GREEN })
    [void](Add-F 10 @{ w = 100; h = 63; wrap = 0; side = 0; hrel = 0; x = 150; vrel = 2; y = 0; dist = @(24, 24, 24, 24) })
  }
  F-Fixture 'layout-float-dist-mixed' 'square wrapping with different distances per edge (T/B/L/R)' {
    Init-Doc (Body 12)
    [void](Add-F 1 @{ w = 100; h = 60; wrap = 0; side = 0; hrel = 0; x = 150; vrel = 2; y = 0; dist = @(20, 0, 0, 0) })
    [void](Add-F 4 @{ w = 100; h = 61; wrap = 0; side = 0; hrel = 0; x = 150; vrel = 2; y = 0; dist = @(0, 20, 0, 0); png = $BLUE })
    [void](Add-F 7 @{ w = 100; h = 62; wrap = 0; side = 0; hrel = 0; x = 150; vrel = 2; y = 0; dist = @(0, 0, 30, 0); png = $GREEN })
    [void](Add-F 10 @{ w = 100; h = 63; wrap = 0; side = 0; hrel = 0; x = 150; vrel = 2; y = 0; dist = @(0, 0, 0, 30) })
  }

  # ---- positions -----------------------------------------------------------------------------------------------
  # In-front floats (wrap none) so only the position rule is measured; offsets are numeric.
  F-Fixture 'layout-float-posh' 'positionH relative to page, margin, column, character, left margin, right margin, inside, outside; numeric offsets' {
    Init-Doc (Body 9)
    $i = 0
    foreach ($rel in 1, 0, 2, 3, 4, 5, 6, 7) {
      $i++
      [void](Add-F ([math]::Min(9, $i)) @{ w = (40 + $i); h = 30; wrap = 3; hrel = $rel; x = 20; vrel = 1; y = (60 * $i); png = $(if ($i % 2) { $RED } else { $BLUE }) })
    }
  }
  F-Fixture 'layout-float-posv' 'positionV relative to page, margin, paragraph, line, top margin, bottom margin; numeric offsets' {
    Init-Doc (Body 12)
    $i = 0
    foreach ($rel in 1, 0, 2, 3, 4, 5) {
      $i++
      [void](Add-F (2 * $i - 1) @{ w = (30 + $i); h = (20 + $i); wrap = 3; hrel = 1; x = (60 * $i); vrel = $rel; y = 30; png = $(if ($i % 2) { $RED } else { $BLUE }) })
    }
  }
  F-Fixture 'layout-float-posv-line' 'positionV relative to line: anchor in the first, second and third line of a paragraph, square wrapping' {
    Init-Doc (Body 6)
    # Anchor mid-paragraph: characters inside paragraph 2 and 4 (second and third line).
    $d = $script:doc
    foreach ($spec in @(@(2, 100, 0), @(4, 200, 1))) {
      $pr = $d.Paragraphs.Item($spec[0]).Range
      $r = $d.Range($pr.Start + $spec[1], $pr.Start + $spec[1])
      $sh = $d.InlineShapes.AddPicture($(if ($spec[2]) { $BLUE } else { $RED }), $false, $true, $r).ConvertToShape()
      $sh.LockAspectRatio = 0; $sh.Width = 60 + 10 * $spec[2]; $sh.Height = 40
      Set-Float $sh @{ wrap = 0; side = 0; hrel = 3; x = 20; vrel = 3; y = 10 }
    }
  }
  F-Fixture 'layout-float-align' 'alignment vs offset: align left/center/right/inside/outside against page and margin, and top/center/bottom' {
    Init-Doc (Body 9)
    $o = @(
      @{ p = 1; hrel = 1; x = $LEFT;    vrel = 1; y = $TOP;    w = 41 },
      @{ p = 1; hrel = 1; x = $CENTER;  vrel = 1; y = $TOP;    w = 42 },
      @{ p = 1; hrel = 1; x = $RIGHT;   vrel = 1; y = $TOP;    w = 43 },
      @{ p = 2; hrel = 0; x = $LEFT;    vrel = 0; y = $CENTER; w = 44 },
      @{ p = 2; hrel = 0; x = $CENTER;  vrel = 0; y = $CENTER; w = 45 },
      @{ p = 2; hrel = 0; x = $RIGHT;   vrel = 0; y = $CENTER; w = 46 },
      @{ p = 3; hrel = 1; x = $INSIDE;  vrel = 1; y = $BOTTOM; w = 47 },
      @{ p = 3; hrel = 1; x = $OUTSIDE; vrel = 1; y = $BOTTOM; w = 48 },
      @{ p = 3; hrel = 0; x = $CENTER;  vrel = 0; y = $BOTTOM; w = 49 },
      @{ p = 4; hrel = 5; x = $CENTER;  vrel = 4; y = $CENTER; w = 50 },
      @{ p = 4; hrel = 4; x = $CENTER;  vrel = 5; y = $CENTER; w = 51 }
    )
    $i = 0
    foreach ($f in $o) { $i++; [void](Add-F $f.p @{ w = $f.w; h = 24; wrap = 3; hrel = $f.hrel; x = $f.x; vrel = $f.vrel; y = $f.y; png = $(if ($i % 2) { $RED } else { $BLUE }) }) }
  }
  F-Fixture 'layout-float-poscol' 'positionH relative to column in a two column section: float anchored in the second column, offset and aligned' {
    Init-Doc (Body 30)
    $script:doc.PageSetup.TextColumns.SetCount(2)
    $script:doc.PageSetup.TextColumns.Spacing = 36
    [void](Add-F 1 @{ w = 60; h = 30; wrap = 3; hrel = 2; x = 20; vrel = 1; y = 100 })
    [void](Add-F 1 @{ w = 61; h = 30; wrap = 3; hrel = 2; x = $RIGHT; vrel = 1; y = 200; png = $BLUE })
    [void](Add-F 24 @{ w = 62; h = 30; wrap = 3; hrel = 2; x = 20; vrel = 1; y = 300 })
    [void](Add-F 24 @{ w = 63; h = 30; wrap = 3; hrel = 2; x = $RIGHT; vrel = 1; y = 400; png = $BLUE })
    [void](Add-F 24 @{ w = 64; h = 30; wrap = 0; side = 0; hrel = 2; x = $LEFT; vrel = 2; y = 0; png = $GREEN })
  }

  # ---- collisions, overlap, pagination ---------------------------------------------------------------------------
  F-Fixture 'layout-float-collide' 'two square-wrapped floats whose positions collide: allowOverlap off (pair 1, pair 2) and on (pair 3)' {
    Init-Doc (Body 12)
    [void](Add-F 1 @{ w = 100; h = 60; wrap = 0; side = 0; hrel = 0; x = 100; vrel = 2; y = 0; overlap = $false })
    [void](Add-F 1 @{ w = 110; h = 50; wrap = 0; side = 0; hrel = 0; x = 140; vrel = 2; y = 20; overlap = $false; png = $BLUE })
    [void](Add-F 5 @{ w = 100; h = 61; wrap = 0; side = 0; hrel = 0; x = $LEFT; vrel = 2; y = 0; overlap = $false; png = $GREEN })
    [void](Add-F 5 @{ w = 110; h = 51; wrap = 0; side = 0; hrel = 0; x = $LEFT; vrel = 2; y = 10; overlap = $false })
    [void](Add-F 9 @{ w = 100; h = 62; wrap = 0; side = 0; hrel = 0; x = 100; vrel = 2; y = 0; overlap = $true })
    [void](Add-F 9 @{ w = 110; h = 52; wrap = 0; side = 0; hrel = 0; x = 140; vrel = 2; y = 20; overlap = $true; png = $BLUE })
  }
  F-Fixture 'layout-float-collide-tb' 'collisions between two top-and-bottom floats and between a square and a top-and-bottom float' {
    Init-Doc (Body 9)
    [void](Add-F 1 @{ w = 100; h = 60; wrap = 4; hrel = 0; x = 50;  vrel = 2; y = 0; overlap = $false })
    [void](Add-F 1 @{ w = 110; h = 50; wrap = 4; hrel = 0; x = 100; vrel = 2; y = 10; overlap = $false; png = $BLUE })
    [void](Add-F 5 @{ w = 100; h = 61; wrap = 0; side = 0; hrel = 0; x = 100; vrel = 2; y = 0; overlap = $false; png = $GREEN })
    [void](Add-F 5 @{ w = 120; h = 51; wrap = 4; hrel = 0; x = 140; vrel = 2; y = 20; overlap = $false })
  }
  # Index of the first paragraph that starts on page 2 (minY = 0), or the first one whose first line lies at or below minY points on page 1.

  function Find-Para([double]$minY) {
    $d = $script:doc; $d.Repaginate(); $script:word.ActiveWindow.View.Type = 3
    for ($i = 1; $i -le $d.Paragraphs.Count; $i++) {
      $pr = $d.Paragraphs.Item($i).Range
      $s = $d.Range($pr.Start, $pr.Start); $e = $d.Range($pr.End - 1, $pr.End - 1)
      if ($env:O365_FLOAT_DEBUG) { Write-Host ("para {0}: start page {1} y {2}, end page {3}" -f $i, $d.Range($pr.Start,$pr.Start).Information(3), $d.Range($pr.Start,$pr.Start).Information(6), $d.Range($pr.End-1,$pr.End-1).Information(3)) }
      if ($minY -gt 0) { if ([int]$s.Information(3) -eq 1 -and [double]$s.Information(6) -ge $minY) { return $i } }
      elseif ([int]$s.Information(3) -ge 2) { return $i }
    }
    throw 'no such paragraph'
  }
  F-Fixture 'layout-float-anchor-nextpage' 'anchor paragraph that does not fit in the space left on page 1 (widow and orphan control) starts page 2 instead; floats relative to paragraph, page and margin follow it' {
    Init-Doc (Body 17)
    $d = $script:doc
    $k = Find-Para 0

    $d.Repaginate()
    [void](Add-F $k @{ w = 100; h = 60; wrap = 0; side = 0; hrel = 0; x = $RIGHT; vrel = 2; y = 0 })
    [void](Add-F $k @{ w = 101; h = 40; wrap = 3; hrel = 1; x = 400; vrel = 1; y = 100; png = $BLUE })
    [void](Add-F $k @{ w = 102; h = 40; wrap = 3; hrel = 0; x = 20;  vrel = 0; y = 20; png = $GREEN })
    [void](Add-F $k @{ w = 103; h = 40; wrap = 3; hrel = 0; x = 300; vrel = 2; y = 0 })
  }
  F-Fixture 'layout-float-anchor-bottom' 'floats whose paragraph-relative offset puts them below the bottom margin or off the page: square, top-and-bottom and in front' {
    Init-Doc (Body 17)
    $k = Find-Para 600
    [void](Add-F $k @{ w = 100; h = 60; wrap = 0; side = 0; hrel = 0; x = $RIGHT; vrel = 2; y = 0 })
    [void](Add-F $k @{ w = 101; h = 60; wrap = 3; hrel = 0; x = 200; vrel = 2; y = 80; png = $BLUE })
    [void](Add-F $k @{ w = 102; h = 60; wrap = 4; hrel = 0; x = 100; vrel = 2; y = 120; png = $GREEN })
    [void](Add-F $k @{ w = 103; h = 60; wrap = 0; side = 0; hrel = 0; x = 20; vrel = 2; y = 200 })
  }
  F-Fixture 'layout-float-pagebreak-anchor' 'anchor paragraph with page-break-before: floats relative to paragraph, page, margin, and square wrapping after the break' {
    Init-Doc (Body 6)
    $d = $script:doc
    $d.Paragraphs.Item(4).PageBreakBefore = -1
    [void](Add-F 4 @{ w = 100; h = 60; wrap = 0; side = 0; hrel = 0; x = $LEFT; vrel = 2; y = 0 })
    [void](Add-F 4 @{ w = 101; h = 40; wrap = 3; hrel = 1; x = 300; vrel = 0; y = 20; png = $BLUE })
    [void](Add-F 2 @{ w = 102; h = 40; wrap = 3; hrel = 1; x = 300; vrel = 2; y = 0; png = $GREEN })
  }

  # ---- layoutInCell ------------------------------------------------------------------------------------------------
  function Cell-Doc($compat) {
    $d = $script:doc
    Init-Doc @($T1, 'Cell placement probe follows.', '')
    $r = $d.Paragraphs.Item(3).Range
    $t = $d.Tables.Add($r, 3, 2)
    $t.Borders.Enable = $true
    $t.Columns.Item(1).Width = 200; $t.Columns.Item(2).Width = 200
    # Every cell has its own text: the oracle pairs PDF rows with cell paragraphs by text, so identical cells all map to one offset (no commas: the PDF decodes them as spaces).
    $cellTexts = @(
      'Lanterns hang along the harbour wall and swing gently whenever the evening wind rises from the bay lighting the cobbles below.',
      'Orchard keepers gather windfall apples in wicker baskets and sort bruised fruit from sound before the cider press opens.',
      'Beneath the viaduct a market sells cheese and rye loaves with pickled beetroot and knitted scarves to anyone passing by.',
      'Snow settles quietly on the signal box while the last freight train rumbles north with its brake lamps glowing amber and red.',
      'Pupils in the village school learn to read weather from clouds and note each morning how high and how fast they travel.',
      'Seed catalogues arrive in January and the allotment holders circle their favourites long before the soil is ready to dig.')
    $k = 0
    foreach ($row in 1..3) { foreach ($col in 1..2) { $t.Cell($row, $col).Range.Text = $cellTexts[$k]; $k++ } }
    $t.Rows.Item(1).Height = 80; $t.Rows.Item(2).Height = 80
    $t.Rows.Item(2).Range.Cells.Item(1).Range.Paragraphs.Item(1).Range | Out-Null
    $t
  }
  # The anchor goes after the cell text unless `before` is set: the oracle takes a cell line's story offset from the paragraph's Range.Text,
  # which has no character for a floating anchor, so it adds each shape's Anchor.Start to the character marks (Walk-Table).
  function Add-FCell($t, $row, $col, $o) {
    $r = $t.Cell($row, $col).Range.Paragraphs.Item(1).Range; $r.End = $r.End - 1; $r.Collapse($(if ($o.before) { 1 } else { 0 }))
    $png = if ($o.png) { $o.png } else { $script:RED }
    $sh = $script:doc.InlineShapes.AddPicture($png, $false, $true, $r).ConvertToShape()
    $sh.LockAspectRatio = 0; $sh.Width = [single]$o.w; $sh.Height = [single]$o.h
    Set-Float $sh $o
    $sh
  }
  $cellBuild = {
    $t = Cell-Doc 0
    # row 1: layoutInCell on, row 2: layoutInCell off; each with page / margin / paragraph relative positions and a square-wrapped one
    [void](Add-FCell $t 1 1 @{ w = 40; h = 30; wrap = 3; hrel = 1; x = 10; vrel = 1; y = 120; incell = $true })
    [void](Add-FCell $t 1 2 @{ w = 41; h = 30; wrap = 3; hrel = 0; x = 10; vrel = 2; y = 10; incell = $true; png = $BLUE })
    [void](Add-FCell $t 2 1 @{ w = 42; h = 30; wrap = 3; hrel = 1; x = 10; vrel = 1; y = 280; incell = $false; png = $GREEN })
    [void](Add-FCell $t 2 2 @{ w = 43; h = 30; wrap = 3; hrel = 0; x = 10; vrel = 2; y = 10; incell = $false })
    [void](Add-FCell $t 3 1 @{ w = 44; h = 30; wrap = 0; side = 0; hrel = 0; x = $RIGHT; vrel = 2; y = 0; incell = $true; png = $BLUE })
    [void](Add-FCell $t 3 2 @{ w = 45; h = 30; wrap = 0; side = 0; hrel = 0; x = $RIGHT; vrel = 2; y = 0; incell = $false; png = $GREEN })
  }
  # The same floats anchored at the start of the cell text (a second anchor in cell 1,1 after the first): line offsets count the anchor characters.
  $cellBeforeBuild = {
    $t = Cell-Doc 0
    [void](Add-FCell $t 1 1 @{ w = 40; h = 30; wrap = 3; hrel = 1; x = 10; vrel = 1; y = 120; incell = $true; before = $true })
    [void](Add-FCell $t 1 1 @{ w = 20; h = 20; wrap = 3; hrel = 0; x = 60; vrel = 2; y = 40; incell = $true; png = $BLUE; before = $true })
    [void](Add-FCell $t 1 2 @{ w = 41; h = 30; wrap = 3; hrel = 0; x = 10; vrel = 2; y = 10; incell = $true; png = $BLUE; before = $true })
    [void](Add-FCell $t 2 1 @{ w = 42; h = 30; wrap = 3; hrel = 1; x = 10; vrel = 1; y = 280; incell = $false; png = $GREEN; before = $true })
    [void](Add-FCell $t 3 1 @{ w = 44; h = 30; wrap = 0; side = 0; hrel = 0; x = $RIGHT; vrel = 2; y = 0; incell = $true; png = $BLUE; before = $true })
  }
  F-Fixture 'layout-float-incell-before' 'floats anchored at the start of table cell text, layoutInCell on and off, current compatibility mode' $cellBeforeBuild
  F-Fixture 'layout-float-incell' 'floats anchored in table cells, layoutInCell on and off, current compatibility mode' $cellBuild
  F-Fixture 'layout-float-incell-compat14' 'floats anchored in table cells, layoutInCell on and off, Word 2010 compatibility mode (14)' $cellBuild 14

  F-Fixture 'layout-float-collide-compat14' 'colliding floats in Word 2010 compatibility mode (14): allowOverlap off and on' {
    Init-Doc (Body 9)
    [void](Add-F 1 @{ w = 100; h = 60; wrap = 0; side = 0; hrel = 0; x = 100; vrel = 2; y = 0; overlap = $false })
    [void](Add-F 1 @{ w = 110; h = 50; wrap = 0; side = 0; hrel = 0; x = 140; vrel = 2; y = 20; overlap = $false; png = $BLUE })
    [void](Add-F 5 @{ w = 100; h = 62; wrap = 0; side = 0; hrel = 0; x = 100; vrel = 2; y = 0; overlap = $true })
    [void](Add-F 5 @{ w = 110; h = 52; wrap = 0; side = 0; hrel = 0; x = 140; vrel = 2; y = 20; overlap = $true; png = $BLUE })
  } 14

} finally {
  if ($word) {
    try { $word.UserName = $oldUser; $word.UserInitials = $oldInit; $word.Options.UseLocalUserInfo = $oldLocal } catch {}
    try { $word.Quit(0) } catch {}; Release $word
  }
  if (Test-Path $imgDir) { Remove-Item $imgDir -Recurse -Force -ErrorAction SilentlyContinue }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($p in $wordPids) { Get-Process -Id $p -ErrorAction SilentlyContinue | Stop-Process -Force }
}
Write-Output "created: $($created -join ', ')"
if ($skipped.Count) { Write-Output 'SKIPPED:'; $skipped | ForEach-Object { Write-Output "  $_" }; exit 1 }
