<#
Records how Excel resolves the built-in number format ids (numFmtId 0..100) for the machine's CURRENT Windows
locale, and the Windows locale data (NLS) that the other locales are derived from.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this is never
run in CI. CI consumes the committed output in fixtures/reference/xlsx/numfmt-locale/ through
crates/xlsx/tests/numfmt_locale*.rs.

  -Measure   Builds a workbook (raw OPC zip, written by this script, no Excel involved) whose cellXfs reference
             numFmtId 0..100 with NO numFmts part, so Excel must resolve the id itself. Opens it read-only in
             Excel's own new instance and records, per id, Range.Text of four probe values, Range.NumberFormat
             and Range.NumberFormatLocal. Output: measured-<tag>.json (tag = the current locale, for example
             en-GB). Only the current locale can be measured: this script never changes Windows regional
             settings or Office settings.
  -Nls       Reads the Windows locale data (CultureInfo with useUserOverride = false, so the user's own
             overrides do not leak in) for every LCID in crates/locale/src/lcid_table.rs and writes
             windows-nls.json plus the Rust table crates/xlsx/src/builtin_numfmt_nls.rs. This is "derived"
             data: it is the data Excel reads, not an Excel measurement. Run it under Windows PowerShell 5.1
             (powershell.exe, .NET Framework = NLS data); pwsh uses ICU data and is refused.

Excel safety: the script starts its own hidden Excel instance and aborts, without touching anything, if COM
attached to an instance that was already running (an existing Excel process owns the Hwnd, or the instance
already has workbooks open). AutomationSecurity = 3 (msoAutomationSecurityForceDisable), Quit in finally. It
never kills a process it did not start.

  pwsh -NoProfile -File tools/office/make-numfmt-locale-refs.ps1 -Measure
  powershell -NoProfile -File tools/office/make-numfmt-locale-refs.ps1 -Nls

Output is stable: LF line endings, ASCII only (non-ASCII is \uXXXX escaped), no paths, no user data.
#>
param(
  [switch]$Measure,
  [switch]$Nls,
  [string]$Scratch = $env:TEMP
)
$ErrorActionPreference = 'Stop'
if (-not ($Measure -or $Nls)) { throw 'pass -Measure and/or -Nls' }
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$outDir = Join-Path $root 'fixtures/reference/xlsx/numfmt-locale'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

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
function Write-Lf([string]$path, [string]$text) {
  [IO.File]::WriteAllText($path, ($text -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
}

# ---------------------------------------------------------------------------------------------------------------
# -Nls: Windows locale data for every LCID of the crate's table.
# ---------------------------------------------------------------------------------------------------------------
if ($Nls) {
  if ($PSVersionTable.PSEdition -eq 'Core') { throw '-Nls needs Windows PowerShell 5.1 (powershell.exe): .NET Core uses ICU, not NLS' }
  $tablePath = Join-Path $root 'crates/locale/src/lcid_table.rs'
  $seen = @{}
  $rows = New-Object System.Collections.Generic.List[object]
  foreach ($line in Get-Content -LiteralPath $tablePath) {
    if ($line -match '^\s*\(0x([0-9A-Fa-f]+),\s*"([^"]+)"\),') {
      $lcid = [Convert]::ToInt32($Matches[1], 16)
      if ($seen.ContainsKey($lcid)) { continue }
      $seen[$lcid] = $true
      try { $c = New-Object Globalization.CultureInfo($lcid, $false) } catch { continue }
      if ($c.LCID -ne $lcid) { continue }   # the OS answered with a different (fallback) culture
      $dt = $c.DateTimeFormat
      $nf = $c.NumberFormat
      $cal = $c.Calendar.GetType().Name
      $rows.Add([pscustomobject]@{
        lcid = $lcid; tag = $Matches[2]; name = $c.Name
        shortDate = $dt.ShortDatePattern; longDate = $dt.LongDatePattern
        shortTime = $dt.ShortTimePattern; longTime = $dt.LongTimePattern
        am = $dt.AMDesignator; pm = $dt.PMDesignator
        currencySymbol = $nf.CurrencySymbol; currencyPos = $nf.CurrencyPositivePattern
        currencyNeg = $nf.CurrencyNegativePattern; currencyDigits = $nf.CurrencyDecimalDigits
        decimalSep = $nf.NumberDecimalSeparator; groupSep = $nf.NumberGroupSeparator
        calendar = $cal
      })
    }
  }
  $json = New-Object System.Text.StringBuilder
  [void]$json.Append("{`n  `"source`": `"System.Globalization.CultureInfo(lcid, useUserOverride=false), Windows PowerShell 5.1 (NLS)`",`n  `"label`": `"derived`",`n  `"locales`": [`n")
  for ($i = 0; $i -lt $rows.Count; $i++) {
    $r = $rows[$i]
    $f = @(
      "`"lcid`": $($r.lcid)", "`"tag`": $(Json-Str $r.tag)", "`"shortDate`": $(Json-Str $r.shortDate)",
      "`"longDate`": $(Json-Str $r.longDate)", "`"shortTime`": $(Json-Str $r.shortTime)", "`"longTime`": $(Json-Str $r.longTime)",
      "`"am`": $(Json-Str $r.am)", "`"pm`": $(Json-Str $r.pm)", "`"currencySymbol`": $(Json-Str $r.currencySymbol)",
      "`"currencyPos`": $($r.currencyPos)", "`"currencyNeg`": $($r.currencyNeg)", "`"currencyDigits`": $($r.currencyDigits)",
      "`"decimalSep`": $(Json-Str $r.decimalSep)", "`"groupSep`": $(Json-Str $r.groupSep)", "`"calendar`": $(Json-Str $r.calendar)")
    [void]$json.Append('    { ' + ($f -join ', ') + ' }')
    if ($i -lt $rows.Count - 1) { [void]$json.Append(',') }
    [void]$json.Append("`n")
  }
  [void]$json.Append("  ]`n}`n")
  Write-Lf (Join-Path $outDir 'windows-nls.json') $json.ToString()

  function Rs($s) {
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append('"')
    foreach ($ch in ([string]$s).ToCharArray()) {
      $code = [int]$ch
      if ($ch -eq '"') { [void]$sb.Append('\"') }
      elseif ($ch -eq '\') { [void]$sb.Append('\\') }
      elseif ($code -lt 32 -or $code -gt 126) { [void]$sb.Append('\u{' + ('{0:x}' -f $code) + '}') }
      else { [void]$sb.Append($ch) }
    }
    [void]$sb.Append('"')
    return $sb.ToString()
  }
  $rs = New-Object System.Text.StringBuilder
  [void]$rs.Append("//! GENERATED by tools/office/make-numfmt-locale-refs.ps1 -Nls from the Windows locale data (NLS, CultureInfo with useUserOverride = false).`n//! Label: derived, not an Excel measurement. Do not edit.`n`n")
  [void]$rs.Append("use crate::builtin_numfmt::Nls;`n`n")
  [void]$rs.Append("#[rustfmt::skip]`npub(crate) const NLS: &[Nls] = &[`n")
  foreach ($r in $rows) {
    [void]$rs.Append(("    Nls {{ lcid: 0x{0:X4}, short_date: {1}, short_time: {2}, time: {3}, currency: {4}, currency_pos: {5}, currency_neg: {6} }},`n" -f `
      $r.lcid, (Rs $r.shortDate), (Rs $r.shortTime), (Rs $r.longTime), (Rs $r.currencySymbol), $r.currencyPos, $r.currencyNeg))
  }
  [void]$rs.Append("];`n")
  Write-Lf (Join-Path $root 'crates/xlsx/src/builtin_numfmt_nls.rs') $rs.ToString()
  Write-Host "NLS: $($rows.Count) locales"
}

# ---------------------------------------------------------------------------------------------------------------
# -Measure: Excel renders numFmtId 0..100 from a workbook that has no numFmts part.
# ---------------------------------------------------------------------------------------------------------------
if ($Measure) {
  Add-Type -AssemblyName System.IO.Compression
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@
  $maxId = 100
  # Probe values (one column each): a date and time, a large positive, a negative, zero.
  $probes = @(
    @{ key = 'dt'; value = '45366.7654321' },     # 2024-03-15 18:22:13
    @{ key = 'pos'; value = '1234567.891' },
    @{ key = 'neg'; value = '-1234567.891' },
    @{ key = 'zero'; value = '0' }
  )
  $dir = Join-Path $Scratch ('o365-numfmt-' + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $dir | Out-Null
  $xlsx = Join-Path $dir 'probe.xlsx'
  $xf = New-Object System.Text.StringBuilder
  for ($i = 0; $i -le $maxId; $i++) { [void]$xf.Append("<xf numFmtId=`"$i`" fontId=`"0`" fillId=`"0`" borderId=`"0`" xfId=`"0`" applyNumberFormat=`"1`"/>") }
  $styles = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="' + ($maxId + 1) + '">' + $xf.ToString() + '</cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>'
  $sd = New-Object System.Text.StringBuilder
  [void]$sd.Append('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="1" max="8" width="48" customWidth="1"/></cols><sheetData>')
  for ($i = 0; $i -le $maxId; $i++) {
    $row = $i + 1
    [void]$sd.Append("<row r=`"$row`">")
    for ($p = 0; $p -lt $probes.Count; $p++) {
      $col = [char](65 + $p)
      [void]$sd.Append("<c r=`"$col$row`" s=`"$i`"><v>$($probes[$p].value)</v></c>")
    }
    [void]$sd.Append('</row>')
  }
  [void]$sd.Append('</sheetData></worksheet>')
  $parts = [ordered]@{
    '[Content_Types].xml' = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>'
    '_rels/.rels' = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>'
    'xl/workbook.xml' = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>'
    'xl/_rels/workbook.xml.rels' = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>'
    'xl/styles.xml' = $styles
    'xl/worksheets/sheet1.xml' = $sd.ToString()
  }
  $zip = [IO.Compression.ZipFile]::Open($xlsx, 'Create')
  try {
    foreach ($k in $parts.Keys) {
      $e = $zip.CreateEntry($k)
      $w = New-Object IO.StreamWriter($e.Open(), (New-Object Text.UTF8Encoding($false)))
      $w.Write($parts[$k]); $w.Dispose()
    }
  } finally { $zip.Dispose() }

  # ---- Excel lifecycle: own instance only ----------------------------------------------------------------------
  $before = @(Get-Process -Name EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $app = $null
  $ownPid = 0
  $result = $null
  try {
    $app = New-Object -ComObject Excel.Application
    $procId = [uint32]0
    [void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
    $ownPid = [int]$procId
    if ($before -contains $ownPid -or $app.Workbooks.Count -gt 0) {
      $ownPid = 0   # not ours: never quit it, never kill it
      $app = $null
      throw 'New-Object -ComObject Excel.Application attached to a pre-existing Excel instance; aborting without touching it'
    }
    $app.Visible = $false
    $app.DisplayAlerts = $false
    $app.ScreenUpdating = $false
    $app.EnableEvents = $false
    $app.AutomationSecurity = 3
    $intl = $app
    $info = [ordered]@{
      excelVersion = [string]$app.Version
      excelBuild = [string]$app.Build
      windowsLocale = (Get-Culture).Name
      windowsLcid = (Get-Culture).LCID
      decimalSeparator = [string]$intl.International(3)
      thousandsSeparator = [string]$intl.International(4)
      listSeparator = [string]$intl.International(5)
      currencyCode = [string]$intl.International(25)
      dateOrder = [int]$intl.International(32)
      dateSeparator = [string]$intl.International(17)
      timeSeparator = [string]$intl.International(18)
      clock24 = [int]([bool]$intl.International(33))
      monthLeadingZero = [int]([bool]$intl.International(41))
      dayLeadingZero = [int]([bool]$intl.International(42))
      fourDigitYears = [int]([bool]$intl.International(43))
      timeLeadingZero = [int]([bool]$intl.International(45))
      currencyNegative = [int]$intl.International(28)
      currencyBefore = [int]([bool]$intl.International(37))
      currencySpaceBefore = [int]([bool]$intl.International(36))
    }
    $wb = $app.Workbooks.Open($xlsx, 0, $true)
    $ws = $wb.Worksheets.Item(1)
    $entries = New-Object System.Collections.Generic.List[string]
    for ($i = 0; $i -le $maxId; $i++) {
      $row = $i + 1
      $texts = @()
      foreach ($p in 0..($probes.Count - 1)) {
        $cell = $ws.Cells.Item($row, $p + 1)
        $texts += "`"$($probes[$p].key)`": $(Json-Str $cell.Text)"
      }
      $c0 = $ws.Cells.Item($row, 1)
      $entries.Add("    { `"id`": $i, `"numberFormat`": $(Json-Str $c0.NumberFormat), `"numberFormatLocal`": $(Json-Str $c0.NumberFormatLocal), `"text`": { $($texts -join ', ') } }")
    }
    $wb.Saved = $true
    $wb.Close($false)
    $tag = $info.windowsLocale
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append("{`n  `"label`": `"measured`",`n")
    foreach ($k in $info.Keys) {
      $v = $info[$k]
      if ($v -is [int]) { [void]$sb.Append("  `"$k`": $v,`n") } else { [void]$sb.Append("  `"$k`": $(Json-Str $v),`n") }
    }
    [void]$sb.Append("  `"probes`": { " + (($probes | ForEach-Object { "`"$($_.key)`": $($_.value)" }) -join ', ') + " },`n")
    [void]$sb.Append("  `"ids`": [`n" + ($entries -join ",`n") + "`n  ]`n}`n")
    $result = @{ tag = $tag; json = $sb.ToString() }
  } finally {
    if ($app -and $ownPid -gt 0) {
      try { $app.Quit() } catch {}
      try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
      [GC]::Collect(); [GC]::WaitForPendingFinalizers()
      $deadline = (Get-Date).AddSeconds(8)
      while ((Get-Date) -lt $deadline -and (Get-Process -Id $ownPid -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
      $p = Get-Process -Id $ownPid -ErrorAction SilentlyContinue
      if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $ownPid -Force -ErrorAction SilentlyContinue }
    }
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
  }
  if ($result) {
    Write-Lf (Join-Path $outDir ("measured-$($result.tag).json")) $result.json
    Write-Host "measured $($result.tag)"
  }
}
