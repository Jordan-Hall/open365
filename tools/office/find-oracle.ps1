<#
Find and replace oracle: Microsoft Word 16.0 (Find.Execute, wdReplaceAll) and Microsoft Excel 16.0 (Range.Find,
Range.Replace) over generated fixtures. Records what each application matched and what the document or workbook held
afterwards, so crates/commands/tests/find.rs can hold Open365's find and replace to the same results.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed output:
  fixtures/reference/docx/find-runs/find.json   (Word)
  fixtures/reference/xlsx/find-cells/find.json  (Excel)

  pwsh -NoProfile -File tools/office/find-oracle.ps1 [-Word] [-Excel] [-MakeFixtures]

-MakeFixtures (re)creates fixtures/docx/find-runs.docx and fixtures/xlsx/find-cells.xlsx through COM. Nothing is
hand-edited. Without -Word or -Excel both run. The fixtures are only opened for the scenarios, never saved: each
scenario works on a fresh copy opened from disk and closed without saving.

WORD. Each scenario counts the matches with a Find loop (wdFindStop, no wrap), then runs Find.Execute with
Replace:=wdReplaceAll (2) and records every paragraph's text (tab = \t, manual line break = \n) and, per character, a
format digit: bold 1 + italic 2 + underline 4 + non-automatic colour 8, in hex. The match count is the oracle's replace
count: Execute returns only True or False.

EXCEL. Replace scenarios record the cells a Find loop reports before the replace (formulas, same options), then run
Range.Replace on the sheet or on every worksheet, and record the formula text (Range.Formula) of every non-empty
cell. Excel does not report how many occurrences it replaced, only which cells changed. Find scenarios record the cells
Range.Find / FindNext visit, in order, starting from the top-left cell.

Process safety: each application is started by this script (its own instance, AutomationSecurity =
msoAutomationSecurityForceDisable (3)), the process id is remembered and only that process is ever stopped (in a
finally block). Other WINWORD / EXCEL processes and the Office and regional settings are never touched. The output has
repo-relative names only; docProps/core.xml dc:creator and cp:lastModifiedBy are scrubbed from the fixtures.
#>
param(
  [switch]$Word,
  [switch]$Excel,
  [switch]$MakeFixtures
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Word -and -not $Excel) { $Word = $true; $Excel = $true }
$wordFixture = Join-Path $root 'fixtures/docx/find-runs.docx'
$excelFixture = Join-Path $root 'fixtures/xlsx/find-cells.xlsx'
$wordOut = Join-Path $root 'fixtures/reference/docx/find-runs/find.json'
$excelOut = Join-Path $root 'fixtures/reference/xlsx/find-cells/find.json'

Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }

function Scrub-Core([string]$path) {
  $zip = [IO.Compression.ZipFile]::Open($path, 'Update')
  try {
    $entry = $zip.GetEntry('docProps/core.xml')
    if ($entry) {
      $reader = New-Object IO.StreamReader($entry.Open(), (New-Object Text.UTF8Encoding $false))
      $xml = $reader.ReadToEnd(); $reader.Close()
      $xml = [regex]::Replace($xml, '<dc:creator>.*?</dc:creator>', '<dc:creator></dc:creator>')
      $xml = [regex]::Replace($xml, '<cp:lastModifiedBy>.*?</cp:lastModifiedBy>', '<cp:lastModifiedBy></cp:lastModifiedBy>')
      $entry.Delete()
      $new = $zip.CreateEntry('docProps/core.xml')
      $w = New-Object IO.StreamWriter($new.Open(), (New-Object Text.UTF8Encoding $false))
      $w.Write($xml); $w.Close()
    }
  }
  finally { $zip.Dispose() }
}

# ASCII-only JSON with LF line endings.
function Write-Json($obj, [string]$path) {
  $json = $obj | ConvertTo-Json -Depth 20
  $sb = New-Object Text.StringBuilder
  foreach ($ch in $json.ToCharArray()) {
    if ([int]$ch -gt 126) { [void]$sb.AppendFormat('\u{0:x4}', [int]$ch) } else { [void]$sb.Append($ch) }
  }
  New-Item -ItemType Directory -Force (Split-Path -Parent $path) | Out-Null
  [IO.File]::WriteAllText($path, ($sb.ToString() -replace "`r`n", "`n") + "`n", (New-Object Text.UTF8Encoding $false))
}

# ---------------------------------------------------------------------------------------------------------------
# Word
# ---------------------------------------------------------------------------------------------------------------
$script:wd = $null
$script:wordPids = @()

function Start-Word {
  $before = Get-Pids 'WINWORD'
  $app = New-Object -ComObject Word.Application
  $script:wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $before })
  if ($script:wordPids.Count -eq 0) { throw 'COM attached to a Word instance this script did not start; refusing to touch it' }
  $app.Visible = $false
  $app.DisplayAlerts = 0
  $app.AutomationSecurity = 3
  $script:wd = $app
}

function Stop-Word {
  $app = $script:wd
  $script:wd = $null
  if ($app) {
    try { foreach ($d in @($app.Documents)) { try { $d.Close(0) } catch {} } } catch {}
    try { $app.Quit(0) } catch {}
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
  }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($p in $script:wordPids) {
    $deadline = (Get-Date).AddSeconds(8)
    while ((Get-Date) -lt $deadline -and (Get-Process -Id $p -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
    $proc = Get-Process -Id $p -ErrorAction SilentlyContinue
    if ($proc -and $proc.ProcessName -eq 'WINWORD') { Stop-Process -Id $p -Force -ErrorAction SilentlyContinue }
  }
  $script:wordPids = @()
}

# Text of the fixture's paragraphs, in order. Table cells come last (Word adds the table after the text).
$wordParagraphs = @(
  'The Cat sat on the cat mat. CAT cat.',
  'concatenate category cat-like scatter concat.',
  'Hello World hello world HELLO',
  'alpha1 beta22 gamma333 delta',
  'foo_bar foo-bar foo bar foobar',
  "tab`tseparated and line$([char]11)break",
  'a*b a?b axxb a b',
  '[bracket] (paren) caret^ back\slash',
  "$([char]0xDC)n$([char]0xEF) c$([char]0xF6)de $([char]0xDC)N$([char]0xCF) $([char]0xFC)n$([char]0xEF)",
  'foobar and foobar',
  'x1 x12 x123 x1234',
  '  leading and trailing  ',
  'word Word WORD wOrd WOrD a A Ab AB'
)
# Character formatting by paragraph (1-based): offset, length, bold, italic, underline, colour (BGR, 0 = none).
$wordFormats = @(
  @(1, 5, 2, 1, 0, 0, 0), @(1, 17, 4, 0, 1, 0, 0),
  @(3, 3, 5, 1, 0, 0, 0), @(3, 4, 5, 0, 0, 0, 255),
  @(5, 3, 3, 1, 0, 0, 0),
  @(10, 3, 3, 1, 0, 0, 0), @(10, 7, 3, 0, 0, 1, 0), @(10, 14, 3, 1, 0, 0, 0),
  @(11, 4, 1, 0, 0, 1, 0)
)

function New-WordFixture {
  Start-Word
  try {
    $doc = $script:wd.Documents.Add()
    $doc.Content.Text = (($wordParagraphs -join "`r") + "`r")
    $t = $doc.Tables.Add($doc.Paragraphs.Item($doc.Paragraphs.Count).Range, 2, 2)
    $t.Cell(1, 1).Range.Text = 'cat in table'
    $t.Cell(1, 2).Range.Text = 'Cat Cat'
    $t.Cell(2, 1).Range.Text = 'no match'
    $t.Cell(2, 2).Range.Text = 'concat'
    foreach ($f in $wordFormats) {
      $p = $doc.Paragraphs.Item($f[0])
      $r = $doc.Range($p.Range.Start + $f[1], $p.Range.Start + $f[1] + $f[2])
      if ($f[3]) { $r.Font.Bold = 1 }
      if ($f[4]) { $r.Font.Italic = 1 }
      if ($f[5]) { $r.Font.Underline = 1 }
      if ($f[6]) { $r.Font.Color = $f[6] }
    }
    New-Item -ItemType Directory -Force (Split-Path -Parent $wordFixture) | Out-Null
    $doc.SaveAs2($wordFixture, 16)
    $doc.Close(0)
  }
  finally { Stop-Word }
  Scrub-Core $wordFixture
}

# Hex digit per character: bold 1, italic 2, underline 4, non-automatic colour 8. A property that is uniform over the
# paragraph (-1 or 0; wdUndefined 9999999 means mixed) is read once.
function Read-WordDoc($doc) {
  $out = @()
  foreach ($p in @($doc.Paragraphs)) {
    $r = $p.Range
    $raw = $r.Text
    if ($raw -eq "`r`a" -or $raw -eq "`a") { continue }   # end-of-row mark
    $text = $raw -replace "[`r`a]+$", ''
    $text = $text -replace [string][char]11, "`n"
    $len = $text.Length
    $flags = @(
      @{ bit = 1; get = { param($f) $f.Bold; } },
      @{ bit = 2; get = { param($f) $f.Italic; } },
      @{ bit = 4; get = { param($f) $f.Underline; } },
      @{ bit = 8; get = { param($f) $f.Color; } }
    )
    $digits = New-Object 'int[]' $len
    foreach ($fl in $flags) {
      $whole = & $fl.get $r.Font
      $isOn = { param($v) if ($fl.bit -eq 8) { $v -ne -16777216 } else { $v -ne 0 } }
      if ($whole -ne 9999999) {
        if (& $isOn $whole) { for ($k = 0; $k -lt $len; $k++) { $digits[$k] += $fl.bit } }
        continue
      }
      for ($k = 0; $k -lt $len; $k++) {
        $c = $doc.Range($r.Start + $k, $r.Start + $k + 1)
        if (& $isOn (& $fl.get $c.Font)) { $digits[$k] += $fl.bit }
      }
    }
    $fmt = -join ($digits | ForEach-Object { $_.ToString('x') })
    $out += [ordered]@{ text = $text; fmt = $fmt }
  }
  , $out
}

# name, find, replace, matchCase, wholeWord, wildcards
$wordScenarios = @(
  @('plain', 'cat', 'dog', $false, $false, $false),
  @('match-case', 'cat', 'dog', $true, $false, $false),
  @('whole-word', 'cat', 'dog', $false, $true, $false),
  @('whole-word-match-case', 'cat', 'dog', $true, $true, $false),
  @('delete', 'cat', '', $false, $false, $false),
  @('found-text-code', 'cat', '[^&]', $false, $false, $false),
  @('grows', 'cat', 'cats', $false, $false, $false),
  @('spans-runs', 'Cat sat', 'Dog', $false, $false, $false),
  @('spans-runs-2', 'e cat', 'E', $false, $false, $false),
  @('first-run-format', 'obar', 'ZZ', $false, $false, $false),
  @('first-run-format-bold', 'ar and f', 'Q', $false, $false, $false),
  @('single-char', 'x', 'yy', $false, $false, $false),
  @('tab', '^t', ' ', $false, $false, $false),
  @('tab-in-text', 'tab^tsep', 'T', $false, $false, $false),
  @('line-break', '^l', '|', $false, $false, $false),
  @('caret-literal', '^^', '!', $false, $false, $false),
  @('replace-tab', ' and', '^t', $false, $false, $false),
  @('replace-caret', 'bracket', 'a^^b', $false, $false, $false),
  @('unicode', "$([char]0xFC)n$([char]0xEF)", 'X', $false, $false, $false),
  @('unicode-match-case', "$([char]0xFC)n$([char]0xEF)", 'X', $true, $false, $false),
  @('star-is-literal', 'a*b', 'Z', $false, $false, $false),
  @('whole-word-underscore', 'foo', 'Z', $false, $true, $false),
  @('whole-word-digits', 'x1', 'Z', $false, $true, $false),
  @('spaces', '  leading', 'L', $false, $false, $false),
  @('wc-qmark', 'a?b', 'Z', $false, $false, $true),
  @('wc-star', 'x1*3', 'Z', $false, $false, $true),
  @('wc-star-lazy', 'f*r', 'Z', $false, $false, $true),
  @('wc-star-crosses-paragraphs', 'i*B', 'Z', $false, $false, $true),
  @('wc-star-prefix', 'T*t', 'Z', $false, $false, $true),
  @('wc-class', '[a-c]at', 'Z', $false, $false, $true),
  @('wc-class-list', '[cm]at', 'Z', $false, $false, $true),
  @('wc-negated-class', '[!c]at', 'Z', $false, $false, $true),
  @('wc-word-start', '<cat', 'Z', $false, $false, $true),
  @('wc-word-end', 'cat>', 'Z', $false, $false, $true),
  @('wc-whole-word', '<cat>', 'Z', $false, $false, $true),
  @('wc-at', 'x[0-9]@', 'Z', $false, $false, $true),
  @('wc-count-exact', '[0-9]{2}', 'Z', $false, $false, $true),
  @('wc-count-min', '[0-9]{2,}', 'Z', $false, $false, $true),
  @('wc-count-range', '[0-9]{1,2}', 'Z', $false, $false, $true),
  @('wc-case-sensitive', 'hello', 'Z', $false, $false, $true),
  @('wc-case-sensitive-capital', 'Hello', 'Z', $false, $false, $true),
  @('wc-escape-star', 'a\*b', 'Z', $false, $false, $true),
  @('wc-escape-bracket', '\[bracket\]', 'Z', $false, $false, $true),
  @('wc-found-text-code', 'x[0-9]@', '<^&>', $false, $false, $true),
  @('wc-at-literal', 'fo@', 'Z', $false, $false, $true),
  @('wc-at-class', '[o]@', 'Z', $false, $false, $true),
  @('wc-at-any', 'f?@', 'Z', $false, $false, $true),
  @('wc-at-lazy-literal', 'fo@_', 'Z', $false, $false, $true),
  @('wc-at-lazy-class', 'x[0-9]@ ', 'Z', $false, $false, $true),
  @('wc-at-lazy-any', 'f?@_', 'Z', $false, $false, $true),
  @('wc-at-lazy-longer', 'a[a-z]@a', 'Z', $false, $false, $true),
  @('wc-digits-run', '[0-9]@', '#', $false, $false, $true),
  @('wc-empty-replace', '[0-9]@', '', $false, $false, $true),
  @('found-text-twice', 'Cat', '<^&^&>', $false, $false, $false),
  @('case-lower', 'word', 'dog', $false, $false, $false),
  @('case-mixed', 'word', 'dOg', $false, $false, $false),
  @('case-title', 'word', 'Dog', $false, $false, $false),
  @('case-upper', 'word', 'DOG', $false, $false, $false),
  @('case-single', 'word', 'd', $false, $false, $false),
  @('case-multiword', 'word', 'dog cat', $false, $false, $false),
  @('case-digit-lead', 'word', '1dog', $false, $false, $false),
  @('case-two-letters', 'ab', 'xy', $false, $false, $false),
  @('case-single-letter', 'a', 'x', $false, $true, $false),
  @('case-match-case', 'Word', 'dog', $true, $false, $false),
  @('case-wildcards', 'wor?', 'dog', $false, $false, $true),
  @('case-tail-mixed', 'ord', 'dog', $false, $false, $false),
  @('wc-qmark-run', 'f?o', 'Z', $false, $false, $true),
  @('wc-star-end', 'foo*bar', 'Z', $false, $false, $true)
)

function Open-WordTemp {
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ('find-oracle-' + [guid]::NewGuid().ToString('N') + '.docx')
  Copy-Item $wordFixture $tmp
  $doc = $script:wd.Documents.Open($tmp, $false, $false, $false)
  $doc.TrackRevisions = $false
  [pscustomobject]@{ Doc = $doc; Path = $tmp }
}

function Invoke-WordScenarios {
  Start-Word
  try {
    $results = @()
    $h = Open-WordTemp
    try { $base = Read-WordDoc $h.Doc; $h.Doc.Close(0) } finally { Remove-Item $h.Path -Force -ErrorAction SilentlyContinue }
    foreach ($s in $wordScenarios) {
      $name, $find, $repl, $mc, $ww, $wc = $s
      $entry = [ordered]@{ name = $name; find = $find; replace = $repl; matchCase = $mc; wholeWord = $ww; wildcards = $wc }
      $h = Open-WordTemp
      try {
        $doc = $h.Doc
        # Count with a Find loop (Execute only says whether replace-all found something).
        $count = 0
        $r = $doc.Content
        $f = $r.Find
        $f.ClearFormatting()
        $f.Text = $find; $f.MatchCase = $mc; $f.MatchWholeWord = $ww; $f.MatchWildcards = $wc
        $f.MatchSoundsLike = $false; $f.MatchAllWordForms = $false; $f.Forward = $true; $f.Wrap = 0; $f.Format = $false
        while ($count -lt 500 -and $f.Execute()) { $count++; $r.Collapse(0) }
        $rng = $doc.Content
        $rng.Find.ClearFormatting(); $rng.Find.Replacement.ClearFormatting()
        [void]$rng.Find.Execute($find, $mc, $ww, $wc, $false, $false, $true, 0, $false, $repl, 2)
        $entry['count'] = $count
        $entry['paragraphs'] = Read-WordDoc $doc
        $doc.Close(0)
      }
      catch { $entry['error'] = $_.Exception.Message }
      finally { Remove-Item $h.Path -Force -ErrorAction SilentlyContinue }
      $results += $entry
      Write-Host ("word  {0,-28} count {1}" -f $name, $entry['count'])
    }
    Write-Json ([ordered]@{ fixture = 'fixtures/docx/find-runs.docx'; app = "Microsoft Word $($script:wd.Version)"; base = $base; scenarios = $results }) $wordOut
  }
  finally { Stop-Word }
}

# ---------------------------------------------------------------------------------------------------------------
# Excel
# ---------------------------------------------------------------------------------------------------------------
$script:xl = $null
$script:xlPid = 0

function Start-Excel {
  $app = New-Object -ComObject Excel.Application
  $app.Visible = $false
  $app.DisplayAlerts = $false
  $app.EnableEvents = $false
  $app.AutomationSecurity = 3
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  $script:xl = $app
  $script:xlPid = [int]$procId
}

function Stop-Excel {
  $app = $script:xl
  $script:xl = $null
  if ($app) {
    try { foreach ($wb in @($app.Workbooks)) { try { $wb.Saved = $true; $wb.Close($false) } catch {} } } catch {}
    try { $app.Quit() } catch {}
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
  }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  if ($script:xlPid -gt 0) {
    $deadline = (Get-Date).AddSeconds(8)
    while ((Get-Date) -lt $deadline -and (Get-Process -Id $script:xlPid -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
    $p = Get-Process -Id $script:xlPid -ErrorAction SilentlyContinue
    if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $script:xlPid -Force -ErrorAction SilentlyContinue }
  }
  $script:xlPid = 0
}

function New-ExcelFixture {
  Start-Excel
  try {
    $wb = $script:xl.Workbooks.Add()
    while ($wb.Worksheets.Count -lt 3) { [void]$wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
    $s1 = $wb.Worksheets.Item(1); $s1.Name = 'S1'
    $s2 = $wb.Worksheets.Item(2); $s2.Name = 'S2'
    $s3 = $wb.Worksheets.Item(3); $s3.Name = 'S3'
    $s1.Range('A1').Value2 = 'Apple';      $s1.Range('B1').Value2 = 'apple pie'; $s1.Range('C1').Value2 = 'APPLE'
    $s1.Range('D1').Value2 = [double]123;  $s1.Range('E1').Formula = '=A1&"x"'
    $s1.Range('F1').Value2 = 'a*b';        $s1.Range('G1').Value2 = 'axb';       $s1.Range('H1').Value2 = $true
    $s1.Range('A2').Value2 = [double]12;   $s1.Range('B2').Formula = '=SUM(A2:A3)'
    $s1.Range('C2').Value2 = 'banana';     $s1.Range('D2').Value2 = 'Apple Apple'; $s1.Range('E2').Value2 = 'pineapple'
    $s1.Range('F2').Value2 = 'a?b';        $s1.Range('G2').Value2 = 'A1';        $s1.Range('H2').Value2 = [double]0.5
    $s1.Range('A3').Formula = '=A2*2';     $s1.Range('B3').Value2 = '~tilde';    $s1.Range('C3').Value2 = 'Apple'
    $s1.Range('D3').Value2 = '12 apples';  $s1.Range('E3').Formula = '=IF(A2>1,"apple","none")'; $s1.Range('F3').Value2 = [double]1.5
    $s1.Range('A4').Value2 = '  space ';    $s1.Range('B4').Value2 = 'x'
    $s2.Range('A1').Value2 = 'apple';      $s2.Range('B1').Value2 = [double]12;  $s2.Range('C1').Formula = '=S1!A1&"!"'
    $s2.Range('A2').Value2 = 'Apple tree'; $s2.Range('B2').Value2 = 'grape'
    $s3.Range('A1').Value2 = 'no match';   $s3.Range('B1').Value2 = 'apple'
    $s1.Activate(); $s1.Range('A1').Select()
    New-Item -ItemType Directory -Force (Split-Path -Parent $excelFixture) | Out-Null
    $wb.SaveAs($excelFixture, 51)
    $wb.Close($false)
  }
  finally { Stop-Excel }
  Scrub-Core $excelFixture
}

function Cell-Name($c) { "$($c.Worksheet.Name)!$($c.Address($false, $false))" }

function Find-Cells($sheets, $what, [int]$lookIn, [int]$lookAt, [int]$order, [bool]$mc) {
  $hits = @()
  foreach ($ws in $sheets) {
    $after = $ws.Cells.Item($ws.Rows.Count, $ws.Columns.Count)
    $first = $ws.Cells.Find($what, $after, $lookIn, $lookAt, $order, 1, $mc, $false, $false)
    if ($null -eq $first) { continue }
    $addr = $first.Address()
    $c = $first
    $n = 0
    do {
      $hits += (Cell-Name $c)
      $c = $ws.Cells.FindNext($c)
      $n++
    } while ($null -ne $c -and $c.Address() -ne $addr -and $n -lt 5000)
  }
  , $hits
}

function Read-Workbook($wb) {
  $out = @()
  foreach ($ws in $wb.Worksheets) {
    foreach ($c in $ws.UsedRange.Cells) {
      $f = [string]$c.Formula
      if ($f -eq '') { continue }
      $out += [ordered]@{ cell = (Cell-Name $c); formula = $f }
    }
  }
  , $out
}

# name, find, replacement, scope (sheet|workbook), lookAt (part|whole), matchCase
$excelReplace = @(
  @('part', 'apple', 'pear', 'sheet', 'part', $false),
  @('part-match-case', 'apple', 'pear', 'sheet', 'part', $true),
  @('whole', 'apple', 'pear', 'sheet', 'whole', $false),
  @('whole-match-case', 'Apple', 'pear', 'sheet', 'whole', $true),
  @('workbook-part', 'apple', 'pear', 'workbook', 'part', $false),
  @('workbook-whole', 'apple', 'pear', 'workbook', 'whole', $false),
  @('workbook-numbers', '12', '77', 'workbook', 'part', $false),
  @('delete', 'apple', '', 'sheet', 'part', $false),
  @('two-in-a-cell', 'Apple', 'X', 'sheet', 'part', $true),
  @('star-part', 'a*e', 'Z', 'sheet', 'part', $false),
  @('star-whole', 'a*e', 'Z', 'sheet', 'whole', $false),
  @('qmark-part', 'ap?le', 'Z', 'sheet', 'part', $false),
  @('qmark-whole', 'a?b', 'Z', 'sheet', 'whole', $false),
  @('escaped-star', 'a~*b', 'Z', 'sheet', 'part', $false),
  @('escaped-qmark', 'a~?b', 'Z', 'sheet', 'part', $false),
  @('escaped-tilde', '~~tilde', 'Z', 'sheet', 'part', $false),
  @('star-lazy', 'a*p', 'Z', 'sheet', 'part', $true),
  @('numbers-part', '1', '9', 'sheet', 'part', $false),
  @('numbers-whole', '12', '7', 'sheet', 'whole', $false),
  @('decimal', '1.5', '2.5', 'sheet', 'whole', $false),
  @('formula-text', 'A2', 'A3', 'sheet', 'part', $false),
  @('formula-function', 'SUM', 'AVERAGE', 'sheet', 'part', $false),
  @('formula-entire', '=A2*2', '=A2*3', 'sheet', 'whole', $false),
  @('formula-equals', '=', '', 'sheet', 'part', $false),
  @('boolean', 'TRUE', 'x', 'sheet', 'part', $false),
  @('boolean-whole', 'true', 'x', 'sheet', 'whole', $false),
  @('to-number', 'pie', '', 'sheet', 'part', $false),
  @('to-number-2', 'Apple', '5', 'sheet', 'whole', $true),
  @('to-formula', 'Apple', '=1+1', 'sheet', 'whole', $true),
  @('spaces', ' ', '_', 'sheet', 'part', $false),
  @('replacement-star-literal', 'apple', 'a*', 'sheet', 'part', $false)
)
# name, find, scope, lookIn (formulas|values), lookAt, searchOrder (rows|columns), matchCase
$excelFind = @(
  @('rows', 'apple', 'sheet', 'formulas', 'part', 'rows', $false),
  @('columns', 'apple', 'sheet', 'formulas', 'part', 'columns', $false),
  @('workbook-rows', 'apple', 'workbook', 'formulas', 'part', 'rows', $false),
  @('workbook-columns', 'apple', 'workbook', 'formulas', 'part', 'columns', $false),
  @('whole-match-case', 'Apple', 'sheet', 'formulas', 'whole', 'rows', $true),
  @('formulas-a2', 'A2', 'sheet', 'formulas', 'part', 'rows', $false),
  @('values-a2', 'A2', 'sheet', 'values', 'part', 'rows', $false),
  @('values-numbers', '12', 'sheet', 'values', 'part', 'rows', $false),
  @('formulas-numbers', '12', 'sheet', 'formulas', 'part', 'rows', $false),
  @('values-formula-result', 'applex', 'sheet', 'values', 'part', 'rows', $false),
  @('formulas-formula-result', 'applex', 'sheet', 'formulas', 'part', 'rows', $false),
  @('values-whole', 'apple', 'sheet', 'values', 'whole', 'rows', $false),
  @('values-boolean', 'true', 'sheet', 'values', 'whole', 'rows', $false),
  @('star', 'a*e', 'sheet', 'values', 'part', 'columns', $false),
  @('none', 'zzz', 'sheet', 'formulas', 'part', 'rows', $false)
)

function Invoke-ExcelScenarios {
  Start-Excel
  try {
    $results = [ordered]@{ fixture = 'fixtures/xlsx/find-cells.xlsx'; app = "Microsoft Excel $($script:xl.Version)" }
    $replaceOut = @()
    $copy = {
      $tmp = Join-Path ([IO.Path]::GetTempPath()) ("find-oracle-" + [guid]::NewGuid().ToString('N') + '.xlsx')
      Copy-Item $excelFixture $tmp
      $tmp
    }
    $lookAtOf = @{ part = 2; whole = 1 }
    $orderOf = @{ rows = 1; columns = 2 }
    $lookInOf = @{ formulas = -4123; values = -4163 }
    foreach ($s in $excelReplace) {
      $name, $find, $repl, $scope, $lookAt, $mc = $s
      $entry = [ordered]@{ name = $name; find = $find; replacement = $repl; scope = $scope; sheet = 0; lookAt = $lookAt; matchCase = $mc }
      $tmp = & $copy
      try {
        $wb = $script:xl.Workbooks.Open($tmp, 0, $false)
        $sheets = if ($scope -eq 'workbook') { @($wb.Worksheets) } else { @($wb.Worksheets.Item(1)) }
        $entry['cells'] = Find-Cells $sheets $find -4123 $lookAtOf[$lookAt] 1 $mc
        foreach ($ws in $sheets) {
          [void]$ws.Cells.Replace($find, $repl, $lookAtOf[$lookAt], 1, $mc, $false, $false, $false)
        }
        $entry['result'] = Read-Workbook $wb
        $wb.Close($false)
      }
      catch { $entry['error'] = $_.Exception.Message }
      finally { Remove-Item $tmp -Force -ErrorAction SilentlyContinue }
      $replaceOut += $entry
      Write-Host ("excel replace {0,-26} cells {1}" -f $name, @($entry['cells']).Count)
    }
    $results['replace'] = $replaceOut
    $findOut = @()
    $wb = $script:xl.Workbooks.Open($excelFixture, 0, $true)
    try {
      foreach ($s in $excelFind) {
        $name, $find, $scope, $lookIn, $lookAt, $order, $mc = $s
        $entry = [ordered]@{ name = $name; find = $find; scope = $scope; sheet = 0; lookIn = $lookIn; lookAt = $lookAt; searchOrder = $order; matchCase = $mc }
        $sheets = if ($scope -eq 'workbook') { @($wb.Worksheets) } else { @($wb.Worksheets.Item(1)) }
        $entry['hits'] = Find-Cells $sheets $find $lookInOf[$lookIn] $lookAtOf[$lookAt] $orderOf[$order] $mc
        $findOut += $entry
        Write-Host ("excel find    {0,-26} hits {1}" -f $name, @($entry['hits']).Count)
      }
    }
    finally { $wb.Close($false) }
    $results['find'] = $findOut
    Write-Json $results $excelOut
  }
  finally { Stop-Excel }
}

if ($MakeFixtures) {
  if ($Word) { New-WordFixture; Write-Host "wrote $wordFixture" }
  if ($Excel) { New-ExcelFixture; Write-Host "wrote $excelFixture" }
}
if ($Word) { Invoke-WordScenarios }
if ($Excel) { Invoke-ExcelScenarios }
