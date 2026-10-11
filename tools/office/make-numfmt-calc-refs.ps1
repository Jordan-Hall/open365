<#
Measures Excel's number formatting of Thai, Buddhist, Hijri, CJK-numeral and locale-tagged codes, and Excel's function
argument-count checks, for the Open365 formula engine (crates/calc).

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is never run
in CI. CI consumes the committed output through crates/calc/tests/numfmt_*.rs and crates/calc/tests/arity*.rs.

Modes
  -Mode Numfmt  (default) for every case of fixtures/calc/numfmt-nonlatin/cases.json
                  {"id", "value": number | "text": string, "code": format code, "d1904": true?}
                writes fixtures/calc/numfmt-nonlatin/results.json
                  {"id", "tfn": TEXT(value, code) result or error text, "text": Range.Text of a cell given NumberFormat = code,
                   "rejected": true when Excel refuses the code}
                The code carries an explicit [$-xxx] tag whenever the answer must not depend on the machine's locale; a case
                without a tag depends on the machine (en-GB on the reference machine). Nothing here changes Windows regional
                settings or any Office setting.
  -Mode Arity   for every function name of fixtures/calc/arity/names.txt (the registry's function_names(), one per line) enter
                =NAME(A1,A1,...) with 0 to 40 arguments (41 to 260 when 38 to 40 include an accepted count) through Range.Formula and
                record which counts Excel accepts (assignment succeeds) and which it refuses (COM error).
                writes fixtures/calc/arity/excel-arity.tsv (`NAME<TAB>min<TAB>max<TAB>step[<TAB>extra]`, step 2 when only every other
                count is accepted, extra one more accepted count below min; `NAME<TAB>irregular<TAB>list` otherwise) and crates/calc/src/parser_arity.rs (the table the parser checks).
                LET and LAMBDA get name arguments where Excel requires them.

Process safety: starts its own hidden Excel instance and aborts if COM attached to one that was already running.
AutomationSecurity is forced to 3 (macros disabled). Excel is quit in a finally block; only the process this script started
(its pid is taken from our own window handle) is ever stopped.

  pwsh -NoProfile -File tools/office/make-numfmt-calc-refs.ps1
  pwsh -NoProfile -File tools/office/make-numfmt-calc-refs.ps1 -Cases $env:TEMP\probe.json -Output $env:TEMP\probe-out.json
  pwsh -NoProfile -File tools/office/make-numfmt-calc-refs.ps1 -Mode Arity
#>
param(
  [ValidateSet('Numfmt', 'Arity')][string]$Mode = 'Numfmt',
  [string]$Cases,
  [string]$Output,
  [string]$RustOutput,
  [int]$BatchTimeoutSeconds = 3000
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if ($Mode -eq 'Numfmt') {
  if (-not $Cases) { $Cases = Join-Path $root 'fixtures/calc/numfmt-nonlatin/cases.json' }
  if (-not $Output) { $Output = Join-Path $root 'fixtures/calc/numfmt-nonlatin/results.json' }
} else {
  if (-not $Cases) { $Cases = Join-Path $root 'fixtures/calc/arity/names.txt' }
  if (-not $Output) { $Output = Join-Path $root 'fixtures/calc/arity/excel-arity.tsv' }
}

Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@
if (-not ('NumfmtRefsWatchdog' -as [type])) {
  Add-Type -TypeDefinition @'
public static class NumfmtRefsWatchdog {
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
  public static void Stop() { if (timer != null) { timer.Dispose(); timer = null; } }
}
'@
}

$inv = [Globalization.CultureInfo]::InvariantCulture
$script:xl = $null
$script:xlPid = 0
$script:wb = $null
$script:ws = $null
$script:d1904 = $false

function New-Book([bool]$d1904) {
  $wb = $script:xl.Workbooks.Add()
  if ($d1904) { $wb.Date1904 = $true }
  $ws = $wb.Worksheets.Item(1)
  $ws.StandardWidth = 60
  $ws.Columns.Item(1).ColumnWidth = 60
  $ws.Columns.Item(2).ColumnWidth = 60
  $ws.Columns.Item(3).ColumnWidth = 60
  $script:wb = $wb; $script:ws = $ws; $script:d1904 = $d1904
}

function Reset-Book([bool]$d1904) {
  if ($script:wb) { try { $script:wb.Saved = $true; $script:wb.Close($false) } catch {} }
  New-Book $d1904
}

function Start-Excel {
  $before = @(Get-Process -Name EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $app = New-Object -ComObject Excel.Application
  $attached = [uint32]0
  [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$attached)
  if ($before -contains [int]$attached) {
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
    throw "make-numfmt-calc-refs: COM attached to the running Excel process $attached; refusing to drive an instance this script did not start"
  }
  $app.Visible = $false
  $app.DisplayAlerts = $false
  $app.ScreenUpdating = $false
  $app.EnableEvents = $false
  $app.AutomationSecurity = 3
  $script:xl = $app
  $script:xlPid = [int]$attached
  New-Book $false
  $app.Calculation = -4105
  [NumfmtRefsWatchdog]::Start($script:xlPid, $BatchTimeoutSeconds)
}

function Stop-Excel {
  [NumfmtRefsWatchdog]::Stop()
  $app = $script:xl
  $script:xl = $null
  if ($app) {
    if ($script:wb) { try { $script:wb.Saved = $true; $script:wb.Close($false) } catch {} }
    try { $app.Quit() } catch {}
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
  }
  $script:wb = $null; $script:ws = $null
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  if ($script:xlPid -gt 0) {
    $deadline = (Get-Date).AddSeconds(8)
    while ((Get-Date) -lt $deadline -and (Get-Process -Id $script:xlPid -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
    $p = Get-Process -Id $script:xlPid -ErrorAction SilentlyContinue
    if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $script:xlPid -Force -ErrorAction SilentlyContinue }
  }
  $script:xlPid = 0
}

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
    elseif ($code -lt 0x20 -or $code -gt 0x7e) { [void]$sb.AppendFormat('\u{0:x4}', $code) }
    else { [void]$sb.Append($ch) }
  }
  [void]$sb.Append('"')
  return $sb.ToString()
}

$errorText = @{
  -2146826288 = '#NULL!'; -2146826281 = '#DIV/0!'; -2146826273 = '#VALUE!'; -2146826265 = '#REF!'
  -2146826259 = '#NAME?'; -2146826252 = '#NUM!'; -2146826246 = '#N/A'
}

function Measure-Numfmt($case) {
  $ws = $script:ws
  $want1904 = [bool]($case.PSObject.Properties['d1904'] -and $case.d1904)
  if ($want1904 -ne $script:d1904) { Reset-Book $want1904; $ws = $script:ws }
  $ws.Cells.Clear() | Out-Null
  $a = $ws.Range('A1'); $b = $ws.Range('B1'); $c = $ws.Range('C1')
  $a.NumberFormat = 'General'
  if ($case.PSObject.Properties['text']) { $a.NumberFormat = '@'; $a.Value2 = [string]$case.text }
  else { $a.Value2 = [double]$case.value }
  $code = [string]$case.code
  $result = [ordered]@{ id = $case.id }
  try {
    $b.Formula = '=TEXT(A1,"' + $code.Replace('"', '""') + '")'
    $v = $b.Value2
    if ($v -is [int]) { $result.tfn = $errorText[[int]$v]; $result.tfn_error = $true }
    else { $result.tfn = [string]$v }
  } catch { $result.tfn = $null; $result.tfn_rejected = $true }
  try {
    $c.Formula = '=A1'
    $c.NumberFormat = $code
    $result.text = [string]$c.Text
    $result.nf = [string]$c.NumberFormat
  } catch { $result.text = $null; $result.rejected = $true }
  return $result
}

function Test-Arity([string]$name, [int]$n) {
  $callArgs = @()
  for ($i = 0; $i -lt $n; $i++) {
    if ($name -eq 'LET' -and ($i % 2) -eq 0 -and $i -lt ($n - 1)) { $callArgs += "x$i" }
    elseif ($name -eq 'LAMBDA' -and $i -lt ($n - 1)) { $callArgs += "p$i" }
    else { $callArgs += 'A1' }
  }
  $f = '=' + $name + '(' + ($callArgs -join ',') + ')'
  try { $script:ws.Range('C3').Formula = $f; return $true } catch { return $false }
}

function Measure-Arity([string]$name) {
  $ok = New-Object System.Collections.Generic.List[int]
  for ($n = 0; $n -le 40; $n++) { if (Test-Arity $name $n) { $ok.Add($n) } }
  $tail = @($ok | Where-Object { $_ -ge 38 })
  if ($tail.Count -ge 1) {
    for ($n = 41; $n -le 260; $n++) { if (Test-Arity $name $n) { $ok.Add($n) } }
  }
  $script:ws.Range('C3').ClearContents() | Out-Null
  if ($ok.Count -eq 0) { return @{ line = "$name`tnone`t"; rust = $null } }
  $min = $ok[0]; $max = $ok[$ok.Count - 1]
  $step = 1
  if ($ok.Count -ge 2) { $step = $ok[1] - $ok[0] }
  $regular = ($ok.Count -eq ((($max - $min) / $step) + 1)) -and ($step -in 1, 2)
  if ($regular) { for ($i = 1; $i -lt $ok.Count; $i++) { if ($ok[$i] - $ok[$i - 1] -ne $step) { $regular = $false } } }
  if (-not $regular -and $ok.Count -ge 3) {
    # One count below an otherwise regular run (SORTBY: 2, then 3, 5, 7, ...): table row for the run plus an extra count.
    $rest = @($ok | Select-Object -Skip 1)
    $step2 = $rest[1] - $rest[0]
    $regular2 = $step2 -in 1, 2
    for ($i = 1; $i -lt $rest.Count; $i++) { if ($rest[$i] - $rest[$i - 1] -ne $step2) { $regular2 = $false } }
    if ($regular2) {
      $extra = $ok[0]; $min2 = $rest[0]
      return @{ line = "$name`t$min2`t$max`t$step2`t$extra"; rust = "    (`"$name`", $min2, $max, $step2),"; extra = "    (`"$name`", $extra)," }
    }
  }
  if (-not $regular) { return @{ line = "$name`tirregular`t" + ($ok -join ','); rust = $null } }
  return @{ line = "$name`t$min`t$max`t$step"; rust = "    (`"$name`", $min, $max, $step)," }
}

try {
  Start-Excel
  $out = New-Object System.Collections.Generic.List[string]
  $count = 0
  if ($Mode -eq 'Numfmt') {
    $list = Get-Content -LiteralPath $Cases -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($case in $list) {
      $count++
      if ($count % 100 -eq 0) { Reset-Book $script:d1904 }
      $r = Measure-Numfmt $case
      $fields = @()
      foreach ($k in $r.Keys) {
        $v = $r[$k]
        if ($null -eq $v) { $fields += ('"' + $k + '":null') }
        elseif ($v -is [bool]) { $fields += ('"' + $k + '":' + $(if ($v) { 'true' } else { 'false' })) }
        else { $fields += ('"' + $k + '":' + (Json-Str ([string]$v))) }
      }
      $out.Add('  {' + ($fields -join ',') + '}')
    }
    $text = "[`n" + ($out -join ",`n") + "`n]`n"
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Output) | Out-Null
    [IO.File]::WriteAllText($Output, $text, (New-Object Text.UTF8Encoding($false)))
    Write-Host "wrote $($out.Count) records to $Output"
  } else {
    $tsv = New-Object System.Collections.Generic.List[string]
    $rust = New-Object System.Collections.Generic.List[string]
    $extras = New-Object System.Collections.Generic.List[string]
    foreach ($name in (Get-Content -LiteralPath $Cases -Encoding UTF8)) {
      $name = $name.Trim()
      if (-not $name -or $name.StartsWith('#')) { continue }
      $count++
      if ($count % 40 -eq 0) { Reset-Book $false }
      $r = Measure-Arity $name
      $tsv.Add($r.line)
      if ($r.rust) { $rust.Add($r.rust) }
      if ($r.extra) { $extras.Add($r.extra) }
    }
    $text = ($tsv -join "`n") + "`n"
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Output) | Out-Null
    [IO.File]::WriteAllText($Output, $text, (New-Object Text.UTF8Encoding($false)))
    $rustPath = $RustOutput
    if (-not $rustPath) { $rustPath = Join-Path $root 'crates/calc/src/parser_arity.rs' }
    $headLines = @(
      '//! GENERATED by tools/office/make-numfmt-calc-refs.ps1 -Mode Arity from Excel''s own behaviour: the argument counts Excel',
      '//! accepts when a formula is entered (`Range.Formula`), for every function of the registry (fixtures/calc/arity/names.txt).',
      '//! Do not edit.',
      '',
      '/// `(NAME, min, max, step)`, sorted by name. `step` 2 means only `min`, `min + 2`, ... are accepted (LET, IFS, SWITCH).',
      '#[rustfmt::skip]',
      'pub(crate) static ARITY: &[(&str, u8, u8, u8)] = &[')
    $tailLines = @('];', '', '/// One more accepted count below `min` (SORTBY takes 2, then 3, 5, 7, ...).', 'pub(crate) static EXTRA: &[(&str, u8)] = &[')
    $head = ($headLines -join "`n") + "`n"
    [IO.File]::WriteAllText($rustPath, $head + ($rust -join "`n") + "`n" + ($tailLines -join "`n") + "`n" + ($extras -join "`n") + "`n];`n", (New-Object Text.UTF8Encoding($false)))
    Write-Host "wrote $($tsv.Count) functions to $Output and $rustPath"
    return
  }
}
finally {
  Stop-Excel
}
