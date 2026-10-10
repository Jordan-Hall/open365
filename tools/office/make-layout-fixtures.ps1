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
      $script:skipped.Add("docx/$name : $($_.Exception.Message)")
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

} finally {
  if ($word) {
    try { $word.UserName = $oldUser; $word.UserInitials = $oldInit; $word.Options.UseLocalUserInfo = $oldLocal } catch {}
    try { $word.Quit(0) } catch {}; Release $word
  }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($p in $wordPids) { Get-Process -Id $p -ErrorAction SilentlyContinue | Stop-Process -Force }
}
Write-Output "created: $($created -join ', ')"
if ($skipped.Count) { Write-Output 'SKIPPED:'; $skipped | ForEach-Object { Write-Output "  $_" } }
