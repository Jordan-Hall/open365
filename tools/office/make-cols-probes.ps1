<#
Generates the section-column probes (fixtures/docx/layout-cols-*.docx + .toml, the directory every parity test reads)
by driving the locally installed Microsoft Word over COM. Each probe is Letter paper, 72 pt margins, Calibri 11,
single spacing, 6 pt after, with generic text and one or more sections whose column count, spacing, separator line,
unequal widths, breaks and section types are set through the object model. The reference JSON (every line's page,
offset and exact baseline/x) comes from tools/office/word-layout-oracle.ps1:

  pwsh -NoProfile -File tools/office/make-cols-probes.ps1
  pwsh -NoProfile -File tools/office/word-layout-oracle.ps1 -Path fixtures/docx/layout-cols-two.docx   # one per probe

LOCAL DEVELOPER TOOL ONLY (Microsoft does not support unattended server-side Office automation).
Starts its own Word instance (AutomationSecurity = force-disable macros), never attaches to or kills another one,
quits it in a finally block, and stops only the WINWORD processes it started. Document author and properties are
generic. All text is original. Nothing outside the repository is read or written (the Word user name is restored).
Constants: WdBreakType column 8, page 7, section next page 2, continuous 3, even page 4, odd page 5; WdSectionStart
continuous 0, new column 1, new page 2, even page 3, odd page 4.
#>
param([string[]]$Only = @())   # fixture names to (re)create; default: all column probes
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$outDir = Join-Path $root 'fixtures\docx'
New-Item -ItemType Directory -Force $outDir | Out-Null
$created = New-Object System.Collections.Generic.List[string]
$skipped = New-Object System.Collections.Generic.List[string]
function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }

$A = 'The quick survey of a quiet harbour town begins at dawn, when the fishing boats drift back past the lighthouse and the market stalls fold open their striped awnings. Gulls wheel over the quay while merchants argue cheerfully about the price of mackerel, and a baker carries warm loaves across the cobbles to the cafe on the corner. By noon the visitors arrive, and the narrow streets fill with the sound of voices, bicycle bells and distant music.'
$B = 'Typesetting depends on small decisions repeated thousands of times: where a line ends, how much space follows a paragraph, and which words travel together across a page boundary. A faithful reproduction must make exactly the same decisions, because one different break early in a document moves every line that follows it onto a different page.'
$C = 'In the workshop the lamps stay lit late. Shavings gather under the bench, a kettle ticks as it cools, and the half-finished chair waits for its last joint. The maker steps back, squints along the rail and decides that tomorrow will do for the glue.'
$S1 = 'A short note of two lines, no more than that.'
$S2 = 'Another brief remark.'
$S3 = 'The third item is a little longer than the others so that it needs a second line in a narrow column.'
$S4 = 'Fourth.'
$S5 = 'A fifth paragraph that is about as long as the third one, give or take a few words in the middle.'
$H1 = 'Chapter heading'

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

  # Calibri 11, single spacing, 0 before / 6 after, Letter, 72 pt margins.
  function Init-Doc() {
    $d = $script:doc
    $n = $d.Styles.Item('Normal')
    $n.Font.Name = 'Calibri'; $n.Font.Size = 11
    $n.ParagraphFormat.SpaceBefore = 0; $n.ParagraphFormat.SpaceAfter = 6; $n.ParagraphFormat.LineSpacingRule = 0
    $ps = $d.PageSetup
    $ps.PageWidth = 612; $ps.PageHeight = 792
    $ps.TopMargin = 72; $ps.BottomMargin = 72; $ps.LeftMargin = 72; $ps.RightMargin = 72
  }
  # A paragraph (mark included); $fmt receives the paragraph's Format, $font its Font.
  function P($text, [scriptblock]$fmt = $null, [scriptblock]$font = $null) {
    $s = $script:word.Selection; [void]$s.EndKey(6)
    $s.ParagraphFormat.Reset(); $s.Font.Reset()
    $s.TypeText($text)
    $p = $s.Paragraphs.Item(1)
    if ($fmt) { & $fmt $p.Format }
    if ($font) { & $font $p.Range.Font }
    $s.TypeParagraph()
    $s.ParagraphFormat.Reset(); $s.Font.Reset()
  }
  # A paragraph without its mark: the break inserted next ends it (a section break replaces the mark).
  function PE($text, [scriptblock]$fmt = $null) {
    $s = $script:word.Selection; [void]$s.EndKey(6)
    $s.ParagraphFormat.Reset(); $s.Font.Reset()
    $s.TypeText($text)
    if ($fmt) { & $fmt $s.Paragraphs.Item(1).Format }
  }
  function Brk($kind) { $s = $script:word.Selection; [void]$s.EndKey(6); $s.InsertBreak($kind); $s.ParagraphFormat.Reset(); $s.Font.Reset() }
  # Columns of section $i (1-based): count, spacing, separator line, optional unequal widths and spaces.
  function Cols($i, $n, $spacing = 36, $line = $false, $widths = $null, $spaces = $null) {
    $tc = $script:doc.Sections.Item($i).PageSetup.TextColumns
    $tc.SetCount($n)
    if ($widths) {
      $tc.EvenlySpaced = 0
      # Spaces first, then the widths from narrow to wide, so that the columns never overflow the text width.
      for ($k = 1; $k -lt $n; $k++) { $tc.Item($k).SpaceAfter = [single]$spaces[$k - 1] }
      foreach ($k in (1..$n | Sort-Object { $widths[$_ - 1] })) { $tc.Item($k).Width = [single]$widths[$k - 1] }
    } elseif ($n -gt 1) {
      $tc.EvenlySpaced = -1
      $tc.Spacing = [single]$spacing
    }
    $tc.LineBetween = [int]([bool]$line) * -1
  }
  function Paras($texts) { foreach ($t in $texts) { P $t } }

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
      Set-Content -Path (Join-Path $script:outDir ("$name.toml")) -Value ("feature = `"layout-cols`"
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

  # ---- equal columns -------------------------------------------------------------------------------------------
  C-Fixture 'layout-cols-two' 'two equal columns, 36 pt apart: text fills the first column to the bottom margin, then the second, then the next page' {
    for ($i = 0; $i -lt 14; $i++) { P @($A, $B, $C)[$i % 3] }
    PE $A
    Cols 1 2 36
  }
  C-Fixture 'layout-cols-three' 'three equal columns, 18 pt apart, left aligned, with a keep-with-next heading and justified paragraphs' {
    for ($i = 0; $i -lt 8; $i++) {
      P $H1 { param($f) $f.KeepWithNext = -1; $f.SpaceBefore = 12 } { param($f) $f.Bold = 1; $f.Size = 14 }
      P @($A, $B, $C)[$i % 3] { param($f) $f.Alignment = 3 }
      P @($B, $C, $A)[$i % 3]
    }
    PE $C
    Cols 1 3 18
  }
  C-Fixture 'layout-cols-line' 'two columns 54 pt apart with a separator line, justified text' {
    for ($i = 0; $i -lt 12; $i++) { P @($A, $B, $C)[$i % 3] { param($f) $f.Alignment = 3 } }
    PE $B { param($f) $f.Alignment = 3 }
    Cols 1 2 54 $true
  }
  C-Fixture 'layout-cols-four' 'four equal columns, 12 pt apart, narrow measure with many short lines' {
    for ($i = 0; $i -lt 16; $i++) { P @($A, $B, $C)[$i % 3] }
    PE $A
    Cols 1 4 12
  }
  C-Fixture 'layout-cols-unequal' 'three columns of unequal width (120, 200, 100 pt) with different gaps (24, 24 pt)' {
    for ($i = 0; $i -lt 12; $i++) { P @($A, $B, $C)[$i % 3] }
    PE $A
    Cols 1 3 0 $true @(120, 200, 100) @(24, 24)
  }
  C-Fixture 'layout-cols-unequal-two' 'two columns, narrow then wide (144 and 288 pt, 36 pt gap), indents and alignment per paragraph' {
    for ($i = 0; $i -lt 8; $i++) {
      P $A { param($f) $f.LeftIndent = 12 }
      P $B { param($f) $f.Alignment = 3; $f.RightIndent = 6 }
      P $C { param($f) $f.FirstLineIndent = 18 }
    }
    PE $B
    Cols 1 2 0 $false @(144, 288) @(36)
  }

  # ---- breaks --------------------------------------------------------------------------------------------------
  C-Fixture 'layout-cols-break' 'two columns with column breaks: after a paragraph (in its own paragraph), inside a paragraph, and at the start of a paragraph' {
    P $A
    P $S1
    Brk 8
    P $B
    P $C
    P ($S2 + ' ')
    $s = $word.Selection; [void]$s.EndKey(6); $s.MoveLeft(1, 1) | Out-Null
    Brk 8
    $s = $word.Selection; [void]$s.EndKey(6)
    PE ($A.Substring(0, 150))
    Brk 8
    P ($A.Substring(150))
    P $S3
    P $B
    P $C
    P $A
    P $B
    PE $C
    Cols 1 2 36
  }
  C-Fixture 'layout-cols-pagebreak' 'two columns with a page break after the first column starts and a page break inside the second page' {
    P $A
    P $B
    Brk 7
    P $C
    P $A
    P $B
    Brk 7
    P $C
    PE $A
    Cols 1 2 36
  }
  C-Fixture 'layout-cols-pagebefore' 'two columns, a page-break-before paragraph and a keep-lines paragraph that does not fit the column bottom' {
    for ($i = 0; $i -lt 5; $i++) { P @($A, $B, $C)[$i % 3] }
    P $A { param($f) $f.PageBreakBefore = -1 }
    for ($i = 0; $i -lt 6; $i++) { P @($B, $C, $A)[$i % 3] }
    P ($A + ' ' + $B) { param($f) $f.KeepTogether = -1 }
    for ($i = 0; $i -lt 8; $i++) { P @($C, $A, $B)[$i % 3] }
    P ($A + ' ' + $B) { param($f) $f.KeepTogether = -1 }
    for ($i = 0; $i -lt 6; $i++) { P @($B, $A, $C)[$i % 3] }
    PE $A
    Cols 1 2 36
  }

  # ---- section types and balancing -----------------------------------------------------------------------------
  C-Fixture 'layout-cols-continuous' 'single column intro, continuous break to two columns (balanced by the next continuous break), single column, then three columns to the end (not balanced)' {
    P $H1 $null { param($f) $f.Bold = 1; $f.Size = 16 }
    P $A
    PE $S1
    Brk 3
    P $B
    P $C
    P $S2
    P $A
    PE $S3
    Brk 3
    P $B
    PE $S1
    Brk 3
    P $A
    P $C
    P $S4
    PE $B
    Cols 1 1
    Cols 2 2 36
    Cols 3 1
    Cols 4 3 18
  }
  C-Fixture 'layout-cols-balance' 'balancing at a continuous break: short sections of 2 to 7 paragraphs in two and three columns, paragraphs of different line counts' {
    P $S1
    PE $S2
    Brk 3
    Paras @($S1, $S2, $S3)
    PE $S4
    Brk 3
    Paras @($S3, $S1, $S2, $S5, $S4)
    PE $S1
    Brk 3
    Paras @($B, $S1)
    PE $S2
    Brk 3
    Paras @($S1, $S2, $S3, $S4, $S5, $S2, $S1)
    PE $S3
    Brk 3
    P $S5
    PE $S4
    Cols 1 1
    Cols 2 2 36
    Cols 3 3 18
    Cols 4 2 36
    Cols 5 3 18
    Cols 6 1
  }
  C-Fixture 'layout-cols-balance-pages' 'a two column section that runs over two pages and ends with a continuous break: only the last page is balanced' {
    for ($i = 0; $i -lt 11; $i++) { P @($A, $B, $C)[$i % 3] }
    PE $A
    Brk 3
    P $S1
    P $S2
    PE $S3
    Cols 1 2 36
    Cols 2 1
  }
  C-Fixture 'layout-cols-nextpage' 'columns before and after a next-page section break (no balancing), then an odd-page break, then a continuous change back to one column' {
    Paras @($A, $B, $S1)
    PE $S2
    Brk 2
    Paras @($C, $S3, $A)
    PE $S4
    Brk 5
    Paras @($B, $S5)
    PE $S1
    Brk 3
    P $C
    PE $A
    Cols 1 2 36
    Cols 2 3 18
    Cols 3 2 36
    Cols 4 1
  }
  C-Fixture 'layout-cols-mid' 'a continuous section with two columns that starts low on the page and runs onto the next page, then a three column continuous section' {
    for ($i = 0; $i -lt 9; $i++) { P @($A, $B, $C)[$i % 3] }
    PE $S1
    Brk 3
    for ($i = 0; $i -lt 22; $i++) { P @($B, $C, $A)[$i % 3] }
    PE $S2
    Brk 3
    for ($i = 0; $i -lt 6; $i++) { P @($S3, $S1, $S5)[$i % 3] }
    PE $S4
    Cols 1 1
    Cols 2 2 36
    Cols 3 3 18
  }
  C-Fixture 'layout-cols-newcol' 'a section that starts in the next column (w:type nextColumn) after a two column section' {
    Paras @($A, $S1)
    PE $S2
    Brk 3
    Paras @($B, $C)
    PE $S3
    Cols 1 2 36
    Cols 2 2 36
    $doc.Sections.Item(2).PageSetup.SectionStart = 1
  }

  # ---- other content in columns --------------------------------------------------------------------------------
  C-Fixture 'layout-cols-spacing' 'two columns with space before and after, exact and 1.5 line spacing, and a centred heading: carry-over of space at a column top' {
    for ($i = 0; $i -lt 6; $i++) {
      P $H1 { param($f) $f.Alignment = 1; $f.SpaceBefore = 18; $f.SpaceAfter = 12 } { param($f) $f.Bold = 1; $f.Size = 16 }
      P $A { param($f) $f.SpaceBefore = 10; $f.SpaceAfter = 4 }
      P $B { param($f) $f.LineSpacingRule = 4; $f.LineSpacing = 16 }
      P $C { param($f) $f.LineSpacingRule = 5; $f.LineSpacing = 18 }
    }
    PE $A
    Cols 1 2 36
  }
  C-Fixture 'layout-cols-table' 'a table in a two column section: it takes the column width and splits across columns; a heading above spans nothing' {
    P $A
    $s = $word.Selection; [void]$s.EndKey(6)
    $t = $doc.Tables.Add($s.Range, 12, 2)
    $t.Borders.Enable = 1
    for ($r = 1; $r -le 12; $r++) {
      $t.Cell($r, 1).Range.Text = "row $r left cell holds a short note of two lines"
      $t.Cell($r, 2).Range.Text = "R$r"
    }
    [void]$s.EndKey(6)
    P $B
    P $C
    PE $A
    Cols 1 2 36
  }
  C-Fixture 'layout-cols-footnote' 'a two column section with footnotes: where the notes sit' {
    for ($i = 0; $i -lt 6; $i++) {
      P @($A, $B, $C)[$i % 3]
      $s = $word.Selection; [void]$s.EndKey(6); $s.MoveLeft(1, 1) | Out-Null
      [void]$doc.Footnotes.Add($s.Range, [Type]::Missing, "Note $($i + 1): a short remark.")
      [void]$word.Selection.EndKey(6)
    }
    PE $A
    Cols 1 2 36
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
