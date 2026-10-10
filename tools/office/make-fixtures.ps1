<#
Generates tier-A DOCX/XLSX fixtures by driving the locally installed Microsoft Word and Excel over COM.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation,
so this script is not run in CI. It writes only under <repo>/fixtures/{docx,xlsx} and $env:TEMP.
Idempotent: existing outputs are overwritten. A fixture that fails is skipped and reported.
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$docxDir = Join-Path $root 'fixtures\docx'
$xlsxDir = Join-Path $root 'fixtures\xlsx'
New-Item -ItemType Directory -Force $docxDir, $xlsxDir | Out-Null
$tmp = $env:TEMP
$created = New-Object System.Collections.Generic.List[string]
$skipped = New-Object System.Collections.Generic.List[string]
$M = [Type]::Missing
Add-Type -AssemblyName System.Drawing

function Write-Manifest($dir, $name, $feature, $app, $notes) {
  $made = "$($app.Name) $($app.Build) (Windows)"
  $t = "feature = `"$feature`"`nmade_with = `"$made`"`nauthor = `"project member`"`nexpect_dropped = []`nnotes = `"$notes`"`n"
  Set-Content -Path (Join-Path $dir "$name.toml") -Value $t -Encoding utf8 -NoNewline
}
function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }

# ---------------- Word ----------------
$wordBefore = Get-Pids 'WINWORD'
$word = $null; $oldUser = $null; $oldInit = $null; $wordPids = @()
try {
  $word = New-Object -ComObject Word.Application
  $word.Visible = $false; $word.DisplayAlerts = 0
  $oldUser = $word.UserName; $oldInit = $word.UserInitials; $oldLocal = $word.Options.UseLocalUserInfo
  $word.Options.UseLocalUserInfo = $true
  $word.UserName = 'project member'; $word.UserInitials = 'PM'
  $wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $wordBefore })

  function W-Para($text, $style = 'Normal') {
    $s = $script:word.Selection; [void]$s.EndKey(6)
    $s.Style = $style; $s.TypeText($text); $s.TypeParagraph()
  }
  function W-Fixture($name, $feature, $notes, [scriptblock]$build) {
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

  W-Fixture 'plain-paragraphs' 'plain-paragraphs' 'three plain paragraphs, default style' {
    W-Para 'First paragraph of plain text.'; W-Para 'Second paragraph with more words in it.'; W-Para 'Third paragraph.'
  }
  W-Fixture 'styles-headings' 'styles-headings' 'Heading 1, Heading 2, custom paragraph style, custom character style' {
    $ps = $doc.Styles.Add('O365Custom', 1); $ps.Font.Name = 'Arial'; $ps.Font.Size = 13; $ps.Font.Italic = 1
    $cs = $doc.Styles.Add('O365Char', 2); $cs.Font.Bold = 1; $cs.Font.Color = 255
    W-Para 'Heading one' 'Heading 1'; W-Para 'Heading two' 'Heading 2'
    W-Para 'Custom paragraph style text.' 'O365Custom'
    $s = $word.Selection; [void]$s.EndKey(6); $s.Style = 'Normal'
    $s.TypeText('Normal text with '); $s.Style = 'O365Char'; $s.TypeText('character styled'); $s.Style = 'Default Paragraph Font'; $s.TypeText(' run.')
  }
  W-Fixture 'numbering' 'numbering' 'bulleted list and three-level numbered list' {
    $s = $word.Selection
    $a = $s.Start; W-Para 'Bullet one'; W-Para 'Bullet two'; W-Para 'Bullet three'
    $doc.Range($a, $s.Start).ListFormat.ApplyListTemplate($word.ListGalleries.Item(1).ListTemplates.Item(1))
    $s.Style = 'Normal'; W-Para 'Separator paragraph.'
    $b = $s.Start; W-Para 'Item one'; W-Para 'Sub item a'; W-Para 'Sub sub item i'; W-Para 'Item two'
    $r = $doc.Range($b, $s.Start)
    $r.ListFormat.ApplyListTemplate($word.ListGalleries.Item(3).ListTemplates.Item(1))
    $r.Paragraphs.Item(2).Range.ListFormat.ListLevelNumber = 2
    $r.Paragraphs.Item(3).Range.ListFormat.ListLevelNumber = 3
  }
  W-Fixture 'table-merged-cells' 'table-merged-cells' '4x4 table with one horizontal and one vertical merge' {
    $s = $word.Selection
    $t = $doc.Tables.Add($s.Range, 4, 4)
    for ($r = 1; $r -le 4; $r++) { for ($c = 1; $c -le 4; $c++) { $t.Cell($r, $c).Range.Text = "r$r c$c" } }
    $t.Borders.Enable = 1
    $t.Cell(1, 1).Merge($t.Cell(1, 2))
    $t.Cell(2, 3).Merge($t.Cell(3, 3))
  }
  W-Fixture 'inline-image' 'inline-image' 'one 32x32 generated PNG as inline picture' {
    $png = Join-Path $tmp 'o365_fixture_img.png'
    $bmp = New-Object System.Drawing.Bitmap 32, 32
    $g = [System.Drawing.Graphics]::FromImage($bmp); $g.Clear([System.Drawing.Color]::SteelBlue)
    $g.FillEllipse([System.Drawing.Brushes]::Orange, 6, 6, 20, 20); $g.Dispose()
    $bmp.Save($png, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    try {
      W-Para 'Text before image.'
      $s = $word.Selection; [void]$s.EndKey(6)
      [void]$doc.InlineShapes.AddPicture($png, $false, $true, $s.Range)
    } finally { Remove-Item $png -Force -ErrorAction SilentlyContinue }
  }
  W-Fixture 'header-footer' 'header-footer' 'different first page; primary footer has PAGE field' {
    W-Para 'Page one body.'
    $s = $word.Selection; [void]$s.EndKey(6); $s.InsertBreak(7); W-Para 'Page two body.'
    $sec = $doc.Sections.Item(1); $sec.PageSetup.DifferentFirstPageHeaderFooter = -1
    $sec.Headers.Item(2).Range.Text = 'First page header'
    $sec.Headers.Item(1).Range.Text = 'Primary header'
    $sec.Footers.Item(2).Range.Text = 'First page footer'
    $fr = $sec.Footers.Item(1).Range; $fr.Text = 'Page '; $fr.Collapse(0)
    [void]$doc.Fields.Add($fr, 33)
  }
  W-Fixture 'comments' 'comments' 'two comments, the first has one reply' {
    W-Para 'Commented sentence one.'; W-Para 'Commented sentence two.'
    $c1 = $doc.Comments.Add($doc.Paragraphs.Item(1).Range, 'First comment')
    [void]$doc.Comments.Add($doc.Paragraphs.Item(2).Range, 'Second comment')
    [void]$c1.Replies.Add($c1.Scope, 'Reply to first comment')
  }
  W-Fixture 'tracked-changes' 'tracked-changes' 'one insertion, one deletion, one formatting change' {
    W-Para 'Delete this sentence. Keep this one. Format this one.'
    $doc.TrackRevisions = $true
    $st = $doc.Paragraphs.Item(1).Range.Start
    [void]$doc.Range($st, $st + 21).Delete()
    $doc.Range($doc.Paragraphs.Item(1).Range.End - 1).InsertAfter(' Inserted text.')
    $p = $doc.Paragraphs.Item(1).Range
    $i = $p.Text.IndexOf('Format this one.')
    $doc.Range($p.Start + $i, $p.Start + $i + 16).Font.Bold = 1
    $doc.TrackRevisions = $false
  }
  W-Fixture 'footnotes-endnotes' 'footnotes-endnotes' 'one footnote and one endnote' {
    W-Para 'Body with footnote here.'; W-Para 'Body with endnote here.'
    $r1 = $doc.Paragraphs.Item(1).Range; [void]$r1.MoveEnd(1, -1); $r1.Collapse(0)
    [void]$doc.Footnotes.Add($r1, $M, 'Footnote text.')
    $r2 = $doc.Paragraphs.Item(2).Range; [void]$r2.MoveEnd(1, -1); $r2.Collapse(0)
    [void]$doc.Endnotes.Add($r2, $M, 'Endnote text.')
  }
  W-Fixture 'fields' 'fields' 'DATE, PAGE, REF to bookmark, TOC' {
    W-Para 'Chapter A' 'Heading 1'; W-Para 'Chapter B' 'Heading 1'
    W-Para 'Bookmarked words.'
    [void]$doc.Bookmarks.Add('target', $doc.Paragraphs.Item(3).Range)
    $s = $word.Selection; [void]$s.EndKey(6)
    [void]$doc.Fields.Add($s.Range, 31); $s.TypeParagraph()
    [void]$doc.Fields.Add($s.Range, 33); $s.TypeParagraph()
    [void]$doc.Fields.Add($s.Range, -1, 'REF target \h', $false); $s.TypeParagraph()
    [void]$doc.TablesOfContents.Add($s.Range, $true, 1, 3)
    [void]$doc.Fields.Update()
  }
  W-Fixture 'sections-landscape' 'sections-landscape' 'portrait section followed by landscape section' {
    W-Para 'Portrait section.'
    $s = $word.Selection; [void]$s.EndKey(6); $s.InsertBreak(2)
    W-Para 'Landscape section.'
    $doc.Sections.Item(2).PageSetup.Orientation = 1
  }
  W-Fixture 'hidden-text' 'hidden-text' 'security: run with Font.Hidden; must be retained or reported, not silently dropped' {
    $s = $word.Selection; $s.TypeText('Visible start. ')
    $s.Font.Hidden = 1; $s.TypeText('HIDDEN-SECRET-TEXT'); $s.Font.Hidden = 0
    $s.TypeText(' Visible end.')
  }
  W-Fixture 'hyperlink-external' 'hyperlink-external' 'security: external hyperlink to https://example.com/ (egress test)' {
    $s = $word.Selection; $s.TypeText('Link: '); $from = $s.Start
    $s.TypeText('example link')
    [void]$doc.Hyperlinks.Add($doc.Range($from, $s.Start), 'https://example.com/open365-fixture', $M, 'tip', 'example link')
  }
  W-Fixture 'custom-xml-part' 'custom-xml-part' 'one custom XML part, namespace urn:open365:fixture' {
    W-Para 'Document carrying a custom XML part.'
    [void]$doc.CustomXMLParts.Add('<root xmlns="urn:open365:fixture"><item>value</item></root>')
  }
} finally {
  if ($word) {
    try { $word.UserName = $oldUser; $word.UserInitials = $oldInit; $word.Options.UseLocalUserInfo = $oldLocal } catch {}
    try { $word.Quit(0) } catch {}; Release $word
  }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($p in $wordPids) { Get-Process -Id $p -ErrorAction SilentlyContinue | Stop-Process -Force }
}

# ---------------- Excel ----------------
$excelBefore = Get-Pids 'EXCEL'
$xl = $null; $oldXUser = $null; $excelPids = @()
try {
  $xl = New-Object -ComObject Excel.Application
  $xl.Visible = $false; $xl.DisplayAlerts = $false
  $oldXUser = $xl.UserName; $xl.UserName = 'project member'
  $excelPids = @(Get-Pids 'EXCEL' | Where-Object { $_ -notin $excelBefore })

  function X-Fixture($name, $feature, $notes, [scriptblock]$build) {
    $wb = $null; $path = Join-Path $xlsxDir "$name.xlsx"
    try {
      if (Test-Path $path) { Remove-Item $path -Force }
      $wb = $script:xl.Workbooks.Add(); $script:wb = $wb; $script:ws = $wb.Worksheets.Item(1)
      & $build
      $wb.RemovePersonalInformation = $true
      $wb.SaveAs($path, 51)
      Write-Manifest $xlsxDir $name $feature $script:xl $notes
      $script:created.Add("xlsx/$name")
    } catch {
      $script:skipped.Add("xlsx/$name : $($_.Exception.Message) @ line $($_.InvocationInfo.ScriptLineNumber)")
      Remove-Item $path, (Join-Path $xlsxDir "$name.toml") -Force -ErrorAction SilentlyContinue
    } finally {
      if ($wb) { try { $wb.Close($false) } catch {}; Release $wb }
      $script:wb = $null; $script:ws = $null
    }
  }

  X-Fixture 'values-types' 'values-types' 'number, string, bool, #N/A error, date' {
    $ws.Range('A1').Value2 = 3.14159; $ws.Range('A2').Value2 = 'text value'; $ws.Range('A3').Value2 = $true
    $ws.Range('A4').Formula = '=NA()'
    $ws.Range('A5').Value2 = ([datetime]'2024-01-15').ToOADate(); $ws.Range('A5').NumberFormat = 'yyyy-mm-dd'
  }
  X-Fixture 'formulas-basic' 'formulas-basic' 'SUM, IF, AVERAGE, ROUND, CONCAT, VLOOKUP' {
    1..5 | ForEach-Object { $ws.Cells.Item($_, 1).Value2 = [double]$_; $ws.Cells.Item($_, 2).Value2 = "item$_" }
    $ws.Range('D1').Formula = '=SUM(A1:A5)'; $ws.Range('D2').Formula = '=IF(D1>10,"big","small")'
    $ws.Range('D3').Formula = '=AVERAGE(A1:A5)'; $ws.Range('D4').Formula = '=ROUND(D3/7,2)'
    $ws.Range('D5').Formula = '=CONCAT(B1,"-",B2)'; $ws.Range('D6').Formula = '=VLOOKUP(3,A1:B5,2,FALSE)'
  }
  X-Fixture 'cross-sheet' 'cross-sheet' 'three sheets referencing each other' {
    $ws.Name = 'One'
    $s2 = $wb.Worksheets.Add($M, $wb.Worksheets.Item(1)); $s2.Name = 'Two'
    $s3 = $wb.Worksheets.Add($M, $wb.Worksheets.Item(2)); $s3.Name = 'Three'
    $ws.Range('A1').Value2 = 10; $s2.Range('A1').Formula = '=One!A1*2'; $s3.Range('A1').Formula = '=Two!A1+One!A1'
    $ws.Range('B1').Formula = '=Three!A1+1'
  }
  X-Fixture 'defined-names' 'defined-names' 'one workbook-scoped and one sheet-scoped name' {
    $ws.Name = 'Data'; $ws.Range('A1').Value2 = 5; $ws.Range('B1').Value2 = 7
    [void]$wb.Names.Add('GlobalRate', '=Data!$A$1'); [void]$ws.Names.Add('LocalVal', '=Data!$B$1')
    $ws.Range('C1').Formula = '=GlobalRate+LocalVal'
  }
  X-Fixture 'shared-formulas' 'shared-formulas' 'formula filled down 100 rows' {
    1..100 | ForEach-Object { $ws.Cells.Item($_, 1).Value2 = [double]$_ }
    $ws.Range('B1:B100').Formula = '=A1*2+1'
  }
  X-Fixture 'dates-1900' 'dates-1900' 'serials 59..62 around 1900-03-01 leap-year quirk (60 = fake 1900-02-29)' {
    59..62 | ForEach-Object { $ws.Cells.Item($_ - 58, 1).Value2 = [double]$_ }
    $ws.Range('A1:A4').NumberFormat = 'yyyy-mm-dd'
    $ws.Range('B1').Formula = '=A3-A1'; $ws.Range('B2').Formula = '=DATE(1900,3,1)'
    $ws.Range('A5').Value2 = ([datetime]'2024-03-01').ToOADate(); $ws.Range('A5').NumberFormat = 'yyyy-mm-dd'
  }
  X-Fixture 'dates-1904' 'dates-1904' 'workbook with Date1904 = true' {
    $wb.Date1904 = $true
    $serials = 0, 1, 1462, 40000
    for ($i = 0; $i -lt 4; $i++) { $ws.Cells.Item($i + 1, 1).Value2 = [double]$serials[$i] }
    $ws.Range('A1:A4').NumberFormat = 'yyyy-mm-dd'; $ws.Range('B1').Formula = '=DATE(2024,1,15)'; $ws.Range('B1').NumberFormat = 'yyyy-mm-dd'
  }
  X-Fixture 'number-formats' 'number-formats' 'currency, percent, custom with colour, date formats' {
    $vals = @(@(1234.5, '"$"#,##0.00'), @(0.256, '0.0%'), @(-12.345, '0.00;[Red]-0.00'), @(12.345, '0.00;[Red]-0.00'), @(45306, 'dd/mm/yyyy'), @(45306, 'dddd, mmmm d, yyyy'), @(0.5, 'h:mm AM/PM'))
    for ($i = 0; $i -lt $vals.Count; $i++) { $c = $ws.Cells.Item($i + 1, 1); $c.Value2 = [double]$vals[$i][0]; $c.NumberFormat = $vals[$i][1] }
  }
  X-Fixture 'merged-cells' 'merged-cells' 'horizontal A1:C1 and vertical A3:A5 merges' {
    $ws.Range('A1').Value2 = 'wide'; $ws.Range('A3').Value2 = 'tall'
    $ws.Range('A1:C1').Merge(); $ws.Range('A3:A5').Merge()
  }
  X-Fixture 'chart' 'chart' 'clustered column chart from A1:B6' {
    $ws.Range('A1').Value2 = 'Item'; $ws.Range('B1').Value2 = 'Qty'
    1..5 | ForEach-Object { $ws.Cells.Item($_ + 1, 1).Value2 = "P$_"; $ws.Cells.Item($_ + 1, 2).Value2 = [double]($_ * 3) }
    $co = $ws.ChartObjects().Add(150, 10, 300, 200)
    $co.Chart.ChartType = 51; $co.Chart.SetSourceData($ws.Range('A1:B6'))
    $co.Chart.HasTitle = $true; $co.Chart.ChartTitle.Text = 'Quantities'
  }
  X-Fixture 'conditional-format' 'conditional-format' '3-colour scale on A1:A10 and expression rule on B1:B10' {
    1..10 | ForEach-Object { $ws.Cells.Item($_, 1).Value2 = [double]($_ * 10); $ws.Cells.Item($_, 2).Value2 = [double]($_ * 10) }
    [void]$ws.Range('A1:A10').FormatConditions.AddColorScale(3)
    $fc = $ws.Range('B1:B10').FormatConditions.Add(2, $M, '=$B1>50'); $fc.Interior.Color = 255
  }
  X-Fixture 'data-validation' 'data-validation' 'list validation Red,Green,Blue on A1:A5' {
    $v = $ws.Range('A1:A5').Validation; $v.Delete(); [void]$v.Add(3, 1, 1, 'Red,Green,Blue')
    $ws.Range('A1').Value2 = 'Red'
  }
  X-Fixture 'comments-notes' 'comments-notes' 'legacy note on A1 (personal info stripped)' {
    $ws.Range('A1').Value2 = 'noted'; [void]$ws.Range('A1').AddComment('This is a note.')
  }
  X-Fixture 'hidden-sheet' 'hidden-sheet' 'security: one Hidden and one VeryHidden sheet; must be retained or reported' {
    $ws.Name = 'Visible'; $ws.Range('A1').Value2 = 'visible'
    $h = $wb.Worksheets.Add($M, $wb.Worksheets.Item(1)); $h.Name = 'HiddenSheet'; $h.Range('A1').Value2 = 'hidden-data'
    $v = $wb.Worksheets.Add($M, $wb.Worksheets.Item(2)); $v.Name = 'VeryHiddenSheet'; $v.Range('A1').Value2 = 'veryhidden-data'
    $h.Visible = 0; $v.Visible = 2
  }
  X-Fixture 'large-sheet' 'large-sheet' '50000 rows x 10 numeric cols plus SUM column K; performance corpus' {
    # Range.Value2 = 2D array fails through this PowerShell COM binder after scalar writes; bulk-load via a temp CSV instead.
    $csv = Join-Path $tmp 'o365_large_sheet.csv'
    try {
      $sb = New-Object System.Text.StringBuilder
      for ($i = 0; $i -lt 50000; $i++) {
        $row = for ($j = 0; $j -lt 10; $j++) { (($i * 10 + $j) % 997) + 0.5 }
        [void]$sb.AppendLine(($row -join ','))
      }
      [IO.File]::WriteAllText($csv, $sb.ToString())
      $src = $xl.Workbooks.Open($csv)
      try { [void]$src.Worksheets.Item(1).UsedRange.Copy($ws.Range('A1')) } finally { $src.Close($false); Release $src }
    } finally { Remove-Item $csv -Force -ErrorAction SilentlyContinue }
    $ws.Range('K1:K50000').Formula = '=SUM(A1:J1)'
  }
  $extDir = Join-Path $env:SystemDrive 'open365-fixture-ext'
  $extSrc = Join-Path $extDir 'source.xlsx'
  try {
    X-Fixture 'external-link-formula' 'external-link-formula' 'security: formula references external workbook (egress); source path is a neutral folder on the system drive, deleted after save' {
      New-Item -ItemType Directory -Force $extDir | Out-Null
      $src = $xl.Workbooks.Add(); $src.Worksheets.Item(1).Range('A1').Value2 = 42; $src.SaveAs($extSrc, 51)
      try { $ws.Range('A1').Formula = "='$extDir\[source.xlsx]Sheet1'!A1" } finally { $src.Close($false); Release $src }
    }
  } finally { Remove-Item $extDir -Recurse -Force -ErrorAction SilentlyContinue }
} finally {
  if ($xl) { try { $xl.UserName = $oldXUser } catch {}; try { $xl.Quit() } catch {}; Release $xl }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($p in $excelPids) { Get-Process -Id $p -ErrorAction SilentlyContinue | Stop-Process -Force }
}

"Created: $($created.Count)"; $created | ForEach-Object { "  $_" }
"Skipped: $($skipped.Count)"; $skipped | ForEach-Object { "  $_" }
