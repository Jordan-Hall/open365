<#
Spell-check oracle: Microsoft Word 16.0 spelling and suggestions over generated word lists and test paragraphs. Records
what Word flagged and what it suggested, so crates/spell/tests/word_parity.rs can hold Open365's checker to the same
results and report an agreement rate. Clean-room: only Word's outputs (flagged ranges, suggestion lists) are observed.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is never
run in CI. CI consumes the committed output:
  fixtures/spell/word-refs-<tag>.tsv        isolated words: word <TAB> 1 if Word accepts, 0 if flagged <TAB> suggestions (|)
  fixtures/spell/paragraph-refs-<tag>.tsv   paragraph <TAB> flagged ranges "start-end,..." (UTF-16 offsets in the paragraph)

  pwsh -NoProfile -File tools/office/make-spell-refs.ps1 [-Tags en-US,en-GB,en-CA,en-AU] [-Stems 700] [-Typos 900] [-RepeatsOnly]

Words. The list is generated here, deterministically (System.Random seed 365), from the stems of the bundled
crates/spell/dictionaries/*.dic files: sampled stems, single-edit typos of other stems (swap, delete, insert, replace,
duplicate) and case variants. Word's verdict on each is the truth, whatever the dictionaries say. Every word is its own
paragraph of a scratch document whose proofing language is set to the tag; the errors come from Document.SpellingErrors
and the suggestions from Range.GetSpellingSuggestions on the flagged range (so the document language applies, not the
application default). Word's own options (ignore UPPERCASE, words with numbers, internet and file addresses) are read,
never changed, and the script stops if they are not the defaults. Repeated words are flagged by Word as spelling errors.

Process safety: Word is started by this script (its own instance, AutomationSecurity = msoAutomationSecurityForceDisable
(3)). The script refuses to continue if COM attached to a WINWORD process that it did not start, quits in a finally
block and only ever stops the process id it started. Other WINWORD processes and all Office and regional settings are
never touched. Output has no machine, user or path names.
#>
param(
  [string[]]$Tags = @('en-US', 'en-GB', 'en-CA', 'en-AU'),
  [int]$Stems = 700,
  [int]$Typos = 900,
  [switch]$RepeatsOnly
)
$ErrorActionPreference = 'Stop'
$Tags = @($Tags | ForEach-Object { $_ -split ',' })
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$dictDir = Join-Path $root 'crates/spell/dictionaries'
$outDir = Join-Path $root 'fixtures/spell'
$langIds = @{ 'en-US' = 1033; 'en-GB' = 2057; 'en-CA' = 4105; 'en-AU' = 3081 }
$paragraphTags = @('en-US', 'en-GB')

function Get-Pids { @(Get-Process -Name WINWORD -ErrorAction SilentlyContinue | ForEach-Object Id) }

$script:wd = $null
$script:wordPids = @()

function Start-Word {
  $before = Get-Pids
  $app = New-Object -ComObject Word.Application
  $script:wordPids = @(Get-Pids | Where-Object { $_ -notin $before })
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

# --- word list ---------------------------------------------------------------------------------------------------
function Read-Stems([string]$tag) {
  $file = Join-Path $dictDir (($tag -replace '-', '_') + '.dic')
  $lines = [IO.File]::ReadAllLines($file, (New-Object Text.UTF8Encoding $false))
  $lines | Select-Object -Skip 1 | ForEach-Object { ($_ -split '/')[0] } | Where-Object { $_ -match "^[A-Za-z][A-Za-z']*$" }
}

function Mutate([string]$w, [Random]$rng) {
  $letters = 'abcdefghijklmnopqrstuvwxyz'
  $i = $rng.Next(1, $w.Length - 1)
  switch ($rng.Next(0, 5)) {
    0 { return $w.Substring(0, $i) + $w[$i + 1] + $w[$i] + $w.Substring($i + 2) }
    1 { return $w.Remove($i, 1) }
    2 { return $w.Insert($i, [string]$letters[$rng.Next(0, 26)]) }
    3 { return $w.Substring(0, $i) + $letters[$rng.Next(0, 26)] + $w.Substring($i + 1) }
    default { return $w.Insert($i, [string]$w[$i]) }
  }
}

function New-WordList {
  $rng = New-Object Random 365
  $set = New-Object 'System.Collections.Generic.List[string]'
  $seen = New-Object 'System.Collections.Generic.HashSet[string]'
  $pool = @()
  foreach ($t in $langIds.Keys | Sort-Object) {
    $tagStems = @(Read-Stems $t)
    $pool += $tagStems
    foreach ($k in 1..$Stems) {
      $w = $tagStems[$rng.Next(0, $tagStems.Count)]
      if ($seen.Add($w)) { $set.Add($w) }
    }
  }
  $lower = @($pool | Where-Object { $_ -match "^[a-z]{4,}$" })
  foreach ($k in 1..$Typos) {
    $w = Mutate $lower[$rng.Next(0, $lower.Count)] $rng
    if ($seen.Add($w)) { $set.Add($w) }
  }
  foreach ($k in 1..150) {
    $w = $lower[$rng.Next(0, $lower.Count)]
    foreach ($v in @(($w.Substring(0, 1).ToUpper() + $w.Substring(1)), $w.ToUpper(), ($w.Substring(0, 1) + $w.Substring(1, 1).ToUpper() + $w.Substring(2)))) {
      if ($seen.Add($v)) { $set.Add($v) }
    }
  }
  return , $set.ToArray()
}

# --- test paragraphs -----------------------------------------------------------------------------------------------
$paragraphs = @(
  'The quick brown fox jumps over the lazy dog.',
  'The quik brown fox jumpd over teh lazy dog.',
  'This is is a test of repeated words.',
  'The the start of a sentence.',
  'Well, well, well. Fox fox. Fox, fox.',
  'It was a very very good day, and and then some.',
  'That that is is that that is not not that that is not.',
  'NASA and NATO and the FBI are fine, but NASAA is ignored too.',
  'Mixed UPPER and lower WORDS here in a sentenec.',
  'abc123 and 123abc and a1b2 and x86 and mp3 words with numbers.',
  'Only 3 apples, 42 pears, 1,000 plums and 3.14 pies.',
  'The 1st, 2nd, 3rd, 4th, 11th, 21st and 101st places.',
  'Visit http://www.example.com/page?x=1 or https://example.org today.',
  'Write to someone@example.com or other.person@mail.example.co.uk now.',
  'Files like C:\Users\Someone\Documents\report.docx and \\server\share\file.txt are paths.',
  'Open /usr/local/bin/tool or ./relative/path/file.txt please.',
  'Go to www.example.com for more, or example.com/page too.',
  'Don''t, can''t, won''t, it''s, they''re, we''ve and I''d are fine.',
  'Dont, cant, wont, its, theyre, weve and Id lack apostrophes.',
  'John''s book and Johns book and the dogs'' bowls.',
  'He said ''hello'' and she said "goodbye" to the world.',
  'Curly don<rsq>t and it<rsq>s and John<rsq>s should match straight ones.',
  'A well-known, state-of-the-art, mother-in-law and wel-known thing.',
  'Co-operate, e-mail, on-line, re-enter and pre-war are hyphenated.',
  'Use and/or, he/she, either/or and yes/no here.',
  'The colour of the neighbour''s favourite centre is grey.',
  'The color of the neighbor''s favorite center is gray.',
  'We organise and organize, analyse and analyze, realise and realize.',
  'Programme, program, cheque, check, tyre, tire, aluminium, aluminum.',
  'A caf<eacute>, a na<iuml>ve r<eacute>sum<eacute>, a fa<ccedil>ade and a jalape<ntilde>o.',
  'The cafe, naive resume, facade and jalapeno without accents.',
  'iPhone, eBay, McDonald, PowerPoint, JavaScript and camelCase words.',
  'Mr. Smith, Dr. Jones, e.g. this, i.e. that, etc. and so on.',
  'U.S.A. and U.K. and a.m. and p.m. are abbreviations.',
  'foo_bar and snake_case_word and CONSTANT_NAME are identifiers.',
  'The wordsmith''s wordcount and the databse in the datbase.',
  'Unhappy, rewrite, disagree, preheat, misspell and overreact are prefixed.',
  'Quickly, happiness, runner, walked, walking and walks are inflected.',
  'Teh adn taht hte recieve seperate definately occured.',
  'Accomodate, occurence, untill, wich, thier, becuase and beleive.',
  'Their, there and they''re are real words used wrongly here.',
  'Lorem ipsum dolor sit amet, consectetur adipiscing elit.',
  'Anna, Bob, Carol, Dave, Eve, Frank and Grace went to Paris.',
  'Zxqv, qwrt, and plkjh are nonsense but Bzzt might be a word.',
  'A I a i a. I. A, b, c, x, y, z are single letters.',
  'Smith-Jones, O''Brien, D''Arcy, McKenzie and MacDonald are names.',
  'Dr, Mr, Mrs, Ms, Prof, St, Ave, Rd are bare abbreviations.',
  'Email, emails, internet, website, online, offline, login, username.',
  'Smartphone, wifi, blog, podcast, hashtag, selfie, emoji, app, apps.',
  'COVID, covid, Covid, laptop, desktop, cloud, spreadsheet, workbook.',
  'Words<emdash>joined with em dashes<emdash>should split at the dash.',
  'Ranges like 10-20 and 3<endash>5 and words like pre<endash>war split.',
  'Ellipsis<ellipsis> and (parentheses) and [brackets] and {braces} wrap words.',
  'Quoted <ldq>curly<rdq> words and <lsq>single<rsq> ones are handled.',
  'Hello, World! How are you? Fine; thanks: really.',
  'Trailing space and  double  spaces  inside.',
  'The percentages 50% and 100% and $5 and <pound>10 and <euro>20 are numbers.',
  'See page 3 of 12, item 4b, section 2.1.3 and figure A1.',
  'Tuesday, Wednesday, January, February and Sept are dates.',
  'Neighbourhood, neighborhood, behaviour, behavior, honour, honor.',
  'Jewellery, jewelry, grey, gray, mum, mom, maths, math.',
  'Enroll, enrol, fulfil, fulfill, skilful, skillful, travelling, traveling.',
  'Judgement, judgment, ageing, aging, practise, practice.',
  'Doin'', goin'', rock''n''roll and ''tis are informal.',
  'Zebra Zebras zebra zebras ZEBRA zEbra.'
)

$map = @{ '<rsq>' = 0x2019; '<lsq>' = 0x2018; '<ldq>' = 0x201C; '<rdq>' = 0x201D; '<emdash>' = 0x2014; '<endash>' = 0x2013; '<ellipsis>' = 0x2026; '<eacute>' = 0xE9; '<iuml>' = 0xEF; '<ccedil>' = 0xE7; '<ntilde>' = 0xF1; '<pound>' = 0xA3; '<euro>' = 0x20AC }
foreach ($k in $map.Keys) { $paragraphs = @($paragraphs | ForEach-Object { $_.Replace($k, [string][char]$map[$k]) }) }

# Candidate doubled words: function words and reduplications that real text repeats on purpose, plus sampled stems.
function New-DoubleList {
  $fixed = 'had that is was be been do did can will would so no yes ha bye boo go yo blah tut wah yum chop dum pom mama papa bon very the a an of to in it we they you he she I and or but not now then there here well oh ah hmm no-no knock rah hush cha pretty really long far near more less many much few little big small little'.Split(' ')
  $rng = New-Object Random 366
  $pool = @(Read-Stems 'en-US' | Where-Object { $_ -match '^[a-z]{2,}$' })
  $list = New-Object 'System.Collections.Generic.List[string]'
  $seen = New-Object 'System.Collections.Generic.HashSet[string]'
  foreach ($w in $fixed) { if ($seen.Add($w)) { $list.Add($w) } }
  foreach ($k in 1..250) { $w = $pool[$rng.Next(0, $pool.Count)]; if ($seen.Add($w)) { $list.Add($w) } }
  return , $list.ToArray()
}
$doubles = New-DoubleList

# --- run ------------------------------------------------------------------------------------------------------------
function Get-ParagraphIndex([long[]]$starts, [long]$pos) {
  $lo = 0; $hi = $starts.Length - 1
  while ($lo -lt $hi) {
    $mid = [int](($lo + $hi + 1) / 2)
    if ($starts[$mid] -le $pos) { $lo = $mid } else { $hi = $mid - 1 }
  }
  return $lo
}

function Invoke-Document($app, [string[]]$texts, [int]$langId, [bool]$wantSuggestions) {
  $doc = $app.Documents.Add()
  try {
    $doc.Content.Text = ($texts -join "`r")
    $doc.Content.LanguageID = $langId
    $doc.Content.NoProofing = 0
    $starts = New-Object 'long[]' $texts.Length
    $pos = 0L
    for ($i = 0; $i -lt $texts.Length; $i++) { $starts[$i] = $pos; $pos += $texts[$i].Length + 1 }
    $flags = @{}
    $suggestions = @{}
    $errors = $doc.SpellingErrors
    $n = $errors.Count
    $skipped = 0
    for ($k = 1; $k -le $n; $k++) {
      try { $e = $errors.Item($k) } catch { $skipped++; continue }
      $s = [long]$e.Start; $t = [long]$e.End
      $p = Get-ParagraphIndex $starts $s
      $range = "{0}-{1}" -f ($s - $starts[$p]), ($t - $starts[$p])
      if (-not $flags.ContainsKey($p)) { $flags[$p] = New-Object 'System.Collections.Generic.List[string]' }
      $flags[$p].Add($range)
      if ($wantSuggestions) {
        $names = @()
        try { foreach ($x in $e.GetSpellingSuggestions()) { $names += [string]$x.Name } } catch {}
        $suggestions[$p] = $names
      }
    }
    if ($skipped -gt 0) { Write-Host "  note: $skipped of $n spelling errors could not be read" }
    return @{ Flags = $flags; Suggestions = $suggestions }
  }
  finally { $doc.Close(0) }
}

function Write-Tsv([string]$path, [string[]]$header, [string[]]$lines) {
  New-Item -ItemType Directory -Force (Split-Path -Parent $path) | Out-Null
  $body = ($header | ForEach-Object { '# ' + $_ }) + $lines
  [IO.File]::WriteAllText($path, (($body -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
}

$words = New-WordList
Write-Host ("word list: {0} words, {1} paragraphs" -f $words.Count, $paragraphs.Count)
Start-Word
try {
  $app = $script:wd
  if (-not ($app.Options.IgnoreUppercase -and $app.Options.IgnoreMixedDigits -and $app.Options.IgnoreInternetAndFileAddresses)) {
    throw 'Word proofing options are not the defaults (ignore UPPERCASE / numbers / addresses); refusing to measure'
  }
  $version = [string]$app.Version + '.' + [string]$app.Build
  foreach ($tag in $Tags) {
    $id = $langIds[$tag]
    # Probe: the proofing tools for this language must exist, or every word would look "fine".
    $probe = Invoke-Document $app @('qzxvkq') $id $false
    if (-not $probe.Flags.ContainsKey(0)) { Write-Host "skip ${tag}: Word has no proofing tools for language $id"; continue }
    if (-not $RepeatsOnly) {
    $lines = New-Object 'System.Collections.Generic.List[string]'
    $okCount = 0
    $chunk = 400
    for ($off = 0; $off -lt $words.Count; $off += $chunk) {
      $slice = @($words[$off..([Math]::Min($off + $chunk, $words.Count) - 1)])
      $r = Invoke-Document $app $slice $id $true
      for ($i = 0; $i -lt $slice.Count; $i++) {
        if ($r.Flags.ContainsKey($i)) { $lines.Add(("{0}`t0`t{1}" -f $slice[$i], ($r.Suggestions[$i] -join '|'))) }
        else { $lines.Add(("{0}`t1`t" -f $slice[$i])); $okCount++ }
      }
    }
    $header = @("Microsoft Word $version spelling oracle, proofing language $tag ($id)", 'word <TAB> 1 accepted / 0 flagged <TAB> suggestions in Word order', 'generated by tools/office/make-spell-refs.ps1')
    Write-Tsv (Join-Path $outDir "word-refs-$tag.tsv") $header $lines.ToArray()
    Write-Host ("{0}: {1} words, {2} accepted, {3} flagged" -f $tag, $words.Count, $okCount, ($words.Count - $okCount))
    }
    if ($paragraphTags -contains $tag) {
      if (-not $RepeatsOnly) {
      $r = Invoke-Document $app $paragraphs $id $false
      $plines = New-Object 'System.Collections.Generic.List[string]'
      for ($i = 0; $i -lt $paragraphs.Count; $i++) {
        $f = if ($r.Flags.ContainsKey($i)) { $r.Flags[$i] -join ',' } else { '' }
        $plines.Add(("{0}`t{1}" -f $paragraphs[$i], $f))
      }
      $header = @("Microsoft Word $version spelling oracle, proofing language $tag ($id)", 'paragraph <TAB> flagged ranges start-end (UTF-16 units, end exclusive), comma separated', 'generated by tools/office/make-spell-refs.ps1')
      Write-Tsv (Join-Path $outDir "paragraph-refs-$tag.tsv") $header $plines.ToArray()
      Write-Host ("{0}: {1} paragraphs" -f $tag, $paragraphs.Count)
      }
      # Doubled words: which pairs does Word treat as a repeated word? One paragraph per pair.
      $pairs = New-Object 'System.Collections.Generic.List[string]'
      foreach ($w in $doubles) { $pairs.Add("$w $w") }
      $pairs.AddRange([string[]]@('The the', 'THE THE', 'The The', 'the  the', 'the, the', 'the - the', "the`u{A0}the", 'the. the', '1 1', 'a a', 'I I', 'x x', 'the (the', 'the "the', 'Hello hello', 'ok ok', 'OK ok', 'e-mail e-mail', 'rock rock rock'))
      $r = Invoke-Document $app $pairs.ToArray() $id $false
      $rlines = New-Object 'System.Collections.Generic.List[string]'
      for ($i = 0; $i -lt $pairs.Count; $i++) {
        $f = if ($r.Flags.ContainsKey($i)) { $r.Flags[$i] -join ',' } else { '' }
        $rlines.Add(("{0}`t{1}" -f $pairs[$i], $f))
      }
      $header = @("Microsoft Word $version spelling oracle, proofing language $tag ($id)", 'doubled-word paragraph <TAB> flagged ranges (a flagged pair of real words is Word''s repeated-word error)', 'generated by tools/office/make-spell-refs.ps1')
      Write-Tsv (Join-Path $outDir "repeat-refs-$tag.tsv") $header $rlines.ToArray()
    }
  }
}
finally { Stop-Word }
