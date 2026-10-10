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
  function L-Fixture($name, $feature, $notes, [scriptblock]$build) {
    $doc = $null; $path = Join-Path $docxDir "$name.docx"
    try {
      if (Test-Path $path) { Remove-Item $path -Force }
      $doc = $script:word.Documents.Add()
      $script:doc = $doc
      & $build
      $doc.RemovePersonalInformation = $true
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
