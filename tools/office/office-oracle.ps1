<#
Uses the locally installed Microsoft Word / Excel 16.0 over COM as the reference oracle for Open365 output.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation,
so this script is never run in CI. CI consumes the committed outputs under fixtures/reference/.

  -Mode verify    open each file read-only, no repair; print PASS/FAIL per file; exit 1 if any FAIL.
  -Mode reference write fixtures/reference/<docx|xlsx>/<name>/ (PDF + pages.json, or values.json + PDF).

Files are opened read-only and never saved. Only Office processes started by this script are killed.
JSON contains repo-relative paths only.
#>
param(
  [Parameter(Mandatory)][ValidateSet('verify', 'reference')][string]$Mode,
  [string[]]$Path,
  [string]$InputDir,  # optional: scan this directory (in addition to -Path)
  [string]$OutDir,    # optional: write <OutDir>/<name>/ instead of fixtures/reference/<docx|xlsx>/<name>/
  [switch]$XlsxPdf  # opt-in: Excel PDF export hangs when no usable printer driver exists (observed on this machine)
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$refRoot = Join-Path $root 'fixtures\reference'
$M = [Type]::Missing
$skipped = New-Object System.Collections.Generic.List[string]
$produced = New-Object System.Collections.Generic.List[string]
$failed = 0

function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }
function Is-Word($f) { $f -match '\.(docx|dotx|dotm|docm)$' }
function Rel($p) { ([IO.Path]::GetRelativePath($root, $p)) -replace '\\', '/' }
function Col-Letter([int]$n) { $s = ''; while ($n -gt 0) { $m = ($n - 1) % 26; $s = [string][char](65 + $m) + $s; $n = [int][math]::Floor(($n - 1) / 26) }; $s }
function Write-Json($obj, $file) {
  $json = $obj | ConvertTo-Json -Depth 10
  [IO.File]::WriteAllText($file, $json + "`n", (New-Object Text.UTF8Encoding $false))
}

# collect files
$files = New-Object System.Collections.Generic.List[string]
if ($InputDir) { $Path = @(@($Path) | Where-Object { $_ }) + $InputDir }
if ($OutDir) { $OutDir = [IO.Path]::GetFullPath($OutDir) }
foreach ($p in @($Path)) {
  $rp = (Resolve-Path -LiteralPath $p).Path
  if (Test-Path -LiteralPath $rp -PathType Container) {
    Get-ChildItem -LiteralPath $rp -Recurse -File -Include *.docx, *.dotx, *.dotm, *.docm, *.xlsx, *.xltx, *.xltm, *.xlsm |
      Where-Object { $_.Name -notlike '~$*' -and $_.FullName -notlike "$refRoot*" } |
      Sort-Object FullName | ForEach-Object { $files.Add($_.FullName) }
  } else { $files.Add($rp) }
}
if ($files.Count -eq 0) { Write-Output 'no Word/Excel files found'; exit 1 }

$wordBefore = Get-Pids 'WINWORD'; $excelBefore = Get-Pids 'EXCEL'
$word = $null; $excel = $null; $wordPids = @(); $excelPids = @()

function Get-Word {
  if ($null -eq $script:word) {
    $script:word = New-Object -ComObject Word.Application
    $script:wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $wordBefore })
    if ($script:wordPids.Count -eq 0) { throw 'COM attached to an Office instance this script did not start; refusing to touch it' }
    $script:word.Visible = $false; $script:word.DisplayAlerts = 0
    $script:word.AutomationSecurity = 3  # msoAutomationSecurityForceDisable: macros never run
  }
  $script:word
}
function Get-Excel {
  if ($null -eq $script:excel) {
    $script:excel = New-Object -ComObject Excel.Application
    $script:excelPids = @(Get-Pids 'EXCEL' | Where-Object { $_ -notin $excelBefore })
    if ($script:excelPids.Count -eq 0) { throw 'COM attached to an Office instance this script did not start; refusing to touch it' }
    $script:excel.Visible = $false; $script:excel.DisplayAlerts = $false
    $script:excel.AutomationSecurity = 3  # msoAutomationSecurityForceDisable: macros never run
  }
  $script:excel
}
function Invoke-Named($obj, $method, $names, $vals) {
  $obj.GetType().InvokeMember($method, [Reflection.BindingFlags]::InvokeMethod, $null, $obj, [object[]]$vals, $null, $null, [string[]]$names)
}
function Open-Doc($path) {
  $w = Get-Word; if ($null -eq $w) { throw "word null" } elseif ($null -eq $w.Documents) { throw "documents null; type=$($w.GetType().FullName)" }
  Invoke-Named $w.Documents 'Open' @('FileName', 'ConfirmConversions', 'ReadOnly', 'AddToRecentFiles', 'Revert', 'Visible', 'OpenAndRepair', 'NoEncodingDialog') @($path, $false, $true, $false, $false, $false, $false, $true)
}
function Open-Book($path) {
  $x = Get-Excel
  Invoke-Named $x.Workbooks 'Open' @('FileName', 'UpdateLinks', 'ReadOnly', 'IgnoreReadOnlyRecommended', 'AddToMru', 'CorruptLoad') @($path, 0, $true, $true, $false, 0)
}

function Reset-App($f) {
  # Office died (RPC error): drop it, kill only our own pids, start a fresh instance on next use.
  if (Is-Word $f) { foreach ($id in $script:wordPids) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }; $script:word = $null; $script:wordBefore = Get-Pids 'WINWORD' }
  else { foreach ($id in $script:excelPids) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }; $script:excel = $null; $script:excelBefore = Get-Pids 'EXCEL' }
  Start-Sleep -Seconds 1
}

function Test-File($f) {
  $doc = $null; $wb = $null
  try {
    if (Is-Word $f) {
      $doc = Open-Doc $f
      if ($doc.FullName -ne $f) { return "opened document is not the original file ($($doc.FullName))" }
      if (-not $doc.ReadOnly) { return 'document did not open read-only' }
    } else {
      $wb = Open-Book $f
      if ($wb.Name -like '*[[]Repaired[]]*') { return "workbook reported repair ($($wb.Name))" }
    }
    return $null
  } catch { return $_.Exception.Message.Trim() }
  finally {
    if ($doc) { try { $doc.Close(0) } catch {}; Release $doc }
    if ($wb) { try { $wb.Close($false) } catch {}; Release $wb }
  }
}

function Ref-Docx($f) {
  $name = [IO.Path]::GetFileNameWithoutExtension($f)
  $out = if ($OutDir) { Join-Path $OutDir $name } else { Join-Path $refRoot "docx\$name" }; New-Item -ItemType Directory -Force $out | Out-Null
  $doc = $null
  try {
    $doc = Open-Doc $f
    $pdf = Join-Path $out "$name.pdf"
    $doc.ExportAsFixedFormat($pdf, 17)
    $doc.Repaginate()
    $n = [int]$doc.ComputeStatistics(2)
    # best effort: first paragraph (up to 160 chars) that starts on each page; paragraphs capped at 5000
    $firsts = @{}; $paras = $doc.Paragraphs; $pc = [math]::Min([int]$paras.Count, 5000)
    for ($k = 1; $k -le $pc -and $firsts.Count -lt $n; $k++) {
      try {
        $pr = $paras.Item($k).Range
        $pg = [int]$pr.Information(3)
        if (-not $firsts.ContainsKey($pg)) {
          $t = ($pr.Text -replace '[

]+', ' ').Trim()
          $firsts[$pg] = $t.Substring(0, [math]::Min(160, $t.Length))
        }
        Release $pr
      } catch { Write-Verbose $_.Exception.Message }
    }
    Release $paras
    $pages = @()
    for ($i = 1; $i -le $n; $i++) { $pages += [ordered]@{ page = $i; firstLine = $(if ($firsts.ContainsKey($i)) { $firsts[$i] } else { $null }) } }
    Write-Json ([ordered]@{ source = (Rel $f); pageCount = $n; pages = $pages }) (Join-Path $out 'pages.json')
    $produced.Add("docx/$name (pdf, pages.json)")
  } finally { if ($doc) { try { $doc.Close(0) } catch {}; Release $doc } }
}

function Ref-Xlsx($f) {
  $name = [IO.Path]::GetFileNameWithoutExtension($f)
  $out = if ($OutDir) { Join-Path $OutDir $name } else { Join-Path $refRoot "xlsx\$name" }; New-Item -ItemType Directory -Force $out | Out-Null
  $wb = $null
  try {
    $wb = Open-Book $f
    $script:excel.CalculateFull()
    $sheets = @(); $totalPages = 0; $pagesOk = $true
    foreach ($ws in $wb.Worksheets) {
      $ur = $ws.UsedRange
      $addr = $ur.Address($false, $false)
      $r0 = $ur.Row; $c0 = $ur.Column
      $v = $ur.Value2; $fm = $ur.Formula
      $one = ($v -isnot [array])
      $cells = [ordered]@{}; $count = 0
      $rows = if ($one) { 1 } else { $v.GetLength(0) }
      $cols = if ($one) { 1 } else { $v.GetLength(1) }
      for ($r = 1; $r -le $rows -and $count -lt 2000; $r++) {
        for ($c = 1; $c -le $cols; $c++) {
          $val = if ($one) { $v } else { $v[$r, $c] }
          if ($null -eq $val -or ($val -is [string] -and $val -eq '')) { continue }
          $ff = if ($one) { $fm } else { $fm[$r, $c] }
          $a = (Col-Letter ($c0 + $c - 1)) + ($r0 + $r - 1)
          $text = $ws.Range($a).Text
          $cells[$a] = [ordered]@{ value2 = $val; text = $text; formula = $(if ($ff -is [string] -and $ff.StartsWith('=')) { $ff } else { $null }) }
          $count++
          if ($count -ge 2000) { break }
        }
      }
      $sheets += [ordered]@{ name = $ws.Name; visible = [int]$ws.Visible; usedRange = $addr; truncatedAt2000 = ($count -ge 2000); cells = $cells }
      if ($XlsxPdf -and $pagesOk) { try { $totalPages += [int]$ws.PageSetup.Pages.Count } catch { $pagesOk = $false } }
      Release $ur; Release $ws
    }
    Write-Json ([ordered]@{ source = (Rel $f); sheets = $sheets }) (Join-Path $out 'values.json')
    $what = 'values.json'
    if (-not $XlsxPdf) { $what += ', pdf not requested (-XlsxPdf)' }
    elseif ($pagesOk -and $totalPages -lt 50) {
      $wb.ExportAsFixedFormat(0, (Join-Path $out "$name.pdf")); $what += ', pdf'
    } else { $what += ", pdf skipped (pages: $(if ($pagesOk) { $totalPages } else { 'unknown' }))" }
    $produced.Add("xlsx/$name ($what)")
  } finally { if ($wb) { try { $wb.Close($false) } catch {}; Release $wb } }
}

try {
  foreach ($f in $files) {
    $rel = Rel $f
    if ($f -notmatch '\.(docx|dotx|dotm|docm|xlsx|xltx|xltm|xlsm)$') { $skipped.Add("$rel : unsupported extension"); continue }
    if ($Mode -eq 'verify') {
      $reason = Test-File $f
      if ($reason) { Write-Output "FAIL ${rel}: $reason"; $failed++ } else { Write-Output "PASS $rel" }
    } else {
      $done = $false
      for ($attempt = 1; $attempt -le 2 -and -not $done; $attempt++) {
        try { if (Is-Word $f) { Ref-Docx $f } else { Ref-Xlsx $f }; $done = $true }
        catch {
          $msg = $_.Exception.Message.Trim()
          if ($attempt -eq 1 -and $msg -match '0x800706|0x80010108|RPC') { Reset-App $f }
          else { $skipped.Add("$rel : $msg"); $done = $true }
        }
      }
    }
  }
} finally {
  if ($word) { try { $word.Quit(0) } catch {}; Release $word }
  if ($excel) { try { $excel.Quit() } catch {}; Release $excel }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  Start-Sleep -Milliseconds 500
  foreach ($id in @($wordPids) + @($excelPids)) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
}
if ($Mode -eq 'reference') {
  foreach ($p in $produced) { Write-Output "OK $p" }
  foreach ($s in $skipped) { Write-Output "SKIPPED $s" }
  Write-Output "reference done: $($produced.Count) produced, $($skipped.Count) skipped"
}
exit $(if ($failed -gt 0) { 1 } else { 0 })
