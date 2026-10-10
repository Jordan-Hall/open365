<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for the Open365 formula engine.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this
script is never run in CI. CI consumes the committed output (fixtures/reference/calc/conformance-results.json)
through crates/calc/tests/excel_conformance.rs.

For every case in fixtures/calc/conformance.json it writes the input cells into a reused workbook, enters
the formula with Range.Formula (NOT Formula2, so Excel evaluates it with legacy implicit-intersection
semantics and never spills), recalculates and records Value2 (typed), the displayed Text and, for errors,
the error text. Formulas Excel refuses to accept are recorded as kind "rejected".

Two workbooks are used: one in the 1900 date system and one in the 1904 system (cases with "date1904": true).
Each holds Sheet1 (cells + formula) and Sheet2 (optional second sheet). Sheets are cleared between cases.

Process safety: this script starts its own hidden Excel instance, remembers its process id and only ever
stops that process. Other EXCEL/WINWORD processes are never touched. If the instance dies (RPC drop) it is
replaced and the current case is retried.

  pwsh -NoProfile -File tools/office/calc-oracle.ps1
  pwsh -NoProfile -File tools/office/calc-oracle.ps1 -Only '^(round|int)\.' -Output $env:TEMP\partial.json

Output is stable: corpus order, LF line endings, ASCII only, no absolute paths and no user information.
#>
param(
  [string]$Corpus,
  [string]$Output,
  [string]$Only,        # regex over case ids; requires -Output so the committed file is never half-written
  [int]$Limit = 0       # first N cases only (debugging); requires -Output
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Corpus) { $Corpus = Join-Path $root 'fixtures/calc/conformance.json' }
$defaultOutput = Join-Path $root 'fixtures/reference/calc/conformance-results.json'
if (($Only -or $Limit -gt 0) -and -not $Output) { throw '-Only and -Limit need an explicit -Output' }
if (-not $Output) { $Output = $defaultOutput }

Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@

$inv = [Globalization.CultureInfo]::InvariantCulture
$errorText = @{
  -2146826288 = '#NULL!'; -2146826281 = '#DIV/0!'; -2146826273 = '#VALUE!'; -2146826265 = '#REF!'
  -2146826259 = '#NAME?'; -2146826252 = '#NUM!'; -2146826246 = '#N/A'
}

# ---- Excel lifecycle (own process only) -------------------------------------------------------------
$script:xl = $null
$script:xlPid = 0
$script:books = @{}      # 'd1900' / 'd1904' -> @{ wb; s1; s2 }
$script:restarts = 0

function Start-Excel {
  $app = New-Object -ComObject Excel.Application
  $app.Visible = $false
  $app.DisplayAlerts = $false
  $app.ScreenUpdating = $false
  $app.EnableEvents = $false
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  $script:xl = $app
  $script:xlPid = [int]$procId
  $script:books = @{}
  foreach ($name in 'd1900', 'd1904') {
    $wb = $app.Workbooks.Add()
    if ($name -eq 'd1904') { $wb.Date1904 = $true }
    while ($wb.Worksheets.Count -lt 2) { [void]$wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
    $s1 = $wb.Worksheets.Item(1); $s1.Name = 'Sheet1'
    $s2 = $wb.Worksheets.Item(2); $s2.Name = 'Sheet2'
    $s1.Cells.ColumnWidth = 60
    $s2.Cells.ColumnWidth = 60
    $script:books[$name] = @{ wb = $wb; s1 = $s1; s2 = $s2 }
  }
  $app.Calculation = -4105   # xlCalculationAutomatic
}

function Stop-Excel {
  $app = $script:xl
  $script:xl = $null
  if ($app) {
    foreach ($b in $script:books.Values) { try { $b.wb.Saved = $true; $b.wb.Close($false) } catch {} }
    try { $app.Quit() } catch {}
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
  }
  $script:books = @{}
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  if ($script:xlPid -gt 0) {
    $deadline = (Get-Date).AddSeconds(8)
    while ((Get-Date) -lt $deadline -and (Get-Process -Id $script:xlPid -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
    $p = Get-Process -Id $script:xlPid -ErrorAction SilentlyContinue
    # Only the process this script started; the id was taken from our own window handle.
    if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $script:xlPid -Force -ErrorAction SilentlyContinue }
  }
  $script:xlPid = 0
}

function Restart-Excel {
  $script:restarts++
  Write-Host "  restarting Excel instance (restart $($script:restarts))"
  try { Stop-Excel } catch {}
  Start-Sleep -Milliseconds 500
  Start-Excel
}

function Test-Transient($ex) {
  $m = $ex.Exception.Message
  return ($m -match '0x800706BA|0x800706BE|0x800706BF|0x800706B5|0x80010108|0x80010105|0x80010001|0x8001010A|0x80010012|RPC|disconnected|rejected by callee|has been disconnected')
}

# ---- JSON writing (manual, for stable output) -------------------------------------------------------------
# Strings are ASCII-escaped per UTF-16 unit. serde_json rejects lone surrogates, so a lone surrogate becomes
# U+FFFD and the record is flagged "lossy_utf16".
$script:lossy = $false
function Json-Str([string]$s) {
  $sb = New-Object System.Text.StringBuilder
  [void]$sb.Append('"')
  for ($i = 0; $i -lt $s.Length; $i++) {
    $ch = $s[$i]; $code = [int]$ch
    if ($ch -eq '"') { [void]$sb.Append('\"') }
    elseif ($ch -eq '\') { [void]$sb.Append('\\') }
    elseif ($code -eq 10) { [void]$sb.Append('\n') }
    elseif ($code -eq 13) { [void]$sb.Append('\r') }
    elseif ($code -eq 9) { [void]$sb.Append('\t') }
    elseif ($code -lt 0x20 -or $code -gt 0x7e) {
      if ([char]::IsHighSurrogate($ch) -and ($i + 1) -lt $s.Length -and [char]::IsLowSurrogate($s[$i + 1])) {
        [void]$sb.AppendFormat('\u{0:x4}\u{1:x4}', $code, [int]$s[$i + 1]); $i++
      }
      elseif ([char]::IsSurrogate($ch)) { [void]$sb.Append('�'); $script:lossy = $true }
      else { [void]$sb.AppendFormat('\u{0:x4}', $code) }
    }
    else { [void]$sb.Append($ch) }
  }
  [void]$sb.Append('"')
  return $sb.ToString()
}

function Json-Num([double]$d) {
  if ([double]::IsNegative($d) -and $d -eq 0) { return '-0' }
  return $d.ToString('R', $inv)
}

# ---- running one case ---------------------------------------------------------------------------------------
function Set-Cell($ws, [string]$addr, $val) {
  $cell = $ws.Range($addr)
  if ($null -eq $val) { return }
  if ($val -is [string]) { $cell.Value2 = "'" + $val; return }     # leading apostrophe keeps text as text, format stays General
  if ($val -is [bool]) { $cell.Formula = $(if ($val) { '=TRUE' } else { '=FALSE' }); return }   # Value2 = $true fails in the PS COM binder
  if ($val -is [System.Collections.IDictionary]) {
    if ($val.ContainsKey('err')) { $cell.Formula = '=' + $val['err']; return }
    if ($val.ContainsKey('f')) { $cell.Formula = $val['f']; return }
    throw 'unknown cell object'
  }
  $cell.Value2 = [double]$val
}

function Invoke-Case($case) {
  $book = if ($case.date1904) { $script:books['d1904'] } else { $script:books['d1900'] }
  $s1 = $book.s1; $s2 = $book.s2
  $s1.Cells.Clear() | Out-Null
  $s2.Cells.Clear() | Out-Null
  if ($case.cells) { foreach ($k in $case.cells.Keys) { Set-Cell $s1 $k $case.cells[$k] } }
  if ($case.sheet2) { foreach ($k in $case.sheet2.Keys) { Set-Cell $s2 $k $case.sheet2[$k] } }
  $at = if ($case.at) { $case.at } else { $script:defaultAt }
  $target = $s1.Range($at)
  try {
    $target.Formula = $case.formula
  } catch {
    if ($_.Exception.Message -match '0x800A03EC') {
      return @{ kind = 'rejected'; hresult = '0x800A03EC' }
    }
    throw
  }
  $script:xl.Calculate()
  $v = $target.Value2
  $text = [string]$target.Text
  if ($v -is [string]) { return @{ kind = 'text'; value = $v; text = $text } }
  if ($v -is [bool]) { return @{ kind = 'bool'; value = $v; text = $text } }
  if ($v -is [int]) {
    $e = $errorText[$v]
    if (-not $e) { $e = "#UNKNOWN($v)" }
    return @{ kind = 'error'; value = $e; text = $text }
  }
  if ($v -is [double]) { return @{ kind = 'number'; value = $v; text = $text } }
  if ($null -eq $v) { return @{ kind = 'blank'; text = $text } }
  throw "unexpected Value2 type $($v.GetType().FullName)"
}

function Format-Record($id, $r) {
  $script:lossy = $false
  $parts = New-Object System.Collections.Generic.List[string]
  $parts.Add('"id": ' + (Json-Str $id))
  $parts.Add('"kind": "' + $r.kind + '"')
  switch ($r.kind) {
    'number' { $parts.Add('"value": ' + (Json-Num $r.value)) }
    'bool'   { $parts.Add('"value": ' + $(if ($r.value) { 'true' } else { 'false' })) }
    'text'   { $parts.Add('"value": ' + (Json-Str $r.value)) }
    'error'  { $parts.Add('"value": ' + (Json-Str $r.value)) }
    'rejected' { $parts.Add('"hresult": ' + (Json-Str $r.hresult)) }
  }
  if ($r.ContainsKey('text')) { $parts.Add('"text": ' + (Json-Str $r.text)) }
  if ($script:lossy) { $parts.Add('"lossy_utf16": true') }
  return '    {' + ($parts -join ', ') + '}'
}

# ---- main -------------------------------------------------------------------------------------------------------
$doc = Get-Content -LiteralPath $Corpus -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
$script:defaultAt = $doc.default_formula_cell
if ($doc.formula_property -ne 'Formula') { throw 'this script only implements Range.Formula' }
$cases = @($doc.cases)
if ($Only) { $cases = @($cases | Where-Object { $_.id -match $Only }) }
if ($Limit -gt 0) { $cases = @($cases | Select-Object -First $Limit) }
Write-Host "calc-oracle: $($cases.Count) cases"

$records = New-Object System.Collections.Generic.List[string]
$failures = 0
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$excelInfo = $null
try {
  Start-Excel
  $excelInfo = @{
    version = [string]$script:xl.Version
    build = [string]$script:xl.Build
    decimal = [string]$script:xl.International(3)
    thousands = [string]$script:xl.International(4)
    country = [int]$script:xl.International(1)
  }
  $n = 0
  foreach ($case in $cases) {
    $n++
    $result = $null
    for ($attempt = 1; $attempt -le 4 -and $null -eq $result; $attempt++) {
      try {
        $result = Invoke-Case $case
      } catch {
        if (Test-Transient $_) {
          Write-Host "  transient COM error on $($case.id) (attempt $attempt): $($_.Exception.Message.Split("`n")[0])"
          Start-Sleep -Milliseconds 300
          # A busy Excel recovers by itself; a dead one needs replacing.
          $alive = $false
          try { $null = $script:xl.Version; $alive = $true } catch {}
          if (-not $alive -or $attempt -ge 2) { Restart-Excel }
        } else {
          Write-Host "  oracle error on $($case.id): $($_.Exception.Message.Split("`n")[0])"
          $failures++
          $result = @{ kind = 'oracle_error'; text = '' }
        }
      }
    }
    if ($null -eq $result) { $failures++; $result = @{ kind = 'oracle_error'; text = '' } }
    $records.Add((Format-Record $case.id $result))
    if ($n % 250 -eq 0) { Write-Host ("  {0}/{1} ({2:n0}s)" -f $n, $cases.Count, $stopwatch.Elapsed.TotalSeconds) }
  }
}
finally {
  Stop-Excel
}

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('{')
$lines.Add('  "format": 1,')
$lines.Add('  "oracle": "Microsoft Excel via COM, Range.Formula (legacy implicit intersection, no spill)",')
$lines.Add('  "excel": {"version": ' + (Json-Str $excelInfo.version) + ', "build": ' + (Json-Str $excelInfo.build) + ', "decimal_separator": ' + (Json-Str $excelInfo.decimal) + ', "thousands_separator": ' + (Json-Str $excelInfo.thousands) + ', "country_code": ' + $excelInfo.country + '},')
$lines.Add('  "results": [')
for ($i = 0; $i -lt $records.Count; $i++) { $lines.Add($records[$i] + $(if ($i + 1 -lt $records.Count) { ',' } else { '' })) }
$lines.Add('  ]')
$lines.Add('}')
$outDir = Split-Path -Parent $Output
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) { New-Item -ItemType Directory -Force -Path $outDir | Out-Null }
[IO.File]::WriteAllText($Output, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
Write-Host ("calc-oracle: wrote {0} results in {1:n0}s ({2} restarts, {3} oracle errors)" -f $records.Count, $stopwatch.Elapsed.TotalSeconds, $script:restarts, $failures)
if ($failures -gt 0) { exit 1 }
