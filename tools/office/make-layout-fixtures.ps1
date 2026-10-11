<#
Generates the Word layout-parity fixtures (fixtures/docx/layout-*.docx + .toml) by driving the locally
installed Microsoft Word over COM. Same conventions as make-fixtures.ps1 (RemovePersonalInformation,
manifest format, idempotent, a failing fixture is skipped and reported) but only touches the layout-*
files, so it can be re-run without regenerating every other fixture.

LOCAL DEVELOPER TOOL ONLY (Microsoft does not support unattended server-side Office automation).

All fixtures use only Calibri and Cambria (bundled metric-compatible fallbacks: Carlito, Caladea),
Letter paper, 1 inch margins, so page geometry is the same on every machine and locale.
All text is original.
#>
param([string[]]$Only = @())   # fixture names to (re)create; default: all layout fixtures
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$docxDir = Join-Path $root 'fixtures\docx'
New-Item -ItemType Directory -Force $docxDir | Out-Null
$created = New-Object System.Collections.Generic.List[string]
$skipped = New-Object System.Collections.Generic.List[string]
$M = [Type]::Missing
$VT = [string][char]11   # vertical tab = manual line break in Word text

function Write-Manifest($dir, $name, $feature, $app, $notes) {
  $made = "$($app.Name) $($app.Build) (Windows)"
  $t = "feature = `"$feature`"`nmade_with = `"$made`"`nauthor = `"project member`"`nexpect_dropped = []`nnotes = `"$notes`"`n"
  Set-Content -Path (Join-Path $dir "$name.toml") -Value $t -Encoding utf8 -NoNewline
}
function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }

$A = 'The quick survey of a quiet harbour town begins at dawn, when the fishing boats drift back past the lighthouse and the market stalls fold open their striped awnings. Gulls wheel over the quay while merchants argue cheerfully about the price of mackerel, and a baker carries warm loaves across the cobbles to the cafe on the corner. By noon the visitors arrive, and the narrow streets fill with the sound of voices, bicycle bells and distant music.'
$B = 'Typesetting depends on small decisions repeated thousands of times: where a line ends, how much space follows a paragraph, and which words travel together across a page boundary. A faithful reproduction must make exactly the same decisions, because one different break early in a document moves every line that follows it onto a different page.'

$wordBefore = Get-Pids 'WINWORD'
$word = $null; $oldUser = $null; $oldInit = $null; $oldLocal = $null; $wordPids = @()
try {
  $word = New-Object -ComObject Word.Application
  $word.Visible = $false; $word.DisplayAlerts = 0
  $oldUser = $word.UserName; $oldInit = $word.UserInitials; $oldLocal = $word.Options.UseLocalUserInfo
  $word.Options.UseLocalUserInfo = $true
  $word.UserName = 'project member'; $word.UserInitials = 'PM'
  $wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $wordBefore })

  # Start every fixture from a known base: Calibri 11, single spacing, 0 before / 0 after, Letter, 72pt margins.
  function Init-Doc($spaceAfter) {
    $d = $script:doc
    $n = $d.Styles.Item('Normal')
    $n.Font.Name = 'Calibri'; $n.Font.Size = 11
    $n.ParagraphFormat.SpaceBefore = 0; $n.ParagraphFormat.SpaceAfter = $spaceAfter
    $n.ParagraphFormat.LineSpacingRule = 0
    $ps = $d.PageSetup
    $ps.PageWidth = 612; $ps.PageHeight = 792
    $ps.TopMargin = 72; $ps.BottomMargin = 72; $ps.LeftMargin = 72; $ps.RightMargin = 72
  }
  # Append a paragraph; $fmt (optional) receives the paragraph's Format object; $font (optional) its Font.
  function L-Para($text, [scriptblock]$fmt = $null, [scriptblock]$font = $null) {
    $s = $script:word.Selection; [void]$s.EndKey(6)
    $s.ParagraphFormat.Reset(); $s.Font.Reset()
    $s.TypeText($text)
    $p = $s.Paragraphs.Item(1)
    if ($fmt) { & $fmt $p.Format }
    if ($font) { & $font $p.Range.Font }
    $s.TypeParagraph()
    $s.ParagraphFormat.Reset(); $s.Font.Reset()
  }
  # Word's own list label (Range.ListFormat.ListString) per paragraph, for fixtures that have lists:
  # fixtures/reference/layout/<name>.lists.json, read by crates/layout/tests/list_labels.rs.
  function Write-ListLabels($d, $name) {
    if ($d.ListParagraphs.Count -eq 0) { return }
    $refDir = Join-Path $script:root 'fixtures\reference\layout'
    $rows = @()
    $i = 0
    foreach ($par in $d.Paragraphs) {
      $lf = $par.Range.ListFormat
      $rows += [ordered]@{ paragraph = $i; level = [int]$lf.ListLevelNumber; label = [string]$lf.ListString }
      $i++
    }
    $json = ConvertTo-Json -InputObject ([ordered]@{ source = "fixtures/docx/$name.docx"; oracle = 'Word Range.ListFormat.ListString'; paragraphs = $rows }) -Depth 4
    [IO.File]::WriteAllText((Join-Path $refDir "$name.lists.json"), $json, (New-Object Text.UTF8Encoding($false)))
  }
  function L-Fixture($name, $feature, $notes, [scriptblock]$build) {
    $doc = $null; $path = Join-Path $docxDir "$name.docx"
    if ($script:Only.Count -gt 0 -and $name -notin $script:Only) { return }
    try {
      if (Test-Path $path) { Remove-Item $path -Force }
      $doc = $script:word.Documents.Add()
      $script:doc = $doc
      & $build
      $doc.RemovePersonalInformation = $true
      Write-ListLabels $doc $name
      $doc.SaveAs2($path, 16)
      Write-Manifest $docxDir $name $feature $script:word $notes
      $script:created.Add("docx/$name")
    } catch {
      $script:skipped.Add("docx/$name : $($_.Exception.Message) @ $($_.InvocationInfo.PositionMessage -replace '\s+', ' ')")
      Remove-Item $path, (Join-Path $docxDir "$name.toml") -Force -ErrorAction SilentlyContinue
    } finally {
      if ($doc) { try { $doc.Close(0) } catch {}; Release $doc }
      $script:doc = $null
    }
  }

  L-Fixture 'layout-wrap' 'layout-wrap' 'long wrapping paragraphs: Calibri 11 and Cambria 12, left aligned, 6pt after' {
    Init-Doc 6
    L-Para $A
    L-Para $B
    L-Para ($A + ' ' + $B) $null { param($f) $f.Name = 'Cambria'; $f.Size = 12 }
    L-Para $B $null { param($f) $f.Name = 'Cambria'; $f.Size = 12 }
    L-Para $A $null { param($f) $f.Name = 'Cambria'; $f.Size = 12; $f.Bold = 1 }
    L-Para $B $null { param($f) $f.Size = 14; $f.Italic = 1 }
  }

  L-Fixture 'layout-align' 'layout-align' 'left, justified, centred, right and justified-with-manual-break paragraphs' {
    Init-Doc 6
    L-Para $A { param($f) $f.Alignment = 3 }
    L-Para $B { param($f) $f.Alignment = 3 } { param($f) $f.Name = 'Cambria'; $f.Size = 12 }
    L-Para $A { param($f) $f.Alignment = 1 }
    L-Para $B { param($f) $f.Alignment = 2 }
    L-Para ('Justified line before a break' + $VT + 'and the line after the manual break closes the paragraph, so the break line is not stretched.') { param($f) $f.Alignment = 3 }
    L-Para 'Short justified paragraph.' { param($f) $f.Alignment = 3 }
  }

  L-Fixture 'layout-mixed-sizes' 'layout-mixed-sizes' 'several font sizes, faces and weights inside single lines' {
    Init-Doc 6
    $s = $word.Selection
    function Run($text, $name, $size, $bold, $italic) {
      $s = $script:word.Selection
      $s.Font.Name = $name; $s.Font.Size = $size; $s.Font.Bold = $bold; $s.Font.Italic = $italic
      $s.TypeText($text)
    }
    Run 'Small ' 'Calibri' 8 0 0; Run 'Normal ' 'Calibri' 11 0 0; Run 'Large ' 'Calibri' 20 0 0
    Run 'Cambria bold ' 'Cambria' 12 1 0; Run 'tiny ' 'Calibri' 6 0 0; Run 'and a huge word ' 'Calibri' 28 0 0
    Run 'followed by ordinary text that keeps going so that the line must wrap at least a couple of times ' 'Calibri' 11 0 0
    Run 'italic Cambria ' 'Cambria' 14 0 1; Run 'and a closing phrase in thirteen point.' 'Calibri' 13 0 0
    $s.TypeParagraph(); $s.Font.Reset()
    L-Para 'A second paragraph where only a few characters are large: ' $null $null
    Run 'XL' 'Calibri' 36 0 0; Run ' then small again, with enough words after it to need another line or two of wrapping.' 'Calibri' 9 0 0
    $s.TypeParagraph(); $s.Font.Reset()
    L-Para ($A.Substring(0, 120)) $null { param($f) $f.Size = 10 }
    L-Para ($A.Substring(0, 120)) $null { param($f) $f.Size = 12.5 }
    L-Para ($A.Substring(0, 120)) $null { param($f) $f.Size = 16 }
  }

  L-Fixture 'layout-indents-spacing' 'layout-indents-spacing' 'left/right/first-line/hanging indents; space before/after; single, 1.15, exact, at-least, multiple line spacing; contextual spacing' {
    Init-Doc 0
    L-Para 'Indents' { param($f) $f.SpaceAfter = 6 } { param($f) $f.Bold = 1 }
    L-Para $A { param($f) $f.LeftIndent = 36; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.RightIndent = 54; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.LeftIndent = 36; $f.RightIndent = 36; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.FirstLineIndent = 36; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.LeftIndent = 54; $f.FirstLineIndent = -54; $f.SpaceAfter = 6 }
    L-Para $B { param($f) $f.LeftIndent = 18; $f.FirstLineIndent = -36; $f.SpaceAfter = 6 }
    L-Para 'Space before and after' { param($f) $f.SpaceBefore = 12; $f.SpaceAfter = 6 } { param($f) $f.Bold = 1 }
    L-Para $A { param($f) $f.SpaceBefore = 18; $f.SpaceAfter = 10 }
    L-Para $A { param($f) $f.SpaceBefore = 4; $f.SpaceAfter = 24 }
    L-Para $A { param($f) $f.SpaceBefore = 6; $f.SpaceAfter = 0 }
    L-Para 'Line spacing rules' { param($f) $f.SpaceBefore = 12; $f.SpaceAfter = 6 } { param($f) $f.Bold = 1 }
    L-Para $A { param($f) $f.LineSpacingRule = 0; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.LineSpacingRule = 5; $f.LineSpacing = 13.8; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.LineSpacingRule = 1; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.LineSpacingRule = 2; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.LineSpacingRule = 5; $f.LineSpacing = 36; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.LineSpacingRule = 4; $f.LineSpacing = 16; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.LineSpacingRule = 3; $f.LineSpacing = 9; $f.SpaceAfter = 6 }
    L-Para $A { param($f) $f.LineSpacingRule = 3; $f.LineSpacing = 30; $f.SpaceAfter = 6 }
    L-Para $B { param($f) $f.LineSpacingRule = 5; $f.LineSpacing = 18; $f.SpaceAfter = 6 } { param($f) $f.Name = 'Cambria'; $f.Size = 12 }
    L-Para 'Contextual spacing' { param($f) $f.SpaceBefore = 12; $f.SpaceAfter = 6 } { param($f) $f.Bold = 1 }
    $cs = $doc.Styles.Add('O365Ctx', 1); $cs.BaseStyle = 'Normal'
    $cs.ParagraphFormat.SpaceBefore = 8; $cs.ParagraphFormat.SpaceAfter = 8; $cs.NoSpaceBetweenParagraphsOfSameStyle = $true
    foreach ($i in 1..3) {
      L-Para "Contextual paragraph $i keeps no space between neighbours of the same style." { param($f) $f.Style = 'O365Ctx' }
    }
    L-Para 'Normal paragraph after the contextual group.' { param($f) $f.SpaceBefore = 8; $f.SpaceAfter = 8 }
  }

  L-Fixture 'layout-pagination' 'layout-pagination' 'orphan/widow control, keep-with-next, keep-lines-together, explicit page break and page-break-before; Letter, 72pt margins, scenarios start on fresh pages' {
    Init-Doc 0
    function Fill($n) { foreach ($i in 1..$n) { L-Para "Filler line $i." } }
    function PageBrk() { $s = $script:word.Selection; [void]$s.EndKey(6); $s.InsertBreak(7) }
    $four = 'Line one of four.' + $VT + 'Line two of four.' + $VT + 'Line three of four.' + $VT + 'Line four of four.'
    # Scenario 1: one line would fit at the page bottom -> orphan control moves the whole paragraph
    Fill 47
    L-Para $four
    L-Para 'After orphan scenario.'
    PageBrk
    # Scenario 2: three of four lines would fit -> widow control pulls one more line over
    Fill 45
    L-Para $four
    L-Para 'After widow scenario.'
    PageBrk
    # Scenario 3: widow/orphan control switched off -> split exactly where the space ends
    Fill 47
    L-Para $four { param($f) $f.WidowControl = 0 }
    L-Para 'After no-control scenario.'
    PageBrk
    # Scenario 4: keep with next moves a heading to the next page with its paragraph
    Fill 46
    L-Para 'Heading kept with next' { param($f) $f.KeepWithNext = -1 } { param($f) $f.Bold = 1; $f.Size = 14 }
    L-Para $four
    L-Para 'After keep-with-next scenario.'
    PageBrk
    # Scenario 5: keep lines together moves a whole paragraph
    Fill 46
    L-Para $four { param($f) $f.KeepTogether = -1 }
    L-Para 'After keep-together scenario.'
    # Scenario 6: page break before (no explicit break precedes it), and a long wrapping paragraph
    # spanning a natural page boundary
    L-Para 'Paragraph with page break before.' { param($f) $f.PageBreakBefore = -1; $f.SpaceBefore = 18 }
    Fill 30
    L-Para ($A + ' ' + $B + ' ' + $A + ' ' + $B + ' ' + $A + ' ' + $B)
    L-Para ($B + ' ' + $A + ' ' + $B + ' ' + $A)
    L-Para 'Closing paragraph in Cambria.' $null { param($f) $f.Name = 'Cambria'; $f.Size = 12 }
    # Scenario 7: space before at the top of a page after a hard break, then after a natural break
    PageBrk
    L-Para 'Space before after a hard page break.' { param($f) $f.SpaceBefore = 18 }
    L-Para 'Second paragraph with space before.' { param($f) $f.SpaceBefore = 18 }
    Fill 43
    L-Para 'Space before after a natural page break.' { param($f) $f.SpaceBefore = 18 }
    Fill 2
  }

  L-Fixture 'layout-tabs' 'layout-tabs' 'default 0.5in tab stops; custom left/centre/right/decimal stops with dot, hyphen and underscore leaders' {
    Init-Doc 6
    L-Para ("Default`tstops`tevery`thalf`tinch`tacross") 
    L-Para ("One`tTwo`tThree`tFour`tFive`tSix`tSeven`tEight`tNine`tTen`tEleven`tTwelve`tThirteen`tFourteen")
    $left = { param($f) $f.TabStops.ClearAll(); [void]$f.TabStops.Add(144, 0, 0) }
    L-Para "Left`tstop at 2 inches`tnext default" $left
    $center = { param($f) $f.TabStops.ClearAll(); [void]$f.TabStops.Add(234, 1, 0) }
    L-Para "Centre`ttext centred on 3.25in" $center
    $right = { param($f) $f.TabStops.ClearAll(); [void]$f.TabStops.Add(450, 2, 0) }
    L-Para "Right`tright aligned at 6.25 inches" $right
    $dec = { param($f) $f.TabStops.ClearAll(); [void]$f.TabStops.Add(216, 3, 0); [void]$f.TabStops.Add(396, 3, 1) }
    L-Para "Price`t12.50`t3.14159" $dec
    L-Para "Total`t1234.5`t0.5" $dec
    L-Para "Whole`t42`t1000" $dec
    $leaders = { param($f) $f.TabStops.ClearAll(); [void]$f.TabStops.Add(200, 0, 1); [void]$f.TabStops.Add(300, 1, 2); [void]$f.TabStops.Add(450, 2, 3) }
    L-Para "Dots`tcentre`tright" $leaders
    $mixed = { param($f) $f.TabStops.ClearAll(); [void]$f.TabStops.Add(100, 0, 0); [void]$f.TabStops.Add(250, 1, 0); [void]$f.TabStops.Add(450, 2, 1) }
    L-Para "Name`tCentre title`t42" $mixed { param($f) $f.Name = 'Cambria'; $f.Size = 12 }
    L-Para "Alpha`tBeta`tGamma`tDelta" $mixed { param($f) $f.Name = 'Cambria'; $f.Size = 12 }
    $hang = { param($f) $f.TabStops.ClearAll(); $f.LeftIndent = 36; $f.FirstLineIndent = -36 }
    L-Para ("1.`t" + $B) $hang
    L-Para ("a)`tShort hanging entry.") $hang
  }

  # Probe: line pitch / baseline for many font sizes (one paragraph per size, 10 lines each, no spacing).
  L-Fixture 'layout-sizes' 'layout-sizes' 'line pitch and baseline offsets for Calibri and Cambria at sizes 8 to 72, 10 lines each, single spacing, no space after' {
    Init-Doc 0
    $ten = (1..10 | ForEach-Object { "Line $_ of ten" }) -join $VT
    foreach ($sz in 8, 9, 10, 10.5, 11, 12, 13, 14, 16, 18, 20, 24, 28, 36, 48, 72) {
      $szv = [single]$sz
      L-Para $ten $null { param($f) $f.Size = $szv }
    }
    foreach ($sz in 9, 11, 12, 14, 24, 40) {
      $szv = [single]$sz
      L-Para $ten $null { param($f) $f.Name = 'Cambria'; $f.Size = $szv }
    }
  }

  # Probe: does the paragraph mark's own font size change the height of its line?
  L-Fixture 'layout-marks' 'layout-marks' 'paragraph mark font size versus text size on the last line, and empty paragraphs with several mark sizes' {
    Init-Doc 0
    function Mark($text, $textSize, $markSize) {
      $ts = [single]$textSize; $ms = [single]$markSize
      L-Para $text $null { param($f) $f.Size = $ts }
      $p = $script:doc.Paragraphs.Item($script:doc.Paragraphs.Count - 1)
      $p.Range.Characters.Last.Font.Size = $ms
    }
    Mark ("Alpha one" + $VT + "Alpha two" + $VT + "Alpha three") 10 28
    Mark ("Beta one" + $VT + "Beta two" + $VT + "Beta three") 20 6
    Mark ("Gamma one" + $VT + "Gamma two") 12 12
    Mark ("Delta one" + $VT + "Delta two") 11 36
    Mark 'Epsilon single line, small text, big mark' 9 24
    foreach ($ms in 8, 11, 20, 36) {
      $mv = [single]$ms
      $s = $script:word.Selection; [void]$s.EndKey(6)
      $s.Font.Size = $mv
      $s.TypeParagraph()
      $s.Font.Reset()
    }
    L-Para 'Closing paragraph.'
  }

  # Probe: justified lines. Each paragraph narrows the text by a different right indent, so many lines end
  # within a few points of the margin (does Word shrink spaces to fit one more word? by how much?).
  L-Fixture 'layout-justify' 'layout-justify' 'justified text in Calibri 11 and Cambria 12 with right indents from 0 to 29.75 pt in 0.25 pt steps; a final group ends with a manual line break' {
    Init-Doc 0
    $C = 'Evening settles over the harbour while the lamps along the breakwater flicker on one by one, and the last ferry of the day slides quietly towards the slipway with its windows glowing amber against the darkening water.'
    foreach ($i in 0..119) {
      $ri = [single]($i * 0.25)
      L-Para $A { param($f) $f.Alignment = 3; $f.RightIndent = $ri }
    }
    foreach ($i in 0..119) {
      $ri = [single]($i * 0.25)
      L-Para ($B + ' ' + $C) { param($f) $f.Alignment = 3; $f.RightIndent = $ri }
    }
    foreach ($i in 0..39) {
      $ri = [single]($i * 0.75)
      L-Para $C { param($f) $f.Alignment = 3; $f.RightIndent = $ri } { param($f) $f.Name = 'Cambria'; $f.Size = 12 }
    }
    foreach ($i in 0..9) {
      $ri = [single]($i * 1.5)
      L-Para ('Justified text that finishes with a manual line break, followed by a short tail. ' + $B.Substring(0, 120) + $VT + 'Tail.') { param($f) $f.Alignment = 3; $f.RightIndent = $ri }
    }
  }
  # ---- lists -------------------------------------------------------------------------------------------
  # Number styles (WdListNumberStyle): 0 arabic, 1 upper roman, 2 lower roman, 3 upper letter, 4 lower letter,
  # 5 ordinal, 6 cardinal text, 7 ordinal text, 22 arabic with leading zero, 23 bullet.
  function New-Level($lt, $n, $fmt, $style, $numPos, $textPos, $trail, $start, $align) {
    $lv = $lt.ListLevels.Item($n)
    $lv.NumberFormat = $fmt; $lv.NumberStyle = $style; $lv.TrailingCharacter = $trail
    $lv.NumberPosition = $numPos; $lv.Alignment = $align; $lv.TextPosition = $textPos
    $lv.TabPosition = [single]9999999; $lv.StartAt = $start; $lv.ResetOnHigher = $n - 1
    $lv.LinkedStyle = ''
    return $lv
  }
  # One list paragraph. $cont false starts the list, true continues the previous list of this template.
  function L-Item($text, $lt, $level, $cont) {
    $s = $script:word.Selection; [void]$s.EndKey(6)
    $s.ParagraphFormat.Reset(); $s.Font.Reset()
    $s.TypeText($text)
    $r = $s.Paragraphs.Item(1).Range
    $r.ListFormat.ApplyListTemplate($lt, $cont, 0, 2)
    $r.ListFormat.ListLevelNumber = $level
    $s.TypeParagraph()
    $s.Range.ListFormat.RemoveNumbers()
    $s.ParagraphFormat.Reset(); $s.Font.Reset()
  }
  $W3 = 'Words long enough to wrap onto a second line so the hanging indent shows: ' + $B.Substring(0, 150)

  L-Fixture 'layout-lists-formats' 'layout-lists-formats' 'numbered and bulleted lists: decimal, letters, roman, zero-padded and ordinal formats, large start values that push the label past the hanging indent, bullets' {
    Init-Doc 6
    $specs = @(
      @('%1.', 0, 1), @('%1)', 4, 1), @('(%1)', 3, 1), @('%1.', 2, 1), @('%1.', 1, 1), @('%1.', 22, 1), @('%1.', 5, 1), @('%1.', 6, 1), @('%1.', 7, 1),
      @('%1.', 0, 98), @('%1.', 4, 24), @('%1.', 2, 8), @('%1.', 1, 38)
    )
    foreach ($sp in $specs) {
      $lt = $doc.ListTemplates.Add($false)
      [void](New-Level $lt 1 $sp[0] $sp[1] 18 36 0 $sp[2] 0)
      L-Para ("List " + $sp[0] + " style " + $sp[1] + " start " + $sp[2]) $null { param($f) $f.Bold = 1 }
      foreach ($i in 1..4) { L-Item "Item $i of the list" $lt 1 ($i -gt 1) }
      L-Item $W3 $lt 1 $true
    }
    $bt = $doc.ListTemplates.Add($false)
    $bl = New-Level $bt 1 ([string][char]0xF0B7) 23 18 36 0 1 0
    $bl.Font.Name = 'Symbol'
    L-Para 'Bulleted list' $null { param($f) $f.Bold = 1 }
    foreach ($i in 1..3) { L-Item "Bullet $i" $bt 1 ($i -gt 1) }
    L-Item $W3 $bt 1 $true
  }

  L-Fixture 'layout-lists-multilevel' 'layout-lists-multilevel' 'multilevel numbering: nested level counters and restarts, joined numbers, continuing versus restarted lists, empty list items' {
    Init-Doc 6
    $lt = $doc.ListTemplates.Add($true)
    [void](New-Level $lt 1 '%1.' 0 18 36 0 1 0)
    [void](New-Level $lt 2 '%1.%2.' 0 36 72 0 1 0)
    [void](New-Level $lt 3 '%1.%2.%3' 0 72 126 0 1 0)
    [void](New-Level $lt 4 '(%4)' 4 108 144 0 1 0)
    [void](New-Level $lt 5 '%5)' 2 144 180 0 1 0)
    L-Para 'Outline' $null { param($f) $f.Bold = 1 }
    $plan = @(1,2,2,3,3,1,2,3,4,5,4,3,2,1,3,1)
    $first = $true
    foreach ($lvl in $plan) { L-Item "Level $lvl entry" $lt $lvl (-not $first); $first = $false }
    L-Para 'Same list continued after a plain paragraph' $null { param($f) $f.Bold = 1 }
    L-Item 'Continues the outline' $lt 1 $true
    L-Item 'Nested under it' $lt 2 $true
    L-Para 'A separate list restarts at one' $null { param($f) $f.Bold = 1 }
    L-Item 'Restarted first' $lt 1 $false
    L-Item 'Restarted second' $lt 1 $true
    L-Item 'Restarted nested' $lt 2 $true
    L-Item '' $lt 1 $true
    L-Item 'After an empty item' $lt 1 $true
  }

  L-Fixture 'layout-lists-suff' 'layout-lists-suff' 'trailing character (tab, space, nothing), label alignment, first-line (non-hanging) lists, label run formatting, direct indent overrides' {
    Init-Doc 6
    $variants = @(
      @('tab, hanging 18', '%1.', 0, 0, 18, 36, 0, 0),
      @('space', '%1.', 0, 1, 18, 36, 0, 0),
      @('nothing', '%1.', 0, 2, 18, 36, 0, 0),
      @('tab, label wider than hanging', 'Step %1:', 0, 0, 18, 36, 0, 0),
      @('tab, no hanging (text at number position)', '%1.', 0, 0, 36, 36, 0, 0),
      @('tab, text before number', '%1.', 0, 0, 54, 36, 0, 0),
      @('right aligned label', '%1.', 0, 0, 54, 72, 0, 2),
      @('centered label', '%1.', 0, 0, 54, 72, 0, 1)
    )
    foreach ($v in $variants) {
      $lt = $doc.ListTemplates.Add($false)
      [void](New-Level $lt 1 $v[1] $v[2] $v[4] $v[5] $v[3] 8 $v[7])
      L-Para ("Variant: " + $v[0]) $null { param($f) $f.Bold = 1 }
      foreach ($i in 1..2) { L-Item "Item $i" $lt 1 ($i -gt 1) }
      L-Item $W3 $lt 1 $true
    }
    $fl = $doc.ListTemplates.Add($false)
    $fv = New-Level $fl 1 '%1.' 0 18 36 0 1 0
    $fv.Font.Bold = 1; $fv.Font.Size = 20; $fv.Font.Name = 'Cambria'
    L-Para 'Variant: label font bold 20 pt Cambria' $null { param($f) $f.Bold = 1 }
    foreach ($i in 1..3) { L-Item "Item $i" $fl 1 ($i -gt 1) }
    $ml = $doc.ListTemplates.Add($false)
    [void](New-Level $ml 1 '%1.' 0 18 36 0 1 0)
    L-Para 'Variant: direct indent override, then a paragraph mark larger than its text' $null { param($f) $f.Bold = 1 }
    L-Item 'Direct left indent 90, first line -18' $ml 1 $false
    $pp = $doc.Paragraphs.Item($doc.Paragraphs.Count - 1)
    $pp.LeftIndent = 90; $pp.FirstLineIndent = -18
    L-Item 'Big mark' $ml 1 $true
    $pp = $doc.Paragraphs.Item($doc.Paragraphs.Count - 1)
    $pp.Range.Characters.Last.Font.Size = 28
    L-Item 'Back to plain' $ml 1 $true
  }
  L-Fixture 'layout-lists-style' 'layout-lists-style' 'numbering linked to paragraph styles (Heading 1 and 2 carry the list, paragraphs have no numPr); direct formatting on a numbered style paragraph' {
    Init-Doc 6
    $lt = $doc.ListTemplates.Add($true)
    $l1 = New-Level $lt 1 '%1.' 0 0 36 0 1 0
    $l1.LinkedStyle = 'Heading 1'
    $l2 = New-Level $lt 2 '%1.%2' 0 0 36 0 1 0
    $l2.LinkedStyle = 'Heading 2'
    $l3 = New-Level $lt 3 '%3)' 4 36 72 0 1 0
    $l3.LinkedStyle = 'Heading 3'
    foreach ($h in 'Heading 1', 'Heading 2', 'Heading 3') {
      $doc.Styles.Item($h).Font.Name = 'Calibri'; $doc.Styles.Item($h).Font.Size = 14
      $doc.Styles.Item($h).ParagraphFormat.SpaceBefore = 6; $doc.Styles.Item($h).ParagraphFormat.SpaceAfter = 6
    }
    foreach ($i in 1..2) {
      L-Para "Chapter $i" { param($f) $f.Style = 'Heading 1' }
      L-Para $W3 $null
      L-Para "Section $i.1" { param($f) $f.Style = 'Heading 2' }
      L-Para "Detail $i.1.a" { param($f) $f.Style = 'Heading 3' }
      L-Para "Detail $i.1.b with a long wrapping title: $($B.Substring(0, 120))" { param($f) $f.Style = 'Heading 3' }
      L-Para "Section $i.2" { param($f) $f.Style = 'Heading 2' }
    }
    L-Para 'Heading with direct indent' { param($f) $f.Style = 'Heading 1'; $f.LeftIndent = 90; $f.FirstLineIndent = -36 }
  }

  L-Fixture 'layout-keep' 'layout-keep' 'keep-with-next chains, keep-with-next before a page-break-before paragraph, keep-with-next on multi-line paragraphs, keep-lines with keep-with-next, space before between kept paragraphs, widow control off with keep-lines; each scenario starts on a fresh page' {
    Init-Doc 0
    function Fill($n) { foreach ($i in 1..$n) { L-Para "Filler line $i." } }
    function PageBrk() { $s = $script:word.Selection; [void]$s.EndKey(6); $s.InsertBreak(7) }
    $four = 'Line one of four.' + $VT + 'Line two of four.' + $VT + 'Line three of four.' + $VT + 'Line four of four.'
    $two = 'Line one of two.' + $VT + 'Line two of two.'
    $keep = { param($f) $f.KeepWithNext = -1 }
    foreach ($fill in 43, 44, 45, 46) {
      # chain of three kept headings, then a body paragraph
      Fill $fill
      L-Para 'Chain heading one' $keep
      L-Para 'Chain heading two' $keep
      L-Para 'Chain heading three' $keep
      L-Para $four
      L-Para 'After chain.'
      PageBrk
    }
    foreach ($fill in 45, 46, 47) {
      # kept paragraph followed by page-break-before: nothing to keep with on this page
      Fill $fill
      L-Para $two $keep
      L-Para 'Starts a new page by itself.' { param($f) $f.PageBreakBefore = -1 }
      PageBrk
    }
    foreach ($fill in 43, 44, 45, 46) {
      # multi-line kept paragraph followed by a multi-line paragraph
      Fill $fill
      L-Para $four $keep
      L-Para $four
      PageBrk
    }
    foreach ($fill in 43, 44, 45, 46) {
      # keep lines together and keep with next
      Fill $fill
      L-Para $four { param($f) $f.KeepTogether = -1; $f.KeepWithNext = -1 }
      L-Para $two
      PageBrk
    }
    foreach ($fill in 44, 45, 46) {
      # space before and after between a kept paragraph and its follower
      Fill $fill
      L-Para $two { param($f) $f.KeepWithNext = -1; $f.SpaceBefore = 20; $f.SpaceAfter = 4 }
      L-Para $two { param($f) $f.SpaceBefore = 14; $f.SpaceAfter = 10 }
      L-Para 'Tail.'
      PageBrk
    }
    foreach ($fill in 44, 45, 46) {
      # widow control off, keep lines
      Fill $fill
      L-Para $four { param($f) $f.WidowControl = 0; $f.KeepTogether = -1 }
      L-Para $four { param($f) $f.WidowControl = 0 }
      PageBrk
    }
  }

  # ---- tables ------------------------------------------------------------------------------------------
  # Cell text is built from unique tokens ("R2C3w1 R2C3w2 ...") so the oracle can tell from the PDF which
  # paragraph and line each drawn row belongs to.
  function Toks($tag, $n) { (1..$n | ForEach-Object { "${tag}w$_" }) -join ' ' }
  # Appends a table after the last paragraph; the document keeps an empty paragraph after it.
  function T-Add($rows, $cols, [scriptblock]$cfg = $null) {
    $d = $script:doc
    $rng = $d.Paragraphs.Item($d.Paragraphs.Count).Range
    $t = $d.Tables.Add($rng, $rows, $cols)
    if ($cfg) { & $cfg $t }
    return $t
  }
  # Fills every cell: $words is a scriptblock (r, c) -> number of tokens (0 leaves "RrCc").
  function T-Fill($t, [scriptblock]$words = $null) {
    foreach ($c in $t.Range.Cells) {
      $r = [int]$c.RowIndex; $k = [int]$c.ColumnIndex
      $n = if ($words) { [int](& $words $r $k) } else { 0 }
      $c.Range.Text = if ($n -gt 0) { Toks "R${r}C${k}" $n } else { "R${r}C${k}" }
    }
  }
  function T-Widths($t, $widths) {
    $t.AllowAutoFit = $false
    for ($i = 1; $i -le $widths.Count; $i++) { $t.Columns.Item($i).Width = [single]$widths[$i - 1] }
  }
  $LB = { param($f) $f.Bold = 1 }

  L-Fixture 'layout-table-basic' 'layout-table-basic' 'fixed-layout tables: equal and uneven columns, wrapping cell text, cell padding, table indent, centred and right aligned tables, borderless and thick-bordered tables; 6pt space after in Normal' {
    Init-Doc 6
    L-Para 'Equal columns' $null $LB
    $t = T-Add 3 3; T-Fill $t
    L-Para 'After the first table.'
    L-Para 'Uneven columns with wrapping text' $null $LB
    $t = T-Add 3 3; T-Widths $t @(60, 200, 208); T-Fill $t { param($r, $c) if ($c -eq 2) { 14 } elseif ($c -eq 1) { 3 } else { 6 } }
    L-Para 'After the second table.'
    L-Para 'Cell padding 14.4 left and right, 3 top and bottom' $null $LB
    $t = T-Add 2 2; $t.LeftPadding = 14.4; $t.RightPadding = 14.4; $t.TopPadding = 3; $t.BottomPadding = 3
    T-Fill $t { param($r, $c) 8 }
    L-Para 'Table indent 36pt, 3 columns of 100pt'
    $t = T-Add 2 3; T-Widths $t @(100, 100, 100); $t.Rows.LeftIndent = 36; T-Fill $t
    L-Para 'Centred table of 3 columns of 100pt'
    $t = T-Add 2 3; T-Widths $t @(100, 100, 100); $t.Rows.Alignment = 1; T-Fill $t
    L-Para 'Right aligned table of 3 columns of 100pt'
    $t = T-Add 2 3; T-Widths $t @(100, 100, 100); $t.Rows.Alignment = 2; T-Fill $t
    L-Para 'No borders'
    $t = T-Add 2 3; T-Widths $t @(120, 120, 120); $t.Borders.Enable = $false; T-Fill $t
    L-Para 'Thick outside border (3pt) and 1.5pt inside'
    $t = T-Add 2 3; T-Widths $t @(120, 120, 120); $t.Borders.OutsideLineStyle = 1; $t.Borders.OutsideLineWidth = 24
    $t.Borders.InsideLineStyle = 1; $t.Borders.InsideLineWidth = 12; T-Fill $t
    L-Para 'Closing paragraph.'
  }

  L-Fixture 'layout-table-width' 'layout-table-width' 'table widths: preferred percent and point widths, autofit to contents, autofit to window, cell preferred widths with autofit, tables wider than the text area' {
    Init-Doc 6
    L-Para 'Percent width 60 percent' $null $LB
    $t = T-Add 2 3; $t.PreferredWidthType = 2; $t.PreferredWidth = 60; T-Fill $t { param($r, $c) 2 }
    L-Para 'Autofit to contents, short and long cells' $null $LB
    $t = T-Add 3 3; T-Fill $t { param($r, $c) if ($r -eq 2) { $c * 3 } else { 1 } }; $t.AutoFitBehavior(1)
    L-Para 'Autofit to window' $null $LB
    $t = T-Add 2 3; T-Fill $t { param($r, $c) $c * 4 }; $t.AutoFitBehavior(1); $t.AutoFitBehavior(2)
    L-Para 'Autofit allowed, preferred column widths 80 100 150 points' $null $LB
    $t = T-Add 3 3; $t.AllowAutoFit = $true; T-Fill $t { param($r, $c) 10 }
    $w = 80, 100, 150
    for ($i = 1; $i -le 3; $i++) { $t.Columns.Item($i).PreferredWidthType = 3; $t.Columns.Item($i).PreferredWidth = [single]$w[$i - 1] }
    L-Para 'Autofit allowed, one long unbreakable word' $null $LB
    $t = T-Add 2 3; $t.AllowAutoFit = $true; T-Fill $t
    $t.Cell(2, 2).Range.Text = 'Supercalifragilisticexpialidocious Pneumonoultramicroscopic'
    L-Para 'Fixed, columns wider than the text area (3 x 200)' $null $LB
    $t = T-Add 2 3; T-Widths $t @(200, 200, 200); T-Fill $t
    L-Para 'Fixed, narrow columns (25pt) with long words' $null $LB
    $t = T-Add 2 4; T-Widths $t @(25, 25, 25, 25); T-Fill $t { param($r, $c) 2 }
    L-Para 'Closing paragraph.'
  }

  L-Fixture 'layout-table-rows' 'layout-table-rows' 'row heights (auto, at least, exact), vertical alignment, paragraph spacing and multiple paragraphs in cells, horizontal and vertical merges' {
    Init-Doc 6
    L-Para 'Row heights: auto, at least 40, at least 10, exactly 40, exactly 10' $null $LB
    $t = T-Add 5 2; T-Widths $t @(200, 200); T-Fill $t { param($r, $c) if ($c -eq 2) { 20 } else { 1 } }
    $t.Rows.Item(2).HeightRule = 1; $t.Rows.Item(2).Height = 40
    $t.Rows.Item(3).HeightRule = 1; $t.Rows.Item(3).Height = 10
    $t.Rows.Item(4).HeightRule = 2; $t.Rows.Item(4).Height = 40
    $t.Rows.Item(5).HeightRule = 2; $t.Rows.Item(5).Height = 10
    L-Para 'Vertical alignment: top, centre, bottom' $null $LB
    $t = T-Add 1 3; T-Widths $t @(130, 130, 130); T-Fill $t { param($r, $c) 1 }
    $t.Rows.Item(1).HeightRule = 1; $t.Rows.Item(1).Height = 60
    $t.Cell(1, 1).VerticalAlignment = 0; $t.Cell(1, 2).VerticalAlignment = 1; $t.Cell(1, 3).VerticalAlignment = 3
    L-Para 'Vertical alignment with a taller neighbour'
    $t = T-Add 1 3; T-Widths $t @(130, 130, 130); T-Fill $t { param($r, $c) if ($c -eq 3) { 25 } else { 1 } }
    $t.Cell(1, 1).VerticalAlignment = 0; $t.Cell(1, 2).VerticalAlignment = 1; $t.Cell(1, 3).VerticalAlignment = 0
    L-Para 'Space before and after, several paragraphs in a cell' $null $LB
    $t = T-Add 2 2; T-Widths $t @(200, 200)
    $t.Cell(1, 1).Range.Text = "R1C1p1`rR1C1p2`rR1C1p3"
    $t.Cell(1, 2).Range.Text = 'R1C2p1'
    $t.Cell(2, 1).Range.Text = 'R2C1p1'; $t.Cell(2, 2).Range.Text = 'R2C2p1'
    $t.Cell(1, 1).Range.Paragraphs.Item(1).Format.SpaceBefore = 10; $t.Cell(1, 1).Range.Paragraphs.Item(1).Format.SpaceAfter = 3
    $t.Cell(1, 1).Range.Paragraphs.Item(2).Format.SpaceBefore = 12; $t.Cell(1, 1).Range.Paragraphs.Item(2).Format.SpaceAfter = 12
    $t.Cell(1, 1).Range.Paragraphs.Item(3).Format.SpaceAfter = 0
    $t.Cell(2, 2).Range.Paragraphs.Item(1).Format.SpaceBefore = 20
    L-Para 'Horizontal merges'
    $t = T-Add 3 4; T-Widths $t @(100, 100, 100, 100); T-Fill $t
    $t.Cell(1, 1).Merge($t.Cell(1, 2)); $t.Cell(2, 2).Merge($t.Cell(2, 4)); $t.Cell(3, 1).Merge($t.Cell(3, 4))
    $t.Cell(1, 1).Range.Text = 'M1 wide cell with several tokens M1w1 M1w2 M1w3 M1w4 M1w5 M1w6 M1w7 M1w8 M1w9 M1w10'
    L-Para 'Vertical merges, tall merged text'
    $t = T-Add 4 3; T-Widths $t @(130, 130, 130); T-Fill $t
    $t.Cell(1, 1).Merge($t.Cell(3, 1)); $t.Cell(2, 3).Merge($t.Cell(4, 3))
    $t.Cell(1, 1).Range.Text = (Toks 'V1' 40)
    $t.Cell(2, 3).Range.Text = 'V2 short'
    L-Para 'Block merge'
    $t = T-Add 3 3; T-Widths $t @(130, 130, 130); T-Fill $t
    $t.Cell(1, 1).Merge($t.Cell(2, 2))
    $t.Cell(1, 1).Range.Text = (Toks 'B1' 30)
    L-Para 'Closing paragraph.'
  }

  L-Fixture 'layout-table-pages' 'layout-table-pages' 'tables across pages: a long table, rows that split, rows that cannot split, repeated header rows, vertical merges over a page break' {
    Init-Doc 0
    function Fill($n) { foreach ($i in 1..$n) { L-Para "Filler line $i." } }
    function PageBrk() { $s = $script:word.Selection; [void]$s.EndKey(6); $s.InsertBreak(7) }
    L-Para 'Long table' $null $LB
    $t = T-Add 60 2; T-Widths $t @(200, 200); T-Fill $t { param($r, $c) 1 }
    L-Para 'After the long table.'
    PageBrk
    Fill 40
    L-Para 'Splitting rows (tall rows may break across pages)' $null $LB
    $t = T-Add 6 2; T-Widths $t @(200, 200); T-Fill $t { param($r, $c) if ($c -eq 1) { 80 } else { 8 } }
    PageBrk
    Fill 40
    L-Para 'Rows that cannot split' $null $LB
    $t = T-Add 6 2; T-Widths $t @(200, 200); T-Fill $t { param($r, $c) if ($c -eq 1) { 80 } else { 8 } }
    foreach ($r in 1..6) { $t.Rows.Item($r).AllowBreakAcrossPages = 0 }
    PageBrk
    Fill 20
    L-Para 'Repeated header row' $null $LB
    $t = T-Add 50 2; T-Widths $t @(200, 200); T-Fill $t { param($r, $c) if ($r -eq 1) { 1 } else { 1 } }
    $t.Rows.Item(1).HeadingFormat = -1
    $t.Rows.Item(1).Range.Font.Bold = 1
    PageBrk
    Fill 30
    L-Para 'Two header rows, tall body rows' $null $LB
    $t = T-Add 16 2; T-Widths $t @(200, 200); T-Fill $t { param($r, $c) if ($r -le 2) { 1 } else { 12 } }
    $t.Rows.Item(1).HeadingFormat = -1; $t.Rows.Item(2).HeadingFormat = -1
    PageBrk
    Fill 36
    L-Para 'Vertical merge over a page break' $null $LB
    $t = T-Add 5 2; T-Widths $t @(200, 200); T-Fill $t { param($r, $c) 6 }
    $t.Cell(1, 1).Merge($t.Cell(5, 1)); $t.Cell(1, 1).Range.Text = (Toks 'VM' 100)
    PageBrk
    Fill 44
    L-Para 'Header row with a row that cannot split' $null $LB
    $t = T-Add 6 2; T-Widths $t @(200, 200); T-Fill $t { param($r, $c) if ($r -eq 1) { 1 } else { 18 } }
    $t.Rows.Item(1).HeadingFormat = -1
    foreach ($r in 2..6) { $t.Rows.Item($r).AllowBreakAcrossPages = 0 }
  }

  L-Fixture 'layout-table-flow' 'layout-table-flow' 'space before and after around tables, adjacent tables, nested tables, table styles with header row, banding and bold first column' {
    Init-Doc 6
    L-Para 'Paragraph with 20pt space after, then a table' { param($f) $f.SpaceAfter = 20 }
    $t = T-Add 2 2; T-Widths $t @(150, 150); T-Fill $t
    L-Para 'Paragraph with 20pt space before after the table' { param($f) $f.SpaceBefore = 20 }
    L-Para 'Paragraph with 30pt space before, then a table' { param($f) $f.SpaceBefore = 30 }
    $t = T-Add 2 2; T-Widths $t @(150, 150); T-Fill $t
    L-Para ''
    $t = T-Add 2 2; T-Widths $t @(150, 150); T-Fill $t
    L-Para 'Paragraph after two tables'
    L-Para 'Nested table' $null $LB
    $t = T-Add 2 2; T-Widths $t @(200, 200); T-Fill $t { param($r, $c) 3 }
    $t.Cell(1, 2).Range.Text = 'Outer cell'
    $inner = $doc.Tables.Add($t.Cell(1, 2).Range.Paragraphs.Item(1).Range, 2, 2)
    $inner.Cell(1, 1).Range.Text = 'I11'; $inner.Cell(1, 2).Range.Text = 'I12'; $inner.Cell(2, 1).Range.Text = 'I21'; $inner.Cell(2, 2).Range.Text = 'I22'
    L-Para 'Table style Grid Table 4 - Accent 1' $null $LB
    $t = T-Add 4 3; T-Fill $t { param($r, $c) 2 }; $t.Style = 'Grid Table 4 - Accent 1'
    L-Para 'Table style List Table 3 - Accent 2' $null $LB
    $t = T-Add 4 3; T-Fill $t { param($r, $c) 2 }; $t.Style = 'List Table 3 - Accent 2'
    L-Para 'Table Grid style with explicit 0 spacing' $null $LB
    $t = T-Add 3 3; T-Fill $t { param($r, $c) 2 }; $t.Style = 'Table Grid'
    L-Para 'Plain table style (Table Normal) with banding look off'
    $t = T-Add 3 3; T-Fill $t { param($r, $c) 2 }; $t.Style = 'Table Normal'
    L-Para 'Closing paragraph.'
  }

  # ---- headers, footers, footnotes, endnotes and fields (layout-hf-*, layout-fn-*) ----
  # Text of the story `$kind` (1 primary, 2 first page, 3 even pages) of section `$sec`.
  function HF-Header($sec, $kind, $text) { $script:doc.Sections.Item($sec).Headers.Item($kind).Range.Text = $text }
  function HF-Footer($sec, $kind, $text) { $script:doc.Sections.Item($sec).Footers.Item($kind).Range.Text = $text }
  # Footer "<prefix>PAGE<middle>NUMPAGES|SECTIONPAGES" built from real fields (wdFieldPage 33, NumPages 26, SectionPages 25).
  function HF-FieldFooter($sec, $kind, $prefix, $middle, $second) {
    $r = $script:doc.Sections.Item($sec).Footers.Item($kind).Range
    $r.Text = $prefix
    $r.Collapse(0); [void]$script:doc.Fields.Add($r, 33)
    $r = $script:doc.Sections.Item($sec).Footers.Item($kind).Range; $r.MoveEnd(1, -1) | Out-Null; $r.Collapse(0)
    if ($middle) { $r.InsertAfter($middle); $r.Collapse(0) }
    if ($second) { [void]$script:doc.Fields.Add($r, $second) }
  }
  # A footnote (or endnote when $end) at the end of the last body paragraph before the trailing empty one.
  function N-Add($text, $end = $false) {
    $d = $script:doc
    $r = $d.Paragraphs.Item($d.Paragraphs.Count - 1).Range
    [void]$r.MoveEnd(1, -1); $r.Collapse(0)
    if ($end) { [void]$d.Endnotes.Add($r, $script:M, $text) } else { [void]$d.Footnotes.Add($r, $script:M, $text) }
  }
  $PARAS = 24   # body paragraphs that fill about three Letter pages with 6 pt after

  L-Fixture 'layout-hf-basic' 'layout-hf-basic' 'one section: header text, footer with PAGE and NUMPAGES, three pages of body text' {
    Init-Doc 6
    for ($i = 0; $i -lt $PARAS; $i++) { L-Para $(if ($i % 2) { $B } else { $A }) }
    HF-Header 1 1 'Quarterly harbour survey'
    HF-FieldFooter 1 1 'Page ' ' of ' 26
  }

  L-Fixture 'layout-hf-first-even' 'layout-hf-first-even' 'titlePg and evenAndOddHeaders: distinct first, even and primary headers and footers over five pages' {
    Init-Doc 6
    $doc.PageSetup.DifferentFirstPageHeaderFooter = -1
    $doc.PageSetup.OddAndEvenPagesHeaderFooter = -1
    for ($i = 0; $i -lt 40; $i++) { L-Para $(if ($i % 2) { $B } else { $A }) }
    HF-Header 1 2 'First page header'
    HF-Header 1 1 'Odd page header'
    HF-Header 1 3 'Even page header'
    HF-Footer 1 2 'First page footer'
    HF-FieldFooter 1 1 'Odd footer page ' '' 0
    HF-FieldFooter 1 3 'Even footer page ' '' 0
  }

  L-Fixture 'layout-hf-distance' 'layout-hf-distance' 'header/footer distances of 0.3 in; a three-paragraph header taller than the top margin pushes the body down, and a tall footer lifts the body bottom' {
    Init-Doc 6
    $ps = $doc.PageSetup
    $ps.TopMargin = 54; $ps.BottomMargin = 54; $ps.HeaderDistance = 21.6; $ps.FooterDistance = 21.6
    for ($i = 0; $i -lt 16; $i++) { L-Para $(if ($i % 2) { $B } else { $A }) }
    HF-Header 1 1 ("Report header line one`rSecond header line in 14 pt`rThird header line")
    $doc.Sections.Item(1).Headers.Item(1).Range.Paragraphs.Item(2).Range.Font.Size = 14
    $doc.Sections.Item(1).Headers.Item(1).Range.Paragraphs.Item(3).Format.SpaceAfter = 12
    HF-Footer 1 1 ("Footer line one`rFooter line two`rFooter line three")
  }

  L-Fixture 'layout-hf-sections' 'layout-hf-sections' 'three sections: own header in section 2, section 3 linked to section 2; footers with PAGE and SECTIONPAGES' {
    Init-Doc 6
    for ($i = 0; $i -lt 8; $i++) { L-Para $(if ($i % 2) { $B } else { $A }) }
    $s = $word.Selection; [void]$s.EndKey(6); $s.InsertBreak(2)
    for ($i = 0; $i -lt 16; $i++) { L-Para $(if ($i % 2) { $B } else { $A }) }
    $s = $word.Selection; [void]$s.EndKey(6); $s.InsertBreak(2)
    for ($i = 0; $i -lt 8; $i++) { L-Para $(if ($i % 2) { $B } else { $A }) }
    HF-Header 1 1 'Section one header'
    $doc.Sections.Item(2).Headers.Item(1).LinkToPrevious = $false
    HF-Header 2 1 'Section two header'
    HF-FieldFooter 1 1 'Page ' ' of section ' 25
    $doc.Sections.Item(2).Footers.Item(1).LinkToPrevious = $true
    $doc.Sections.Item(3).Footers.Item(1).LinkToPrevious = $true
  }

  L-Fixture 'layout-fn-basic' 'layout-fn-basic' 'a footnote on most paragraphs over several pages: the notes reduce the body height and stay on their reference page' {
    Init-Doc 6
    for ($i = 0; $i -lt 18; $i++) {
      L-Para $(if ($i % 2) { $B } else { $A })
      if ($i % 3 -ne 2) { N-Add ("Note $($i + 1): a short explanatory remark for the paragraph above.") }
    }
  }

  L-Fixture 'layout-fn-long' 'layout-fn-long' 'a very long footnote that must continue on the next page, with the continuation separator' {
    Init-Doc 6
    for ($i = 0; $i -lt 6; $i++) { L-Para $(if ($i % 2) { $B } else { $A }) }
    L-Para $A; N-Add ($B + ' ' + $A + ' ' + $B + ' ' + $A + ' ' + $B + ' ' + $A + ' ' + $B)
    L-Para $B; N-Add ($A + ' ' + $B + ' ' + $A + ' ' + $B)
    for ($i = 0; $i -lt 9; $i++) { L-Para $(if ($i % 2) { $B } else { $A }) }
    L-Para $A; N-Add 'Short closing note.'
  }

  L-Fixture 'layout-fn-endnotes' 'layout-fn-endnotes' 'endnotes collect at the end of the document after the body text, over a page break' {
    Init-Doc 6
    for ($i = 0; $i -lt 16; $i++) {
      L-Para $(if ($i % 2) { $B } else { $A })
      N-Add ("Endnote $($i + 1): a source reference for the paragraph above, with a few more words.") $true
    }
  }

  L-Fixture 'layout-fn-mixed' 'layout-fn-mixed' 'footnotes and endnotes together with a header and a PAGE footer' {
    Init-Doc 6
    for ($i = 0; $i -lt 14; $i++) {
      L-Para $(if ($i % 2) { $B } else { $A })
      N-Add ("Footnote $($i + 1) text.")
      if ($i % 2) { N-Add ("Endnote $($i + 1) text.") $true }
    }
    HF-Header 1 1 'Notes and furniture'
    HF-FieldFooter 1 1 'Page ' '' 0
  }


  # ---- Pictures: layout-img-* -------------------------------------------------------------------
  # Solid-colour PNGs drawn here (no external images), inserted at an explicit size in points.
  Add-Type -AssemblyName System.Drawing
  $imgDir = Join-Path ([IO.Path]::GetTempPath()) ("o365_img_{0}" -f $PID)
  New-Item -ItemType Directory -Force $imgDir | Out-Null
  function New-Png($name, $pxW, $pxH, [int[]]$rgb) {
    $path = Join-Path $script:imgDir "$name.png"
    $bmp = New-Object System.Drawing.Bitmap $pxW, $pxH
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::FromArgb($rgb[0], $rgb[1], $rgb[2]))
    $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::White), 2
    $g.DrawRectangle($pen, 1, 1, $pxW - 3, $pxH - 3); $g.DrawLine($pen, 0, 0, $pxW - 1, $pxH - 1)
    $pen.Dispose(); $g.Dispose(); $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    $path
  }
  $PNG_RED = New-Png 'red' 64 32 @(200, 40, 40)
  $PNG_BLUE = New-Png 'blue' 48 48 @(40, 70, 200)
  $PNG_GREEN = New-Png 'green' 80 40 @(40, 160, 70)
  # Inline picture of $w x $h points at the end of the document.
  function Add-Inline($png, $w, $h, [scriptblock]$fmt = $null) {
    $s = $script:word.Selection; [void]$s.EndKey(6)
    $ils = $script:doc.InlineShapes.AddPicture($png, $false, $true, $s.Range)
    $ils.LockAspectRatio = 0; $ils.Width = $w; $ils.Height = $h
    if ($fmt) { & $fmt $ils }
    [void]$s.EndKey(6)
  }
  function Type-Text($t) { $s = $script:word.Selection; [void]$s.EndKey(6); $s.TypeText($t) }
  function End-Para() { $s = $script:word.Selection; [void]$s.EndKey(6); $s.TypeParagraph(); $s.ParagraphFormat.Reset(); $s.Font.Reset() }
  function Last-Format() { $script:doc.Paragraphs.Item($script:doc.Paragraphs.Count).Format }
  # Floating picture anchored in paragraph $pi (1-based): wrap = WdWrapType (0 square, 1 tight, 3 none/front, 4 top+bottom, 5 behind),
  # $hrel/$vrel = WdRelativeHorizontal/VerticalPosition, $x/$y = offsets or WdShapePosition constants, $side = WdWrapSideType.
  function Add-Float($pi, $png, $w, $h, $wrap, $hrel, $x, $vrel, $y, $dist = $null, $side = 0) {
    # An inline picture converted to a shape anchors in exactly this paragraph (Shapes.AddPicture's Anchor argument did not).
    $r = $script:doc.Paragraphs.Item($pi).Range; $r.Collapse(1)
    $sh = $script:doc.InlineShapes.AddPicture($png, $false, $true, $r).ConvertToShape()
    $sh.LockAspectRatio = 0; $sh.Width = $w; $sh.Height = $h
    $sh.WrapFormat.Type = $wrap
    if ($wrap -in 0, 1) { $sh.WrapFormat.Side = $side }
    if ($dist) { $sh.WrapFormat.DistanceTop = $dist[0]; $sh.WrapFormat.DistanceBottom = $dist[1]; $sh.WrapFormat.DistanceLeft = $dist[2]; $sh.WrapFormat.DistanceRight = $dist[3] }
    $sh.RelativeHorizontalPosition = $hrel; $sh.RelativeVerticalPosition = $vrel
    $sh.Left = [single]$x; $sh.Top = [single]$y
    $sh
  }

  L-Fixture 'layout-img-inline' 'layout-img-inline' 'inline pictures of several sizes: line height from the picture, wrapping, mixed font sizes, spacing rules, picture border' {
    Init-Doc 6
    Type-Text 'Text before '; Add-Inline $PNG_RED 20 10; Type-Text ' and after the small picture on one line, with enough further words that the paragraph needs a second and a third line to finish.'; End-Para
    Add-Inline $PNG_BLUE 36 36; End-Para
    Type-Text 'Tall: '; Add-Inline $PNG_BLUE 30 72; Type-Text ' tail text after a tall picture.'; End-Para
    Add-Inline $PNG_GREEN 150 30; Type-Text ' wide picture then text that follows it on the same line and wraps onto the next line.'; End-Para
    Type-Text 'Wrap test: some words that nearly fill the first line so the next picture '; Add-Inline $PNG_GREEN 120 40; Type-Text ' moves to the following line.'; End-Para
    Type-Text 'Multiple 1.5 '; Add-Inline $PNG_BLUE 40 40; Type-Text ' with a second line of ordinary text after the picture to see the leading, and a third.'
    $p = Last-Format; $p.LineSpacingRule = 5; $p.LineSpacing = 18; End-Para
    Type-Text 'At least 20 '; Add-Inline $PNG_BLUE 40 40; Type-Text ' second line of ordinary text.'
    $p = Last-Format; $p.LineSpacingRule = 3; $p.LineSpacing = 20; End-Para
    Type-Text 'Exactly 30 '; Add-Inline $PNG_BLUE 40 40; Type-Text ' second line of ordinary text.'
    $p = Last-Format; $p.LineSpacingRule = 4; $p.LineSpacing = 30; End-Para
    Type-Text 'Bordered '; Add-Inline $PNG_GREEN 60 30 { param($i) $i.Line.Visible = -1; $i.Line.Weight = 6; $i.Line.ForeColor.RGB = 0 }; Type-Text ' picture with a 6 pt border.'; End-Para
    Type-Text 'Centred: '; Add-Inline $PNG_RED 50 25; $p = Last-Format; $p.Alignment = 1; End-Para
    Type-Text 'Two pictures '; Add-Inline $PNG_RED 24 12; Add-Inline $PNG_BLUE 24 36; Type-Text ' side by side.'; End-Para
    # Last: the picture's run is 24 pt, which raises the line above what the 11 pt text and the picture alone give.
    Type-Text 'Large run '; Add-Inline $PNG_RED 12 8 { param($i) $i.Range.Font.Size = 24 }; Type-Text ' small picture in a 24 pt run.'
  }

  # Line height of a line holding an inline picture whose run has its own font (size, face): `ab <pic> cd` in 11 pt Calibri
  # with the picture's run at $size pt in $font. A marker paragraph before and after each case lets the oracle's baselines
  # give the case line's ascent and descent. Measured rule: crates/layout/src/metrics.rs, docs/architecture/word-layout.md.
  function Marker() { Type-Text 'M'; End-Para }
  # Case paragraph: `ab <picture> cd`, the picture $w x $h pt in a run of $size pt $font ($size 0: the paragraph's own), under
  # the line spacing $rule/$val (WdLineSpacing; $rule -1: single).
  function Pic-Case($font, $size, $h, $rule = -1, $val = 0, $text = $true) {
    Marker
    if ($text) { Type-Text 'ab' }
    Add-Inline $PNG_RED 24 $h { param($i) if ($size -gt 0) { $i.Range.Font.Size = $size }; $i.Range.Font.Name = $font }
    if ($text) { Type-Text 'cd' }
    if ($rule -ge 0) { $p = Last-Format; $p.LineSpacingRule = $rule; $p.LineSpacing = $val }
    End-Para
  }
  L-Fixture 'layout-img-line' 'layout-img-line' 'line height with an inline picture in a run of another size or face: picture height against the run font height, single spacing' {
    Init-Doc 0
    foreach ($sz in 11, 24, 36) { foreach ($h in 4, 8, 16, 30) { Pic-Case 'Calibri' $sz $h } }
    Pic-Case 'Calibri' 11 8 -1 0 $false
    Pic-Case 'Calibri' 24 8 -1 0 $false
    Marker
  }

  # Faces whose hhea line gap is not covered by the Windows metrics (Arial, Times New Roman) next to ones that are (Cambria, Courier New;
  # the bundled Caladea has other vertical metrics than Cambria, so Cambria cannot go in a bundled-font fixture):
  # 24 pt text, a picture alone in a 24 pt run, and a picture in a 24 pt run among 11 pt Calibri text. System fonts only.
  L-Fixture 'layout-img-face' 'layout-img-face' 'line height of 24 pt text and of an inline picture in a 24 pt run, in Cambria, Arial, Times New Roman and Courier New' {
    Init-Doc 0
    foreach ($f in 'Cambria', 'Arial', 'Times New Roman', 'Courier New') {
      Marker
      $s = $script:word.Selection; [void]$s.EndKey(6); $s.Font.Name = $f; $s.Font.Size = 24; $s.TypeText('abcd'); End-Para
      Pic-Case $f 24 8 -1 0 $false
      Pic-Case $f 24 8
    }
    Marker
  }

  L-Fixture 'layout-img-spacing' 'layout-img-spacing' 'line spacing rules with an inline picture: multiple (above and below one), at least and exactly, against picture height and run font' {
    Init-Doc 0
    foreach ($spec in @(@(5, 18), @(5, 24), @(5, 9.6), @(3, 20), @(3, 8), @(4, 30), @(4, 10))) {
      $rule = [int]$spec[0]; $val = [single]$spec[1]
      Marker; Type-Text 'abcd'; $p = Last-Format; $p.LineSpacingRule = $rule; $p.LineSpacing = $val; End-Para
      Pic-Case 'Calibri' 11 40 $rule $val
      Pic-Case 'Calibri' 24 8 $rule $val
      Pic-Case 'Calibri' 24 30 $rule $val
      Pic-Case 'Calibri' 11 8 $rule $val $false
    }
    Marker
  }

  $LONG1 = 'The harbour wall keeps the swell away from the moored boats, and the old lamps along the quay come on one by one as the light fades behind the hills. A few walkers stop to read the notice boards, and a dog sleeps in the doorway of the chandler.'
  $LONG2 = 'Careful typesetting never relies on luck: every paragraph is measured, every break is chosen, and the space around an illustration is set by rules that can be written down. A reader notices only when those rules are broken, and then the page feels wrong.'

  L-Fixture 'layout-img-tb' 'layout-img-tb' 'floating pictures with top and bottom wrapping: positions relative to paragraph, margin and page, with distances' {
    Init-Doc 6
    L-Para $LONG1; L-Para $LONG2; L-Para $LONG1; L-Para $LONG2; L-Para $LONG1; L-Para $LONG2; L-Para $LONG1; L-Para $LONG2
    [void](Add-Float 2 $PNG_RED 100 50 4 0 -999998 2 0)
    [void](Add-Float 4 $PNG_BLUE 80 80 4 0 -999995 2 20)
    [void](Add-Float 6 $PNG_GREEN 160 40 4 0 -999996 2 40 @(14, 20, 0, 0))
    [void](Add-Float 1 $PNG_RED 90 60 4 1 200 1 500)   # anchored in the first paragraph: Word also lets an anchor further down move earlier lines, which is not modelled
  }

  L-Fixture 'layout-img-square' 'layout-img-square' 'floating pictures with square wrapping: left, right and centre, distances, wrap sides, tight' {
    Init-Doc 6
    L-Para ($LONG1 + ' ' + $LONG2 + ' ' + $LONG1 + ' ' + $LONG2)
    L-Para ($LONG2 + ' ' + $LONG1 + ' ' + $LONG2 + ' ' + $LONG1)
    L-Para ($LONG1 + ' ' + $LONG2 + ' ' + $LONG1 + ' ' + $LONG2)
    L-Para ($LONG2 + ' ' + $LONG1 + ' ' + $LONG2 + ' ' + $LONG1)
    L-Para ($LONG1 + ' ' + $LONG2 + ' ' + $LONG1 + ' ' + $LONG2)
    L-Para ($LONG2 + ' ' + $LONG1 + ' ' + $LONG2 + ' ' + $LONG1)
    [void](Add-Float 1 $PNG_RED 100 60 0 0 -999998 2 0)
    [void](Add-Float 2 $PNG_BLUE 90 90 0 0 -999996 2 0 @(0, 0, 10, 10))
    [void](Add-Float 3 $PNG_GREEN 120 50 0 0 -999996 2 0 @(6, 6, 6, 6))
    [void](Add-Float 4 $PNG_RED 80 70 0 0 -999998 2 12 @(0, 0, 18, 18) 2)
    [void](Add-Float 5 $PNG_BLUE 100 60 1 0 -999998 2 0)
    [void](Add-Float 6 $PNG_GREEN 100 60 0 0 -999998 2 0 @(0, 0, 0, 0) 3)
  }

  L-Fixture 'layout-img-none' 'layout-img-none' 'floating pictures in front of and behind text: no displacement' {
    Init-Doc 6
    L-Para $LONG1; L-Para $LONG2; L-Para $LONG1
    [void](Add-Float 1 $PNG_RED 100 50 3 0 300 2 5)
    [void](Add-Float 2 $PNG_BLUE 80 40 5 1 400 1 200)
  }

} finally {
  if ($word) {
    try { $word.UserName = $oldUser; $word.UserInitials = $oldInit; $word.Options.UseLocalUserInfo = $oldLocal } catch {}
    try { $word.Quit(0) } catch {}; Release $word
  }
  if ($imgDir) { Remove-Item $imgDir -Recurse -Force -ErrorAction SilentlyContinue }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($p in $wordPids) { Get-Process -Id $p -ErrorAction SilentlyContinue | Stop-Process -Force }
}
Write-Output "created: $($created -join ', ')"
if ($skipped.Count) { Write-Output 'SKIPPED:'; $skipped | ForEach-Object { Write-Output "  $_" } }
