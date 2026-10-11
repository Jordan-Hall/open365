<#
Generates the floating table (w:tblpPr) and frame (w:framePr) probes (fixtures/docx/layout-tblp-*.docx and layout-frame-*.docx + .toml, the directory every parity test reads)
by driving the locally installed Microsoft Word over COM. Each probe is Letter paper, 72 pt margins, Calibri 11,
single spacing, no space before or after, with generic text, hyperlinks and anchored objects built through the object model. The reference JSON (every line's page, offset, exact
baseline/x and every cell rectangle) comes from tools/office/word-layout-oracle.ps1:

  pwsh -NoProfile -File tools/office/make-frame-probes.ps1
  pwsh -NoProfile -File tools/office/word-layout-oracle.ps1 -Path fixtures/docx/layout-frame-hyperlink.docx   # one per probe

LOCAL DEVELOPER TOOL ONLY (Microsoft does not support unattended server-side Office automation).
Starts its own Word instance (AutomationSecurity = force-disable macros), never attaches to or kills another one,
quits it in a finally block, and stops only the WINWORD processes it started. Document author and properties are
generic. All text is original. Nothing outside the repository is read or written (the Word user name is restored).
Constants: WdRowHeightRule auto 0, at least 1, exactly 2; WdCellVerticalAlignment top 0, center 1, bottom 3; WdBreakType page 7.
#>
param([string[]]$Only = @())   # fixture names to (re)create; default: all anchor probes
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$outDir = Join-Path $root 'fixtures\docx'
New-Item -ItemType Directory -Force $outDir | Out-Null
$created = New-Object System.Collections.Generic.List[string]
$skipped = New-Object System.Collections.Generic.List[string]
function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }

$wordBefore = Get-Pids 'WINWORD'
$word = $null; $oldUser = $null; $oldInit = $null; $oldLocal = $null; $wordPids = @()
try {
  $word = New-Object -ComObject Word.Application
  $wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $wordBefore })
  if ($wordPids.Count -eq 0) { throw 'COM attached to a Word instance this script did not start; refusing to touch it' }
  $word.Visible = $false; $word.DisplayAlerts = 0
  $word.AutomationSecurity = 3   # msoAutomationSecurityForceDisable
  $oldUser = $word.UserName; $oldInit = $word.UserInitials; $oldLocal = $word.Options.UseLocalUserInfo
  $word.Options.UseLocalUserInfo = $true
  $word.UserName = 'project member'; $word.UserInitials = 'PM'

  # Calibri 11, single spacing, 0 before / 0 after, Letter, 72 pt margins.
  function Init-Doc() {
    $d = $script:doc
    $n = $d.Styles.Item('Normal')
    $n.Font.Name = 'Calibri'; $n.Font.Size = 11
    $n.ParagraphFormat.SpaceBefore = 0; $n.ParagraphFormat.SpaceAfter = 0; $n.ParagraphFormat.LineSpacingRule = 0
    $ps = $d.PageSetup
    $ps.PageWidth = 612; $ps.PageHeight = 792
    $ps.TopMargin = 72; $ps.BottomMargin = 72; $ps.LeftMargin = 72; $ps.RightMargin = 72
  }
  function P($text) {
    $s = $script:word.Selection; [void]$s.EndKey(6)
    $s.TypeText($text)
    $s.TypeParagraph()
  }
  function Brk($kind) { $s = $script:word.Selection; [void]$s.EndKey(6); $s.InsertBreak($kind) }
  # A table of $rows x 2 cells at the end of the document; cell text "<tag>r<row>c<col>", plain borders.
  # $spec: one hashtable per row: Rule (0 auto, 1 at least, 2 exactly), Height (pt), Lines (text lines in the first
  # cell), Valign (0/1/3).
  function T($tag, $pad, $spec, $padBottom = $null) {
    $s = $script:word.Selection; [void]$s.EndKey(6)
    $t = $script:doc.Tables.Add($s.Range, $spec.Count, 2)
    $t.Borders.Enable = 0
    $t.TopPadding = [single]$pad
    $t.BottomPadding = [single]$(if ($null -ne $padBottom) { $padBottom } else { $pad })
    for ($r = 1; $r -le $spec.Count; $r++) {
      $sp = $spec[$r - 1]
      $lines = if ($sp.Lines) { $sp.Lines } else { 1 }
      $txt = "$($tag)r$($r)c1"
      for ($k = 2; $k -le $lines; $k++) { $txt += "`vline$k" }
      $t.Cell($r, 1).Range.Text = $txt
      $t.Cell($r, 2).Range.Text = "$($tag)r$($r)c2"
      if ($sp.Rule) { $t.Rows.Item($r).HeightRule = [int]$sp.Rule; $t.Rows.Item($r).Height = [single]$sp.Height }
      if ($sp.Valign) { $t.Cell($r, 1).VerticalAlignment = [int]$sp.Valign }
    }
    [void]$s.EndKey(6)
    return $t
  }

  function C-Fixture($name, $notes, [scriptblock]$build) {
    $doc = $null; $path = Join-Path $script:outDir "$name.docx"
    if ($script:Only.Count -gt 0 -and $name -notin $script:Only) { return }
    try {
      if (Test-Path $path) { Remove-Item $path -Force }
      $doc = $script:word.Documents.Add()
      $script:doc = $doc
      Init-Doc
      & $build
      $doc.RemovePersonalInformation = $true
      $doc.SaveAs2($path, 16)
      Set-Content -Path (Join-Path $script:outDir ("$name.toml")) -Value ("feature = `"layout-frame`"
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


  # Body text: original filler lines, long enough to wrap beside a 200 pt wide object.
  $TXT = @(
    'The harbour road climbs past the old salt warehouses before it bends toward the quiet market square where the stalls open early.',
    'Children carry baskets of bread while the bakers stack trays of rolls beside the fountain and count the morning takings aloud.',
    'A narrow bridge crosses the stream behind the chapel, and travellers pause there to watch the water turn the slow wooden wheel.',
    'By noon the square fills with voices, and the clock above the arcade strikes the hour a little later than the church bell does.'
  )
  function Body($n) { for ($i = 0; $i -lt $n; $i++) { P ("L$($i + 1) " + $TXT[$i % 4]) } }
  # A floating table of 2 rows x 2 cells at the start of paragraph $pi (all dimensions in points).
  function TblFloat($pi, $o) {
    $r = $script:doc.Paragraphs.Item($pi).Range; $r.Collapse(1)
    $t = $script:doc.Tables.Add($r, 2, 2)
    $t.Borders.Enable = -1
    $t.AllowAutoFit = $false
    $t.Columns.Item(1).Width = [single]$o.cw; $t.Columns.Item(2).Width = [single]$o.cw
    $t.TopPadding = 0; $t.BottomPadding = 0
    for ($i = 1; $i -le 2; $i++) { for ($j = 1; $j -le 2; $j++) { $t.Cell($i, $j).Range.Text = "$($o.tag)r$($i)c$($j)" } }
    $rw = $t.Rows
    $rw.WrapAroundText = -1
    $rw.RelativeHorizontalPosition = $o.hrel; $rw.HorizontalPosition = [single]$o.x
    $rw.RelativeVerticalPosition = $o.vrel; $rw.VerticalPosition = [single]$o.y
    if ($o.dist) { $rw.DistanceTop = $o.dist[0]; $rw.DistanceBottom = $o.dist[1]; $rw.DistanceLeft = $o.dist[2]; $rw.DistanceRight = $o.dist[3] }
    $rw.AllowOverlap = 0
    $t
  }
  # A frame around paragraph $pi.
  function FR($pi, $o) {
    $r = $script:doc.Paragraphs.Item($pi).Range
    $f = $script:doc.Frames.Add($r)
    $f.WidthRule = 1; $f.Width = [single]$o.w
    if ($o.h) { $f.HeightRule = $(if ($o.hrule) { $o.hrule } else { 1 }); $f.Height = [single]$o.h }
    $f.TextWrap = -1
    $f.RelativeHorizontalPosition = $o.hrel; $f.HorizontalPosition = [single]$o.x
    $f.RelativeVerticalPosition = $o.vrel; $f.VerticalPosition = [single]$o.y
    if ($o.dist) { $f.HorizontalDistanceFromText = [single]$o.dist[0]; $f.VerticalDistanceFromText = [single]$o.dist[1] }
    $f
  }

  # WdRelativeHorizontalPosition: margin 0, page 1, column 2. WdRelativeVerticalPosition: margin 0, page 1, paragraph 2.
  # WdFramePosition left -999998, right -999996, center -999995 (also WdFramePosition for frames).
  C-Fixture 'layout-tblp-basic' 'a 2 x 2 floating table of two 100 pt columns, left at the column and 0 pt below the top of paragraph 3, 9 pt text distance on the sides: how the text of paragraphs 3 to 9 wraps beside it' {
    Body 12
    [void](TblFloat 3 @{ tag = 'ta'; cw = 100; x = 0; hrel = 2; y = 0; vrel = 2; dist = @(0, 0, 9, 9) })
  }
  C-Fixture 'layout-tblp-right' 'the same table aligned to the right margin and 20 pt below the paragraph top, with 0 and 20 pt distances' {
    Body 12
    [void](TblFloat 3 @{ tag = 'tb'; cw = 100; x = -999996; hrel = 0; y = 20; vrel = 2; dist = @(6, 6, 20, 0) })
  }
  C-Fixture 'layout-tblp-page' 'a table positioned 150 pt from the left page edge and 300 pt from the page top (anchor in paragraph 2), and a second centred one in paragraph 8 at 40 pt from the paragraph top' {
    Body 16
    [void](TblFloat 2 @{ tag = 'tc'; cw = 90; x = 150; hrel = 1; y = 300; vrel = 1; dist = @(0, 0, 9, 9) })
    [void](TblFloat 8 @{ tag = 'td'; cw = 80; x = -999995; hrel = 2; y = 40; vrel = 2; dist = @(0, 0, 9, 9) })
  }
  C-Fixture 'layout-frame-basic' 'a framed paragraph 150 pt wide, left of the column, top aligned with paragraph 3, text wrapping beside it; and a second frame on the right margin in paragraph 9' {
    Body 14
    [void](FR 3 @{ w = 150; x = 0; hrel = 2; y = 0; vrel = 2; dist = @(9, 0) })
    [void](FR 9 @{ w = 120; x = -999996; hrel = 0; y = 0; vrel = 2; dist = @(14, 0) })
  }
  C-Fixture 'layout-frame-page' 'a framed paragraph at 100 pt from the page left and 250 pt from the page top (exact height 60 pt), another one centred and 30 pt below the paragraph top, with distances' {
    Body 14
    [void](FR 2 @{ w = 140; x = 100; hrel = 1; y = 250; vrel = 1; h = 60; hrule = 2; dist = @(9, 4) })
    [void](FR 8 @{ w = 130; x = -999995; hrel = 2; y = 30; vrel = 2; dist = @(9, 0) })
  }
  C-Fixture 'layout-frame-dropcap' 'three paragraphs of body text, the first letter of the first and the third as a dropped capital over 3 and 2 lines' {
    Body 6
    $d = $doc.Paragraphs.Item(1).DropCap; $d.Enable(); $d.Position = 1; $d.LinesToDrop = 3
    $d = $doc.Paragraphs.Item(4).DropCap; $d.Enable(); $d.Position = 1; $d.LinesToDrop = 2
  }

} finally {
  if ($word) {
    try { $word.UserName = $oldUser; $word.UserInitials = $oldInit; $word.Options.UseLocalUserInfo = $oldLocal } catch {}
    try { $word.Quit(0) } catch {}; Release $word
  }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($p in $wordPids) { Get-Process -Id $p -ErrorAction SilentlyContinue | Stop-Process -Force }
}
Write-Output "created: $($created -join ', ')"
if ($skipped.Count) { Write-Output 'SKIPPED:'; $skipped | ForEach-Object { Write-Output "  $_" }; exit 1 }
