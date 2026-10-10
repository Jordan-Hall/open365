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
  Slower alternatives rejected: per-character Range.Information (thousands of COM calls), and
  Selection.HomeKey/EndKey/MoveDown stepping (3-4 calls per line plus selection side effects).
#>
param(
  [string[]]$Path,
  [string]$InputDir,  # optional: scan this directory (in addition to -Path)
  [string]$OutDir,    # optional: write <OutDir>/<name>/layout.json instead of fixtures/reference/layout/<name>.json
  [switch]$NoPdf   # skip the PDF baseline pass (then baseline/inkLeft are null)
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
    $script:word.Visible = $false; $script:word.DisplayAlerts = 0
    $script:wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $script:wordBefore })
  }
  $script:word
}
function Open-Doc($path) {
  $w = Get-Word
  $w.Documents.GetType().InvokeMember('Open', [Reflection.BindingFlags]::InvokeMethod, $null, $w.Documents,
    [object[]]@($path, $false, $true, $false, $false, $true, $false, $true), $null, $null,
    [string[]]@('FileName', 'ConfirmConversions', 'ReadOnly', 'AddToRecentFiles', 'Revert', 'Visible', 'OpenAndRepair', 'NoEncodingDialog'))
}

# Parse a Word-exported PDF: per page, the distinct text baselines (points from page top) and the left-most
# first-ink x of each baseline group, top to bottom.
function Read-PdfLines([string]$pdf) {
  $bytes = [IO.File]::ReadAllBytes($pdf)
  $txt = [Text.Encoding]::Latin1.GetString($bytes)
  $pages = New-Object System.Collections.Generic.List[object]
  $idx = 0
  while (($s = $txt.IndexOf('stream', $idx, [StringComparison]::Ordinal)) -ge 0) {
    $e = $txt.IndexOf('endstream', $s, [StringComparison]::Ordinal); if ($e -lt 0) { break }
    $st = $s + 6; if ($txt[$st] -eq "`r") { $st++ }; if ($txt[$st] -eq "`n") { $st++ }
    $idx = $e + 9
    $len = $e - $st; if ($len -lt 8) { continue }
    try {
      $ms = New-Object IO.MemoryStream($bytes, ($st + 2), ($len - 2))
      $ds = New-Object IO.Compression.DeflateStream($ms, [IO.Compression.CompressionMode]::Decompress)
      $o = New-Object IO.MemoryStream; $ds.CopyTo($o)
      $c = [Text.Encoding]::Latin1.GetString($o.ToArray())
    } catch { continue }
    if ($c -notmatch '\bBT\b' -or $c -notmatch ' Tm') { continue }
    $ph = 792.0
    $m = [regex]::Match($c, '(?m)^[\d.\-]+ [\d.\-]+ ([\d.\-]+) ([\d.\-]+) re')
    if ($m.Success) { $ph = [double]::Parse($m.Groups[2].Value, $inv) }
    $byY = [ordered]@{}
    foreach ($t in [regex]::Matches($c, '1 0 0 1 ([\d.\-]+) ([\d.\-]+) Tm')) {
      $x = [double]::Parse($t.Groups[1].Value, $inv); $y = [double]::Parse($t.Groups[2].Value, $inv)
      $key = [math]::Round($y, 2).ToString($inv)
      if (-not $byY.Contains($key)) { $byY[$key] = @{ y = $y; x = $x } } elseif ($x -lt $byY[$key].x) { $byY[$key].x = $x }
    }
    $rows = @($byY.Values | Sort-Object { - $_.y } | ForEach-Object { @{ baseline = $ph - $_.y; x = $_.x } })
    $pages.Add($rows)
  }
  , $pages
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
          $c = $doc.Range($s, $s)
          $lines.Add([ordered]@{
              page = $pi; start = $s; end = $e
              top = [double]$c.Information(6); left = [double]$c.Information(5)
              lastX = [double]$doc.Range([math]::Max($s, $e - 1), [math]::Max($s, $e - 1)).Information(5)
              text = [string]$r.Text
            })
        }
      }
    }
    # exact baselines from the print layout
    $baselines = $null
    if (-not $NoPdf) {
      $pdf = Join-Path $env:TEMP ("o365_layout_{0}_{1}.pdf" -f $name, $PID)
      $doc.ExportAsFixedFormat($pdf, 17)
      $baselines = Read-PdfLines $pdf
    }
    $byPage = @{}; foreach ($l in $lines) { if (-not $byPage.ContainsKey($l.page)) { $byPage[$l.page] = New-Object System.Collections.Generic.List[object] }; $byPage[$l.page].Add($l) }
    foreach ($l in $lines) { $l['baseline'] = $null; $l['inkLeft'] = $null }
    $warn = @()
    if ($baselines) {
      if ($baselines.Count -ne $pcount) { $warn += "PDF has $($baselines.Count) text pages, Word layout has $pcount" }
      for ($pi = 1; $pi -le [math]::Min($pcount, $baselines.Count); $pi++) {
        if (-not $byPage.ContainsKey($pi)) { continue }
        $inked = $byPage[$pi].ToArray()   # every line, even an empty paragraph, prints a space glyph for its mark
        $rows = @($baselines[$pi - 1])
        if ($inked.Count -ne $rows.Count) { $warn += "page ${pi}: $($inked.Count) inked lines vs $($rows.Count) PDF baselines; baselines left null"; continue }
        for ($k = 0; $k -lt $inked.Count; $k++) { $inked[$k]['baseline'] = $rows[$k].baseline; $inked[$k]['inkLeft'] = $rows[$k].x }
      }
    }
    $sb = New-Object Text.StringBuilder
    [void]$sb.Append("{`n")
    [void]$sb.Append('  "source": ' + (Jstr (Rel $f)) + ",`n")
    [void]$sb.Append('  "oracle": "Microsoft Word 16.0 (' + $w.Build + '), Print Layout, tools/office/word-layout-oracle.ps1"' + ",`n")
    [void]$sb.Append('  "units": "points; page and top/left/baseline measured from the page top-left corner; start/end are UTF-16 offsets in the main story"' + ",`n")
    [void]$sb.Append('  "precision": "top/left/lastX are screen-quantised (+-0.3 pt); baseline/inkLeft come from the 600 dpi PDF and are exact"' + ",`n")
    [void]$sb.Append('  "pageCount": ' + $pcount + ",`n")
    [void]$sb.Append('  "lines": [' + "`n")
    for ($i = 0; $i -lt $lines.Count; $i++) {
      $l = $lines[$i]
      $row = '    {"page": ' + $l.page + ', "start": ' + $l.start + ', "end": ' + $l.end + ', "top": ' + (Num $l.top) + ', "left": ' + (Num $l.left) +
        ', "lastX": ' + (Num $l.lastX) + ', "baseline": ' + (Num $l.baseline) + ', "inkLeft": ' + (Num $l.inkLeft) + ', "text": ' + (Jstr $l.text) + '}'
      if ($i -lt $lines.Count - 1) { $row += ',' }
      [void]$sb.Append($row + "`n")
    }
    [void]$sb.Append("  ]`n}`n")
    $jsonPath = if ($perDocDirs) { New-Item -ItemType Directory -Force (Join-Path $outDir $name) | Out-Null; Join-Path $outDir "$name\layout.json" } else { Join-Path $outDir "$name.json" }
    [IO.File]::WriteAllText($jsonPath, $sb.ToString().Replace("`r`n", "`n"), (New-Object Text.UTF8Encoding $false))
    $produced.Add("layout/$name ($($lines.Count) lines, $pcount pages)")
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
