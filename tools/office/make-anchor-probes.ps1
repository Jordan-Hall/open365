<#
Generates the anchor and hyperlink probes (fixtures/docx/layout-anc-*.docx + .toml, the directory every parity test reads)
by driving the locally installed Microsoft Word over COM. Each probe is Letter paper, 72 pt margins, Calibri 11,
single spacing, no space before or after, with generic text, hyperlinks and anchored objects built through the object model. The reference JSON (every line's page, offset, exact
baseline/x and every cell rectangle) comes from tools/office/word-layout-oracle.ps1:

  pwsh -NoProfile -File tools/office/make-anchor-probes.ps1
  pwsh -NoProfile -File tools/office/word-layout-oracle.ps1 -Path fixtures/docx/layout-anc-hyperlink.docx   # one per probe

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
      Set-Content -Path (Join-Path $script:outDir ("$name.toml")) -Value ("feature = `"layout-anc`"
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


  # A copy of a saved probe with word/document.xml rewritten by $transform (text -> text): attributes Word itself
  # never writes, as the Microsoft templates have them.
  function D-Fixture($from, $name, $notes, [scriptblock]$transform) {
    if ($script:Only.Count -gt 0 -and $name -notin $script:Only) { return }
    $src = Join-Path $script:outDir "$from.docx"; $path = Join-Path $script:outDir "$name.docx"
    if (-not (Test-Path $src)) { $script:skipped.Add("$name : $from.docx missing (create it first)"); return }
    Copy-Item $src $path -Force
    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::Open($path, 'Update')
    try {
      $e = $zip.GetEntry('word/document.xml')
      $sr = New-Object System.IO.StreamReader($e.Open()); $xml = $sr.ReadToEnd(); $sr.Dispose(); $e.Delete()
      $xml = & $transform $xml
      $ne = $zip.CreateEntry('word/document.xml')
      $sw = New-Object System.IO.StreamWriter($ne.Open(), (New-Object System.Text.UTF8Encoding($false))); $sw.Write($xml); $sw.Dispose()
    } finally { $zip.Dispose() }
    Set-Content -Path (Join-Path $script:outDir ("$name.toml")) -Value ("feature = `"layout-anc`"
made_with = `"$($script:word.Name) $($script:word.Build) (Windows), document.xml edited`"
author = `"project member`"
expect_dropped = []
notes = `"$notes`"
") -Encoding utf8 -NoNewline
    $script:created.Add($name)
  }

  # --- hyperlinks: how many story characters Word counts for the field behind each link
  C-Fixture 'layout-anc-hyperlink' 'hyperlinks with web, mail, short, long and bookmark targets and a screen tip, each followed by a tail and a one-line paragraph: how many story characters the hidden HYPERLINK field adds to the offsets' {
    $s = $word.Selection
    P 'intro'
    [void]$s.EndKey(6)
    $s.TypeText('x1 ')
    [void]$doc.Hyperlinks.Add($s.Range, 'http://a.example/', '', '', 'one')
    [void]$s.EndKey(6); $s.TypeText(' tail1'); $s.TypeParagraph()
    $s.TypeText('x2 ')
    [void]$doc.Hyperlinks.Add($s.Range, 'http://a.example/some/much/longer/path/to/a/page.html?query=1&other=2', '', '', 'two')
    [void]$s.EndKey(6); $s.TypeText(' tail2'); $s.TypeParagraph()
    $s.TypeText('x3 ')
    [void]$doc.Hyperlinks.Add($s.Range, 'mailto:someone@a.example', '', '', 'three')
    [void]$s.EndKey(6); $s.TypeText(' tail3'); $s.TypeParagraph()
    $s.TypeText('x4 ')
    [void]$doc.Hyperlinks.Add($s.Range, 'http://a.example/', '', 'a screen tip', 'four')
    [void]$s.EndKey(6); $s.TypeText(' tail4'); $s.TypeParagraph()
    $s.TypeText('target')
    [void]$doc.Bookmarks.Add('bm1', $s.Range)
    $s.TypeParagraph()
    $s.TypeText('x5 ')
    [void]$doc.Hyperlinks.Add($s.Range, '', 'bm1', '', 'five')
    [void]$s.EndKey(6); $s.TypeText(' tail5'); $s.TypeParagraph()
    $s.TypeText('x6 ')
    [void]$doc.Hyperlinks.Add($s.Range, 'http://a.example/', '', '', 'six')
    [void]$s.EndKey(6); $s.TypeText(' '); [void]$doc.Hyperlinks.Add($s.Range, 'http://b.example/', '', '', 'seven')
    [void]$s.EndKey(6); $s.TypeText(' tail6'); $s.TypeParagraph()
    P 'last'
  }


  # --- hyperlinks in header, footer and note parts: their relationship ids (rId1...) belong to that part, not to the main part
  C-Fixture 'layout-anc-hyperlink-hf' 'hyperlinks in the primary header, the primary footer, a footnote and an endnote, each followed by a tail, plus one in the body: whether the hidden HYPERLINK field of those parts has the length of the URL in the part own relationships' {
    $s = $word.Selection
    $sec = $doc.Sections.Item(1)
    $h = $sec.Headers.Item(1).Range
    $h.Text = 'head '
    $h.Collapse(0)
    [void]$doc.Hyperlinks.Add($h, 'http://h.example/header/path/page.html', '', '', 'hlink')
    $sec.Headers.Item(1).Range.InsertAfter(' htail')
    $f = $sec.Footers.Item(1).Range
    $f.Text = 'foot '
    $f.Collapse(0)
    [void]$doc.Hyperlinks.Add($f, 'http://f.example/', '', '', 'flink')
    $sec.Footers.Item(1).Range.InsertAfter(' ftail')
    P 'intro text before the note'
    $s.TypeText('x1 ')
    [void]$doc.Hyperlinks.Add($s.Range, 'http://a.example/body', '', '', 'one')
    [void]$s.EndKey(6); $s.TypeText(' tail1'); $s.TypeParagraph()
    $s.TypeText('note anchor here')
    $fn = $doc.Footnotes.Add($s.Range, [Type]::Missing, 'note ')
    $nr = $fn.Range
    $nr.Collapse(0)
    [void]$doc.Hyperlinks.Add($nr, 'http://n.example/some/note/target', '', '', 'nlink')
    $fn.Range.InsertAfter(' ntail')
    [void]$s.EndKey(6); $s.TypeParagraph()
    $s.TypeText('endnote anchor here')
    $en = $doc.Endnotes.Add($s.Range, [Type]::Missing, 'end ')
    $er = $en.Range
    $er.Collapse(0)
    [void]$doc.Hyperlinks.Add($er, 'http://e.example/an/endnote/target', '', '', 'elink')
    $en.Range.InsertAfter(' etail')
    [void]$s.EndKey(6); $s.TypeParagraph()
    P 'last paragraph'
  }


  D-Fixture 'layout-anc-hyperlink' 'layout-anc-hyperlink-nohist' 'layout-anc-hyperlink with every w:history attribute removed (the Microsoft templates have none): whether the field code length changes' { param($x) $x -replace ' w:history="1"', '' }
  D-Fixture 'layout-anc-hyperlink' 'layout-anc-hyperlink-hist0' 'layout-anc-hyperlink with w:history="0": whether the field code length changes' { param($x) $x -replace ' w:history="1"', ' w:history="0"' }
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
