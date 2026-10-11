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
  pwsh -NoProfile -File tools/office/calc-oracle.ps1 -Update                   # only cases without a stored result
  pwsh -NoProfile -File tools/office/calc-oracle.ps1 -Update -Only '^(sin|cos)\.'   # rerun these and keep the rest

A case with "format" gets that number format on the formula cell (applied after the formula, so a text format
cannot turn the formula into text); the recorded "text" is Range.Text, which the numfmt conformance group checks.
A case with "locale": "en-GB" depends on the machine's system locale (date order, currency symbol, date text); the
script refuses to run it on a machine whose culture is different, because the stored answer would describe another
locale. Cases without "locale" must give the same answer in every locale. A format code Excel refuses is recorded as kind "rejected" (hresult "NumberFormat"). Excel keeps every distinct custom
format in the workbook and fails after roughly 250, so the script swaps in a fresh workbook every 100 formats.
Both workbooks use a 60-character-wide column, so Range.Text of "####" overflow, "*x" fill and the like reflects that
width.

Output is stable: corpus order, LF line endings, ASCII only, no absolute paths and no user information.
#>
param(
  [int]$BatchTimeoutSeconds = 600,   # watchdog: one batch (the whole run, or one -Update chunk) may take this long
  [string]$Corpus,
  [string]$Output,
  [string]$Only,        # regex over case ids; requires -Output so the committed file is never half-written
  [int]$Limit = 0,      # first N cases only (debugging); requires -Output
  [switch]$Update       # merge into the existing results file: run only missing cases (and those matching -Only)
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Corpus) { $Corpus = Join-Path $root 'fixtures/calc/conformance.json' }
$defaultOutput = Join-Path $root 'fixtures/reference/calc/conformance-results.json'
if (($Only -or $Limit -gt 0) -and -not $Output -and -not $Update) { throw '-Only and -Limit need an explicit -Output' }
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

function New-Book($app, [string]$name) {
  $wb = $app.Workbooks.Add()
  if ($name -eq 'd1904') { $wb.Date1904 = $true }
  while ($wb.Worksheets.Count -lt 2) { [void]$wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
  $s1 = $wb.Worksheets.Item(1); $s1.Name = 'Sheet1'
  $s2 = $wb.Worksheets.Item(2); $s2.Name = 'Sheet2'
  $s1.StandardWidth = 60   # the standard width, so CELL("width") reports the second element TRUE (not a custom width)
  $s2.StandardWidth = 60
  return @{ wb = $wb; s1 = $s1; s2 = $s2; formats = @{}; widthDirty = $null }
}

# Excel keeps every distinct custom number format in the workbook and refuses new ones after roughly 250, so a
# workbook that has seen many formats is replaced by a fresh one.
function Reset-Book([string]$name) {
  $old = $script:books[$name]
  try { $old.wb.Saved = $true; $old.wb.Close($false) } catch {}
  $script:books[$name] = New-Book $script:xl $name
}

function Start-Excel {
  # Only an instance this script created may be driven (and later quit): snapshot the running Excel processes and, if the
  # new COM object is one of them (it attached to someone else's Excel), release it untouched and abort.
  $before = @(Get-Process -Name EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $app = New-Object -ComObject Excel.Application
  $attachedPid = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$attachedPid)
  if ($before -contains [int]$attachedPid) {
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
    throw "calc-oracle: New-Object attached to the running Excel process $attachedPid; refusing to drive an instance this script did not start"
  }
  $app.Visible = $false
  $app.DisplayAlerts = $false
  $app.ScreenUpdating = $false
  $app.EnableEvents = $false
  $app.AutomationSecurity = 3   # msoAutomationSecurityForceDisable: no macros or add-in code run in the oracle instance
  $procId = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
  $script:xl = $app
  $script:xlPid = [int]$procId
  $script:books = @{}
  foreach ($name in 'd1900', 'd1904') { $script:books[$name] = New-Book $app $name }
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

# Watchdog: a .NET timer that kills ONLY the Excel process this script started (same pid and name check as
# Stop-Excel) when the batch exceeds its timeout, so a hung Excel (for example a 1E+300 shape parameter) cannot
# block the run forever. It is compiled C#: a PowerShell Register-ObjectEvent action cannot run while the main
# thread is blocked inside a COM call, which is exactly when the watchdog is needed.
if (-not ('CalcOracleWatchdog' -as [type])) {
  Add-Type -TypeDefinition @'
public static class CalcOracleWatchdog {
  static System.Threading.Timer timer;
  public static void Start(int pid, int seconds) {
    Stop();
    timer = new System.Threading.Timer(delegate(object state) {
      try {
        var p = System.Diagnostics.Process.GetProcessById(pid);
        if (p.ProcessName == "EXCEL") { p.Kill(); }
      } catch (System.Exception) { }
    }, null, seconds * 1000, System.Threading.Timeout.Infinite);
  }
  public static void Stop() {
    if (timer != null) { timer.Dispose(); timer = null; }
  }
}
'@
}
function Start-Watchdog([int]$seconds) {
  [CalcOracleWatchdog]::Start([int]$script:xlPid, $seconds)
}
function Stop-Watchdog {
  [CalcOracleWatchdog]::Stop()
}

function Restart-Excel {
  $script:restarts++
  Write-Host "  restarting Excel instance (restart $($script:restarts))"
  try { Stop-Excel } catch {}
  Start-Sleep -Milliseconds 500
  Start-Excel
  Start-Watchdog $BatchTimeoutSeconds   # the replacement has a new pid
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

# Cell styles for the information functions (CELL): "nf" number format, "ha" horizontal alignment, "locked",
# "width" column width of the cell's column (the other columns keep the 60-character default).
$script:hAlign = @{ general = 1; left = -4131; right = -4152; center = -4108; fill = 5; justify = -4130; across = 7; distributed = -4117 }
function Set-Style($ws, [string]$addr, $style, $book) {
  $r = $ws.Range($addr)
  if ($style.ContainsKey('nf')) { $r.NumberFormat = $style['nf']; $book.formats[$style['nf']] = $true }
  if ($style.ContainsKey('ha')) { $r.HorizontalAlignment = $script:hAlign[$style['ha']] }
  if ($style.ContainsKey('locked')) { $r.Locked = [bool]$style['locked'] }
  if ($style.ContainsKey('width')) {
    $r.ColumnWidth = [double]$style['width']
    if (-not $book.widthDirty) { $book.widthDirty = New-Object System.Collections.Generic.List[int] }
    $book.widthDirty.Add([int]$r.Column)
  }
}

function Invoke-Case($case) {
  $bookName = if ($case.date1904) { 'd1904' } else { 'd1900' }
  if ($case.format -and -not $script:books[$bookName].formats.ContainsKey($case.format) -and $script:books[$bookName].formats.Count -ge 100) { Reset-Book $bookName }
  if ($case.styles -and $script:books[$bookName].formats.Count -ge 100) { Reset-Book $bookName }
  $book = $script:books[$bookName]
  if ($case.format) { $book.formats[$case.format] = $true }
  $s1 = $book.s1; $s2 = $book.s2
  $s1.Cells.Clear() | Out-Null
  $s2.Cells.Clear() | Out-Null
  if ($book.widthDirty) { foreach ($c in $book.widthDirty) { $s1.Columns.Item($c).UseStandardWidth = $true }; $book.widthDirty = $null }
  if ($case.cells) { foreach ($k in $case.cells.Keys) { Set-Cell $s1 $k $case.cells[$k] } }
  if ($case.sheet2) { foreach ($k in $case.sheet2.Keys) { Set-Cell $s2 $k $case.sheet2[$k] } }
  if ($case.styles) { foreach ($k in $case.styles.Keys) { Set-Style $s1 $k $case.styles[$k] $book } }
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
  if ($case.format) {
    try { $target.NumberFormat = $case.format }
    catch {
      # An invalid format code is refused like an invalid formula.
      if ($_.Exception.Message -match 'NumberFormat property') { return @{ kind = 'rejected'; hresult = 'NumberFormat' } }
      throw
    }
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
$allCases = @($doc.cases)
$cases = $allCases
# -Update: keep the stored record of every case that is not selected (record lines are keyed by their id).
$stored = New-Object 'System.Collections.Generic.Dictionary[string,string]' ([StringComparer]::Ordinal)   # ids differ by case, so no plain hashtable
if ($Update) {
  if (Test-Path -LiteralPath $Output) {
    foreach ($line in [IO.File]::ReadAllLines($Output)) {
      if ($line -match '^    \{"id": ("(?:[^"\\]|\\.)*")') { $stored[$Matches[1]] = $line.TrimEnd(',') }
    }
  }
  $cases = @($allCases | Where-Object { -not $stored.ContainsKey((Json-Str $_.id)) -or ($Only -and $_.id -match $Only) })
}
elseif ($Only) { $cases = @($cases | Where-Object { $_.id -match $Only }) }
if ($Limit -gt 0) { $cases = @($cases | Select-Object -First $Limit) }
Write-Host "calc-oracle: $($cases.Count) cases"

$records = New-Object System.Collections.Generic.List[string]
$failures = 0
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$excelInfo = $null
try {
  Start-Excel
  Start-Watchdog $BatchTimeoutSeconds
  $excelInfo = @{
    version = [string]$script:xl.Version
    build = [string]$script:xl.Build
    decimal = [string]$script:xl.International(3)
    thousands = [string]$script:xl.International(4)
    country = [int]$script:xl.International(1)
    date_order = [int]$script:xl.International(32)    # 0 month-day-year, 1 day-month-year, 2 year-month-day
    list_sep = [string]$script:xl.International(5)
    culture = [string][Globalization.CultureInfo]::CurrentCulture.Name
  }
  $n = 0
  foreach ($case in $cases) {
    $n++
    if ($case.locale -and $case.locale -ne $excelInfo.culture) {
      throw "case $($case.id) needs the $($case.locale) system locale but this machine is $($excelInfo.culture)"
    }
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
  Stop-Watchdog
  try { if ($script:xl) { $script:xl.Quit() } } catch {}   # always Quit, even when the run failed half-way
  Stop-Excel
}

if ($Update) {
  $fresh = New-Object 'System.Collections.Generic.Dictionary[string,string]' ([StringComparer]::Ordinal)
  for ($i = 0; $i -lt $cases.Count; $i++) { $fresh[(Json-Str $cases[$i].id)] = $records[$i] }
  $records = New-Object System.Collections.Generic.List[string]
  foreach ($c in $allCases) {
    $key = Json-Str $c.id
    if ($fresh.ContainsKey($key)) { $records.Add($fresh[$key]) }
    elseif ($stored.ContainsKey($key)) { $records.Add($stored[$key]) }
    else { throw "no result for $($c.id)" }
  }
}
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('{')
$lines.Add('  "format": 1,')
$lines.Add('  "oracle": "Microsoft Excel via COM, Range.Formula (legacy implicit intersection, no spill)",')
$lines.Add('  "excel": {"version": ' + (Json-Str $excelInfo.version) + ', "build": ' + (Json-Str $excelInfo.build) + ', "decimal_separator": ' + (Json-Str $excelInfo.decimal) + ', "thousands_separator": ' + (Json-Str $excelInfo.thousands) + ', "country_code": ' + $excelInfo.country + ', "date_order": ' + $excelInfo.date_order + ', "list_separator": ' + (Json-Str $excelInfo.list_sep) + ', "culture": ' + (Json-Str $excelInfo.culture) + '},')
$lines.Add('  "results": [')
for ($i = 0; $i -lt $records.Count; $i++) { $lines.Add($records[$i] + $(if ($i + 1 -lt $records.Count) { ',' } else { '' })) }
$lines.Add('  ]')
$lines.Add('}')
$outDir = Split-Path -Parent $Output
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) { New-Item -ItemType Directory -Force -Path $outDir | Out-Null }
[IO.File]::WriteAllText($Output, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
Write-Host ("calc-oracle: wrote {0} results in {1:n0}s ({2} restarts, {3} oracle errors)" -f $records.Count, $stopwatch.Elapsed.TotalSeconds, $script:restarts, $failures)
if ($failures -gt 0) { exit 1 }
