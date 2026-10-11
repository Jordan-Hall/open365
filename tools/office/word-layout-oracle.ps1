<#
Word layout oracle: records, for every line of the main story, where Microsoft Word 16.0 actually put it.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this
is never run in CI. CI consumes the committed output: fixtures/reference/layout/<name>.json.

  pwsh -NoProfile -File tools/office/word-layout-oracle.ps1 -Path fixtures/docx            # all fixtures
  pwsh -NoProfile -File tools/office/word-layout-oracle.ps1 -Path fixtures/docx/layout-wrap.docx

Each document is opened read-only (never saved) in a Word instance started by this script; only that
instance is quit/killed. JSON contains repo-relative paths only, LF line endings, no user info.

HOW (fast path; ~0.4 s for a 7-line page, ~0.05 s per additional page, no per-character COM calls)
  1. Window.Panes(1).Pages(i).Rectangles(j).Lines(k): Word's own line objects. For a text rectangle
     (RectangleType 0 = main text) each Line gives Range.Start / Range.End, so the line break positions
     are exact and `start` is the document offset of the line's first character (UTF-16 units, paragraph
     marks, tabs and line-break characters each count 1, same as Range.Start).
  2. Information(wdVerticalPositionRelativeToPage = 6) and Information(wdHorizontalPositionRelativeToPage = 5)
     on a collapsed range at the line start give `top` / `left`. WARNING: these are quantised by the
     screen layout to device pixels (0.6 pt on a 125 % display, 0.75 pt at 100 %), so they are only
     accurate to +-0.3 pt. Line.Top/Height/Rectangle are integers and worse.
  3. For exact geometry the document is also exported to PDF (ExportAsFixedFormat, the print layout at
     600 dpi); the content streams are inflated and each text line's baseline (`baseline`, points from
     page top) and first-ink x (`inkLeft`) are read from the `Tm` operators. Every line, even an empty
     paragraph, prints a glyph (its paragraph mark), so lines and baselines pair one to one in order.
     If the counts differ on a page (headers, footnotes, tables, shapes add baselines) that page's
     baselines are left null and a warning is printed (never silently misaligned).
  The document window must exist (Documents.Open Visible=true) although the Word application stays hidden.
  4. `lastX` is Information(5) at the line's last character (its left edge): the right end of the text
     for a line ending in a paragraph mark or a space, so it exposes justification, centring, right
     alignment and tab widths. Quantised like `left`.
  5. Tables. Word reports a whole table row as one "line", so table content is read from the PDF instead: Word clips
     the content of each table cell to the cell rectangle, so every drawn text row is known to be in a cell, and the
     clip rectangles are the cell boxes ("cells": page, x, y (top), w, h in points; one per cell fragment per page).
     Row text is decoded with the font's ToUnicode map and matched to the table paragraph (and offset) whose
     unconsumed text starts with it, which gives every cell line the same `start` key as a body line; fixtures use
     unique tokens per cell. A row that matches no unconsumed text but occurs in a paragraph is a repeated header row
     ("repeat": true). Lines in cells carry "cell": true. Rectangles carry the 0.024 pt PDF origin offset as glyphs do.
  6. Pictures. Word's PDF draws each picture with `w 0 0 h x y cm /ImageN Do`; the rectangles are written as "images"
     (page, x, y (top), w, h in points, page top-left origin, same 0.024 pt x offset as glyphs).
  Slower alternatives rejected: per-character Range.Information (thousands of COM calls), and
  Selection.HomeKey/EndKey/MoveDown stepping (3-4 calls per line plus selection side effects).
#>
param(
  [string[]]$Path,
  [string]$InputDir,  # optional: scan this directory (in addition to -Path)
  [string]$OutDir,    # optional: write <OutDir>/<name>/layout.json instead of fixtures/reference/layout/<name>.json
  [switch]$NoPdf,  # skip the PDF baseline pass (then baseline/inkLeft are null)
  [switch]$Rules   # also write "rules": every filled rectangle of the PDF (paragraph borders, rules, shading), page-top origin
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$refRoot = Join-Path $root 'fixtures\reference'
$perDocDirs = [bool]$OutDir
$outDir = if ($OutDir) { [IO.Path]::GetFullPath($OutDir) } else { Join-Path $refRoot 'layout' }
if ($InputDir) { $Path = @(@($Path) | Where-Object { $_ }) + $InputDir }
$inv = [Globalization.CultureInfo]::InvariantCulture
$skipped = New-Object System.Collections.Generic.List[string]
$produced = New-Object System.Collections.Generic.List[string]
Add-Type -AssemblyName System.IO.Compression

function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }
function Rel($p) { ([IO.Path]::GetRelativePath($root, $p)) -replace '\\', '/' }
function Num($v, $digits = 3) { if ($null -eq $v) { 'null' } else { ([math]::Round([double]$v, $digits)).ToString('0.0##', $inv) } }
function Jstr([string]$s) {
  $sb = New-Object Text.StringBuilder; [void]$sb.Append('"')
  foreach ($ch in $s.ToCharArray()) {
    $c = [int]$ch
    if ($ch -eq '"') { [void]$sb.Append('\"') } elseif ($ch -eq '\') { [void]$sb.Append('\\') }
    elseif ($c -lt 32 -or $c -gt 126) { [void]$sb.Append('\u' + $c.ToString('x4')) } else { [void]$sb.Append($ch) }
  }
  [void]$sb.Append('"'); $sb.ToString()
}

$files = New-Object System.Collections.Generic.List[string]
foreach ($p in $Path) {
  $rp = (Resolve-Path -LiteralPath $p).Path
  if (Test-Path -LiteralPath $rp -PathType Container) {
    Get-ChildItem -LiteralPath $rp -Recurse -File -Include *.docx |
      Where-Object { $_.Name -notlike '~$*' -and $_.FullName -notlike "$refRoot*" } |
      Sort-Object FullName | ForEach-Object { $files.Add($_.FullName) }
  } else { $files.Add($rp) }
}
if ($files.Count -eq 0) { Write-Output 'no .docx files found'; exit 1 }
New-Item -ItemType Directory -Force $outDir | Out-Null

$wordBefore = Get-Pids 'WINWORD'
$word = $null; $wordPids = @()
function Get-Word {
  if ($null -eq $script:word) {
    $script:word = New-Object -ComObject Word.Application
    $script:wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $script:wordBefore })
    if ($script:wordPids.Count -eq 0) { throw 'COM attached to a Word instance this script did not start; refusing to touch it' }
    $script:word.Visible = $false; $script:word.DisplayAlerts = 0
    $script:word.AutomationSecurity = 3  # msoAutomationSecurityForceDisable: macros never run
  }
  $script:word
}
function Open-Doc($path) {
  $w = Get-Word
  $w.Documents.GetType().InvokeMember('Open', [Reflection.BindingFlags]::InvokeMethod, $null, $w.Documents,
    [object[]]@($path, $false, $true, $false, $false, $true, $false, $true), $null, $null,
    [string[]]@('FileName', 'ConfirmConversions', 'ReadOnly', 'AddToRecentFiles', 'Revert', 'Visible', 'OpenAndRepair', 'NoEncodingDialog'))
}

# Parse a Word-exported PDF into pages. Each page: width/height (points), rows (text lines in drawing order) and cells
# (the distinct clip rectangles of table cells). A row: y = baseline from the page top, x = left-most first-ink x,
# text (decoded through the font's ToUnicode map), clip = index into the page's cells, or -1 for text outside any
# table cell. Word clips the content of every table cell to the cell rectangle and all other text to the page.
function Read-PdfPages([string]$pdf) {
  $bytes = [IO.File]::ReadAllBytes($pdf)
  $txt = [Text.Encoding]::Latin1.GetString($bytes)
  $streams = @{}   # object number -> decoded stream text
  foreach ($m in [regex]::Matches($txt, '(\d+) 0 obj\s*<<((?:(?!endobj).)*?)>>\s*stream\r?\n', 'Singleline')) {
    $st = $m.Index + $m.Length
    $e = $txt.IndexOf('endstream', $st, [StringComparison]::Ordinal); if ($e -lt 0) { continue }
    $len = $e - $st; if ($len -lt 3) { continue }
    try {
      if ($m.Groups[2].Value -match 'FlateDecode') {
        $ms = New-Object IO.MemoryStream($bytes, ($st + 2), ($len - 2))
        $ds = New-Object IO.Compression.DeflateStream($ms, [IO.Compression.CompressionMode]::Decompress)
        $o = New-Object IO.MemoryStream; $ds.CopyTo($o)
        $streams[[int]$m.Groups[1].Value] = [Text.Encoding]::Latin1.GetString($o.ToArray())
      } else { $streams[[int]$m.Groups[1].Value] = $txt.Substring($st, $len) }
    } catch { }
  }
  $objs = @{}
  foreach ($m in [regex]::Matches($txt, '(\d+) 0 obj\s*<<((?:(?!endobj).)*?)>>\s*(?:endobj|stream)', 'Singleline')) { $objs[[int]$m.Groups[1].Value] = $m.Groups[2].Value }
  # ToUnicode maps by font object
  $cmaps = @{}
  foreach ($k in $objs.Keys) {
    $tu = [regex]::Match($objs[$k], '/ToUnicode (\d+) 0 R')
    if (-not $tu.Success -or -not $streams.ContainsKey([int]$tu.Groups[1].Value)) { continue }
    $cm = $streams[[int]$tu.Groups[1].Value]; $map = @{}
    foreach ($b in [regex]::Matches($cm, 'beginbfchar(.*?)endbfchar', 'Singleline')) {
      foreach ($p in [regex]::Matches($b.Groups[1].Value, '<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>')) {
        $s = ''; $h = $p.Groups[2].Value
        for ($i = 0; $i -lt $h.Length; $i += 4) { $s += [char][Convert]::ToInt32($h.Substring($i, [math]::Min(4, $h.Length - $i)), 16) }
        $map[[Convert]::ToInt32($p.Groups[1].Value, 16)] = $s
      }
    }
    foreach ($b in [regex]::Matches($cm, 'beginbfrange(.*?)endbfrange', 'Singleline')) {
      foreach ($p in [regex]::Matches($b.Groups[1].Value, '<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>')) {
        $lo = [Convert]::ToInt32($p.Groups[1].Value, 16); $hi = [Convert]::ToInt32($p.Groups[2].Value, 16); $d = [Convert]::ToInt32($p.Groups[3].Value, 16)
        for ($c = $lo; $c -le $hi; $c++) { $map[$c] = [string][char]($d + $c - $lo) }
      }
    }
    $cmaps[$k] = $map
  }
  $pagesObj = $objs.Keys | Where-Object { $objs[$_] -match '/Type\s*/Pages' } | Select-Object -First 1
  $kids = @([regex]::Matches([regex]::Match($objs[$pagesObj], '/Kids\s*\[(.*?)\]', 'Singleline').Groups[1].Value, '(\d+) 0 R') | ForEach-Object { [int]$_.Groups[1].Value })
  $rx = '(?<rx>[\d.\-]+) (?<ry>[\d.\-]+) (?<rw>[\d.\-]+) (?<rh>[\d.\-]+) re\r?\nW\* n|(?<fx>[\d.\-]+) (?<fy>[\d.\-]+) (?<fw>[\d.\-]+) (?<fh>[\d.\-]+) re\r?\nf\*?|(?<cr>[\d.]+) (?<cg>[\d.]+) (?<cb>[\d.]+) rg|(?<gy>[\d.]+) g\r?\n|/(?<fn>F\d+) (?<fs>[\d.]+) Tf|1 0 0 1 (?<tx>[\d.\-]+) (?<ty>[\d.\-]+) Tm|\[(?<tj>[^\]]*)\] TJ|(?<iw>[\d.\-]+) 0 0 (?<ih>[\d.\-]+) (?<ix>[\d.\-]+) (?<iy>[\d.\-]+) cm
?
/(?<im>\w+) Do'
  $pages = New-Object System.Collections.Generic.List[object]
  foreach ($po in $kids) {
    $pd = $objs[$po]
    $mb = [regex]::Match($pd, '/MediaBox\s*\[\s*[\d.\-]+\s+[\d.\-]+\s+([\d.\-]+)\s+([\d.\-]+)\s*\]')
    $pw = [double]::Parse($mb.Groups[1].Value, $inv); $ph = [double]::Parse($mb.Groups[2].Value, $inv)
    $fonts = @{}
    foreach ($fm in [regex]::Matches([regex]::Match($pd, '/Font\s*<<(.*?)>>', 'Singleline').Groups[1].Value, '/(F\d+) (\d+) 0 R')) { $fonts[$fm.Groups[1].Value] = [int]$fm.Groups[2].Value }
    $content = ''
    $cm = [regex]::Match($pd, '/Contents\s*(?:\[([^\]]*)\]|(\d+) 0 R)')
    if ($cm.Groups[2].Success) { $content = $streams[[int]$cm.Groups[2].Value] }
    else { foreach ($r in [regex]::Matches($cm.Groups[1].Value, '(\d+) 0 R')) { $content += $streams[[int]$r.Groups[1].Value] } }
    if ($env:O365_ORACLE_DUMP_DIR) { [IO.File]::WriteAllText((Join-Path $env:O365_ORACLE_DUMP_DIR ("page{0}.txt" -f ($pages.Count + 1))), $content) }
    $cells = New-Object System.Collections.Generic.List[object]
    $rows = New-Object System.Collections.Generic.List[object]
    $images = New-Object System.Collections.Generic.List[object]
    $clip = -1; $font = $null; $fsize = ''; $tx = 0.0; $ty = 0.0; $fill = '000000'; $fills = New-Object System.Collections.Generic.List[object]
    $byKey = @{}
    foreach ($m in [regex]::Matches($content, $rx)) {
      if ($m.Groups['rx'].Success) {
        $x = [double]::Parse($m.Groups['rx'].Value, $inv); $y = [double]::Parse($m.Groups['ry'].Value, $inv)
        $w = [double]::Parse($m.Groups['rw'].Value, $inv); $h = [double]::Parse($m.Groups['rh'].Value, $inv)
        if ($w -ge $pw - 2 -and $h -ge $ph - 2) { $clip = -1 }
        else {
          $clip = -1; $top = $ph - $y - $h
          for ($i = 0; $i -lt $cells.Count; $i++) { $c = $cells[$i]; if ([math]::Abs($c.x - $x) -lt 0.0005 -and [math]::Abs($c.w - $w) -lt 0.0005 -and [math]::Abs($c.top - $top) -lt 0.0005 -and [math]::Abs($c.h - $h) -lt 0.0005) { $clip = $i; break } }
          if ($clip -lt 0) { $cells.Add(@{ x = $x; top = $top; w = $w; h = $h }); $clip = $cells.Count - 1 }
        }
      } elseif ($m.Groups['fx'].Success) {
        $fh = [double]::Parse($m.Groups['fh'].Value, $inv); $fy = [double]::Parse($m.Groups['fy'].Value, $inv)
        $fills.Add(@{ x = [double]::Parse($m.Groups['fx'].Value, $inv); y = $ph - $fy - $fh; w = [double]::Parse($m.Groups['fw'].Value, $inv); h = $fh; color = $fill; clip = $clip })
      } elseif ($m.Groups['cr'].Success) {
        $fill = (@('cr', 'cg', 'cb') | ForEach-Object { '{0:x2}' -f [int][math]::Round(255 * [double]::Parse($m.Groups[$_].Value, $inv)) }) -join ''
      } elseif ($m.Groups['gy'].Success) {
        $fill = ('{0:x2}' -f [int][math]::Round(255 * [double]::Parse($m.Groups['gy'].Value, $inv))) * 3
      } elseif ($m.Groups['iw'].Success) {
        # A picture: `w 0 0 h x y cm /ImageN Do` (PDF user space, y up). x/y/w/h in points from the page top-left.
        $iw = [double]::Parse($m.Groups['iw'].Value, $inv); $ih = [double]::Parse($m.Groups['ih'].Value, $inv)
        $ix = [double]::Parse($m.Groups['ix'].Value, $inv); $iy = [double]::Parse($m.Groups['iy'].Value, $inv)
        $images.Add(@{ x = $ix; y = $ph - $iy - $ih; w = $iw; h = $ih })
      } elseif ($m.Groups['fn'].Success) { $font = $m.Groups['fn'].Value; $fsize = $m.Groups['fs'].Value }
      elseif ($m.Groups['tx'].Success) { $tx = [double]::Parse($m.Groups['tx'].Value, $inv); $ty = [double]::Parse($m.Groups['ty'].Value, $inv) }
      elseif ($m.Groups['tj'].Success) {
        $map = if ($font -and $fonts.ContainsKey($font) -and $cmaps.ContainsKey($fonts[$font])) { $cmaps[$fonts[$font]] } else { $null }
        $s = ''
        foreach ($t in [regex]::Matches($m.Groups['tj'].Value, '<([0-9A-Fa-f]*)>|\(((?:[^\\)]|\\.)*)\)')) {
          if ($t.Groups[1].Success) {
            $h = $t.Groups[1].Value
            for ($i = 0; $i + 3 -lt $h.Length; $i += 4) { $code = [Convert]::ToInt32($h.Substring($i, 4), 16); if ($map -and $map.ContainsKey($code)) { $s += $map[$code] } else { $s += ' ' } }
          } else { $s += ([regex]::Replace($t.Groups[2].Value, '\\(.)', '$1')) }
        }
        $yb = [math]::Round($ph - $ty, 3)
        $key = "$clip|$yb"
        if ($byKey.ContainsKey($key)) { $r = $byKey[$key]; $r.parts.Add(@{ x = $tx; text = $s }); if ($tx -lt $r.x) { $r.x = $tx } }
        else { $r = @{ y = $ph - $ty; x = $tx; clip = $clip; size = $fsize; parts = (New-Object System.Collections.Generic.List[object]) }; $r.parts.Add(@{ x = $tx; text = $s }); $byKey[$key] = $r; $rows.Add($r) }
      }
    }
    foreach ($r in $rows) { $r['text'] = (($r.parts | Sort-Object { $_.x } | ForEach-Object { $_.text }) -join '').Trim() }
    $pages.Add(@{ pw = $pw; ph = $ph; rows = $rows; cells = $cells; images = $images; fills = $fills })
  }
  , $pages
}

# Vertical order of the stories on a page (WdStoryType): headers, body, endnotes, footnotes, footers.
function Story-Rank($l) {
  if (-not $l.Contains('story')) { return 1.0 }
  switch ([int]$l.story) {
    { $_ -in 6, 7, 10 } { 0.0 }
    15 { 2.0 } 16 { 2.0 } 17 { 2.1 } 3 { 2.5 }
    12 { 3.0 } 13 { 3.0 } 14 { 3.1 } 2 { 3.5 }
    { $_ -in 8, 9, 11 } { 4.0 }
    default { 1.5 }
  }
}

# Paragraphs inside tables (nested ones included), by story start, with their text without the end mark.
function Walk-Table($t, $out, $seen, $tab) {
  foreach ($cell in $t.Range.Cells) {
    foreach ($p in $cell.Range.Paragraphs) {
      $rg = $p.Range; $s = [int]$rg.Start
      if ($seen.ContainsKey($s)) { continue }
      $seen[$s] = 1
      $out.Add(@{ start = $s; text = (([string]$rg.Text) -replace '[\r\a]+$', ''); used = 0; lines = 0; tab = $tab })
    }
  }
  foreach ($n in $t.Tables) { Walk-Table $n $out $seen $tab }
}

function Ref-Layout($f) {
  $name = [IO.Path]::GetFileNameWithoutExtension($f)
  $doc = $null; $pdf = $null
  try {
    $doc = Open-Doc $f
    $w = Get-Word
    $doc.Repaginate()
    $pane = $w.ActiveWindow.Panes.Item(1)
    $pcount = [int]$pane.Pages.Count
    $hasTables = [int]$doc.Tables.Count -gt 0
    $lines = New-Object System.Collections.Generic.List[object]
    for ($pi = 1; $pi -le $pcount; $pi++) {
      $page = $pane.Pages.Item($pi)
      $rects = $page.Rectangles
      $rc = [int]$rects.Count
      for ($ri = 1; $ri -le $rc; $ri++) {
        $rect = $rects.Item($ri)
        if ([int]$rect.RectangleType -ne 0) { continue }   # wdTextRectangle only (main text; skips headers, notes, shapes)
        $ls = $rect.Lines; $lc = [int]$ls.Count
        for ($li = 1; $li -le $lc; $li++) {
          $r = $ls.Item($li).Range
          $s = [int]$r.Start; $e = [int]$r.End
          $st = [int]$r.StoryType
          if ($st -ne 1) {
            # Headers, footers, footnotes, endnotes and their separators: another story, so the offsets are
            # relative to that story and the position is read from a range inside it. Optional `story` output
            # (Word's WdStoryType). Word lists some notes twice; a repeated line is dropped.
            $c = $r.Duplicate; $c.Collapse(1)
            $ce = $r.Duplicate; $ce.SetRange([math]::Max($s, $e - 1), [math]::Max($s, $e - 1))
            $top = [double]$c.Information(6)
            if ($lines.Exists([Predicate[object]]{ param($o) $o.page -eq $pi -and $o.Contains('story') -and $o.story -eq $st -and $o.start -eq $s -and $o.end -eq $e -and $o.top -eq $top })) { continue }
            $lines.Add([ordered]@{
                page = $pi; start = $s; end = $e; story = $st
                top = $top; left = [double]$c.Information(5)
                lastX = [double]$ce.Information(5)
                text = [string]$r.Text
              })
            continue
          }
          $c = $doc.Range($s, $s)
          # Word reports a whole table row as one line; table text is read from the PDF instead (below).
          if ($hasTables -and $c.Information(12)) { continue }
          $mtop = [double]$c.Information(6)
          if ($lines.Exists([Predicate[object]]{ param($o) $o.page -eq $pi -and -not $o.Contains('story') -and $o.start -eq $s -and $o.end -eq $e -and $o.top -eq $mtop })) { continue }
          $lines.Add([ordered]@{
              page = $pi; start = $s; end = $e
              top = $mtop; left = [double]$c.Information(5)
              lastX = [double]$doc.Range([math]::Max($s, $e - 1), [math]::Max($s, $e - 1)).Information(5)
              text = [string]$r.Text
            })
        }
      }
    }
    # exact baselines from the print layout
    $pdfPages = $null
    if (-not $NoPdf) {
      $pdf = Join-Path $env:TEMP ("o365_layout_{0}_{1}.pdf" -f $name, $PID)
      $doc.ExportAsFixedFormat($pdf, 17)
      $pdfPages = Read-PdfPages $pdf
    }
    $byPage = @{}; foreach ($l in $lines) { if (-not $byPage.ContainsKey($l.page)) { $byPage[$l.page] = New-Object System.Collections.Generic.List[object] }; $byPage[$l.page].Add($l) }
    foreach ($l in $lines) { $l['baseline'] = $null; $l['inkLeft'] = $null }
    $warn = @()
    $cellRects = New-Object System.Collections.Generic.List[object]
    $imageRects = New-Object System.Collections.Generic.List[object]
    $ruleRects = New-Object System.Collections.Generic.List[object]
    if ($pdfPages) {
      if ($pdfPages.Count -ne $pcount) { $warn += "PDF has $($pdfPages.Count) text pages, Word layout has $pcount" }
      for ($pi = 1; $pi -le [math]::Min($pcount, $pdfPages.Count); $pi++) {
        $pg = $pdfPages[$pi - 1]
        # A clip around a picture (within its border) is the picture, not a cell.
        # A clip that reaches the page edge is the page clipping a cell that runs off the page: not a cell box.
        if ($Rules) { foreach ($fl in $pg.fills) { $ruleRects.Add([ordered]@{ page = $pi; x = $fl.x; y = $fl.y; w = $fl.w; h = $fl.h; color = $fl.color; cell = ($fl.clip -ge 0) }) } }
        foreach ($im in $pg.images) { $imageRects.Add([ordered]@{ page = $pi; x = $im.x; y = $im.y; w = $im.w; h = $im.h }) }
        foreach ($c in $pg.cells) { if ($c.x + $c.w -ge $pg.pw - 0.01) { continue }; if (@($pg.images | Where-Object { $c.x -ge $_.x - 8 -and $c.x + $c.w -le $_.x + $_.w + 8 -and $c.top -ge $_.y - 8 -and $c.top + $c.h -le $_.y + $_.h + 8 }).Count -gt 0) { continue }; $cellRects.Add([ordered]@{ page = $pi; x = $c.x; y = $c.top; w = $c.w; h = $c.h }) }
        if (-not $byPage.ContainsKey($pi)) { continue }
        $inked = $byPage[$pi].ToArray()   # every line, even an empty paragraph, prints a space glyph for its mark
        if (@($inked | Where-Object { $_.Contains('story') }).Count -gt 0) {
          # Lines of other stories: top-to-bottom order is header, body, endnotes, footnotes, footer.
          $inked = @($inked | Sort-Object @{ Expression = { Story-Rank $_ } }, @{ Expression = { $_.start } })
        }
        $rows = @($pg.rows | Where-Object { $_.clip -lt 0 } | Sort-Object { $_.y })
        # Superscript marks (note references) are separate small rows just above their line: not lines.
        if (@($inked | Where-Object { $_.Contains('story') -or $_.text -match '[]' }).Count -gt 0) {
          $rows = @($rows | Where-Object {
              $r0 = $_
              -not ($r0.text.Length -le 3 -and $r0.text -ne '' -and @($rows | Where-Object { $_.y -gt $r0.y -and $_.y - $r0.y -le 5 -and [double]$_.size -gt 1.3 * [double]$r0.size }).Count -gt 0)
            })
          # Word prints the empty header and footer paragraphs of a page that has notes without listing them.
          $firstIsHeader = $inked.Count -gt 0 -and $inked[0].Contains('story') -and $inked[0].story -in 6, 7, 10
          $lastIsFooter = $inked.Count -gt 0 -and $inked[-1].Contains('story') -and $inked[-1].story -in 8, 9, 11
          while ($rows.Count -gt $inked.Count) {
            if ($rows[0].text -eq '' -and -not $firstIsHeader) { $rows = @($rows | Select-Object -Skip 1) }
            elseif ($rows[-1].text -eq '' -and -not $lastIsFooter) { $rows = @($rows | Select-Object -SkipLast 1) }
            else { break }
          }
        }
        if ($env:O365_ORACLE_DEBUG) { foreach ($d in $rows) { Write-Output ("PDF p{0} y={1:N3} x={2:N3} size={3} '{4}'" -f $pi, $d.y, $d.x, $d.size, $d.text) } }
        if ($inked.Count -ne $rows.Count) {
          # Separators draw a rule, not a glyph: pair the rest.
          $inkedText = @($inked | Where-Object { -not ($_.Contains('story') -and $_.story -in 12, 13, 15, 16) })
          if ($inkedText.Count -eq $rows.Count) { $inked = $inkedText }
        }
        if ($inked.Count -ne $rows.Count) { $warn += "page ${pi}: $($inked.Count) inked lines vs $($rows.Count) PDF baselines; baselines left null"; continue }
        for ($k = 0; $k -lt $inked.Count; $k++) { $inked[$k]['baseline'] = $rows[$k].y; $inked[$k]['inkLeft'] = $rows[$k].x }
      }
      # Table text: each drawn row is matched to the paragraph (and offset) whose unconsumed text starts with it.
      if ($hasTables) {
        $tp = New-Object System.Collections.Generic.List[object]; $seen = @{}
        $active = -1
        $tabId = 0; foreach ($t in $doc.Tables) { Walk-Table $t $tp $seen $tabId; $tabId++ }
        for ($pi = 1; $pi -le [math]::Min($pcount, $pdfPages.Count); $pi++) {
          foreach ($row in @($pdfPages[$pi - 1].rows | Where-Object { $_.clip -ge 0 })) {
            $t = $row.text; $hit = $null; $off = 0
            $repeat = $false
            # Cell texts are not unique across tables (every table has an R1C1w1), so a drawn row is matched
            # inside the table being drawn first: its next unconsumed text, else (while the table still has
            # unconsumed text) a repeated header row. Only then does the next table start.
            $activeLeft = $false
            if ($active -ge 0) { foreach ($p in $tp) { if ($p.tab -eq $active -and $p.text.Substring($p.used).Trim() -ne '') { $activeLeft = $true; break } } }
            foreach ($pass in 1, 2) {
              if ($hit) { break }
              foreach ($p in $tp) {
                if ($pass -eq 1 -and $activeLeft -and $p.tab -ne $active) { continue }
                $rem = $p.text.Substring($p.used); $trim = $rem.TrimStart()
                if ($t -eq '') { if ($trim -eq '' -and $p.lines -eq 0) { $hit = $p; $off = $p.used; break } }
                elseif ($trim.StartsWith($t, [StringComparison]::Ordinal)) { $hit = $p; $off = $p.used + ($rem.Length - $trim.Length); break }
              }
              if ($pass -eq 1 -and -not $hit -and $activeLeft) {
                foreach ($p in $tp) {
                  if ($p.tab -ne $active -or $p.lines -eq 0 -or $t -eq '') { continue }
                  $ix = $p.text.IndexOf($t, [StringComparison]::Ordinal)
                  if ($ix -ge 0) { $hit = $p; $off = $ix; $repeat = $true; break }
                }
              }
            }
            if (-not $hit) {
              foreach ($p in $tp) { $ix = if ($t -eq '') { -1 } else { $p.text.IndexOf($t, [StringComparison]::Ordinal) }; if ($ix -ge 0) { $hit = $p; $off = $ix; $repeat = $true; break } }
            }
            if ($hit -and -not $repeat) { $active = $hit.tab }
            if (-not $hit) { $warn += "page ${pi}: table row '$t' matches no paragraph"; continue }
            if (-not $repeat) { $hit.used = $off + $t.Length; $hit.lines++ }
            $st = $hit.start + $off
            $lines.Add([ordered]@{ page = $pi; start = $st; end = $st + $t.Length; top = $row.y; left = $row.x; lastX = $null; baseline = $row.y; inkLeft = $row.x; text = $t; cell = $true; repeat = $repeat })
          }
        }
      }
    }
    $lines = @($lines | Sort-Object @{ Expression = { $_.page } }, @{ Expression = { Story-Rank $_ } }, @{ Expression = { $_.start } }, @{ Expression = { $_.top } })
    $sb = New-Object Text.StringBuilder
    [void]$sb.Append("{`n")
    [void]$sb.Append('  "source": ' + (Jstr (Rel $f)) + ",`n")
    [void]$sb.Append('  "oracle": "Microsoft Word 16.0 (' + $w.Build + '), Print Layout, tools/office/word-layout-oracle.ps1"' + ",`n")
    [void]$sb.Append('  "units": "points; page and top/left/baseline measured from the page top-left corner; start/end are UTF-16 offsets in the main story"' + ",`n")
    [void]$sb.Append('  "precision": "top/left/lastX are screen-quantised (+-0.3 pt); baseline/inkLeft and cell rectangles come from the 600 dpi PDF and are exact"' + ",`n")
    [void]$sb.Append('  "pageCount": ' + $pcount + ",`n")
    if ($cellRects.Count -gt 0) {
      [void]$sb.Append('  "cells": [' + "`n")
      for ($i = 0; $i -lt $cellRects.Count; $i++) {
        $c = $cellRects[$i]
        $row = '    {"page": ' + $c.page + ', "x": ' + (Num $c.x) + ', "y": ' + (Num $c.y) + ', "w": ' + (Num $c.w) + ', "h": ' + (Num $c.h) + '}'
        if ($i -lt $cellRects.Count - 1) { $row += ',' }
        [void]$sb.Append($row + "`n")
      }
      [void]$sb.Append("  ],`n")
    }
    if ($imageRects.Count -gt 0) {
      [void]$sb.Append('  "images": [' + "`n")
      for ($i = 0; $i -lt $imageRects.Count; $i++) {
        $c = $imageRects[$i]
        $row = '    {"page": ' + $c.page + ', "x": ' + (Num $c.x) + ', "y": ' + (Num $c.y) + ', "w": ' + (Num $c.w) + ', "h": ' + (Num $c.h) + '}'
        if ($i -lt $imageRects.Count - 1) { $row += ',' }
        [void]$sb.Append($row + "`n")
      }
      [void]$sb.Append("  ],`n")
    }
    if ($Rules -and $ruleRects.Count -gt 0) {
      [void]$sb.Append('  "rules": [' + "`n")
      for ($i = 0; $i -lt $ruleRects.Count; $i++) {
        $c = $ruleRects[$i]
        $row = '    {"page": ' + $c.page + ', "x": ' + (Num $c.x) + ', "y": ' + (Num $c.y) + ', "w": ' + (Num $c.w) + ', "h": ' + (Num $c.h) + ', "color": "' + $c.color + '"' + $(if ($c.cell) { ', "cell": true' } else { '' }) + '}'
        if ($i -lt $ruleRects.Count - 1) { $row += ',' }
        [void]$sb.Append($row + "`n")
      }
      [void]$sb.Append("  ],`n")
    }
    [void]$sb.Append('  "lines": [' + "`n")
    for ($i = 0; $i -lt $lines.Count; $i++) {
      $l = $lines[$i]
      $row = '    {"page": ' + $l.page + ', "start": ' + $l.start + ', "end": ' + $l.end + ', "top": ' + (Num $l.top) + ', "left": ' + (Num $l.left) +
        ', "lastX": ' + (Num $l.lastX) + ', "baseline": ' + (Num $l.baseline) + ', "inkLeft": ' + (Num $l.inkLeft) + ', "text": ' + (Jstr $l.text)
      if ($l.Contains('story')) { $row += ', "story": ' + $l.story }
      if ($l.Contains('cell')) { $row += ', "cell": true' + $(if ($l.repeat) { ', "repeat": true' } else { '' }) }
      $row += '}'
      if ($i -lt $lines.Count - 1) { $row += ',' }
      [void]$sb.Append($row + "`n")
    }
    [void]$sb.Append("  ]`n}`n")
    $jsonPath = if ($perDocDirs) { New-Item -ItemType Directory -Force (Join-Path $outDir $name) | Out-Null; Join-Path $outDir "$name\layout.json" } else { Join-Path $outDir "$name.json" }
    [IO.File]::WriteAllText($jsonPath, $sb.ToString().Replace("`r`n", "`n"), (New-Object Text.UTF8Encoding $false))
    $produced.Add("layout/$name ($($lines.Count) lines, $($cellRects.Count) cells, $($imageRects.Count) images, $pcount pages)")
    foreach ($m in $warn) { Write-Output "WARN $name : $m" }
  } finally {
    if ($doc) { try { $doc.Close(0) } catch {}; Release $doc }
    if ($pdf) { Remove-Item $pdf -Force -ErrorAction SilentlyContinue }
  }
}

$sw = [Diagnostics.Stopwatch]::StartNew()
try {
  foreach ($f in $files) {
    try { Ref-Layout $f } catch { $skipped.Add("$(Rel $f) : $($_.Exception.Message.Trim())") }
  }
} finally {
  if ($word) { try { $word.Quit(0) } catch {}; Release $word }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($id in $wordPids) { Get-Process -Id $id -ErrorAction SilentlyContinue | Stop-Process -Force }
}
Write-Output ("produced: " + ($produced -join '; '))
Write-Output ("elapsed: {0:N1}s for {1} file(s)" -f $sw.Elapsed.TotalSeconds, $files.Count)
if ($skipped.Count) { Write-Output 'SKIPPED:'; $skipped | ForEach-Object { Write-Output "  $_" }; exit 1 }
