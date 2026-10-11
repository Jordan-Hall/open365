<#
Generates the table probes (fixtures/docx/layout-trh-*.docx + .toml, the directory every parity test reads)
by driving the locally installed Microsoft Word over COM. Each probe is Letter paper, 72 pt margins, Calibri 11,
single spacing, no space before or after, with generic text and tables whose row height rule, row height and
top/bottom cell padding are set through the object model. The reference JSON (every line's page, offset, exact
baseline/x and every cell rectangle) comes from tools/office/word-layout-oracle.ps1:

  pwsh -NoProfile -File tools/office/make-table-probes.ps1
  pwsh -NoProfile -File tools/office/word-layout-oracle.ps1 -Path fixtures/docx/layout-trh-pad.docx   # one per probe

LOCAL DEVELOPER TOOL ONLY (Microsoft does not support unattended server-side Office automation).
Starts its own Word instance (AutomationSecurity = force-disable macros), never attaches to or kills another one,
quits it in a finally block, and stops only the WINWORD processes it started. Document author and properties are
generic. All text is original. Nothing outside the repository is read or written (the Word user name is restored).
Constants: WdRowHeightRule auto 0, at least 1, exactly 2; WdCellVerticalAlignment top 0, center 1, bottom 3; WdBreakType page 7.
#>
param([string[]]$Only = @())   # fixture names to (re)create; default: all table probes
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
      Set-Content -Path (Join-Path $script:outDir ("$name.toml")) -Value ("feature = `"layout-trh`"
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

  $rows = @(
    @{ },
    @{ Rule = 1; Height = 30 },
    @{ Rule = 1; Height = 5 },
    @{ Rule = 2; Height = 30 },
    @{ Rule = 2; Height = 8 },
    @{ Rule = 1; Height = 20; Lines = 2 },
    @{ Rule = 1; Height = 40; Valign = 1 },
    @{ Rule = 1; Height = 40; Valign = 3 }
  )
  C-Fixture 'layout-trh-pad' 'three tables with 0, 3.6 and 10 pt top and bottom cell padding; rows of every height rule (auto, at least 30/5, exactly 30/8), a two line row, centred and bottom aligned cells: whether the row height counts the padding' {
    P 'intro'
    [void](T 'a' 0 $rows)
    P 'gap1'
    [void](T 'b' 3.6 $rows)
    P 'gap2'
    [void](T 'c' 10 $rows)
    P 'tail'
  }
  C-Fixture 'layout-trh-asym' 'padding of 12 pt above and 2 pt below with at least and exactly rows: which padding the height rule adds' {
    P 'intro'
    [void](T 'a' 12 $rows 2)
    P 'gap1'
    [void](T 'b' 2 $rows 12)
    P 'tail'
  }
  $small = @(
    @{ }, @{ Rule = 2; Height = 4 },
    @{ }, @{ Rule = 2; Height = 8 },
    @{ }, @{ Rule = 2; Height = 11.5 },
    @{ }, @{ Rule = 2; Height = 12 },
    @{ }, @{ Rule = 2; Height = 12.5 },
    @{ }, @{ Rule = 2; Height = 16 },
    @{ }, @{ Rule = 2; Height = 24 },
    @{ }
  )
  C-Fixture 'layout-trh-small' 'tables whose exact rows (4 to 24 pt) sit between auto rows, with cell padding 12/2, 2/12, 12/12 and 6/6 pt above and below: how high an exact row is when its height does not exceed the padding' {
    P 'intro'
    [void](T 'a' 12 $small 2)
    P 'gap1'
    [void](T 'b' 2 $small 12)
    P 'gap2'
    [void](T 'c' 12 $small 12)
    P 'gap3'
    [void](T 'd' 6 $small 6)
    P 'tail'
  }
  C-Fixture 'layout-trh-tall' 'one row per page, at least 540..700 pt high with 0, 36 and 72 pt padding, followed by a paragraph: what Word does when the height and the padding together exceed the page' {
    $cases = @(@(540, 36), @(600, 36), @(640, 0), @(700, 0), @(560, 72), @(400, 72))
    $i = 0
    foreach ($c in $cases) {
      $i++
      if ($i -gt 1) { Brk 7 }
      [void](T "p$i" $c[1] @(@{ Rule = 1; Height = $c[0] }))
      P "after$i"
    }
  }

  C-Fixture 'layout-sect-tail' 'five next-page sections with 46 to 50 single-line paragraphs each, the break in an empty paragraph of its own: where the section break paragraph lands when it does not fit the page' {
    foreach ($k in 46..50) {
      for ($i = 1; $i -le $k; $i++) { P "s$($k)l$($i)" }
      Brk 2
    }
    P 'last'
  }

  C-Fixture 'layout-sect-empty' 'sections whose break sits in an empty paragraph of its own: continuous, next page, even page and odd page, with 6 pt after on some paragraphs: whether the break paragraph takes a line' {
    P 'a1'
    P 'a2'
    Brk 3
    P 'b1'
    P 'b2'
    Brk 3
    P 'c1'
    Brk 2
    P 'd1'
    P 'd2'
    Brk 3
    P 'e1'
    Brk 4
    P 'f1'
    Brk 5
    P 'g1'
    Brk 3
    P 'h1'
  }

  C-Fixture 'layout-sect-space' 'like layout-sect-empty with 3 pt before and 8 pt after in Normal (and a 14 pt break paragraph): whether the space of an empty section break paragraph counts' {
    $n = $doc.Styles.Item('Normal'); $n.ParagraphFormat.SpaceBefore = 3; $n.ParagraphFormat.SpaceAfter = 8
    P 'a1'
    P 'a2'
    Brk 3
    P 'b1'
    P 'b2'
    Brk 3
    P 'c1'
    Brk 2
    P 'd1'
    P 'd2'
    $s = $word.Selection; [void]$s.EndKey(6)
    $s.Font.Size = 24
    Brk 3
    $s.Font.Size = 11
    P 'e1'
    P 'e2'
    Brk 2
    P 'f1'
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
