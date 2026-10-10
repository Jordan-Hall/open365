<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for Open365's locale names (ADR-011).

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script
is never run in CI. CI consumes the committed output (fixtures/reference/locale/excel-names.json) through
crates/locale/tests/excel_names.rs.

For every LCID in crates/locale/src/lcid_table.rs (the [MS-LCID] list) it asks Excel to format fixed dates and
times with a "[$-LCID]" prefix, which makes TEXT() use that locale's names WITHOUT touching the machine's regional
settings or culture:

  =TEXT(DATE(2024,m,1),"[$-407]mmmm")      full month name, m = 1..12
  =TEXT(DATE(2024,m,1),"[$-407]mmm")       abbreviated month name
  =TEXT(DATE(2024,m,1),"[$-407]d mmmm")    month name next to a day number (genitive forms), day 1 stripped
  =TEXT(DATE(2024,3,3+i),"[$-407]dddd")    full day name, i = 0..6 (2024-03-03 is a Sunday)
  =TEXT(DATE(2024,3,3+i),"[$-407]ddd")     abbreviated day name
  =TEXT(TIME(h,0,0),"[$-407]AM/PM")        AM (h = 9) and PM (h = 15) designators

Cells that Excel cannot evaluate are recorded as null. Formulas go in through Range.Formula (always en-US syntax).

Process safety: this script starts its own hidden Excel instance, remembers its process id and only ever stops that
process (Quit in a finally block). Other EXCEL/WINWORD processes are never touched.

  pwsh -NoProfile -File tools/office/locale-oracle.ps1
  pwsh -NoProfile -File tools/office/locale-oracle.ps1 -Only '^(1031|1049)$' -Output $env:TEMP\partial.json

Output is stable: LCID order, LF line endings, ASCII only (non-ASCII is \uXXXX escaped), no paths, no user data.
#>
param(
  [string]$Output,
  [string]$Only   # regex over decimal LCIDs; requires -Output so the committed file is never half-written
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$defaultOutput = Join-Path $root 'fixtures/reference/locale/excel-names.json'
if ($Only -and -not $Output) { throw '-Only needs an explicit -Output' }
if (-not $Output) { $Output = $defaultOutput }

Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@

# ---- LCID list: the crate's own table, so the oracle and the tests cannot drift ------------------------------
$tablePath = Join-Path $root 'crates/locale/src/lcid_table.rs'
$entries = New-Object System.Collections.Generic.List[object]
$seen = @{}
foreach ($line in Get-Content -LiteralPath $tablePath) {
  if ($line -match '^\s*\(0x([0-9A-Fa-f]+),\s*"([^"]+)"\),') {
    $lcid = [Convert]::ToInt32($Matches[1], 16)
    if (-not $seen.ContainsKey($lcid)) {
      $seen[$lcid] = $true
      $entries.Add([pscustomobject]@{ lcid = $lcid; tag = $Matches[2] })
    }
  }
}
if ($Only) { $entries = @($entries | Where-Object { "$($_.lcid)" -match $Only }) }
if ($entries.Count -eq 0) { throw 'no LCIDs selected' }

# ---- Excel lifecycle (own process only) ---------------------------------------------------------------------
$script:xl = $null
$script:xlPid = 0
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
    # Only the process this script started; the id was taken from our own window handle.
    if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $script:xlPid -Force -ErrorAction SilentlyContinue }
  }
  $script:xlPid = 0
}

# ---- JSON writing (manual, for stable ASCII output) ---------------------------------------------------------
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
function Json-Arr($items) { return '[' + (($items | ForEach-Object { Json-Str $_ }) -join ', ') + ']' }

# Cell layout per LCID (column A, 64 rows): 12 mmmm, 12 mmm, 12 "d mmmm", 12 "d mmm", 7 dddd, 7 ddd, AM, PM.
$layout = @('mmmm', 'mmm', 'd mmmm', 'd mmm')
function Build-Formulas([int]$lcid) {
  $hex = '{0:X}' -f $lcid
  $f = New-Object 'object[,]' 64, 1
  $r = 0
  foreach ($code in $layout) {
    for ($m = 1; $m -le 12; $m++) { $f[$r, 0] = "=TEXT(DATE(2024,$m,1),`"[`$-$hex]$code`")"; $r++ }
  }
  foreach ($code in @('dddd', 'ddd')) {
    for ($i = 0; $i -lt 7; $i++) { $f[$r, 0] = "=TEXT(DATE(2024,3,$(3 + $i)),`"[`$-$hex]$code`")"; $r++ }
  }
  $f[$r, 0] = "=TEXT(TIME(9,0,0),`"[`$-$hex]AM/PM`")"; $r++
  $f[$r, 0] = "=TEXT(TIME(15,0,0),`"[`$-$hex]AM/PM`")"; $r++
  return , $f
}
function Cell-Text($v) {
  # Error values arrive as Int32 HRESULT-style codes; text arrives as string.
  if ($v -is [string]) { return $v }
  return $null
}

$records = New-Object System.Collections.Generic.List[string]
$excelVersion = ''
$excelBuild = ''
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
try {
  Start-Excel
  $excelVersion = [string]$script:xl.Version
  $excelBuild = [string]$script:xl.Build
  $wb = $script:xl.Workbooks.Add()
  $ws = $wb.Worksheets.Item(1)
  $range = $ws.Range('A1:A64')
  $range.NumberFormat = 'General'
  foreach ($e in $entries) {
    $range.Formula = (Build-Formulas $e.lcid)
    $script:xl.Calculate()
    $v = $range.Value2
    $col = @(for ($i = 1; $i -le 64; $i++) { Cell-Text $v[$i, 1] })
    $monthFull = $col[0..11]; $monthAbbr = $col[12..23]
    $dayMonthFull = $col[24..35]; $dayMonthAbbr = $col[36..47]
    $dayFull = $col[48..54]; $dayAbbr = $col[55..61]
    $am = $col[62]; $pm = $col[63]
    $line = '    {"lcid": ' + $e.lcid + ', "tag": ' + (Json-Str $e.tag) +
      ', "month_full": ' + (Json-Arr $monthFull) +
      ', "month_abbr": ' + (Json-Arr $monthAbbr) +
      ', "month_full_with_day": ' + (Json-Arr $dayMonthFull) +
      ', "month_abbr_with_day": ' + (Json-Arr $dayMonthAbbr) +
      ', "day_full": ' + (Json-Arr $dayFull) +
      ', "day_abbr": ' + (Json-Arr $dayAbbr) +
      ', "am": ' + (Json-Str $am) + ', "pm": ' + (Json-Str $pm) + '}'
    $records.Add($line)
  }
}
finally {
  Stop-Excel
}

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('{')
$lines.Add('  "format": 1,')
$lines.Add('  "oracle": "Microsoft Excel via COM, TEXT() with [$-LCID] prefixes; dates 2024-03-03 (Sunday) and 2024-m-1, times 09:00 and 15:00",')
$lines.Add('  "excel": {"version": ' + (Json-Str $excelVersion) + ', "build": ' + (Json-Str $excelBuild) + '},')
$lines.Add('  "locales": [')
for ($i = 0; $i -lt $records.Count; $i++) { $lines.Add($records[$i] + $(if ($i + 1 -lt $records.Count) { ',' } else { '' })) }
$lines.Add('  ]')
$lines.Add('}')
$outDir = Split-Path -Parent $Output
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) { New-Item -ItemType Directory -Force -Path $outDir | Out-Null }
[IO.File]::WriteAllText($Output, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
Write-Host ("locale-oracle: wrote {0} locales in {1:n0}s" -f $records.Count, $stopwatch.Elapsed.TotalSeconds)
