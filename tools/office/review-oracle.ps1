<#
Word review oracle: what Microsoft Word 16.0 does with tracked changes and comments.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this is
never run in CI. CI consumes the committed outputs: fixtures/reference/docx/review-<name>/review.tsv.

  pwsh -NoProfile -File tools/office/review-oracle.ps1 -Make     # (re)creates fixtures/docx/review-*.docx
  pwsh -NoProfile -File tools/office/review-oracle.ps1           # measures every fixtures/docx/review-*.docx
  pwsh -NoProfile -File tools/office/review-oracle.ps1 -Only review-ins-del

The fixtures are generated from XML written in this script (generic authors "Reviewer A" and "Reviewer B", no
personal data). Each is opened read-only in a Word instance this script starts (AutomationSecurity = 3: macros
never run) and is never saved; only that instance is quit, in the finally block. Nothing else is touched.

review.tsv is tab separated; text fields escape backslash, tab, CR, LF and every other control or non-ASCII
character as \\, \t, \r, \n, \uXXXX. Line kinds:
  rev      index  type  author  text            Document.Revisions, in order (type = WdRevisionType name)
  markup   i      text                          paragraph i text with all markup shown (deleted text included)
  final    i      text                          paragraph i text with the markup hidden (deleted text hidden)
  accept   i      text  alignment               paragraph i after Revisions.AcceptAll
  reject   i      text  alignment               paragraph i after Revisions.RejectAll
  accept1  r  i   text                          after accepting only revision r (its paragraphs, joined by U+241E)
  reject1  r  i   text                          after rejecting only revision r
  comment  index  author  initial  scope  text  done  ancestor  replies
Paragraph text has its trailing paragraph / cell mark removed and the comment reference mark (U+0005) removed.
Paragraph indices are Document.Paragraphs order; table row-end marks are skipped.
#>
param(
  [switch]$Make,
  [string]$Only
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixDir = Join-Path $root 'fixtures\docx'
$refDir = Join-Path $root 'fixtures\reference\docx'
Add-Type -AssemblyName System.IO.Compression

# ---------------------------------------------------------------------------
# Fixture generation
# ---------------------------------------------------------------------------
$script:rid = 100
function Id { $script:rid++; $script:rid }
$NS = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="w14 w15"'
$DATE = '2026-01-02T03:04:00Z'
function Attrs($author) { "w:id=`"$(Id)`" w:author=`"$author`" w:date=`"$DATE`"" }
function RunT($t, $rpr = '') { $p = if ($rpr) { "<w:rPr>$rpr</w:rPr>" } else { '' }; "<w:r>$p<w:t xml:space=`"preserve`">$t</w:t></w:r>" }
function DelT($t) { "<w:r><w:delText xml:space=`"preserve`">$t</w:delText></w:r>" }
function InsEl($author, $inner) { "<w:ins $(Attrs $author)>$inner</w:ins>" }
function DelEl($author, $inner) { "<w:del $(Attrs $author)>$inner</w:del>" }
function Para($inner, $ppr = '') { $p = if ($ppr) { "<w:pPr>$ppr</w:pPr>" } else { '' }; "<w:p>$p$inner</w:p>" }
$A = 'Reviewer A'; $B = 'Reviewer B'
$SECT = '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>'

function Table($rows, $tblpr = '') {
  "<w:tbl><w:tblPr><w:tblW w:w=`"0`" w:type=`"auto`"/>$tblpr</w:tblPr><w:tblGrid><w:gridCol w:w=`"4000`"/><w:gridCol w:w=`"4000`"/></w:tblGrid>$rows</w:tbl>"
}
function Cell($inner) { "<w:tc><w:tcPr><w:tcW w:w=`"4000`" w:type=`"dxa`"/></w:tcPr>$inner</w:tc>" }
function Row($cells, $trpr = '') { $p = if ($trpr) { "<w:trPr>$trpr</w:trPr>" } else { '' }; "<w:tr>$p$cells</w:tr>" }

function Fixtures {
  $f = [ordered]@{}
  # Insertions and deletions: plain, split runs, nested by another author, adjacent elements.
  $f['review-ins-del'] = @{ body = (
      (Para ((RunT 'Alpha ') + (InsEl $A (RunT 'beta ')) + (RunT 'gamma ') + (DelEl $A (DelT 'delta ')) + (RunT 'epsilon'))) +
      (Para ((RunT 'One ') + (InsEl $B (RunT 'two')) + (DelEl $B (DelT ' three')) + (RunT ' four'))) +
      (Para (InsEl $A ((RunT 'keep ') + (DelEl $B (DelT 'gone')) + (RunT 'tail')))) +
      (Para (InsEl $A ((RunT 'bold ' '<w:b/>') + (RunT 'plain')))) +
      (Para ((InsEl $A (RunT 'x')) + (InsEl $A (RunT 'y')) + (RunT ' end'))) +
      (Para ((DelEl $A (DelT 'whole paragraph deleted text')) )) +
      $SECT) }
  # Paragraph marks: inserted, deleted with differing paragraph properties, deleted empty paragraph.
  $f['review-para-marks'] = @{ body = (
      (Para (RunT 'First paragraph') ('<w:jc w:val="center"/><w:rPr>' + (InsEl $A '') + '</w:rPr>')) +
      (Para (RunT 'Second paragraph') '<w:jc w:val="right"/>') +
      (Para (RunT 'Third paragraph') ('<w:jc w:val="left"/><w:rPr>' + (DelEl $A '') + '</w:rPr>')) +
      (Para (RunT 'Fourth paragraph') '<w:jc w:val="right"/>') +
      (Para '' ('<w:jc w:val="center"/><w:rPr>' + (DelEl $A '') + '</w:rPr>')) +
      (Para (RunT 'Sixth paragraph') '<w:jc w:val="right"/>') +
      (Para (RunT 'Last')) +
      $SECT) }
  # Formatting changes: run, paragraph, table, section.
  $oldRpr = '<w:rPr><w:i/></w:rPr>'
  $f['review-format'] = @{ body = (
      (Para ((RunT 'Plain ') + "<w:r><w:rPr><w:b/><w:rPrChange $(Attrs $A)>$oldRpr</w:rPrChange></w:rPr><w:t>bold now</w:t></w:r>")) +
      (Para (RunT 'Centred now') ('<w:jc w:val="center"/>' + "<w:pPrChange $(Attrs $B)><w:pPr><w:jc w:val=`"left`"/></w:pPr></w:pPrChange>")) +
      (Table (Row ((Cell (Para (RunT 'Cell one'))) + (Cell (Para (RunT 'Cell two'))))) ("<w:tblPrChange $(Attrs $A)><w:tblPr><w:tblW w:w=`"0`" w:type=`"auto`"/></w:tblPr></w:tblPrChange>")) +
      (Para (RunT 'After table')) +
      ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/>' + "<w:sectPrChange $(Attrs $A)><w:sectPr><w:pgSz w:w=`"12240`" w:h=`"15840`"/></w:sectPr></w:sectPrChange></w:sectPr>")) }
  # Moves.
  $m = Id; $m2 = Id
  $f['review-move'] = @{ body = (
      (Para ("<w:moveFromRangeStart w:id=`"$m`" w:name=`"move1`" w:author=`"$A`" w:date=`"$DATE`"/>" + "<w:moveFrom $(Attrs $A)>" + (RunT 'Moved sentence. ') + '</w:moveFrom>' + "<w:moveFromRangeEnd w:id=`"$m`"/>" + (RunT 'Stay A. '))) +
      (Para ((RunT 'Stay B. ') + "<w:moveToRangeStart w:id=`"$m2`" w:name=`"move1`" w:author=`"$A`" w:date=`"$DATE`"/>" + "<w:moveTo $(Attrs $A)>" + (RunT 'Moved sentence. ') + '</w:moveTo>' + "<w:moveToRangeEnd w:id=`"$m2`"/>")) +
      $SECT) }
  # Table rows: inserted and deleted.
  $f['review-rows'] = @{ body = (
      (Para (RunT 'Before table')) +
      (Table (
          (Row ((Cell (Para (RunT 'Kept left'))) + (Cell (Para (RunT 'Kept right'))))) +
          (Row ((Cell (Para (InsEl $A (RunT 'Added left')))) + (Cell (Para (InsEl $A (RunT 'Added right'))))) (InsEl $A '')) +
          (Row ((Cell (Para (DelEl $A (DelT 'Removed left')))) + (Cell (Para (DelEl $A (DelT 'Removed right'))))) (DelEl $A '')))) +
      (Para (RunT 'After table')) +
      $SECT) }
  # Comments: reply, multi-paragraph range, resolved thread, comment in a table cell.
  $cref = { param($id) "<w:r><w:rPr><w:rStyle w:val=`"CommentReference`"/></w:rPr><w:commentReference w:id=`"$id`"/></w:r>" }
  $f['review-comments'] = @{
    body = (
      (Para ('<w:commentRangeStart w:id="0"/><w:commentRangeStart w:id="1"/>' + (RunT 'Commented words') + '<w:commentRangeEnd w:id="0"/>' + (& $cref 0) + '<w:commentRangeEnd w:id="1"/>' + (& $cref 1) + (RunT ' and the rest'))) +
      (Para ((RunT 'Before ') + '<w:commentRangeStart w:id="2"/>' + (RunT 'span start'))) +
      (Para ((RunT 'span end') + '<w:commentRangeEnd w:id="2"/>' + (& $cref 2) + (RunT ' after'))) +
      (Para ('<w:commentRangeStart w:id="3"/>' + (RunT 'Resolved thread') + '<w:commentRangeEnd w:id="3"/>' + (& $cref 3) + '<w:commentRangeStart w:id="4"/><w:commentRangeEnd w:id="4"/>' + (& $cref 4))) +
      (Table (Row ((Cell (Para ('<w:commentRangeStart w:id="5"/>' + (RunT 'In a cell') + '<w:commentRangeEnd w:id="5"/>' + (& $cref 5)))) + (Cell (Para (RunT 'Plain cell')))))) +
      (Para (RunT 'Tail')) +
      $SECT)
    comments = @(
      @{ id = 0; para = '10000001'; parent = $null; done = 0; text = 'First comment' },
      @{ id = 1; para = '10000002'; parent = '10000001'; done = 0; text = 'Reply to first' },
      @{ id = 2; para = '10000003'; parent = $null; done = 0; text = "Spans two paragraphs`nsecond comment paragraph" },
      @{ id = 3; para = '10000004'; parent = $null; done = 1; text = 'Resolved comment' },
      @{ id = 4; para = '10000005'; parent = '10000004'; done = 1; text = 'Reply in resolved thread' },
      @{ id = 5; para = '10000006'; parent = $null; done = 0; text = 'Cell comment' }) }
  $f
}

function Write-Zip($path, $entries) {
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
  $fs = [IO.File]::Create($path)
  try {
    $zip = New-Object IO.Compression.ZipArchive($fs, [IO.Compression.ZipArchiveMode]::Create)
    try {
      foreach ($k in $entries.Keys) {
        $e = $zip.CreateEntry($k, [IO.Compression.CompressionLevel]::Optimal)
        $s = $e.Open()
        try { $b = (New-Object Text.UTF8Encoding $false).GetBytes($entries[$k]); $s.Write($b, 0, $b.Length) } finally { $s.Dispose() }
      }
    } finally { $zip.Dispose() }
  } finally { $fs.Dispose() }
}

function Make-Fixtures {
  $decl = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' + "`n"
  foreach ($kv in (Fixtures).GetEnumerator()) {
    $name = $kv.Key; $spec = $kv.Value
    if ($Only -and $name -ne $Only) { continue }
    $hasComments = $null -ne $spec.comments
    $ct = $decl + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>'
    if ($hasComments) {
      $ct += '<Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/><Override PartName="/word/commentsExtended.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml"/>'
    }
    $ct += '</Types>'
    $rels = $decl + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>'
    $drels = $decl + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
    if ($hasComments) {
      $drels += '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2011/relationships/commentsExtended" Target="commentsExtended.xml"/>'
    }
    $drels += '</Relationships>'
    $entries = [ordered]@{
      '[Content_Types].xml' = $ct
      '_rels/.rels' = $rels
      'word/_rels/document.xml.rels' = $drels
      'word/styles.xml' = ($decl + '<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>')
      'word/document.xml' = ($decl + "<w:document $NS><w:body>" + $spec.body + '</w:body></w:document>')
    }
    if ($hasComments) {
      $c = $decl + "<w:comments $NS>"
      $x = $decl + "<w15:commentsEx $NS>"
      foreach ($cm in $spec.comments) {
        $paras = @($cm.text -split "`n")
        $c += "<w:comment w:id=`"$($cm.id)`" w:author=`"$(if ($cm.id % 2 -eq 0) { 'Reviewer A' } else { 'Reviewer B' })`" w:date=`"$DATE`" w:initials=`"$(if ($cm.id % 2 -eq 0) { 'RA' } else { 'RB' })`">"
        for ($i = 0; $i -lt $paras.Count; $i++) {
          $pid2 = if ($i -eq $paras.Count - 1) { $cm.para } else { '2000000' + $i }
          $ann = if ($i -eq 0) { '<w:r><w:annotationRef/></w:r>' } else { '' }
          $c += "<w:p w14:paraId=`"$pid2`">$ann<w:r><w:t>$($paras[$i])</w:t></w:r></w:p>"
        }
        $c += '</w:comment>'
        $par = if ($cm.parent) { " w15:paraIdParent=`"$($cm.parent)`"" } else { '' }
        $x += "<w15:commentEx w15:paraId=`"$($cm.para)`"$par w15:done=`"$($cm.done)`"/>"
      }
      $entries['word/comments.xml'] = $c + '</w:comments>'
      $entries['word/commentsExtended.xml'] = $x + '</w15:commentsEx>'
    }
    $path = Join-Path $fixDir "$name.docx"
    Write-Zip $path $entries
    Write-Output "made $name"
  }
}

if ($Make) { Make-Fixtures; exit 0 }

# ---------------------------------------------------------------------------
# Measurement
# ---------------------------------------------------------------------------
function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }
function Esc([string]$s) {
  $sb = New-Object Text.StringBuilder
  foreach ($ch in $s.ToCharArray()) {
    $c = [int]$ch
    if ($ch -eq '\') { [void]$sb.Append('\\') } elseif ($c -eq 9) { [void]$sb.Append('\t') }
    elseif ($c -eq 13) { [void]$sb.Append('\r') } elseif ($c -eq 10) { [void]$sb.Append('\n') }
    elseif ($c -lt 32 -or $c -gt 126) { [void]$sb.Append('\u' + $c.ToString('x4')) } else { [void]$sb.Append($ch) }
  }
  $sb.ToString()
}
$typeNames = @{ 0 = 'NoRevision'; 1 = 'Insert'; 2 = 'Delete'; 3 = 'Property'; 4 = 'ParagraphNumber'; 5 = 'DisplayField'; 6 = 'Reconcile'; 7 = 'Conflict'; 8 = 'Style'; 9 = 'Replace'; 10 = 'ParagraphProperty'; 11 = 'TableProperty'; 12 = 'SectionProperty'; 13 = 'StyleDefinition'; 14 = 'MovedFrom'; 15 = 'MovedTo'; 16 = 'CellInsertion'; 17 = 'CellDeletion'; 18 = 'CellMerge' }

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
function Clean([string]$t) { ($t -replace [char]5, '').TrimEnd("`r", "`a") }
# Paragraph texts (row-end marks skipped), optionally with the alignment.
function Para-Lines($doc, [string]$kind, [bool]$withAlign) {
  $out = New-Object System.Collections.Generic.List[string]
  $i = 0
  foreach ($p in $doc.Paragraphs) {
    $r = $p.Range
    $raw = $r.Text
    if ($raw -eq "`r`a" -and $r.Information(12) -and $r.Cells.Count -eq 0) { continue }
    $line = "$kind`t$i`t$(Esc (Clean $raw))"
    if ($withAlign) { $line += "`t$($p.Alignment)" }
    $out.Add($line); $i++
  }
  $out
}
function Joined($doc) { ((Para-Lines $doc 'x' $false | ForEach-Object { ($_ -split "`t", 3)[2] }) -join [string][char]0x241E) }
function Set-View($doc, [bool]$markup) {
  $v = $doc.ActiveWindow.View
  $v.ShowRevisionsAndComments = $markup
  $v.RevisionsView = 0   # wdRevisionsViewFinal
}

function Measure-Doc([string]$file) {
  $name = [IO.Path]::GetFileNameWithoutExtension($file)
  $lines = New-Object System.Collections.Generic.List[string]
  $doc = Open-Doc $file
  try {
    $doc.TrackRevisions = $false
    $revCount = $doc.Revisions.Count
    for ($i = 1; $i -le $revCount; $i++) {
      $rv = $doc.Revisions.Item($i)
      $tn = $typeNames[[int]$rv.Type]; if (-not $tn) { $tn = "Type$([int]$rv.Type)" }
      $lines.Add("rev`t$($i - 1)`t$tn`t$(Esc $rv.Author)`t$(Esc (Clean $rv.Range.Text))")
    }
    Set-View $doc $true;  foreach ($l in (Para-Lines $doc 'markup' $false)) { $lines.Add($l) }
    Set-View $doc $false; foreach ($l in (Para-Lines $doc 'final' $false)) { $lines.Add($l) }
    Set-View $doc $true
    $cc = $doc.Comments.Count
    for ($i = 1; $i -le $cc; $i++) {
      $c = $doc.Comments.Item($i)
      $anc = -1
      try { if ($c.Ancestor) { $a = $c.Ancestor; for ($k = 1; $k -le $cc; $k++) { if ($doc.Comments.Item($k).Index -eq $a.Index) { $anc = $k - 1 } } } } catch {}
      $lines.Add("comment`t$($i - 1)`t$(Esc $c.Author)`t$(Esc $c.Initial)`t$(Esc (Clean $c.Scope.Text))`t$(Esc (Clean $c.Range.Text))`t$([int][bool]$c.Done)`t$anc`t$($c.Replies.Count)")
    }
    $doc.Revisions.AcceptAll(); foreach ($l in (Para-Lines $doc 'accept' $true)) { $lines.Add($l) }
  } finally { try { $doc.Close(0) } catch {}; Release $doc }
  $doc = Open-Doc $file
  try {
    $doc.TrackRevisions = $false
    $doc.Revisions.RejectAll(); foreach ($l in (Para-Lines $doc 'reject' $true)) { $lines.Add($l) }
  } finally { try { $doc.Close(0) } catch {}; Release $doc }
  # One revision at a time (fresh open each time: Word's revision list changes as they resolve).
  for ($r = 1; $r -le $revCount; $r++) {
    foreach ($act in 'accept1', 'reject1') {
      $doc = Open-Doc $file
      try {
        $doc.TrackRevisions = $false
        $rv = $doc.Revisions.Item($r)
        if ($act -eq 'accept1') { $rv.Accept() } else { $rv.Reject() }
        $lines.Add("$act`t$($r - 1)`t$(Esc (Joined $doc))")
      } finally { try { $doc.Close(0) } catch {}; Release $doc }
    }
  }
  $outDir = Join-Path $refDir $name
  New-Item -ItemType Directory -Force $outDir | Out-Null
  [IO.File]::WriteAllText((Join-Path $outDir 'review.tsv'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  "$name : $revCount revisions, $cc comments"
}

$files = Get-ChildItem -LiteralPath $fixDir -Filter 'review-*.docx' | Sort-Object Name
if ($Only) { $files = $files | Where-Object { $_.BaseName -eq $Only } }
if (-not $files) { Write-Output 'no fixtures; run with -Make first'; exit 1 }
try {
  foreach ($f in $files) { Write-Output (Measure-Doc $f.FullName) }
} finally {
  if ($word) { try { $word.Quit(0) } catch {}; Release $word }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($id in $wordPids) { Get-Process -Id $id -ErrorAction SilentlyContinue | Stop-Process -Force }
}
