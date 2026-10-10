<#
Fetches the Microsoft template corpus listed in fixtures/external/ms-templates.toml into
fixtures/external/ms-templates/ (gitignored; never committed). See docs/research/corpus-licensing.md.

  pwsh -File tools/corpus/fetch-ms-templates.ps1                 # sync: download missing/changed files, verify sha256
  pwsh -File tools/corpus/fetch-ms-templates.ps1 -Discover       # scrape the public category pages, append NEW entries
  pwsh -File tools/corpus/fetch-ms-templates.ps1 -Reference      # sync, then generate missing reference outputs (local Office oracle)
  pwsh -File tools/corpus/fetch-ms-templates.ps1 -Verify         # sync + require sha256 match and reference outputs for every entry; exit 0 only if all OK

Rules enforced: https only; host must be Microsoft-owned (allowlist below); redirects are followed by hand and
every hop is re-checked; at most 50 MB per file. No login, cookies or credentials are used or sent.
Idempotent: a file whose sha256 already matches is not downloaded again.
#>
[CmdletBinding()]
param(
  [switch]$Discover,
  [switch]$Reference,
  [switch]$Verify,
  [int]$PerCategory = 14,     # -Discover: max new entries added per category in one run
  [string[]]$Pages            # -Discover: override the page list (slug=category, e.g. "resumes=resumes")
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$manifestPath = Join-Path $root 'fixtures\external\ms-templates.toml'
$cacheDir = Join-Path $root 'fixtures\external\ms-templates'
$refDir = Join-Path $root 'fixtures\reference\ms-templates'
$MaxBytes = 50MB
$AllowedKinds = 'dotx', 'xltx', 'docx', 'xlsx'
$AllowedHostSuffixes = @('.microsoft', '.microsoft.com', '.office.net', '.office.com', '.cloud.microsoft', '.live.com', '.officeapps.live.com', '.azureedge.net')
$Licence = 'Copyright Microsoft. Downloaded from Microsoft''s public template catalog for local interoperability testing only; not redistributed and not committed. See docs/research/corpus-licensing.md.'
$DefaultPages = @(
  'resumes=resumes', 'cover-letters=letters', 'letters=letters', 'business-reports=reports',
  'newsletters=newsletters', 'calendars=calendars', 'budgets=budgets', 'invoices=invoices',
  'wedding-planners=planners', 'timelines=planners', 'trackers=trackers', 'schedules=schedules',
  'charts=charts', 'gantt-charts=charts', 'business-plans=reports', 'agendas=schedules'
)

function Test-AllowedUri([uri]$u) {
  if ($u.Scheme -ne 'https') { return $false }
  $h = $u.Host.ToLowerInvariant()
  foreach ($s in $AllowedHostSuffixes) { if ($h.EndsWith($s) -and $h.Length -gt $s.Length) { return $true } }
  return $false
}

$script:http = $null
function Get-Http {
  if (-not $script:http) {
    $handler = [Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $handler.UseCookies = $false
    $script:http = [Net.Http.HttpClient]::new($handler)
    $script:http.Timeout = [TimeSpan]::FromSeconds(120)
    $script:http.DefaultRequestHeaders.UserAgent.ParseAdd('Open365-corpus-fetch/1.0 (interoperability test corpus)')
  }
  $script:http
}

# GET with manual, allowlist-checked redirects. Returns the response (headers read); caller disposes.
function Invoke-SafeGet([string]$url) {
  $cur = [uri]$url
  for ($hop = 0; $hop -le 5; $hop++) {
    if (-not (Test-AllowedUri $cur)) { throw "blocked: $cur is not https on a Microsoft-owned host" }
    $resp = (Get-Http).GetAsync($cur, [Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
    $code = [int]$resp.StatusCode
    if ($code -ge 300 -and $code -lt 400 -and $resp.Headers.Location) {
      $cur = [uri]::new($cur, $resp.Headers.Location); $resp.Dispose(); continue
    }
    if (-not $resp.IsSuccessStatusCode) { $resp.Dispose(); throw "HTTP $code for $cur" }
    return $resp
  }
  throw "too many redirects for $url"
}

function Get-FileBytes([string]$url, [string]$dest) {
  $resp = Invoke-SafeGet $url
  try {
    $len = $resp.Content.Headers.ContentLength
    if ($len -and $len -gt $MaxBytes) { throw "size $len exceeds the 50 MB limit" }
    $tmp = "$dest.part"
    $in = $resp.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
    $out = [IO.File]::Create($tmp)
    try {
      $buf = New-Object byte[] 81920; $total = 0L
      while (($n = $in.Read($buf, 0, $buf.Length)) -gt 0) {
        $total += $n
        if ($total -gt $MaxBytes) { throw 'download exceeds the 50 MB limit' }
        $out.Write($buf, 0, $n)
      }
    } finally { $out.Dispose(); $in.Dispose() }
    Move-Item -LiteralPath $tmp -Destination $dest -Force
  } catch { Remove-Item -LiteralPath "$dest.part" -Force -ErrorAction SilentlyContinue; throw }
  finally { $resp.Dispose() }
}

function Get-Sha([string]$p) { (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLowerInvariant() }

# ---- manifest (minimal TOML subset: [[template]] tables with string/int keys) ----
function Read-Manifest {
  $list = New-Object System.Collections.Generic.List[object]
  if (-not (Test-Path -LiteralPath $manifestPath)) { return , $list }
  $cur = $null
  foreach ($line in [IO.File]::ReadAllLines($manifestPath)) {
    $t = $line.Trim()
    if ($t -eq '' -or $t.StartsWith('#')) { continue }
    if ($t -eq '[[template]]') { $cur = [ordered]@{}; $list.Add($cur); continue }
    if ($t -match '^(\w+)\s*=\s*"(.*)"$') { $cur[$Matches[1]] = ($Matches[2] -replace '\\(["\\])', '$1') }
    elseif ($t -match '^(\w+)\s*=\s*(\d+)$') { $cur[$Matches[1]] = [long]$Matches[2] }
  }
  , $list
}
function Esc([string]$s) { '"' + ($s -replace '\\', '\\' -replace '"', '\"') + '"' }
function Write-Manifest($list) {
  $sb = New-Object Text.StringBuilder
  [void]$sb.Append("# Microsoft template corpus manifest. Files are NOT committed; fetch with tools/corpus/fetch-ms-templates.ps1.`n")
  [void]$sb.Append("# Source: Microsoft's public template catalog (create.microsoft.com). Terms: docs/research/corpus-licensing.md`n")
  foreach ($e in ($list | Sort-Object { $_.category }, { $_.title }, { $_.id })) {
    [void]$sb.Append("`n[[template]]`n")
    foreach ($k in 'id', 'title', 'category', 'kind', 'url', 'sha256') { [void]$sb.Append("$k = $(Esc $e[$k])`n") }
    [void]$sb.Append("size = $($e['size'])`n")
    [void]$sb.Append("licence = $(Esc $e['licence'])`n")
  }
  New-Item -ItemType Directory -Force (Split-Path $manifestPath) | Out-Null
  [IO.File]::WriteAllText($manifestPath, $sb.ToString(), (New-Object Text.UTF8Encoding $false))
}

function Get-Ext([string]$url) { ([IO.Path]::GetExtension(([uri]$url).AbsolutePath)).TrimStart('.').ToLowerInvariant() }
function Cache-Path($e) { Join-Path $cacheDir ("{0}.{1}" -f $e.id, $e.kind) }

# ---- discover ----
function Invoke-Discover($list) {
  $known = @{}; foreach ($e in $list) { $known[$e.id] = $true }
  $perCat = @{}
  foreach ($e in $list) { $perCat[$e.category] = 0 }   # cap counts NEW entries only
  $pageSpecs = if ($Pages) { $Pages } else { $DefaultPages }
  New-Item -ItemType Directory -Force $cacheDir | Out-Null
  foreach ($spec in $pageSpecs) {
    $slug, $cat = $spec -split '=', 2
    if (-not $cat) { $cat = $slug }
    if (-not $perCat.ContainsKey($cat)) { $perCat[$cat] = 0 }
    $pageUrl = "https://create.microsoft.com/en-us/templates/$slug"
    try {
      $resp = Invoke-SafeGet $pageUrl
      $html = $resp.Content.ReadAsStringAsync().GetAwaiter().GetResult(); $resp.Dispose()
    } catch { Write-Warning "page ${slug}: $($_.Exception.Message)"; continue }
    $rx = 'href="https://view\.officeapps\.live\.com/op/view\.aspx\?src=([^"]+)"[^>]*>.*?__Title">([^<]*)<'
    foreach ($m in [regex]::Matches($html, $rx, 'Singleline')) {
      if ($perCat[$cat] -ge $PerCategory) { break }
      $url = [uri]::UnescapeDataString($m.Groups[1].Value)
      $title = [Net.WebUtility]::HtmlDecode($m.Groups[2].Value).Trim()
      if ($url -notmatch '/catalog-assets/en-us/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/') { continue }
      $id = $Matches[1]
      $kind = Get-Ext $url
      if ($known.ContainsKey($id) -or $kind -notin $AllowedKinds -or -not (Test-AllowedUri $url)) { continue }
      $dest = Join-Path $cacheDir "$id.$kind"
      try { Get-FileBytes $url $dest } catch { Write-Warning "skip ${id}: $($_.Exception.Message)"; continue }
      $fi = Get-Item -LiteralPath $dest
      $list.Add([ordered]@{ id = $id; title = $title; category = $cat; kind = $kind; url = $url; sha256 = (Get-Sha $dest); size = [long]$fi.Length; licence = $Licence })
      $known[$id] = $true; $perCat[$cat]++
      Write-Output "added $cat/$id ($title)"
    }
  }
  Write-Manifest $list
}

# ---- sync ----
function Invoke-Sync($list) {
  New-Item -ItemType Directory -Force $cacheDir | Out-Null
  $bad = 0
  foreach ($e in $list) {
    $p = Cache-Path $e
    try {
      if ($e.kind -notin $AllowedKinds) { throw "kind '$($e.kind)' not allowed" }
      if (-not (Test-AllowedUri $e.url)) { throw 'url not https on a Microsoft-owned host' }
      if ($e.size -gt $MaxBytes) { throw 'manifest size exceeds 50 MB' }
      if (-not ((Test-Path -LiteralPath $p) -and (Get-Sha $p) -eq $e.sha256)) {
        Get-FileBytes $e.url $p
        $got = Get-Sha $p
        if ($got -ne $e.sha256) { Remove-Item -LiteralPath $p -Force; throw "sha256 mismatch (manifest $($e.sha256), got $got)" }
      }
    } catch { Write-Output "FAIL $($e.id): $($_.Exception.Message)"; $bad++ }
  }
  $bad
}

function Test-Reference($e) {
  $d = Join-Path $refDir $e.id
  if ($e.kind -in 'docx', 'dotx') { return (Test-Path (Join-Path $d 'pages.json')) -and (Test-Path (Join-Path $d 'layout.json')) }
  return (Test-Path (Join-Path $d 'values.json'))
}

# Runs one oracle script on one file in a child pwsh with a hard timeout. If it hangs (Excel CalculateFull and
# Word layout can hang on some files), the child and only the Office processes that appeared after it started are killed.
function Invoke-Oracle([string]$script, [string]$file, [string[]]$extra, [int]$TimeoutSec = 90) {
  $before = @(Get-Process -Name WINWORD, EXCEL -ErrorAction SilentlyContinue | ForEach-Object Id)
  $log = [IO.Path]::GetTempFileName()
  $argList = @('-NoProfile', '-File', $script) + $extra + @('-Path', $file, '-OutDir', $refDir)
  $p = Start-Process pwsh -ArgumentList $argList -PassThru -WindowStyle Hidden -RedirectStandardOutput $log
  try {
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
      Write-Output "TIMEOUT $([IO.Path]::GetFileName($file)) after ${TimeoutSec}s; killing the oracle and its Office instance"
      Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
      Get-Process -Name WINWORD, EXCEL -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $before } |
        Stop-Process -Force -ErrorAction SilentlyContinue
    }
    Get-Content -LiteralPath $log | Where-Object { $_ -match '^(SKIPPED|WARN)' }
  } finally { Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue }
}

function Invoke-Reference($list) {
  $todo = @($list | Where-Object { -not (Test-Reference $_) -and $_.kind -in 'docx', 'xlsx' })
  if ($todo.Count -eq 0) { Write-Output 'reference outputs complete'; return }
  New-Item -ItemType Directory -Force $refDir | Out-Null
  $office = Join-Path $root 'tools\office\office-oracle.ps1'
  $layout = Join-Path $root 'tools\office\word-layout-oracle.ps1'
  foreach ($e in $todo) {
    $p = Cache-Path $e
    $d = Join-Path $refDir $e.id
    if (-not ((Test-Path (Join-Path $d 'pages.json')) -or (Test-Path (Join-Path $d 'values.json')))) {
      Invoke-Oracle $office $p @('-Mode', 'reference')
    }
    if ($e.kind -eq 'docx' -and -not (Test-Path (Join-Path $d 'layout.json'))) { Invoke-Oracle $layout $p @() }
    Write-Output "ref $($e.id) $(if (Test-Reference $e) { 'ok' } else { 'INCOMPLETE' })"
  }
}
$list = Read-Manifest
if ($Discover) { Invoke-Discover $list }
$bad = Invoke-Sync $list
if ($Reference -or $Discover) { Invoke-Reference $list }
if ($Verify) {
  $missing = @($list | Where-Object { -not (Test-Reference $_) })
  foreach ($e in $missing) { Write-Output "NO-REFERENCE $($e.id) ($($e.kind) $($e.title))" }
  Write-Output "entries: $($list.Count), download/sha failures: $bad, missing reference: $($missing.Count)"
  if ($script:http) { $script:http.Dispose() }
  exit $(if ($bad -eq 0 -and $missing.Count -eq 0 -and $list.Count -gt 0) { 0 } else { 1 })
}
if ($script:http) { $script:http.Dispose() }
exit $(if ($bad -eq 0) { 0 } else { 1 })
