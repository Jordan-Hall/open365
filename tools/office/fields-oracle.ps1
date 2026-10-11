<#
Word field oracle: Microsoft Word 16.0 (COM) as the reference for Open365's field update (crates/docx, fields*.rs).

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed output through crates/docx/tests/fields.rs:
  fixtures/docx/fields-<set>.docx                      fixture with deliberately stale field results
  fixtures/reference/docx/fields-<set>/fields.json     what Word produced after Fields.Update / TablesOfContents.Update

  pwsh -NoProfile -File tools/office/fields-oracle.ps1 [-Set format,date,props,refs,toc] [-Probe codes.txt]

Per set the script (1) builds the fixture through COM (every field is inserted with Fields.Add and then given the
result text STALE, so the fixture needs an update), saves it as .docx and pins docProps/core.xml and custom.xml to
fixed values (dates, author, title, ...), (2) re-opens the saved file in Word, runs Document.Fields.Update (and
TablesOfContents.Update for TOC fields), and (3) records, for every field in Document.Fields, its code and result
text and, for every paragraph, its style name, text and page. Nothing is hand-edited.

Result text follows Range.Text with field codes hidden: tab = \t, manual line break = \u000b, paragraph end = \r.
The JSON also records the context a caller has to supply (the UTC offset Word applied to pinned instants, the
clock reading around a DATE/TIME update, the file name, the page of every paragraph, the number of pages) so the
test can feed the same inputs.

-Probe <file> runs one code per line (optionally "@LCID|code") in a scratch document, prints "code => result" and
writes nothing to the repository.

Process safety: the script starts its own hidden Word instance with AutomationSecurity = 3 (disable macros),
remembers the process id and only ever stops that process (in a finally block). Other WINWORD/EXCEL processes and
the Office, Windows and regional settings are never touched. Output is stable: LF line endings, ASCII only
(non-ASCII is \uXXXX escaped), no absolute paths, no user data (dc:creator / lastModifiedBy are pinned).
#>
param(
  [string[]]$Set = @('format', 'date', 'props', 'refs', 'toc'),
  [string]$Probe,
  [string]$LocaleTable,
  [switch]$TocStyles
)
$ErrorActionPreference = 'Stop'
$Set = @($Set | ForEach-Object { $_ -split ',' })
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/docx'
$refRoot = Join-Path $root 'fixtures/reference/docx'
$scratch = Join-Path ([IO.Path]::GetTempPath()) ('o365-fields-' + [Guid]::NewGuid().ToString('N'))

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }

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

# ---- JSON ----------------------------------------------------------------------------------------------------------
function Json-Str($s) {
  if ($null -eq $s) { return 'null' }
  $sb = New-Object System.Text.StringBuilder
  [void]$sb.Append('"')
  foreach ($ch in ([string]$s).ToCharArray()) {
    $code = [int]$ch
    if ($ch -eq '"') { [void]$sb.Append('\"') }
    elseif ($ch -eq '\') { [void]$sb.Append('\\') }
    elseif ($code -lt 32 -or $code -gt 126) { [void]$sb.AppendFormat('\u{0:x4}', $code) }
    else { [void]$sb.Append($ch) }
  }
  [void]$sb.Append('"')
  return $sb.ToString()
}

function Write-Text([string]$path, [string]$text) {
  $dir = Split-Path -Parent $path
  if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
  [IO.File]::WriteAllText($path, ($text -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding $false))
}

# ---- zip edits (pin core / custom properties) -----------------------------------------------------------------------
function Edit-ZipText([string]$path, [string]$entryName, [scriptblock]$edit) {
  $zip = [IO.Compression.ZipFile]::Open($path, 'Update')
  try {
    $entry = $zip.GetEntry($entryName)
    if (-not $entry) { return }
    $reader = New-Object IO.StreamReader($entry.Open(), (New-Object Text.UTF8Encoding $false))
    $xml = $reader.ReadToEnd(); $reader.Close()
    $xml = & $edit $xml
    $entry.Delete()
    $new = $zip.CreateEntry($entryName)
    $w = New-Object IO.StreamWriter($new.Open(), (New-Object Text.UTF8Encoding $false))
    $w.Write($xml); $w.Close()
  }
  finally { $zip.Dispose() }
}

function Pin-Core([string]$path, $core) {
  Edit-ZipText $path 'docProps/core.xml' {
    param($xml)
    foreach ($k in $core.Keys) {
      $v = [Security.SecurityElement]::Escape([string]$core[$k])
      $pattern = "<$k(\s[^>]*)?>.*?</$k>"
      if ([regex]::IsMatch($xml, $pattern)) {
        $xml = [regex]::Replace($xml, $pattern, { param($m) "<$k$($m.Groups[1].Value)>$v</$k>" })
      }
      else {
        $xml = $xml -replace '</cp:coreProperties>', "<$k>$v</$k></cp:coreProperties>"
      }
    }
    $xml
  }
}

function Pin-Custom([string]$path, $values) {
  # $values: property name -> ISO instant. Replaces the vt:filetime of each named date property.
  Edit-ZipText $path 'docProps/custom.xml' {
    param($xml)
    foreach ($k in $values.Keys) {
      $pattern = '(<property [^>]*name="' + [regex]::Escape($k) + '"[^>]*>)<vt:filetime>[^<]*</vt:filetime>'
      $iso = $values[$k]
      $xml = [regex]::Replace($xml, $pattern, { param($m) $m.Groups[1].Value + '<vt:filetime>' + $iso + '</vt:filetime>' })
    }
    $xml
  }
}

# ---- document properties (late-bound COM collections need InvokeMember) -----------------------------------------------
function Set-BuiltIn($doc, [string]$name, $value) {
  $bf = [Reflection.BindingFlags]
  $props = $doc.BuiltInDocumentProperties
  $prop = [System.__ComObject].InvokeMember('Item', $bf::GetProperty, $null, $props, @($name))
  [void][System.__ComObject].InvokeMember('Value', $bf::SetProperty, $null, $prop, @($value))
}
function Add-Custom($doc, [string]$name, [int]$type, $value) {
  $bf = [Reflection.BindingFlags]
  $props = $doc.CustomDocumentProperties
  [void][System.__ComObject].InvokeMember('Add', $bf::InvokeMethod, $null, $props, @($name, $false, $type, $value))
}

# ---- fixture building -----------------------------------------------------------------------------------------------
# A case is a hashtable: code (field code incl. type), lang (LCID, optional), stale (default true), text (paragraph
# text before the field), style (paragraph style name).
function Add-FieldParagraph($doc, $case) {
  $end = $doc.Content.End - 1
  $r = $doc.Range($end, $end)
  if ($case.style) { $r.Style = $case.style }
  if ($case.text) { $r.InsertAfter([string]$case.text); $end = $doc.Content.End - 1; $r = $doc.Range($end, $end) }
  if ($case.lang) { }
  $f = $doc.Fields.Add($r, -1, $case.code, $false)
  if ($case.lang) { $f.Code.LanguageID = [int]$case.lang }
  if ($case.stale -ne $false) { $f.Result.Text = 'STALE' }
  $doc.Content.InsertParagraphAfter()
  $end = $doc.Content.End - 1
  $doc.Range($end, $end).Style = $doc.Styles.Item(-1)   # wdStyleNormal
}

function Add-Paragraph($doc, [string]$text, $style) {
  $end = $doc.Content.End - 1
  $r = $doc.Range($end, $end)
  $r.Style = $doc.Styles.Item($style)
  $r.InsertAfter($text)
  $doc.Content.InsertParagraphAfter()
  $end = $doc.Content.End - 1
  $doc.Range($end, $end).Style = $doc.Styles.Item(-1)
}

# ---- recording --------------------------------------------------------------------------------------------------------
function Clean-Text([string]$s) {
  # Range.Text of a field result: keep \r, \t, \v; drop the cell marker.
  return $s
}

function Scrub([string]$s) {
  # The scratch folder lives under the user profile: never record it.
  if ($null -eq $s) { return $s }
  return ($s -replace [regex]::Escape($scratch), '<scratch>')
}

function Dump-Reference($doc, $ctx) {
  $fields = New-Object System.Collections.Generic.List[string]
  $i = 0
  foreach ($f in @($doc.Fields)) {
    $level = 0
    $line = '    {"i": ' + $i + ', "type": ' + [int]$f.Type + ', "start": ' + $f.Code.Start +
      ', "code": ' + (Json-Str (Scrub $f.Code.Text)) + ', "result": ' + (Json-Str (Scrub $f.Result.Text)) + '}'
    $fields.Add($line)
    $i++
  }
  $paras = New-Object System.Collections.Generic.List[string]
  foreach ($p in @($doc.Paragraphs)) {
    $r = $p.Range
    $text = Scrub $r.Text
    $style = ''
    try { $style = [string]$p.Style.NameLocal } catch {}
    $page = $r.Information(3)
    $adj = $r.Information(1)
    $paras.Add('    {"style": ' + (Json-Str $style) + ', "text": ' + (Json-Str $text) + ', "page": ' + $page + ', "label": ' + $adj + '}')
  }
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add('{')
  $lines.Add('  "format": 1,')
  $lines.Add('  "oracle": "Microsoft Word ' + $script:wd.Version + ' build ' + $script:wd.Build + ' via COM: Document.Fields.Update, TablesOfContents.Update",')
  $lines.Add('  "context": ' + $ctx + ',')
  $lines.Add('  "fields": [')
  $lines.Add(($fields -join ",`n"))
  $lines.Add('  ],')
  $lines.Add('  "paragraphs": [')
  $lines.Add(($paras -join ",`n"))
  $lines.Add('  ]')
  $lines.Add('}')
  return ($lines -join "`n") + "`n"
}

# ---- sets -------------------------------------------------------------------------------------------------------------
$numberPictures = @(
  '0', '00', '000', '0.0', '0.00', '0.000', '#', '##', '#.#', '#.##', '#,##0', '#,##0.00', '#,###', '#,###.##',
  '0%', '0.0%', '#%', '$#,##0.00', '$#,##0.00;($#,##0.00)', '#,##0.00;-#,##0.00;zero', '0.00;(0.00);nil',
  '0.00 kg', "0.00 'kg'", "'Total:' 0.00", 'x0', '0x', '#.##0', '0.0#', '00.00', '+0;-0;0', '00#',
  '-0', '+0', '-0.00', '0-', '(0)', '[0.0]', '0 USD', 'USD 0', '-#,##0.00', '+#,##0.00;-#,##0.00', '0;(0)', '0;;zero', '#,##0;(#,##0);-', '000-000', '##-##', '0.00 %', "0 'x' 0", '0\,0'
)
$numberValues = @('0', '1', '-1', '5', '12', '1234.5678', '0.5', '-0.5', '1000000', '0.000123', '99.995', '-1234567.891', '0.256')

function Get-FormatCases {
  $c = New-Object System.Collections.Generic.List[hashtable]
  foreach ($e in @(
      '1+2', '10/3', '2*3+4', '2+3*4', '(2+3)*4', '2^10', '2^0.5', '-5+3', '10-3-2', '100/8', '7%', '50%*3', '5=5', '5=6', '3<4', '3>4',
      '3<=3', '3>=4', '3<>4', 'ABS(-5)', 'ABS(5)', 'AND(1,1)', 'AND(1,0)', 'OR(0,1)', 'OR(0,0)', 'NOT(1)', 'NOT(0)', 'AVERAGE(1,2,3,4)',
      'COUNT(1,2,3)', 'DEFINED(NoSuch)', 'FALSE', 'TRUE', 'IF(1,2,3)', 'IF(0,2,3)', 'IF(3>2,"yes","no")', 'INT(5.9)', 'INT(-5.9)',
      'MAX(1,5,3)', 'MIN(4,2,8)', 'MOD(10,3)', 'MOD(-10,3)', 'PRODUCT(2,3,4)', 'ROUND(3.14159,2)', 'ROUND(1234.567,-2)', 'ROUND(2.5,0)',
      'ROUND(-2.5,0)', 'SIGN(-3)', 'SIGN(0)', 'SUM(1,2,3)', 'SUM(1,2,3)*2', '1/0', '1+', '2*(3', '10/4', '1/3', '1000000*1000000', '0.1+0.2',
      '123456789.123456789', '-0', '3 - 2', 'MAX(1,2)+MIN(3,4)',
      '6/2', '7/2', '1/3*3', '3/3', '0.5*2', '1.5+1.5', '2.50*2', '0.001*0.001', '0.5/0.25', '1.25/5', '1.000/3', '0.123/3', '10/4.5', '1/3+1/3', '2/3*3', '100*1.1',
      '0.1*3', '1/3*1000', '1000000/3', '12345.6789/3', 'AVERAGE(1,2)', 'AVERAGE(1,2,3)', 'AVERAGE(1.5,2.5)', 'SUM(1.5,2.25)', 'SUM(0.1,0.2)', 'MAX(1.5,2)',
      'INT(5.9)+0.5', 'ROUND(2.345,2)', 'ROUND(2.345,1)', 'ROUND(3.14159,4)', 'ROUND(3.5,0)', 'ROUND(1/3,3)', '5%', '12.5%', '100*5%', '2^-1', '2^-2', '2^3.5', '4^0.5',
      '10^2', '10^-2', '0.1^2', '1.5^2', '7/0', '0/0', 'MOD(5,0)', 'ABS(-2.5)', 'ABS(2.50)', '-2.5', '+2.5', '--2', '1.0', '1.50', '1.0+1', '3.14159 * 2', '5.', '.5',
      '12345678901234567890', '0.30000000000000004', '3*1.1', '99999999999999', '999999999999999', '2 3', '(1+2', '1)', '1 +* 2', 'IF(1>0,1,0)', 'PRODUCT(1.5,2)',
      '-2^2', '2^3^2', '2*-3', '10-(-3)', '2^2*3', '8/2/2', '8-2-2', '2+3*4^2', '(2+3)^2', '2^(1+1)', '-(2+3)', '5 5', '1==2', '1=1=1', '1<2<3', '2*3%', '10%%', '5 % 2', '1,5',
      '999999.4 \* CardText', '999999.5 \* CardText', '0.49 \* CardText', '0.51 \* CardText', '-0.4 \* CardText', '780 \* ALPHABETIC', '781 \* ALPHABETIC', '65535 \* Hex', '65536 \* Hex',
      '99999 \* Roman', '100000 \* Roman', '4000 \* Roman', '1.999 \* DollarText', '1.995 \* DollarText', '12.5 \* DollarText', '-3 \* Ordinal', '-3 \* DollarText'
    )) { $c.Add(@{ code = '= ' + $e }) }
  foreach ($v in $numberValues) {
    foreach ($p in $numberPictures) {
      $c.Add(@{ code = '= ' + $v + ' \# "' + $p + '"' })
    }
  }
  # Format switches on numbers.
  foreach ($v in @('0', '1', '5', '11', '12', '21', '22', '23', '40', '99', '100', '101', '115', '1234', '12345', '1000000', '2147483647', '-5', '3.7', '0.5', '1234.56', '0.05', '1.005')) {
    foreach ($s in @('CardText', 'OrdText', 'DollarText', 'Ordinal', 'Roman', 'roman', 'ROMAN', 'ALPHABETIC', 'alphabetic', 'Arabic', 'Hex', 'Upper', 'Lower', 'FirstCap', 'Caps', 'CHARFORMAT', 'MERGEFORMAT', 'CardText \* Upper', 'CardText \* FirstCap', 'CardText \* Caps', 'OrdText \* Caps', 'DollarText \* Upper', 'DollarText \* Caps', 'Ordinal \* Upper')) {
      $c.Add(@{ code = '= ' + $v + ' \* ' + $s })
    }
  }
  foreach ($s in @('Upper', 'Lower', 'FirstCap', 'Caps', 'CHARFORMAT', 'MERGEFORMAT', 'Upper \* Lower', 'Lower \* Upper', 'Caps \* Lower', 'Lower \* FirstCap')) {
    foreach ($t in @('hello world', 'HELLO WORLD', 'hello. world: it''s a test', 'the quick-brown fox  jumps', '  padded  ', 'one two  three', 'mIxEd cAsE 123abc', 'a', '', 'ex-ample o''neil mcDonald', 'tab-sep')) {
      $c.Add(@{ code = 'QUOTE "' + $t + '" \* ' + $s })
    }
  }
  foreach ($c2 in @('QUOTE "plain"', 'QUOTE plain', 'QUOTE a b c', 'QUOTE "a" "b"', 'QUOTE "say \"hi\""', 'QUOTE "" ', 'QUOTE "3" \# "0.00"', 'QUOTE "1234.5" \# "#,##0.00"', 'QUOTE "abc" \# "0.00"')) { $c.Add(@{ code = $c2 }) }
  return $c
}

# Date-valued custom properties: name -> ISO instant (UTC). 2024-mm-15 at 12:00Z for the months, then weekday and
# clock probes in January (Greenwich time, no daylight saving, so UTC = local time on this machine).
$dateProps = [ordered]@{}
for ($m = 1; $m -le 12; $m++) { $dateProps['DM{0:00}' -f $m] = ('2024-{0:00}-15T12:00:00Z' -f $m) }
for ($d = 0; $d -lt 7; $d++) { $dateProps['DW{0}' -f $d] = ('2024-03-{0:00}T12:00:00Z' -f (3 + $d)) }
$dateProps['DT1'] = '2024-01-05T14:05:09Z'
$dateProps['DT2'] = '2024-01-05T00:30:00Z'
$dateProps['DT3'] = '2024-01-05T12:00:00Z'
$dateProps['DT4'] = '2024-01-05T09:07:08Z'
$dateProps['DT5'] = '2024-12-31T23:59:59Z'
$dateProps['DT6'] = '1999-02-28T00:00:00Z'

$datePictures = @(
  'd/M/yyyy', 'dd/MM/yy', 'M/d/yy', 'MMMM d, yyyy', 'dddd, d MMMM yyyy', 'ddd d MMM', 'dd-MMM-yy', 'yyyy-MM-dd', 'yyyyMMdd', 'yy', 'yyy', 'y',
  'M', 'MM', 'MMM', 'MMMM', 'MMMMM', 'd', 'dd', 'ddd', 'dddd', 'dddddd', 'h:mm AM/PM', 'hh:mm:ss a/p', 'H:mm', 'HH:mm:ss', 'h:mm am/pm', 'AM/PM', 'A/P',
  'a/p', 'am/pm', 'h', 'hh', 'H', 'HH', 'm', 'mm', 's', 'ss', 'h:m:s', "yyyy-MM-dd'T'HH:mm:ss", "'Today is' dddd", "d 'of' MMMM", 'MMM-yy',
  'dddd dd MMMM yyyy HH:mm', 'd MMMM yyyy', 'MMMM yyyy', 'dd.MM.yyyy', 'Day d', 'Month MMMM', 'ddMMMyyyy', 'h:mm:ss AM/PM dd/MM', '\d\a\y d', 'd\t\h', 'x d y',
  'MMMM d', 'w', 'g', 'D', 'MMM d, yyyy h:mm AM/PM', 'hh:mm AM/PM', 'h.mm', 'HHmm', 'h''h''mm', 'dd/MMM/yyyy HH:mm:ss'
)

function Get-DateCases {
  $c = New-Object System.Collections.Generic.List[hashtable]
  foreach ($p in $datePictures) {
    foreach ($n in @('DT1', 'DM03', 'DT4')) {
      $c.Add(@{ code = 'DOCPROPERTY ' + $n + ' \@ "' + $p + '"' })
    }
  }
  foreach ($n in $dateProps.Keys) {
    foreach ($p in @('MMMM', 'MMM', 'dddd', 'ddd', 'd MMMM yyyy', 'HH:mm', 'h:mm AM/PM', 'MMMM d, yyyy h:mm:ss am/pm')) {
      $c.Add(@{ code = 'DOCPROPERTY ' + $n + ' \@ "' + $p + '"' })
    }
  }
  foreach ($t in @('CREATEDATE', 'SAVEDATE', 'PRINTDATE')) {
    foreach ($p in @('', ' \@ "d MMMM yyyy"', ' \@ "dddd, dd MMMM yyyy HH:mm:ss"', ' \@ "h:mm am/pm"', ' \@ "dd/MM/yy" \* Upper', ' \@ "MMMM" \* Lower', ' \@ "dddd" \* Caps')) {
      $c.Add(@{ code = $t + $p })
    }
  }
  foreach ($lang in @(1031, 1036, 1040, 3082, 1049, 1043, 1046, 1053, 1045, 1055, 1041, 2052, 1042, 1025, 1037, 1029, 1038, 1030, 1035, 1032, 1081, 1054, 1057, 1058, 1066, 1099, 1033, 2057, 4105, 1050, 1060, 1061, 1062, 1063, 1048, 1051, 1024)) {
    foreach ($n in @('DM03', 'DT1', 'DT4', 'DM09')) {
      $c.Add(@{ code = 'DOCPROPERTY ' + $n + ' \@ "dddd, d MMMM yyyy ddd MMM h:mm AM/PM"'; lang = $lang })
    }
  }
  return $c
}

# ---- refs set: bookmarks, references, sequences, styles, pages ------------------------------------------------------
function Add-Para($doc, [string]$text, $style) {
  # Appends a paragraph; returns the start offset of its text.
  $end = $doc.Content.End - 1
  $r = $doc.Range($end, $end)
  $r.Style = $doc.Styles.Item($style)
  $r.InsertAfter($text)
  $doc.Content.InsertParagraphAfter()
  $e2 = $doc.Content.End - 1
  $doc.Range($e2, $e2).Style = $doc.Styles.Item(-1)
  return $end
}

function Add-Bookmark($doc, [string]$name, [int]$start, [int]$length) {
  [void]$doc.Bookmarks.Add($name, $doc.Range($start, $start + $length))
}

function Add-FieldPara($doc, [string]$code, [string]$before = '', [string]$after = '', $style = -1, [switch]$Fresh) {
  $end = $doc.Content.End - 1
  $r = $doc.Range($end, $end)
  $r.Style = $doc.Styles.Item($style)
  if ($before) { $r.InsertAfter($before); $end = $doc.Content.End - 1; $r = $doc.Range($end, $end) }
  $f = $doc.Fields.Add($r, -1, $code, $false)
  if (-not $Fresh) { $f.Result.Text = 'STALE' }
  if ($after) { $e3 = $doc.Content.End - 1; $doc.Range($e3, $e3).InsertAfter($after) }
  $doc.Content.InsertParagraphAfter()
  $e2 = $doc.Content.End - 1
  $doc.Range($e2, $e2).Style = $doc.Styles.Item(-1)
}

function Add-PageBreak($doc) {
  $end = $doc.Content.End - 1
  $doc.Range($end, $end).InsertBreak(7)
}

function Add-HeadingNumbering($doc) {
  $lt = $doc.ListTemplates.Add($true)
  $l1 = $lt.ListLevels.Item(1)
  $l1.NumberFormat = '%1'
  $l1.NumberStyle = 0
  $l1.LinkedStyle = 'Heading 1'
  $l2 = $lt.ListLevels.Item(2)
  $l2.NumberFormat = '%1.%2'
  $l2.NumberStyle = 0
  $l2.LinkedStyle = 'Heading 2'
}

function Build-Refs($doc) {
  Add-HeadingNumbering $doc
  [void](Add-Para $doc 'Reference Test' 'Title')
  $s = Add-Para $doc 'Introduction' 'Heading 1'; Add-Bookmark $doc 'h_intro' $s 12
  $s = Add-Para $doc 'Intro text with marked words inside and more.' 'Normal'; Add-Bookmark $doc 'bm_words' ($s + 16) 12
  $s = Add-Para $doc 'Background' 'Heading 2'; Add-Bookmark $doc 'h_background' $s 10
  $s = Add-Para $doc 'Methods' 'Heading 1'; Add-Bookmark $doc 'h_methods' $s 7
  $s = Add-Para $doc 'Whole paragraph bookmark.' 'Normal'
  $r = $doc.Range($s, $s + 25 + 1); [void]$doc.Bookmarks.Add('bm_para', $r)
  $s = Add-Para $doc 'Numbers 12 and 3.5 here.' 'Normal'; Add-Bookmark $doc 'bm_num' ($s + 8) 2; Add-Bookmark $doc 'bm_dec' ($s + 15) 3
  # Group A: references on page 1.
  foreach ($c in @(
      'REF bm_words', 'REF bm_words \h', 'REF bm_words \* Upper', 'REF bm_words \* Caps', 'REF bm_words \* MERGEFORMAT', 'REF bm_words \* CHARFORMAT',
      'REF bm_para', 'REF h_intro', 'REF h_intro \r', 'REF h_intro \n', 'REF h_intro \w', 'REF h_background \r', 'REF h_background \n', 'REF h_background \w',
      'REF h_methods \r \h', 'REF h_methods \p', 'REF bm_words \p', 'REF h_background \p',
      'REF NoSuchBookmark', 'REF bm_num \# "0.00"', 'REF bm_dec \# "0.00"', '= bm_num + bm_dec', '= bm_num * 2 \# "000"', '= bm_dec * bm_num',
      'REF bm_words \d "-"', 'PAGEREF bm_words', 'PAGEREF bm_words \h', 'PAGEREF h_intro \p', 'PAGEREF bm_words \p',
      'STYLEREF "Heading 1"', 'STYLEREF "Heading 1" \n', 'STYLEREF "Heading 1" \w', 'STYLEREF "Heading 1" \r', 'STYLEREF "Heading 2"', 'STYLEREF "Heading 2" \n',
      'STYLEREF "Heading 1" \l', 'STYLEREF "Heading 1" \p', 'STYLEREF "Title"', 'STYLEREF 1', 'STYLEREF "Heading 3"', 'STYLEREF NoSuchStyle', 'STYLEREF "Heading 1" \* Upper',
      'IF 1 = 1 "yes" "no"', 'IF 1 = 2 "yes" "no"', 'IF 5 > 3 "gt" "le"', 'IF 5 < 3 "lt" "ge"', 'IF 5 >= 5 "ge" "lt"', 'IF 5 <= 4 "le" "gt"', 'IF 5 <> 4 "ne" "eq"',
      'IF "abc" = "abc" "same" "diff"', 'IF "abc" = "ABC" "same" "diff"', 'IF "abc" = "a*" "wild" "nowild"', 'IF "abc" = "a?c" "q" "nq"', 'IF "abc" <> "abd" "ne" "eq"',
      'IF "b" > "a" "gt" "le"', 'IF "10" > "9" "numgt" "numle"', 'IF bm_num = 12 "twelve" "other"', 'IF 1 = 1 "yes"', 'IF 1 = 2 "yes"', 'IF 1 = 1 "yes" ""',
      'IF 1 = 1 yes no', 'IF "" = "" "empty" "nonempty"', 'IF bm_words = "marked words" "ok" "bad"', 'IF 1 = 1 "a b c" "d" \* Upper',
      'IF { DOCPROPERTY Missing } = "" "blank" "set"', 'IF 2 > 1 { QUOTE "nested" } "no"', 'MERGEFIELD FirstName', 'MERGEFIELD FirstName \b "Dear " \f ","', 'MERGEFIELD FirstName \* Upper',
      'MERGEFIELD FirstName \m', 'MERGEFIELD FirstName \v', 'MERGEFIELD "Last Name"', 'MERGEFIELD First_Name \* Caps',
      'HYPERLINK "http://example.com/"', 'HYPERLINK "http://example.com/" \o "tip"', 'HYPERLINK \l "bm_words"', 'HYPERLINK "mailto:a@b.example"', 'HYPERLINK "http://example.com/" \t "_blank"',
      'QUOTE "quoted"', 'AUTHOR', 'TITLE', 'SUBJECT', 'KEYWORDS', 'COMMENTS', 'FILENAME', 'FILENAME \* Lower', 'LASTSAVEDBY', 'DOCPROPERTY Title', 'DOCPROPERTY "Title" \* Upper'
    )) { Add-FieldPara $doc $c }
  Add-PageBreak $doc
  # Page 2: sequences and a table.
  [void](Add-Para $doc 'Results' 'Heading 1')
  foreach ($c in @(
      'SEQ Figure', 'SEQ Figure', 'SEQ Figure \c', 'SEQ Figure \n', 'SEQ Figure \h', 'SEQ Figure \r 10', 'SEQ Figure', 'SEQ Figure \* ROMAN', 'SEQ Figure \* roman',
      'SEQ Figure \* ALPHABETIC', 'SEQ Figure \* alphabetic', 'SEQ Figure \* CardText', 'SEQ Figure \* Ordinal', 'SEQ Figure \* OrdText', 'SEQ Figure \# "00"', 'SEQ Figure \* Arabic',
      'SEQ Figure \r 1', 'SEQ Figure', 'SEQ Table', 'SEQ Table', 'SEQ Table \c', 'SEQ Figure \s 1', 'SEQ Figure \s 1', 'SEQ Table \s 2', 'SEQ Equation \* ARABIC', 'SEQ Equation \* ARABIC',
      'SEQ Equation \r 5 \* ARABIC', 'SEQ Equation \n', 'SEQ "Quoted Id"', 'SEQ Figure \* MERGEFORMAT', 'SEQ Figure \* Hex', 'SEQ Figure \r 0', 'SEQ Figure \r -1'
    )) { Add-FieldPara $doc $c -before 'Figure ' -after ' end' }
  [void](Add-Para $doc 'Table data' 'Normal')
  $end = $doc.Content.End - 1
  $t = $doc.Tables.Add($doc.Range($end, $end), 4, 3)
  $vals = @(@('10', '20', ''), @('1.5', '2.25', ''), @('3', '-4', ''), @('', '', ''))
  for ($r = 1; $r -le 4; $r++) { for ($c = 1; $c -le 3; $c++) { $t.Cell($r, $c).Range.Text = $vals[$r - 1][$c - 1] } }
  $tableFormulas = @(@(4, 1, '= SUM(ABOVE)'), @(4, 2, '= SUM(ABOVE) \# "0.00"'), @(1, 3, '= SUM(LEFT)'), @(2, 3, '= SUM(LEFT) \# "0.00"'), @(3, 3, '= AVERAGE(LEFT)'), @(4, 3, '= MAX(A1:B3)'))
  foreach ($cell in $tableFormulas) {
    $rng = $t.Cell($cell[0], $cell[1]).Range; [void]$rng.MoveEnd(1, -1)
    $f = $doc.Fields.Add($rng, -1, $cell[2], $false)
    $f.Result.Text = 'STALE'
  }
  $doc.Content.InsertParagraphAfter()
  Add-PageBreak $doc
  # Page 3: notes and page fields.
  [void](Add-Para $doc 'Discussion' 'Heading 1')
  $s = Add-Para $doc 'Text with a note here.' 'Normal'
  $fn = $doc.Footnotes.Add($doc.Range($s + 21, $s + 21), [Type]::Missing, 'The footnote text.')
  [void]$doc.Bookmarks.Add('fn_bm', $fn.Reference)
  foreach ($c in @('NOTEREF fn_bm', 'NOTEREF fn_bm \h', 'NOTEREF fn_bm \f', 'NOTEREF fn_bm \p', 'PAGE', 'PAGE \* roman', 'PAGE \* ALPHABETIC', 'PAGE \* ROMAN', 'PAGE \* Ordinal',
      'PAGE \# "00"', 'NUMPAGES', 'NUMPAGES \* roman', 'NUMPAGES \# "000"', 'SECTIONPAGES', 'SECTION', 'PAGEREF bm_words', 'PAGEREF bm_words \* roman', 'PAGEREF fn_bm', 'PAGEREF h_methods \h',
      'PAGEREF bm_para \p', 'PAGE \* CardText')) { Add-FieldPara $doc $c }
  # Section 2: new page, restart numbering, lower-case roman page numbers.
  $end = $doc.Content.End - 1
  $doc.Range($end, $end).InsertBreak(2)
  $sec = $doc.Sections.Item($doc.Sections.Count)
  $pn = $sec.Footers.Item(1).PageNumbers
  $pn.RestartNumberingAtSection = $true
  $pn.StartingNumber = 3
  $pn.NumberStyle = 2
  [void](Add-Para $doc 'Appendix' 'Heading 1')
  foreach ($c in @('PAGE', 'PAGE \* Arabic', 'NUMPAGES', 'SECTIONPAGES', 'SECTION', 'PAGEREF bm_words', 'PAGEREF h_intro', 'PAGEREF h_methods \* Arabic', 'STYLEREF "Heading 1"', 'STYLEREF "Heading 1" \n',
      'STYLEREF "Heading 2"', 'SEQ Figure', 'SEQ Table', 'REF bm_words', 'NUMPAGES \* Arabic')) { Add-FieldPara $doc $c }
  # Extra: backward and forward searches, relative positions, wildcard and numeric comparisons, tables with text and blanks.
  $s = Add-Para $doc 'Late Notes' 'Heading 5'; Add-Bookmark $doc 'bm_late' $s 10
  foreach ($c in @(
      'STYLEREF "Heading 5"', 'STYLEREF "Heading 5" \p', 'STYLEREF "Heading 4"', 'STYLEREF "Heading 4" \l', 'STYLEREF "Heading 4" \p', 'STYLEREF "Heading 4" \n', 'STYLEREF "Heading 1" \l',
      'STYLEREF "heading 1"', 'STYLEREF "Heading1"', 'STYLEREF 2', 'STYLEREF "Normal"', 'STYLEREF "Heading 1" \r', 'STYLEREF "Heading 2" \w', 'STYLEREF "Heading 1" \p \n',
      'REF bm_final', 'REF bm_final \p', 'REF bm_final \n', 'REF bm_late \p', 'REF bm_words \p \n', 'REF h_methods \n \p', 'REF h_methods \w \p', 'PAGEREF bm_final \p', 'PAGEREF bm_final', 'PAGEREF bm_words \p',
      'PAGEREF bm_late \p', 'PAGEREF NoSuchBookmark', 'NOTEREF NoSuchBookmark', 'NOTEREF bm_words',
      'IF "abcd" = "ab*" "wild" "nowild"', 'IF "a*c" = "abc" "wl" "nwl"', 'IF "abc" <> "a*" "ne" "eq"', 'IF 5 = 5.0 "numeq" "numne"', 'IF "5" = 5 "x" "y"', 'IF "ab" = "AB" "ci" "cs"', 'IF 10 > 9 "num" "str"',
      'IF "10" > 9 "mixnum" "mixstr"', 'IF a = a "bare" "nobare"', 'IF a < b "lt" "ge"', 'IF "a b" = "a b" "sp" "nsp"', 'IF 1 = 1 "t1" "f1" "extra"', 'IF "x" = "y" "t" ', 'IF 1.50 = 1.5 "decimal" "nodec"', 'IF -1 < 0 "neg" "pos"',
      'IF "" <> "a" "emptyne" "emptyeq"', 'IF 1 == 1 "dbl" "nodbl"', 'IF 1 ~ 1 "tilde" "notilde"', 'IF 1 = "1" "q1" "q2"', 'IF "A" < "a" "upperlt" "upperge"', 'IF "apple" < "Banana" "ci" "cs"',
      'IF bm_num > 5 "bignum" "smallnum"', 'IF bm_words = "marked*" "bmwild" "bmnowild"', 'IF "1,000" = 1000 "comma" "nocomma"', 'IF " a" = "a" "trim" "notrim"'
    )) { Add-FieldPara $doc $c }
  [void](Add-Para $doc 'Table two' 'Normal')
  $end = $doc.Content.End - 1
  $t2 = $doc.Tables.Add($doc.Range($end, $end), 7, 3)
  $vals2 = @(@('Item', 'Qty', 'Price'), @('a', '5', '$12.50'), @('b', '', '3'), @('c', '7', '10%'), @('Total', '', ''), @('', '', ''), @('', '', ''))
  for ($r = 1; $r -le 7; $r++) { for ($c = 1; $c -le 3; $c++) { $t2.Cell($r, $c).Range.Text = $vals2[$r - 1][$c - 1] } }
  $tf2 = @(@(5, 2, '= SUM(ABOVE)'), @(5, 3, '= SUM(ABOVE)'), @(6, 1, '= SUM(B2:B4)'), @(6, 2, '= PRODUCT(ABOVE)'), @(6, 3, '= MAX(B1:C4)'), @(7, 1, '= SUM(RIGHT)'), @(7, 2, '= SUM(LEFT)'), @(7, 3, '= SUM(LEFT)'))
  foreach ($cell in $tf2) {
    $rng = $t2.Cell($cell[0], $cell[1]).Range; [void]$rng.MoveEnd(1, -1)
    $f = $doc.Fields.Add($rng, -1, $cell[2], $false)
    $f.Result.Text = 'STALE'
  }
  $doc.Content.InsertParagraphAfter()
  [void](Add-Para $doc 'Table three' 'Normal')
  $end = $doc.Content.End - 1
  $t3 = $doc.Tables.Add($doc.Range($end, $end), 4, 5)
  $vals3 = @(@('5', '4', '1', '1,000', '(5)'), @('x', '$3', '2 apples', '5', '2'), @('7', '2', '3', '-2', '1e2'), @('', '', '', '', ''))
  for ($r = 1; $r -le 4; $r++) { for ($c = 1; $c -le 5; $c++) { $t3.Cell($r, $c).Range.Text = $vals3[$r - 1][$c - 1] } }
  for ($c = 1; $c -le 5; $c++) {
    $rng = $t3.Cell(4, $c).Range; [void]$rng.MoveEnd(1, -1)
    $f = $doc.Fields.Add($rng, -1, '= SUM(ABOVE)', $false)
    $f.Result.Text = 'STALE'
  }
  $doc.Content.InsertParagraphAfter()
  Add-PageBreak $doc
  $s = Add-Para $doc 'Final Page' 'Heading 4'; Add-Bookmark $doc 'bm_final' $s 10
}

# ---- props set --------------------------------------------------------------------------------------------------------
function Build-Props($doc) {
  Set-BuiltIn $doc 'Category' 'Test Category'
  Set-BuiltIn $doc 'Manager' 'Test Manager'
  Set-BuiltIn $doc 'Company' 'Test Company'
  Add-Custom $doc 'CText' 4 'Custom Text'
  Add-Custom $doc 'CNum' 1 42
  Add-Custom $doc 'CReal' 5 3.5
  Add-Custom $doc 'CBool' 2 $true
  Add-Custom $doc 'CDate' 3 ([datetime]'2000-01-01')
  Add-Custom $doc 'CEmpty' 4 ' '
  foreach ($c in @(
      'AUTHOR', 'AUTHOR \* Upper', 'AUTHOR \* Lower', 'AUTHOR \* FirstCap', 'AUTHOR \* Caps', 'AUTHOR \* MERGEFORMAT', 'TITLE', 'TITLE \* Upper', 'SUBJECT', 'SUBJECT \* Caps', 'KEYWORDS', 'KEYWORDS \* Upper',
      'COMMENTS', 'COMMENTS \* Lower', 'LASTSAVEDBY', 'FILENAME', 'FILENAME \p', 'FILENAME \* Upper', 'FILENAME \* Lower', 'FILENAME \* Caps', 'FILENAME \* FirstCap',
      'DOCPROPERTY Title', 'DOCPROPERTY "Title"', 'DOCPROPERTY title', 'DOCPROPERTY Subject', 'DOCPROPERTY Author', 'DOCPROPERTY Keywords', 'DOCPROPERTY Comments', 'DOCPROPERTY LastSavedBy',
      'DOCPROPERTY Category', 'DOCPROPERTY Manager', 'DOCPROPERTY Company', 'DOCPROPERTY RevisionNumber', 'DOCPROPERTY CreateTime', 'DOCPROPERTY LastSavedTime', 'DOCPROPERTY LastPrinted',
      'DOCPROPERTY CText', 'DOCPROPERTY ctext', 'DOCPROPERTY CNum', 'DOCPROPERTY CNum \# "0000"', 'DOCPROPERTY CNum \* Roman', 'DOCPROPERTY CNum \* CardText', 'DOCPROPERTY CReal', 'DOCPROPERTY CReal \# "0.00"',
      'DOCPROPERTY CBool', 'DOCPROPERTY CDate', 'DOCPROPERTY CDate \@ "d MMMM yyyy"', 'DOCPROPERTY CDate \@ "yyyy-MM-dd"', 'DOCPROPERTY CEmpty', 'DOCPROPERTY NoSuchProperty', 'DOCPROPERTY',
      'DOCPROPERTY CText \* Upper', 'DOCPROPERTY CText \* Caps', 'INFO Author', 'INFO Title', 'INFO Subject', 'INFO Keywords', 'INFO Comments', 'INFO LastSavedBy', 'INFO FileName',
      'CREATEDATE', 'CREATEDATE \@ "yyyy-MM-dd HH:mm:ss"', 'SAVEDATE', 'SAVEDATE \@ "yyyy-MM-dd HH:mm:ss"', 'PRINTDATE', 'PRINTDATE \@ "yyyy-MM-dd HH:mm:ss"',
      'DOCPROPERTY Template', 'AUTHOR "Replacement Author"', 'TITLE "Replacement Title"'
    )) { Add-FieldPara $doc $c }
}

# ---- toc set ----------------------------------------------------------------------------------------------------------
function Build-Toc($doc) {
  Add-HeadingNumbering $doc
  [void](Add-Para $doc 'Table of Contents Test' 'Title')
  $names = @('TOC-01', 'TOC-02', 'TOC-03', 'TOC-04', 'TOC-05', 'TOC-06', 'TOC-07', 'TOC-08', 'TOC-09', 'TOC-10', 'TOC-11', 'TOC-12', 'TOC-13', 'TOC-14', 'TOC-15', 'TOC-16', 'TOC-17')
  foreach ($n in $names) { [void](Add-Para $doc $n 'Normal') }
  Add-PageBreak $doc
  [void](Add-Para $doc 'First Chapter' 'Heading 1')
  [void](Add-Para $doc 'Some body text.' 'Normal')
  [void](Add-Para $doc 'Text with entries' 'Normal')
  [void](Add-Para $doc 'First Section' 'Heading 2')
  [void](Add-Para $doc 'A deeper level' 'Heading 3')
  [void](Add-Para $doc 'Second Section' 'Heading 2')
  [void](Add-Para $doc 'Second Chapter' 'Heading 1')
  [void](Add-Para $doc 'TabbedXHeading' 'Heading 2')
  [void](Add-Para $doc 'BrokenXHeading' 'Heading 2')
  Add-PageBreak $doc
  $bmStart = Add-Para $doc 'Third Chapter' 'Heading 1'
  [void](Add-Para $doc 'Subtopic' 'Heading 2')
  [void](Add-Para $doc 'Custom styled paragraph' 'Quote')
  [void](Add-Para $doc 'Outline level paragraph' 'Normal')
  $p = $doc.Paragraphs.Item($doc.Paragraphs.Count - 1)
  $p.OutlineLevel = 2
  [void]$doc.Bookmarks.Add('toc_range', $doc.Range($bmStart, $p.Range.End - 1))
  Add-PageBreak $doc
  [void](Add-Para $doc 'Caption text one' 'Caption')
  [void](Add-Para $doc 'Caption text two' 'Caption')
  [void](Add-Para $doc 'Last Chapter' 'Heading 1')
  # Heading text with a tab and a manual line break.
  foreach ($pp in @($doc.Paragraphs)) {
    $t = $pp.Range.Text
    if ($t -like 'TabbedXHeading*') { $r = $pp.Range; [void]$r.MoveEnd(1, -1); $r.Text = "Tabbed`tHeading" }
    if ($t -like 'BrokenXHeading*') { $r = $pp.Range; [void]$r.MoveEnd(1, -1); $r.Text = 'Broken' + [char]11 + 'Heading' }
  }
  # TC entries in the "Text with entries" paragraph.
  foreach ($pp in @($doc.Paragraphs)) {
    if ($pp.Range.Text -like 'Text with entries*') {
      foreach ($tc in @('TC "Manual entry one" \l 2', 'TC "Identified entry" \f X \l 1', 'TC "Default level" \n', 'TC "Level three" \l 3')) {
        $r = $pp.Range; [void]$r.MoveEnd(1, -1); $r.Collapse(0)
        [void]$doc.Fields.Add($r, -1, $tc, $false)
      }
      break
    }
  }
  $codes = @(
    'TOC \o "1-3" \h \z \u',
    'TOC \o "1-2"',
    'TOC \o "1-3" \n',
    'TOC \o "1-3" \h \z \t "Quote,3"',
    'TOC \h \z \c "Figure"',
    'TOC \o "2-3" \p " " \h',
    'TOC \o "1-3" \b toc_range \h',
    'TOC \f \h',
    'TOC \f X \h',
    'TOC \l "1-2" \h',
    'TOC \o "1-3" \w',
    'TOC \o "1-3" \x',
    'TOC \o "1-3" \n "2-3"',
    'TOC \u',
    'TOC \o "7-9"',
    'TOC \h \z \t "Heading 1,1,Heading 2,2"',
    'TOC \o "1-3" \f \u \h'
  )
  for ($i = 0; $i -lt $codes.Count; $i++) {
    $para = $null
    foreach ($pp in @($doc.Paragraphs)) { if ($pp.Range.Text.TrimEnd("`r") -eq $names[$i]) { $para = $pp; break } }
    $r = $para.Range; [void]$r.MoveEnd(1, -1)
    $r.Text = ''
    [void]$doc.Fields.Add($r, -1, $codes[$i], $false)
  }
  # SEQ fields in the Caption paragraphs.
  foreach ($pp in @($doc.Paragraphs)) {
    if ($pp.Range.Text -like 'Caption text*') {
      $r = $pp.Range; $r.Collapse(1)
      [void]$doc.Fields.Add($r, -1, 'SEQ Figure', $false)
      $r = $pp.Range; $r.Collapse(1); $r.InsertBefore('Figure ')
    }
  }
  $doc.Repaginate()
  [void]$doc.Fields.Update()
  foreach ($toc in @($doc.TablesOfContents)) { [void]$toc.Update() }
  # Make the saved TOCs stale: change headings after they were generated.
  foreach ($pp in @($doc.Paragraphs)) {
    $t = $pp.Range.Text.TrimEnd("`r")
    if ($t -eq 'Second Section') { $r = $pp.Range; [void]$r.MoveEnd(1, -1); $r.Text = 'Renamed Section' }
    if ($t -eq 'Subtopic') { $r = $pp.Range; [void]$r.MoveEnd(1, -1); $r.Text = 'Changed Subtopic' }
  }
  [void](Add-Para $doc 'Appended Chapter' 'Heading 1')
}

function Get-Sets {
  $sets = [ordered]@{}
  $sets['format'] = @{ cases = (Get-FormatCases) }
  $sets['date'] = @{ cases = (Get-DateCases); dateProps = $dateProps }
  $sets['props'] = @{ build = ${function:Build-Props} }
  $sets['refs'] = @{ build = ${function:Build-Refs} }
  $sets['toc'] = @{ build = ${function:Build-Toc} }
  return $sets
}

# ---- driver -----------------------------------------------------------------------------------------------------------
function Build-Set([string]$name, $spec) {
  New-Item -ItemType Directory -Force -Path $scratch | Out-Null
  $fixture = Join-Path $fixtureDir ("fields-$name.docx")
  $tmp = Join-Path $scratch ("fields-$name.docx")
  Start-Word
  try {
    $doc = $script:wd.Documents.Add()
    Set-BuiltIn $doc 'Title' 'Field Oracle Title'
    Set-BuiltIn $doc 'Subject' 'Field Oracle Subject'
    Set-BuiltIn $doc 'Author' 'Test Author'
    Set-BuiltIn $doc 'Keywords' 'alpha, beta'
    Set-BuiltIn $doc 'Comments' 'A comment'
    if ($spec.dateProps) {
      foreach ($k in $spec.dateProps.Keys) { Add-Custom $doc $k 3 ([datetime]'2000-01-01') }
    }
    $script:wd.ScreenUpdating = $false
    if ($spec.build) { & $spec.build $doc }
    else { foreach ($case in $spec.cases) { Add-FieldParagraph $doc $case } }
    $doc.SaveAs2($tmp, 16)
    $doc.Close(0)
  }
  finally { Stop-Word }
  $core = [ordered]@{
    'dc:creator'          = 'Test Author'
    'cp:lastModifiedBy'   = 'Test Author'
    'dcterms:created'     = '2023-11-02T08:09:10Z'
    'dcterms:modified'    = '2024-03-03T14:05:09Z'
    'cp:lastPrinted'      = '2024-01-05T09:07:08Z'
  }
  Pin-Core $tmp $core
  $dates = [ordered]@{}
  if ($spec.dateProps) { foreach ($k in $spec.dateProps.Keys) { $dates[$k] = $spec.dateProps[$k] } }
  if ($name -eq 'props') { $dates['CDate'] = '2021-07-04T10:20:30Z' }
  if ($dates.Count -gt 0) { Pin-Custom $tmp $dates }
  New-Item -ItemType Directory -Force -Path $fixtureDir | Out-Null
  Copy-Item -LiteralPath $tmp -Destination $fixture -Force

  # Re-open the pinned copy, update, record.
  Start-Word
  try {
    $doc = $script:wd.Documents.Open($tmp, $false, $true)   # read-only: never saved
    $doc.Repaginate()
    $before = Get-Date
    [void]$doc.Fields.Update()
    foreach ($toc in @($doc.TablesOfContents)) { [void]$toc.Update() }
    $after = Get-Date
    $doc.Repaginate()
    $offset = [int][TimeZoneInfo]::Local.GetUtcOffset([datetime]'2024-01-05T12:00:00').TotalMinutes
    $filePath = ($tmp -replace [regex]::Escape($scratch), '<scratch>')
    $ctx = '{"utc_offset_minutes": ' + $offset + ', "file_name": ' + (Json-Str ("fields-$name.docx")) +
      ', "file_path": ' + (Json-Str $filePath) +
      ', "now_before": ' + (Json-Str ($before.ToString('yyyy-MM-ddTHH:mm:ss'))) + ', "now_after": ' + (Json-Str ($after.ToString('yyyy-MM-ddTHH:mm:ss'))) +
      ', "pages": ' + $doc.ComputeStatistics(2) + ', "sections": ' + $doc.Sections.Count + ', "number_styles": [' + ((@($doc.Sections) | ForEach-Object { [int]$_.Footers.Item(1).PageNumbers.NumberStyle }) -join ', ') + ']}'
    $json = Dump-Reference $doc $ctx
    $doc.Close(0)
  }
  finally { Stop-Word }
  Write-Text (Join-Path $refRoot "fields-$name/fields.json") $json
  Write-Host "fields-$name written"
}

# Word's built-in TOC styles, as Word writes them to styles.xml of a new document (rsid elements dropped).
function Run-TocStyles {
  New-Item -ItemType Directory -Force -Path $scratch | Out-Null
  $tmp = Join-Path $scratch 'tocstyles.docx'
  Start-Word
  try {
    $doc = $script:wd.Documents.Add()
    $ids = @(-20, -21, -22, -23, -24, -25, -26, -27, -28, -36)
    foreach ($id in $ids) { [void](Add-Para $doc ('style ' + $id) $id) }
    [void](Add-Para $doc 'TOC Heading sample' 'TOC Heading')
    $end = $doc.Content.End - 1
    $r = $doc.Range($end, $end); $r.InsertAfter('link'); $r = $doc.Range($end, $end + 4); $r.Style = $doc.Styles.Item(-86)
    $doc.SaveAs2($tmp, 16)
    $doc.Close(0)
  }
  finally { Stop-Word }
  $zip = [IO.Compression.ZipFile]::OpenRead($tmp)
  try {
    $reader = New-Object IO.StreamReader($zip.GetEntry('word/styles.xml').Open(), (New-Object Text.UTF8Encoding $false))
    $xml = $reader.ReadToEnd(); $reader.Close()
  }
  finally { $zip.Dispose() }
  $want = @('TOC1', 'TOC2', 'TOC3', 'TOC4', 'TOC5', 'TOC6', 'TOC7', 'TOC8', 'TOC9', 'TableofFigures', 'TOCHeading', 'Hyperlink', 'Heading1')
  $lines = New-Object System.Collections.Generic.List[string]
  foreach ($id in $want) {
    $m = [regex]::Match($xml, '<w:style [^>]*w:styleId="' + $id + '".*?</w:style>')
    if ($m.Success) { $lines.Add(([regex]::Replace($m.Value, '<w:rsid [^>]*/>', ''))) } else { $lines.Add('<!-- missing ' + $id + ' -->') }
  }
  $m = [regex]::Match($xml, '<w:docDefaults>.*?</w:docDefaults>')
  $lines.Add('<!-- ' + $m.Value + ' -->')
  Write-Text (Join-Path $refRoot 'fields-toc/styles.xml') (($lines -join "`n") + "`n")
  Write-Host 'fields-toc/styles.xml written'
}

# ---- locale table: month/day/AM-PM names of every language Word accepts ------------------------------------------------
# Names come from Word itself: one field per name list in a scratch document, each given a language through
# Field.Code.LanguageID (no regional or Office setting is changed). Default date and time pictures come from the .NET
# culture data of the same language (the Windows short-date and long-time patterns Word's DATE/TIME fields use).
function Convert-NetPicture([string]$pattern, $culture) {
  $dt = $culture.DateTimeFormat
  $out = New-Object System.Text.StringBuilder
  $i = 0
  while ($i -lt $pattern.Length) {
    $c = $pattern[$i]
    if ($c -eq "'") {
      $j = $pattern.IndexOf("'", $i + 1); if ($j -lt 0) { $j = $pattern.Length }
      [void]$out.Append("'" + $pattern.Substring($i + 1, $j - $i - 1) + "'"); $i = $j + 1; continue
    }
    if ($c -eq '/') { [void]$out.Append($dt.DateSeparator); $i++; continue }
    if ($c -eq ':') { [void]$out.Append($dt.TimeSeparator); $i++; continue }
    if ($c -eq 't') {
      $n = 1; while ($i + $n -lt $pattern.Length -and $pattern[$i + $n] -eq 't') { $n++ }
      if ($n -ge 2) { [void]$out.Append('AM/PM') } else { [void]$out.Append('A/P') }
      $i += $n; continue
    }
    if ('dMyhHmsf'.Contains([string]$c)) {
      $n = 1; while ($i + $n -lt $pattern.Length -and $pattern[$i + $n] -eq $c) { $n++ }
      if ($c -eq 'f') { $i += $n; continue }
      if ($c -eq 'y' -and $n -ge 3) { $n = 4 }
      if ($c -eq 'M' -and $n -gt 4) { $n = 4 }
      [void]$out.Append([string]::new($c, $n)); $i += $n; continue
    }
    if ($c -eq '\') { $i++; if ($i -lt $pattern.Length) { [void]$out.Append("'" + $pattern[$i] + "'"); $i++ }; continue }
    if ($c -match '[A-Za-z]') { [void]$out.Append("'" + $c + "'"); $i++; continue }
    [void]$out.Append($c); $i++
  }
  return $out.ToString().Trim()
}

function Run-LocaleTable([string]$outPath) {
  New-Item -ItemType Directory -Force -Path $scratch | Out-Null
  $tmp = Join-Path $scratch 'locales.docx'
  $cultures = @([Globalization.CultureInfo]::GetCultures([Globalization.CultureTypes]::SpecificCultures) |
      Where-Object { $_.LCID -gt 0 -and $_.LCID -ne 4096 -and $_.Name } | Sort-Object Name)
  $props = [ordered]@{}
  for ($m = 1; $m -le 12; $m++) { $props['DM{0:00}' -f $m] = ('2024-{0:00}-15T12:00:00Z' -f $m) }
  for ($d = 0; $d -lt 7; $d++) { $props['DW{0}' -f $d] = ('2024-03-{0:00}T12:00:00Z' -f (3 + $d)) }
  $props['DT2'] = '2024-01-05T00:30:00Z'
  $props['DT3'] = '2024-01-05T12:00:00Z'
  Start-Word
  try {
    $doc = $script:wd.Documents.Add()
    foreach ($k in $props.Keys) { Add-Custom $doc $k 3 ([datetime]'2000-01-01') }
    $doc.SaveAs2($tmp, 16); $doc.Close(0)
  }
  finally { Stop-Word }
  Pin-Custom $tmp $props
  Pin-Core $tmp ([ordered]@{ 'dcterms:created' = '2023-11-02T08:09:10Z'; 'dcterms:modified' = '2024-03-03T14:05:09Z' })
  $codes = New-Object System.Collections.Generic.List[string]
  $pipeM = 'MMMM' + [char]39 + '|' + [char]39 + 'MMM'
  $pipeD = 'dddd' + [char]39 + '|' + [char]39 + 'ddd'
  for ($m = 1; $m -le 12; $m++) { $codes.Add(('DOCPROPERTY DM{0:00} \@ "{1}"' -f $m, $pipeM)) }
  for ($d = 0; $d -lt 7; $d++) { $codes.Add(('DOCPROPERTY DW{0} \@ "{1}"' -f $d, $pipeD)) }
  # Custom date properties have no time of day: the morning and afternoon names come from the pinned core dates.
  $codes.Add('CREATEDATE \@ "AM/PM"')
  $codes.Add('SAVEDATE \@ "AM/PM"')
  $codes.Add('DOCPROPERTY DM03 \@ "yyyy"')
  $per = $codes.Count
  $results = New-Object System.Collections.Generic.List[object]
  Start-Word
  try {
    $doc = $script:wd.Documents.Open($tmp, $false, $false)
    $script:wd.ScreenUpdating = $false
    foreach ($cu in $cultures) {
      foreach ($code in $codes) {
        $end = $doc.Content.End - 1
        $r = $doc.Range($end, $end)
        $f = $doc.Fields.Add($r, -1, $code, $false)
        try { $f.Code.LanguageID = [int]$cu.LCID } catch { }
        $doc.Content.InsertParagraphAfter()
      }
    }
    [void]$doc.Fields.Update()
    $all = @($doc.Fields)
    $idx = 0
    foreach ($cu in $cultures) {
      $vals = New-Object System.Collections.Generic.List[object]
      for ($k = 0; $k -lt $per; $k++) { $f = $all[$idx + $k]; $vals.Add(@([int]$f.Code.LanguageID, [string]$f.Result.Text)) }
      $idx += $per
      $results.Add(@($cu, $vals))
    }
    $doc.Close(0)
  }
  finally { Stop-Word }
  $rows = New-Object System.Collections.Generic.List[object]
  $skipped = New-Object System.Collections.Generic.List[string]
  foreach ($entry in $results) {
    $cu = $entry[0]; $vals = $entry[1]
    $bad = @($vals | Where-Object { $_[0] -ne $cu.LCID })
    if ($bad.Count -gt 0) { $skipped.Add($cu.Name + ' (language not accepted)'); continue }
    if ($vals[21][1] -ne '2024') { $skipped.Add($cu.Name + ' (calendar is not Gregorian)'); continue }
    $full = @(); $abbr = @()
    for ($m = 0; $m -lt 12; $m++) { $p = $vals[$m][1] -split '\|'; $full += $p[0]; $abbr += $p[1] }
    $dfull = @(); $dabbr = @()
    for ($d = 0; $d -lt 7; $d++) { $p = $vals[12 + $d][1] -split '\|'; $dfull += $p[0]; $dabbr += $p[1] }
    $dtf = $cu.DateTimeFormat
    $datePic = Convert-NetPicture $dtf.ShortDatePattern $cu
    $timePic = Convert-NetPicture $dtf.LongTimePattern $cu
    $line = (($full -join ',') + '|' + ($abbr -join ',') + '|' + ($dfull -join ',') + '|' + ($dabbr -join ',') + '|' + $vals[19][1] + '|' + $vals[20][1] + '|' + $datePic + '|' + $timePic)
    if ($line.Contains('"') -or $line.Contains('\')) { $skipped.Add($cu.Name + ' (quote or backslash in a name)'); continue }
    $rows.Add(@($cu.Name, $line))
  }
  # en-US first (LocaleNames::english reads DATA[0]).
  $sorted = @($rows | Sort-Object { if ($_[0] -eq 'en-US') { '' } else { $_[0] } })
  $data = New-Object System.Collections.Generic.List[string]
  $tags = New-Object System.Collections.Generic.List[string]
  foreach ($row in $sorted) {
    $i = $data.IndexOf($row[1])
    if ($i -lt 0) { $data.Add($row[1]); $i = $data.Count - 1 }
    $tags.Add('("' + $row[0] + '", ' + $i + ')')
  }
  $sb = New-Object System.Text.StringBuilder
  [void]$sb.AppendLine('//! Generated by `tools/office/fields-oracle.ps1 -LocaleTable`. Do not edit by hand.')
  [void]$sb.AppendLine('//!')
  [void]$sb.AppendLine('//! Names come from Word 16 (Field.Code.LanguageID per field); default date and time pictures')
  [void]$sb.AppendLine('//! are the Windows short-date and long-time patterns of the language. Left out (the default')
  [void]$sb.AppendLine('//! calendar is not Gregorian, or Word did not take the language): ' + $skipped.Count + ' tags.')
  [void]$sb.AppendLine('')
  [void]$sb.AppendLine('/// `(tag, index into DATA)`.')
  [void]$sb.AppendLine('pub(crate) static TAGS: &[(&str, usize)] = &[')
  foreach ($t in $tags) { [void]$sb.AppendLine('    ' + $t + ',') }
  [void]$sb.AppendLine('];')
  [void]$sb.AppendLine('')
  [void]$sb.AppendLine('/// `month_full|month_abbr|day_full|day_abbr|am|pm|date picture|time picture`; names separated by commas.')
  [void]$sb.AppendLine('pub(crate) static DATA: &[&str] = &[')
  foreach ($d in $data) { [void]$sb.AppendLine('    "' + $d + '",') }
  [void]$sb.AppendLine('];')
  Write-Text $outPath $sb.ToString()
  Write-Host ('locale table: ' + $tags.Count + ' tags, ' + $data.Count + ' distinct records, ' + $skipped.Count + ' left out')
}

function Run-Probe([string]$file) {
  $codes = Get-Content -LiteralPath $file -Encoding UTF8 | Where-Object { $_.Trim() -ne '' }
  Start-Word
  try {
    $doc = $script:wd.Documents.Add()
    foreach ($line in $codes) {
      $lang = $null
      $c = $line
      if ($c -match '^@(\d+)\|(.*)$') { $lang = [int]$Matches[1]; $c = $Matches[2] }
      Add-FieldParagraph $doc @{ code = $c; lang = $lang; stale = $false }
    }
    [void]$doc.Fields.Update()
    foreach ($f in @($doc.Fields)) {
      $res = $f.Result.Text -replace "`r", '<P>' -replace [string][char]7, '<A>' -replace [string][char]11, '<BR>' -replace "`t", '<T>'
      '[{0}] => [{1}]' -f $f.Code.Text.Trim(), (Json-Str $res)
    }
    $doc.Close(0)
  }
  finally { Stop-Word }
}

try {
  if ($Probe) { Run-Probe $Probe; return }
  if ($TocStyles) { Run-TocStyles; return }
  if ($LocaleTable) { Run-LocaleTable $LocaleTable; return }
  $sets = Get-Sets
  foreach ($s in $Set) {
    if (-not $sets.Contains($s)) { Write-Host "skipping unknown set '$s'"; continue }
    Build-Set $s $sets[$s]
  }
}
finally {
  if (Test-Path -LiteralPath $scratch) {
    # $scratch is a plain temp folder this script created (no links): safe to remove.
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
  }
}
