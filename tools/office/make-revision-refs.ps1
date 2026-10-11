<#
Word revision and comment oracle: creates tracked-change and comment documents with Microsoft Word 16.0 itself
(Application.TrackRevisions, Comments.Add) and records what Word does when they are accepted or rejected.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this is never run
in CI. CI consumes the committed outputs: fixtures/docx/rev-*.docx, fixtures/docx/cmt-*.docx and
fixtures/reference/docx/{rev,cmt}-<name>/review.tsv.

  pwsh -NoProfile -File tools/office/make-revision-refs.ps1 -Make            # (re)creates fixtures/docx/{rev,cmt}-*.docx
  pwsh -NoProfile -File tools/office/make-revision-refs.ps1 -Measure         # measures every rev-*/cmt-* fixture
  pwsh -NoProfile -File tools/office/make-revision-refs.ps1 -Make -Measure -Only rev-text

Safety: the script starts its OWN Word (New-Object -ComObject Word.Application) and aborts when the instance it got
belongs to a process that existed before (it never attaches to or touches yours). AutomationSecurity = 3, so macros never run.
Only that process id is quit in the finally block (Quit, then Stop-Process on that PID only). Documents are generated;
the author names Word stamps (the signed-in user) are rewritten to "Author A" in every part and docProps is scrubbed,
so no personal data is committed. Fixtures are written through a temp folder and the originals are opened ReadOnly.

review.tsv is tab separated; text is escaped as \\, \t, \r, \n and \uXXXX for every control / non-ASCII character.
  rev     index  type  author  date  text           Document.Revisions, in order
  comment index  author initial scope text done ancestor replies
  para    label  i  text  align  style  list  runs    one line per paragraph (row-end marks skipped)
  tbl     label  t  cells                              one line per table: cells "row,col,width,shade,text" joined by U+241E
  label is: markup (as loaded), final (markup hidden), accept, reject, accept1:r, reject1:r (one revision, 0-based),
  acceptp:i, rejectp:i (Range.Revisions on paragraph i, 0-based).
  runs is the run-length encoded character formatting: B<bold>I<italic>U<underline>S<half-points>x<count> joined by ';'.
#>
param(
  [switch]$Make,
  [switch]$Measure,
  [string]$Only
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixDir = Join-Path $root 'fixtures\docx'
$refDir = Join-Path $root 'fixtures\reference\docx'
$tmpDir = Join-Path ([IO.Path]::GetTempPath()) ('open365-revrefs-' + [guid]::NewGuid().ToString('N'))
if (-not ($Make -or $Measure)) { $Make = $true; $Measure = $true }
Add-Type -AssemblyName System.IO.Compression
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class W32 { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid); }'

function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }

$wordBefore = Get-Pids 'WINWORD'
$script:word = $null; $script:wordPid = 0
function Get-Word {
  if ($null -eq $script:word) {
    $w = New-Object -ComObject Word.Application
    $w.Visible = $false; $w.DisplayAlerts = 0
    $w.AutomationSecurity = 3  # msoAutomationSecurityForceDisable: macros never run
    [uint32]$wpid = 0
    # An invisible Word has no application window handle until it has a document window.
    $probe = $w.Documents.Add()
    $hwnd = 0
    try { $hwnd = $w.ActiveWindow.Hwnd } catch {}
    if ($hwnd) { [void][W32]::GetWindowThreadProcessId([IntPtr][int]$hwnd, [ref]$wpid) }
    try { $probe.Close(0) } catch {}
    if ($wpid -eq 0 -or $wordBefore -contains [int]$wpid) {
      # attached to somebody else's Word, or the owner is unknown: do not touch it
      throw "COM returned a Word instance (pid $wpid) that this script cannot prove it started; aborting"
    }
    $script:wordPid = [int]$wpid
    $script:word = $w
  }
  $script:word
}
function Open-Doc($path, [bool]$readOnly = $true) {
  $w = Get-Word
  $w.Documents.GetType().InvokeMember('Open', [Reflection.BindingFlags]::InvokeMethod, $null, $w.Documents,
    [object[]]@($path, $false, $readOnly, $false, $false, $true, $false, $true), $null, $null,
    [string[]]@('FileName', 'ConfirmConversions', 'ReadOnly', 'AddToRecentFiles', 'Revert', 'Visible', 'OpenAndRepair', 'NoEncodingDialog'))
}

# ---------------------------------------------------------------------------
# Generation (Word's own TrackRevisions)
# ---------------------------------------------------------------------------
function Find-Range($doc, [int]$para, [string]$text) {
  $p = $doc.Paragraphs.Item($para).Range
  $i = $p.Text.IndexOf($text)
  if ($i -lt 0) { throw "text '$text' not in paragraph $para" }
  $doc.Range($p.Start + $i, $p.Start + $i + $text.Length)
}
function Make-Table($doc, [int]$after, [int]$rows, [int]$cols) {
  $r = $doc.Paragraphs.Item($after).Range; $r.Collapse(0)
  $t = $doc.Tables.Add($r, $rows, $cols)
  for ($i = 1; $i -le $rows; $i++) { for ($j = 1; $j -le $cols; $j++) { $t.Cell($i, $j).Range.Text = "R${i}C${j}" } }
  $t
}
$Scenarios = [ordered]@{
  'rev-text' = @{ base = "The quick brown fox jumps over the lazy dog.`rSecond paragraph here.`rThird paragraph here.`rFourth paragraph here."; edit = {
      param($d)
      (Find-Range $d 4 'here').InsertAfter(' and more')
      $d.Paragraphs.Item(4).Range.InsertParagraphAfter()
      $d.Paragraphs.Item(5).Range.InsertAfter('Fifth inserted paragraph.')
      (Find-Range $d 3 'Third ').Delete() | Out-Null
      $d.Paragraphs.Item(2).Range.Delete() | Out-Null
      (Find-Range $d 1 'lazy').Text = 'sleepy'
      (Find-Range $d 1 'brown ').Delete() | Out-Null
      (Find-Range $d 1 'quick').InsertAfter(' very')
    } }
  'rev-format' = @{ base = "Plain words with bold target here.`rItalic and underline and size targets.`rCentred later paragraph.`rHeading candidate line.`rColour and strike line."; edit = {
      param($d)
      (Find-Range $d 1 'bold target').Font.Bold = $true
      (Find-Range $d 2 'Italic').Font.Italic = $true
      (Find-Range $d 2 'underline').Font.Underline = 1
      (Find-Range $d 2 'size').Font.Size = 18
      $d.Paragraphs.Item(3).Alignment = 1
      $d.Paragraphs.Item(3).LeftIndent = 36
      $d.Paragraphs.Item(3).SpaceAfter = 18
      $d.Paragraphs.Item(4).Style = -2
      (Find-Range $d 5 'Colour').Font.Color = 255
      (Find-Range $d 5 'strike').Font.StrikeThrough = $true
    } }
  'rev-mixed' = @{ base = "Alpha beta gamma delta.`rSecond line of text.`rThird line of text."; edit = {
      param($d)
      (Find-Range $d 1 'beta').InsertAfter(' inserted')
      (Find-Range $d 1 'inserted').Font.Bold = $true
      (Find-Range $d 1 'gamma').Font.Italic = $true
      (Find-Range $d 1 'gamma').Delete() | Out-Null
      $d.Paragraphs.Item(2).Alignment = 2
      (Find-Range $d 2 'line').Text = 'LINE'
      $d.Paragraphs.Item(3).Range.InsertParagraphBefore()
      $d.Paragraphs.Item(3).Range.InsertAfter('Inserted before third')
    } }
  'rev-table-rows' = @{ base = "Before table.`rAfter table."; table = @(1, 3, 3); edit = {
      param($d)
      $t = $d.Tables.Item(1)
      $t.Rows.Item(3).Delete()
      $t.Rows.Add($t.Rows.Item(2)) | Out-Null
      $t.Cell(2, 1).Range.Text = 'New'
    } }
  'rev-table-cols' = @{ base = "Before table.`rAfter table."; table = @(1, 3, 3); edit = {
      param($d)
      $t = $d.Tables.Item(1)
      $t.Columns.Item(2).Delete()
      $t.Columns.Add() | Out-Null
    } }
  'rev-table-merge' = @{ base = "Before table.`rAfter table."; table = @(1, 3, 3); edit = {
      param($d)
      $t = $d.Tables.Item(1)
      $t.Cell(1, 1).Merge($t.Cell(1, 2))
      $t.Cell(3, 2).Split(1, 2)
    } }
  'rev-table-props' = @{ base = "Before table.`rAfter table."; table = @(1, 3, 3); edit = {
      param($d)
      $t = $d.Tables.Item(1)
      $t.Cell(1, 1).Shading.BackgroundPatternColor = 65535
      $t.Cell(2, 2).Width = 90
      $t.Rows.Item(3).Height = 40
      $t.Rows.Item(3).HeightRule = 2
      $t.Rows.Alignment = 1
      $t.Cell(2, 1).VerticalAlignment = 1
      $t.Rows.Item(1).HeadingFormat = $true
      $t.Rows.Item(2).AllowBreakAcrossPages = 0
    } }
  'rev-table-cells' = @{ base = "Before table.`rAfter table."; table = @(1, 3, 3); edit = {
      param($d)
      $t = $d.Tables.Item(1)
      $t.Cell(3, 3).Delete(1)
      [void]$t.Range.Cells.Add($t.Cell(2, 2))
    } }
  'rev-numbering' = @{ base = "First list item.`rSecond list item.`rBullet item.`rPlain item."; edit = {
      param($d)
      $d.Range($d.Paragraphs.Item(1).Range.Start, $d.Paragraphs.Item(2).Range.End).ListFormat.ApplyNumberDefault()
      $d.Paragraphs.Item(3).Range.ListFormat.ApplyBulletDefault()
      $d.Paragraphs.Item(2).Range.ListFormat.ListIndent()
    } }
  'rev-move' = @{ base = "Move alpha paragraph.`rStay beta paragraph.`rMove gamma paragraph.`rStay delta paragraph."; edit = {
      param($d)
      $d.Paragraphs.Item(3).Range.Relocate(0)
      $d.Paragraphs.Item(1).Range.Relocate(1)
    } }
  'rev-section' = @{ base = "Section one text.`rSection two text."; edit = {
      param($d)
      $d.PageSetup.Orientation = 1
      $d.PageSetup.LeftMargin = 90
      $r = $d.Paragraphs.Item(1).Range; $r.Collapse(0); $r.MoveEnd(1, -1) | Out-Null
      $r.InsertBreak(2)
    } }
  'cmt-basic' = @{ base = "Comment target one here.`rSecond paragraph target.`rThird paragraph without comment."; track = $false; edit = {
      param($d)
      $c1 = $d.Comments.Add((Find-Range $d 1 'target one'), 'First comment')
      [void]$c1.Replies.Add($c1.Scope, 'First reply')
      [void]$c1.Replies.Add($c1.Scope, 'Second reply')
      $c2 = $d.Comments.Add((Find-Range $d 2 'Second'), 'Resolved comment')
      [void]$c2.Replies.Add($c2.Scope, 'Reply to resolved')
      $c2.Done = $true
      [void]$d.Comments.Add((Find-Range $d 3 'Third paragraph'), "Two paragraph comment`rsecond comment paragraph")
    } }
  'cmt-ranges' = @{ base = "Overlap one two three four.`rCrosses paragraph start.`rCrosses paragraph end.`rFinal paragraph."; table = @(4, 2, 2); track = $false; edit = {
      param($d)
      $a = $d.Range((Find-Range $d 1 'one').Start, (Find-Range $d 1 'three').End)
      [void]$d.Comments.Add($a, 'Wide overlap')
      [void]$d.Comments.Add((Find-Range $d 1 'two'), 'Inner overlap')
      $r = $d.Range((Find-Range $d 2 'paragraph start').Start, (Find-Range $d 3 'Crosses').End)
      [void]$d.Comments.Add($r, 'Spans paragraphs')
      $ins = $d.Paragraphs.Item(4).Range; $ins.Collapse(1)
      [void]$d.Comments.Add($ins, 'Point comment')
      [void]$d.Comments.Add($d.Tables.Item(1).Cell(2, 2).Range, 'Cell comment')
    } }
  'rev-two-authors' = @{ base = "Alpha beta gamma delta epsilon.`rSecond line of text here.`rThird line goes.`rFourth line stays."; post = 'alternate'; edit = {
      param($d)
      (Find-Range $d 1 'beta ').Delete() | Out-Null
      (Find-Range $d 1 'gamma').Text = 'GAMMA'
      (Find-Range $d 1 'epsilon').InsertAfter(' zeta')
      (Find-Range $d 2 'line').Font.Bold = $true
      (Find-Range $d 2 'text').Delete() | Out-Null
      $d.Paragraphs.Item(3).Range.Delete() | Out-Null
      $d.Paragraphs.Item(3).Range.InsertParagraphAfter()
      $d.Paragraphs.Item(4).Range.InsertAfter('Inserted last')
    } }
  'rev-numbering-deep' = @{ base = "Item one.`rItem two.`rItem three.`rItem four.`rItem five.`rItem six."; edit = {
      param($d)
      $d.Range($d.Paragraphs.Item(1).Range.Start, $d.Paragraphs.Item(5).Range.End).ListFormat.ApplyNumberDefault()
      $d.Paragraphs.Item(2).Range.ListFormat.ListIndent()
      $d.Paragraphs.Item(3).Range.ListFormat.ListIndent()
      $d.Paragraphs.Item(3).Range.ListFormat.ListIndent()
      $d.Paragraphs.Item(4).Range.ListFormat.ListIndent()
      $d.Paragraphs.Item(4).Range.ListFormat.ListOutdent()
      $d.Paragraphs.Item(5).Range.ListFormat.RemoveNumbers()
      $d.Paragraphs.Item(6).Range.ListFormat.ApplyBulletDefault()
    } }
  'rev-section-deep' = @{ base = "Section one.`rSection two.`rSection three.`rSection four."; edit = {
      param($d)
      foreach ($pi in 3, 2, 1) {
        $r = $d.Paragraphs.Item($pi).Range; $r.Collapse(0); $r.MoveEnd(1, -1) | Out-Null
        $r.InsertBreak($(if ($pi -eq 2) { 3 } else { 2 }))
      }
      $d.Sections.Item(1).PageSetup.TopMargin = 40
      $d.Sections.Item(2).PageSetup.Orientation = 1
      $d.Sections.Item(3).PageSetup.LeftMargin = 100
      $d.Sections.Item(4).PageSetup.TextColumns.SetCount(2)
    } }
  'cmt-stories' = @{ base = "Body text with a note here.`rSecond body paragraph."; track = $false; edit = {
      param($d)
      [void]$d.Comments.Add((Find-Range $d 2 'body'), 'Body comment')
      [void]$d.Footnotes.Add((Find-Range $d 1 'note'), [Type]::Missing, 'Footnote text here.')
      try { [void]$d.Comments.Add($d.Footnotes.Item(1).Range, 'Comment in footnote') } catch { Write-Output "footnote comment refused: $($_.Exception.Message)" }
      try { $s = $d.Shapes.AddTextbox(1, 100, 200, 200, 50); $s.TextFrame.TextRange.Text = 'Text box text'; [void]$d.Comments.Add($s.TextFrame.TextRange, 'Comment in text box') } catch { Write-Output "text box comment refused: $($_.Exception.Message)" }
      try { $h = $d.Sections.Item(1).Headers.Item(1).Range; $h.Text = 'Header text'; [void]$d.Comments.Add($h, 'Comment in header') } catch { Write-Output "header comment refused: $($_.Exception.Message)" }
      try { $f = $d.Sections.Item(1).Footers.Item(1).Range; $f.Text = 'Footer text'; [void]$d.Comments.Add($f, 'Comment in footer') } catch { Write-Output "footer comment refused: $($_.Exception.Message)" }
    } }
  'cmt-tracked' = @{ base = "Comment on tracked text here.`rSecond paragraph text."; edit = {
      param($d)
      (Find-Range $d 1 'tracked').InsertAfter(' new')
      (Find-Range $d 2 'paragraph').Delete() | Out-Null
      [void]$d.Comments.Add((Find-Range $d 1 'tracked new'), 'Comment over an insertion')
      [void]$d.Comments.Add($d.Paragraphs.Item(2).Range, 'Comment over a deletion')
    } }
}

# ---------------------------------------------------------------------------
# Hand-written revision markup Word 16 does not emit itself (cell insert/delete/merge, numbering change).
# Word opens these and the oracle measures what it makes of them; they are not produced by TrackRevisions.
# ---------------------------------------------------------------------------
$XA = 'w:id="{0}" w:author="Author A" w:date="2026-01-02T03:04:00Z"'
$script:xid = 700
function XA { $script:xid++; $XA -f $script:xid }
function XR($t) { "<w:r><w:t xml:space=`"preserve`">$t</w:t></w:r>" }
function XP($inner, $ppr = '') { if ($ppr) { "<w:p><w:pPr>$ppr</w:pPr>$inner</w:p>" } else { "<w:p>$inner</w:p>" } }
function XC($inner, $tcpr = '') { "<w:tc><w:tcPr><w:tcW w:w=`"2000`" w:type=`"dxa`"/>$tcpr</w:tcPr>$inner</w:tc>" }
function XT($rows, $cols = 3) { "<w:tbl><w:tblPr><w:tblW w:w=`"0`" w:type=`"auto`"/></w:tblPr><w:tblGrid>" + ('<w:gridCol w:w="2000"/>' * $cols) + "</w:tblGrid>$rows</w:tbl>" }
$XmlScenarios = [ordered]@{
  'rev-cell-ins' = @{ body = { (XP (XR 'Before')) + (XT ((('<w:tr>' + (XC (XP (XR 'A1'))) + (XC (XP "<w:ins $(XA)>$(XR 'B1 new')</w:ins>") "<w:cellIns $(XA)/>") + (XC (XP (XR 'C1'))) + '</w:tr>') + ('<w:tr>' + (XC (XP (XR 'A2'))) + (XC (XP (XR 'B2'))) + (XC (XP (XR 'C2'))) + '</w:tr>')))) + (XP (XR 'After')) } }
  'rev-cell-del' = @{ body = { (XP (XR 'Before')) + (XT ((('<w:tr>' + (XC (XP (XR 'A1'))) + (XC (XP "<w:del $(XA)><w:r><w:delText>B1 old</w:delText></w:r></w:del>") "<w:cellDel $(XA)/>") + (XC (XP (XR 'C1'))) + '</w:tr>') + ('<w:tr>' + (XC (XP (XR 'A2'))) + (XC (XP (XR 'B2'))) + (XC (XP (XR 'C2'))) + '</w:tr>')))) + (XP (XR 'After')) } }
  'rev-cell-merge' = @{ body = { (XP (XR 'Before')) + (XT ((('<w:tr>' + (XC (XP (XR 'A1')) '<w:vMerge w:val="restart"/>') + (XC (XP (XR 'B1'))) + (XC (XP (XR 'C1'))) + '</w:tr>') + ('<w:tr>' + (XC (XP '') ('<w:vMerge/>' + "<w:cellMerge $(XA) w:vMerge=`"rest`" w:vMergeOrig=`"cont`"/>")) + (XC (XP (XR 'B2'))) + (XC (XP (XR 'C2'))) + '</w:tr>')))) + (XP (XR 'After')) } }
  'rev-numbering-xml' = @{ numbering = $true; body = { (XP (XR 'Numbered by tracked change') ('<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/>' + "<w:numberingChange $(XA) w:original=`"%1.`"/><w:ins $(XA)/></w:numPr>")) + (XP (XR 'Plain paragraph')) + (XP (XR 'Level change') ('<w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr>' + "<w:pPrChange $(XA)><w:pPr><w:numPr><w:ilvl w:val=`"0`"/><w:numId w:val=`"1`"/></w:numPr></w:pPr></w:pPrChange>")) } }
}
function Make-XmlFixture([string]$name, $spec) {
  $decl = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' + "`n"
  $W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
  $ct = $decl + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>'
  if ($spec.numbering) { $ct += '<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>' }
  $ct += '</Types>'
  $rels = $decl + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>'
  $drels = $decl + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
  if ($spec.numbering) { $drels += '<Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>' }
  $drels += '</Relationships>'
  $sect = '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>'
  $entries = [ordered]@{
    '[Content_Types].xml' = $ct
    '_rels/.rels' = $rels
    'word/_rels/document.xml.rels' = $drels
    'word/styles.xml' = ($decl + "<w:styles $W><w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val=`"22`"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type=`"paragraph`" w:default=`"1`" w:styleId=`"Normal`"><w:name w:val=`"Normal`"/></w:style></w:styles>")
    'word/document.xml' = ($decl + "<w:document $W><w:body>" + (& $spec.body) + $sect + '</w:body></w:document>')
  }
  if ($spec.numbering) {
    $entries['word/numbering.xml'] = $decl + "<w:numbering $W><w:abstractNum w:abstractNumId=`"0`"><w:multiLevelType w:val=`"multilevel`"/><w:lvl w:ilvl=`"0`"><w:start w:val=`"1`"/><w:numFmt w:val=`"decimal`"/><w:lvlText w:val=`"%1.`"/><w:lvlJc w:val=`"left`"/><w:pPr><w:ind w:left=`"720`" w:hanging=`"360`"/></w:pPr></w:lvl><w:lvl w:ilvl=`"1`"><w:start w:val=`"1`"/><w:numFmt w:val=`"lowerLetter`"/><w:lvlText w:val=`"%2)`"/><w:lvlJc w:val=`"left`"/><w:pPr><w:ind w:left=`"1440`" w:hanging=`"360`"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId=`"1`"><w:abstractNumId w:val=`"0`"/></w:num></w:numbering>"
  }
  $path = Join-Path $fixDir "$name.docx"
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
  "made $name (hand-written)"
}

# Rewrites the zip at $path with $scrub applied to every XML-ish entry.
function Rewrite-Zip([string]$path, [scriptblock]$scrub) {
  $entries = [ordered]@{}
  $zip = [IO.Compression.ZipFile]::OpenRead($path)
  try {
    foreach ($e in $zip.Entries) {
      $ms = New-Object IO.MemoryStream
      $s = $e.Open(); try { $s.CopyTo($ms) } finally { $s.Dispose() }
      $entries[$e.FullName] = $ms.ToArray()
    }
  } finally { $zip.Dispose() }
  $out = [IO.File]::Create($path)
  try {
    $za = New-Object IO.Compression.ZipArchive($out, [IO.Compression.ZipArchiveMode]::Create)
    try {
      foreach ($k in $entries.Keys) {
        $bytes = $entries[$k]
        if ($k -match '\.(xml|rels)$') {
          $txt = [Text.Encoding]::UTF8.GetString($bytes)
          $bytes = (New-Object Text.UTF8Encoding $false).GetBytes((& $scrub $k $txt))
        }
        $ze = $za.CreateEntry($k, [IO.Compression.CompressionLevel]::Optimal)
        $s = $ze.Open(); try { $s.Write($bytes, 0, $bytes.Length) } finally { $s.Dispose() }
      }
    } finally { $za.Dispose() }
  } finally { $out.Dispose() }
}

function Make-Fixture([string]$name, $spec) {
  $w = Get-Word
  $names = @($w.UserName, $w.UserInitials, $env:USERNAME) | Where-Object { $_ -and $_.Length -ge 2 } | Select-Object -Unique
  $doc = $w.Documents.Add()
  $tmp = Join-Path $tmpDir "$name.docx"
  try {
    $doc.TrackRevisions = $false
    $doc.Content.Text = $spec.base
    if ($spec.table) { [void](Make-Table $doc $spec.table[0] $spec.table[1] $spec.table[2]) }
    $track = $true; if ($spec.ContainsKey('track')) { $track = $spec.track }
    $doc.TrackRevisions = $track
    & $spec.edit $doc
    $doc.TrackRevisions = $false
    $doc.SaveAs2($tmp, 16)
  } finally { try { $doc.Close(0) } catch {}; Release $doc }
  $scrub = {
    param($part, $txt)
    foreach ($n in $names) { $txt = $txt -replace [regex]::Escape($n), 'Author A' }
    if ($part -eq 'docProps/core.xml') {
      $txt = $txt -replace '<dc:creator>.*?</dc:creator>', '<dc:creator>Author A</dc:creator>' -replace '<cp:lastModifiedBy>.*?</cp:lastModifiedBy>', '<cp:lastModifiedBy>Author A</cp:lastModifiedBy>'
    }
    if ($part -eq 'docProps/app.xml') { $txt = $txt -replace '<Company>.*?</Company>', '<Company></Company>' }
    if ($part -eq 'word/people.xml') {
      $txt = $txt -replace 'w15:providerId="[^"]*"', 'w15:providerId="None"' -replace 'w15:userId="[^"]*"', 'w15:userId="Author A"'
    }
    $txt = $txt -replace 'w:initials="[^"]*"', 'w:initials="AA"'
    $txt
  }
  Rewrite-Zip $tmp $scrub
  if ($spec.post -eq 'alternate') {
    # A second author: every second revision element of the document is by "Author B".
    $script:altN = 0
    Rewrite-Zip $tmp {
      param($part, $txt)
      if ($part -ne 'word/document.xml') { return $txt }
      [regex]::Replace($txt, 'w:author="Author A"', [Text.RegularExpressions.MatchEvaluator]{ param($m) $script:altN++; if ($script:altN % 2 -eq 0) { 'w:author="Author B"' } else { $m.Value } })
    }
  }
  Copy-Item -LiteralPath $tmp -Destination (Join-Path $fixDir "$name.docx") -Force
  "made $name"
}

# ---------------------------------------------------------------------------
# Measurement
# ---------------------------------------------------------------------------
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
function Clean([string]$t) { ($t -replace [char]5, '').TrimEnd("`r", "`a") }

function Runs-Sig($r) {
  $n = $r.Characters.Count
  $parts = New-Object System.Collections.Generic.List[string]
  $cur = $null; $cnt = 0
  for ($k = 1; $k -le $n; $k++) {
    $c = $r.Characters.Item($k)
    $t = $c.Text
    if ($t -eq "`r" -or $t -eq "`a" -or $t -eq [string][char]5) { continue }
    $f = $c.Font
    $key = 'B{0}I{1}U{2}S{3}' -f ([int]($f.Bold -ne 0)), ([int]($f.Italic -ne 0)), ([int]($f.Underline -ne 0)), ([int]($f.Size * 2))
    if ($key -eq $cur) { $cnt++ } else { if ($cur) { $parts.Add("${cur}x$cnt") }; $cur = $key; $cnt = 1 }
  }
  if ($cur) { $parts.Add("${cur}x$cnt") }
  $parts -join ';'
}
function State-Lines($doc, [string]$label) {
  $out = New-Object System.Collections.Generic.List[string]
  $i = 0
  foreach ($p in $doc.Paragraphs) {
    $r = $p.Range
    $raw = $r.Text
    if ($raw -eq "`r`a" -and $r.Information(12) -and $r.Cells.Count -eq 0) { continue }
    $out.Add("para`t$label`t$i`t$(Esc (Clean $raw))`t$($p.Alignment)`t$(Esc $p.Style.NameLocal)`t$($r.ListFormat.ListType)`t$(Runs-Sig $r)")
    $i++
  }
  $t = 0
  foreach ($tb in $doc.Tables) {
    $cells = New-Object System.Collections.Generic.List[string]
    foreach ($cell in $tb.Range.Cells) {
      $cells.Add(('{0},{1},{2},{3},{4}' -f $cell.RowIndex, $cell.ColumnIndex, [math]::Round($cell.Width, 1), $cell.Shading.BackgroundPatternColor, (Clean $cell.Range.Text)))
    }
    $out.Add("tbl`t$label`t$t`t$(Esc ($cells -join [string][char]0x241E))")
    $t++
  }
  $out
}
function Set-View($doc, [bool]$markup) {
  $v = $doc.ActiveWindow.View
  $v.ShowRevisionsAndComments = $markup
  $v.RevisionsView = 0
}
function Open-Fresh([string]$file) {
  $d = Open-Doc $file
  $d.TrackRevisions = $false
  Set-View $d $true
  $d
}

function Measure-Doc([string]$file) {
  $name = [IO.Path]::GetFileNameWithoutExtension($file)
  $lines = New-Object System.Collections.Generic.List[string]
  $doc = Open-Fresh $file
  try {
    $revCount = $doc.Revisions.Count
    for ($i = 1; $i -le $revCount; $i++) {
      $rv = $doc.Revisions.Item($i)
      $tn = $typeNames[[int]$rv.Type]; if (-not $tn) { $tn = "Type$([int]$rv.Type)" }
      $lines.Add("rev`t$($i - 1)`t$tn`t$(Esc $rv.Author)`t$(Esc ([datetime]$rv.Date).ToString('yyyy-MM-ddTHH:mm:00Z'))`t$(Esc (Clean $rv.Range.Text))")
    }
    $cc = $doc.Comments.Count
    for ($i = 1; $i -le $cc; $i++) {
      $c = $doc.Comments.Item($i)
      $anc = -1
      try { if ($c.Ancestor) { $a = $c.Ancestor; for ($k = 1; $k -le $cc; $k++) { if ($doc.Comments.Item($k).Index -eq $a.Index) { $anc = $k - 1 } } } } catch {}
      $lines.Add("comment`t$($i - 1)`t$(Esc $c.Author)`t$(Esc $c.Initial)`t$(Esc (Clean $c.Scope.Text))`t$(Esc (Clean $c.Range.Text))`t$([int][bool]$c.Done)`t$anc`t$($c.Replies.Count)")
    }
    foreach ($l in (State-Lines $doc 'markup')) { $lines.Add($l) }
    Set-View $doc $false
    foreach ($l in (State-Lines $doc 'final')) { $lines.Add($l) }
    Set-View $doc $true
    $doc.Revisions.AcceptAll()
    foreach ($l in (State-Lines $doc 'accept')) { $lines.Add($l) }
  } finally { try { $doc.Close(0) } catch {}; Release $doc }
  $doc = Open-Fresh $file
  try {
    $doc.Revisions.RejectAll()
    foreach ($l in (State-Lines $doc 'reject')) { $lines.Add($l) }
  } finally { try { $doc.Close(0) } catch {}; Release $doc }
  for ($r = 1; $r -le $revCount; $r++) {
    foreach ($act in 'accept', 'reject') {
      $doc = Open-Fresh $file
      try {
        $rv = $doc.Revisions.Item($r)
        if ($act -eq 'accept') { $rv.Accept() } else { $rv.Reject() }
        foreach ($l in (State-Lines $doc "${act}1:$($r - 1)")) { $lines.Add($l) }
      } finally { try { $doc.Close(0) } catch {}; Release $doc }
    }
  }
  # Range.Revisions on each paragraph.
  if ($revCount -gt 0) {
  $doc = Open-Fresh $file
  try { $np = $doc.Paragraphs.Count } finally { try { $doc.Close(0) } catch {}; Release $doc }
  $logical = 0
  for ($pi = 1; $pi -le $np; $pi++) {
    foreach ($act in 'accept', 'reject') {
      $doc = Open-Fresh $file
      try {
        $pr = $doc.Paragraphs.Item($pi).Range
        if ($pr.Text -eq "`r`a" -and $pr.Information(12) -and $pr.Cells.Count -eq 0) { continue }
        if ($act -eq 'accept') { $pr.Revisions.AcceptAll() } else { $pr.Revisions.RejectAll() }
        foreach ($l in (State-Lines $doc "${act}p:$logical")) { $lines.Add($l) }
      } finally { try { $doc.Close(0) } catch {}; Release $doc }
    }
    $doc = Open-Fresh $file
    try { $pr = $doc.Paragraphs.Item($pi).Range; if (-not ($pr.Text -eq "`r`a" -and $pr.Information(12) -and $pr.Cells.Count -eq 0)) { $logical++ } } finally { try { $doc.Close(0) } catch {}; Release $doc }
  }
  }
  $outDir = Join-Path $refDir $name
  New-Item -ItemType Directory -Force $outDir | Out-Null
  [IO.File]::WriteAllText((Join-Path $outDir 'review.tsv'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  "$name : $revCount revisions, $cc comments"
}

New-Item -ItemType Directory -Force $tmpDir | Out-Null
try {
  if ($Make) {
    foreach ($kv in $Scenarios.GetEnumerator()) {
      if ($Only -and $kv.Key -ne $Only) { continue }
      Write-Output (Make-Fixture $kv.Key $kv.Value)
    }
    foreach ($kv in $XmlScenarios.GetEnumerator()) {
      if ($Only -and $kv.Key -ne $Only) { continue }
      Write-Output (Make-XmlFixture $kv.Key $kv.Value)
    }
  }
  if ($Measure) {
    $files = Get-ChildItem -LiteralPath $fixDir | Where-Object { $_.Name -match '^(rev|cmt)-.*\.docx$' } | Sort-Object Name
    if ($Only) { $files = $files | Where-Object { $_.BaseName -eq $Only } }
    foreach ($f in $files) { Write-Output (Measure-Doc $f.FullName) }
  }
} finally {
  if ($script:word) { try { $script:word.Quit(0) } catch {}; Release $script:word }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  if ($script:wordPid -ne 0) { Get-Process -Id $script:wordPid -ErrorAction SilentlyContinue | Stop-Process -Force }
  Remove-Item -LiteralPath $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
}
