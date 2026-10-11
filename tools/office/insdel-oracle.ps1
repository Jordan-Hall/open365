<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for inserting and deleting whole rows and
columns: how Excel rewrites formulas (Range.Formula), values (Value2), defined names (RefersTo), tables, conditional
formats, data validation and merged cells.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed output (fixtures/reference/xlsx/insdel-refs/scenarios.json) through
crates/commands/tests/insdel.rs.

-MakeFixture (re)creates fixtures/xlsx/insdel-refs.xlsx through COM: sheet S (data A1:H12, a column of formulas in
N with relative, absolute, mixed, range, whole-row, whole-column, union, intersection and self-qualified references,
a table, an array formula, a merge, conditional formats, data validation), sheet O and sheet "Q R" (formulas that
point at S) and defined names (workbook, relative, sheet-scoped, constant-mixed). docProps/core.xml dc:creator and
cp:lastModifiedBy are scrubbed. Nothing is hand-edited.

The oracle then opens the fixture once per scenario, applies the edit to sheet S (Rows(n).Insert, Rows(a:b).Delete,
Columns likewise), and records every non-empty cell of every sheet (formula, Value2), the defined names, tables,
conditional formats, validation areas and merges, or the error Excel raised.

  pwsh -NoProfile -File tools/office/insdel-oracle.ps1 [-MakeFixture]

Process safety: starts its own hidden Excel instance with AutomationSecurity = msoAutomationSecurityForceDisable (3),
remembers its process id and only ever stops that process (in a finally block). Other EXCEL/WINWORD processes and the
Office and regional settings are never touched.
#>
param(
  [switch]$MakeFixture
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'

Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@

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
    # Only the process this script started; the id was taken from our own window handle.
    if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $script:xlPid -Force -ErrorAction SilentlyContinue }
  }
  $script:xlPid = 0
}

function Json-Str([string]$s) {
  $sb = New-Object System.Text.StringBuilder
  [void]$sb.Append('"')
  foreach ($ch in $s.ToCharArray()) {
    $code = [int]$ch
    if ($ch -eq '"') { [void]$sb.Append('\"') }
    elseif ($ch -eq '\') { [void]$sb.Append('\\') }
    elseif ($code -lt 0x20 -or $code -gt 0x7e) { [void]$sb.AppendFormat('\u{0:x4}', $code) }
    else { [void]$sb.Append($ch) }
  }
  [void]$sb.Append('"')
  return $sb.ToString()
}

function Scrub-Core([string]$path) {
  Add-Type -AssemblyName System.IO.Compression
  Add-Type -AssemblyName System.IO.Compression.FileSystem
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

# Formulas on S, column N from row 1. Order matters: the corpus test reads them back by address.
$sFormulas = @(
  '=C3', '=$C$3', '=$C3', '=C$3',
  '=SUM(C3:F9)', '=SUM($C$3:$F$9)', '=SUM(C:C)', '=SUM(3:5)', '=SUM(C:E)', '=SUM(C3:C9)',
  '=C3+F9', '=SUM(C3,F9)', '=SUM(C3:F3)', '=INDEX(C3:F9,2,2)', '=OFFSET(C3,1,1)',
  '=SUM((C3:C5,E3:E5))', '=SUM(D4:E8 C5:F6)', '=SUM(RngAll)', '=SUM(RelRng)', '=CellOne+1',
  '=CalcNm', '=S!C3', '=SUM(S!C3:F9)', '=ROW(C5)', '=COLUMN(F9)',
  '=SUM(Tbl[Col2])', '=ROWS(Tbl)', '=SUM(O!B1:B3)', '=O!A1', '=SUM(ScopedNm)',
  '=SUM(C9:C3)', '=SUM($C3:F$9)', '=SUM(A1:H12)', '=SUM(A:H)', '=SUM(1:12)'
)
$oFormulas = @(
  '=S!C3', '=SUM(S!C3:F9)', '=S!$C$3', '=SUM(S!C:C)', '=SUM(S!3:5)', '=SUM(S!C3:F9)*2', '=SUM(RngAll)', '=B1',
  '=SUM(S!ScopedNm)', '=S!N1', '=INDEX(S!C3:F9,2,2)', '=SUM(S!C3:F9,S!A1)', '=SUM(S!C3:C5)+SUM(B1:B3)'
)

function Setup-Fixture($app, [string]$path) {
  $wb = $app.Workbooks.Add()
  while ($wb.Worksheets.Count -lt 3) { [void]$wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
  $s = $wb.Worksheets.Item(1); $s.Name = 'S'
  $o = $wb.Worksheets.Item(2); $o.Name = 'O'
  $q = $wb.Worksheets.Item(3); $q.Name = 'Q R'
  for ($r = 1; $r -le 12; $r++) { for ($c = 1; $c -le 8; $c++) { $s.Cells.Item($r, $c).Value2 = [double]($r * 10 + $c) } }
  # Table at C20:E24 (header row 20).
  $s.Range('C20').Value2 = 'Col1'; $s.Range('D20').Value2 = 'Col2'; $s.Range('E20').Value2 = 'Col3'
  for ($r = 21; $r -le 24; $r++) { for ($c = 3; $c -le 5; $c++) { $s.Cells.Item($r, $c).Value2 = [double]($r * 100 + $c) } }
  [void]$s.ListObjects.Add(1, $s.Range('C20:E24'), [Type]::Missing, 1)
  $s.ListObjects.Item(1).Name = 'Tbl'
  $s.Range('B14:C15').Merge()
  $o.Range('B1').Value2 = 1.0; $o.Range('B2').Value2 = 2.0; $o.Range('B3').Value2 = 3.0
  $o.Range('A14').Value2 = 7.0
  # Names. Relative ones are defined with A1 active on S, so the stored reference moves with the cell that uses it.
  [void]$s.Activate(); [void]$s.Range('A1').Select()
  [void]$wb.Names.Add('RngAll', '=S!$C$3:$F$9')
  [void]$wb.Names.Add('CellOne', '=S!$C$3')
  [void]$wb.Names.Add('RelRng', '=S!C3:C5')
  [void]$wb.Names.Add('CalcNm', '=S!$C$3*2+O!$B$1')
  [void]$s.Names.Add('ScopedNm', '=S!$D$4:$D$8')
  # Array formula over P40:P42.
  $s.Range('P40:P42').FormulaArray = '=C3:C5*2'
  # Conditional formats.
  $fc = $s.Range('C3:F9').FormatConditions.Add(2, 0, '=C3>40'); $fc.Interior.Color = 65535
  $fc2 = $s.Range('H1:H12').FormatConditions.Add(1, 5, '=50'); $fc2.Interior.Color = 255
  # A conditional format whose relative formula points away from its own range.
  [void]$s.Range('J2').Select()
  $fc3 = $s.Range('J2:J8').FormatConditions.Add(2, 0, '=$C3>40'); $fc3.Interior.Color = 65280
  # Data validation.
  $v = $s.Range('A14:A16').Validation; $v.Delete(); [void]$v.Add(3, 1, 1, '=$C$3:$C$9')
  $v2 = $s.Range('D14:D16').Validation; $v2.Delete(); [void]$v2.Add(1, 1, 1, '=$C$3', '=$F$9')
  [void]$s.Range('L14').Select()
  $v3 = $s.Range('L14:L16').Validation; $v3.Delete(); [void]$v3.Add(7, 1, 1, '=C3>0')
  for ($i = 0; $i -lt $sFormulas.Count; $i++) { $s.Cells.Item($i + 1, 14).Formula = $sFormulas[$i] }
  for ($i = 0; $i -lt $oFormulas.Count; $i++) { $o.Cells.Item($i + 1, 1).Formula = $oFormulas[$i] }
  $q.Range('A1').Formula = '=S!C3+O!B1'
  $q.Range('A2').Formula = "=SUM('Q R'!B1:B2)"
  $q.Range('B1').Value2 = 5.0; $q.Range('B2').Value2 = 6.0
  $app.CalculateFull()
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
  $wb.SaveAs($path, 51)
  $wb.Close($false)
  Scrub-Core $path
  $toml = [IO.Path]::ChangeExtension($path, '.toml')
  $made = "Microsoft Excel $($app.Build) (Windows)"
  $lines = @(
    'feature = "insdel-refs"',
    ('made_with = ' + (Json-Str $made)),
    'author = "project member"',
    'expect_dropped = []',
    'notes = "reference shifting under insert and delete of rows and columns: relative, absolute, mixed, range, whole row and column, other sheet, names, table, array formula, conditional format, validation, merge"'
  )
  [IO.File]::WriteAllText($toml, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  Write-Host "insdel-oracle: wrote $path"
}

function Err-Name($v) {
  switch ([int]$v) {
    -2146826281 { return '#DIV/0!' }
    -2146826246 { return '#N/A' }
    -2146826259 { return '#NAME?' }
    -2146826288 { return '#NULL!' }
    -2146826252 { return '#NUM!' }
    -2146826265 { return '#REF!' }
    -2146826273 { return '#VALUE!' }
    default { return '#ERR' }
  }
}

function Cell-Json($f, $v) {
  $ftxt = ''
  if ($f -is [string] -and $f.StartsWith('=')) { $ftxt = '"f":' + (Json-Str $f) + ',' }
  $vtxt = ''
  if ($null -eq $v) { $vtxt = '"v":null' }
  elseif ($v -is [string]) { $vtxt = '"s":' + (Json-Str $v) }
  elseif ($v -is [bool]) { $vtxt = '"b":' + $v.ToString().ToLowerInvariant() }
  elseif ($v -is [int]) { $vtxt = '"e":' + (Json-Str (Err-Name $v)) }
  else { $vtxt = '"v":' + ([double]$v).ToString('R', [Globalization.CultureInfo]::InvariantCulture) }
  return '{' + $ftxt + $vtxt + '}'
}

function Dump-Sheet($ws) {
  $ur = $ws.UsedRange
  $r0 = $ur.Row; $c0 = $ur.Column
  $nr = $ur.Rows.Count; $nc = $ur.Columns.Count
  $f = $ur.Formula; $v = $ur.Value2
  $items = @()
  if ($nr -eq 1 -and $nc -eq 1) {
    if ($null -ne $v -and "$v" -ne '') { $items += ('"' + $ur.Address($false, $false) + '":' + (Cell-Json $f $v)) }
    return '{' + ($items -join ',') + '}'
  }
  for ($r = 1; $r -le $nr; $r++) {
    for ($c = 1; $c -le $nc; $c++) {
      $val = $v[$r, $c]
      if ($null -eq $val -or ($val -is [string] -and $val -eq '')) { continue }
      $addr = $ws.Cells.Item($r0 + $r - 1, $c0 + $c - 1).Address($false, $false)
      $items += ('"' + $addr + '":' + (Cell-Json $f[$r, $c] $val))
    }
  }
  return '{' + ($items -join ',') + '}'
}

function Dump-Extras($wb, $ws) {
  $names = @()
  foreach ($n in $wb.Names) { $names += ('{"name":' + (Json-Str $n.Name) + ',"refers":' + (Json-Str $n.RefersTo) + '}') }
  $tables = @()
  foreach ($lo in $ws.ListObjects) {
    $cols = @(); foreach ($lc in $lo.ListColumns) { $cols += (Json-Str $lc.Name) }
    $tables += ('{"name":' + (Json-Str $lo.Name) + ',"range":' + (Json-Str $lo.Range.Address($false, $false)) + ',"columns":[' + ($cols -join ',') + ']}')
  }
  $cf = @()
  $count = 0; try { $count = $ws.Cells.FormatConditions.Count } catch {}
  for ($i = 1; $i -le $count; $i++) {
    $fc = $ws.Cells.FormatConditions.Item($i)
    $applies = $fc.AppliesTo
    try { [void]$ws.Activate(); [void]$applies.Cells.Item(1).Select() } catch {}
    $f1 = ''; try { $f1 = [string]$fc.Formula1 } catch {}
    $cf += ('{"applies":' + (Json-Str $applies.Address($false, $false)) + ',"type":' + [int]$fc.Type + ',"formula1":' + (Json-Str $f1) + '}')
  }
  $dv = @()
  $areas = $null; try { $areas = $ws.Cells.SpecialCells(-4174) } catch {}
  if ($areas) {
    foreach ($a in $areas.Areas) {
      try { [void]$ws.Activate(); [void]$a.Cells.Item(1).Select() } catch {}
      $f1 = ''; $f2 = ''
      try { $f1 = [string]$a.Validation.Formula1 } catch {}
      try { $f2 = [string]$a.Validation.Formula2 } catch {}
      $dv += ('{"range":' + (Json-Str $a.Address($false, $false)) + ',"type":' + [int]$a.Validation.Type + ',"formula1":' + (Json-Str $f1) + ',"formula2":' + (Json-Str $f2) + '}')
    }
  }
  $merges = @()
  $seen = @{}
  $ur = $ws.UsedRange
  foreach ($cell in $ur.Cells) {
    if ($cell.MergeCells) {
      $ma = $cell.MergeArea.Address($false, $false)
      if (-not $seen.ContainsKey($ma)) { $seen[$ma] = 1; $merges += (Json-Str $ma) }
    }
  }
  return '"names":[' + ($names -join ',') + '],"tables":[' + ($tables -join ',') + '],"cf":[' + ($cf -join ',') + '],"dv":[' + ($dv -join ',') + '],"merges":[' + ($merges -join ',') + ']'
}

function Apply-Edit($ws, [string]$op, [int]$at, [int]$count) {
  $last = $at + $count - 1
  switch ($op) {
    'insert_rows' { [void]$ws.Range("$at`:$last").EntireRow.Insert() }
    'delete_rows' { [void]$ws.Range("$at`:$last").EntireRow.Delete() }
    'insert_cols' {
      $a = $ws.Cells.Item(1, $at).Address($false, $false) -replace '\d', ''
      $b = $ws.Cells.Item(1, $last).Address($false, $false) -replace '\d', ''
      [void]$ws.Range("$a`:$b").EntireColumn.Insert()
    }
    'delete_cols' {
      $a = $ws.Cells.Item(1, $at).Address($false, $false) -replace '\d', ''
      $b = $ws.Cells.Item(1, $last).Address($false, $false) -replace '\d', ''
      [void]$ws.Range("$a`:$b").EntireColumn.Delete()
    }
  }
}

function Scenario-Json($app, [string]$path, [string]$op, [int]$at, [int]$count) {
  $wb = $app.Workbooks.Open($path)
  try {
    $s = $wb.Worksheets.Item('S')
    $err = ''
    if ($op -ne 'none') {
      try { Apply-Edit $s $op $at $count } catch { $err = $_.Exception.Message }
    }
    $app.CalculateFull()
    $sheets = @()
    foreach ($ws in $wb.Worksheets) { $sheets += ((Json-Str $ws.Name) + ':' + (Dump-Sheet $ws)) }
    $extras = Dump-Extras $wb $s
    $e = ''
    if ($err -ne '') { $e = ',"error":' + (Json-Str $err) }
    return '{"op":' + (Json-Str $op) + ',"at":' + $at + ',"count":' + $count + $e + ',"sheets":{' + ($sheets -join ',') + '},' + $extras + '}'
  } finally { $wb.Close($false) }
}

function Setup-Format($app, [string]$path) {
  $wb = $app.Workbooks.Add()
  $f = $wb.Worksheets.Item(1); $f.Name = 'F'
  for ($r = 1; $r -le 6; $r++) { for ($c = 1; $c -le 4; $c++) { $f.Cells.Item($r, $c).Value2 = [double]($r * 10 + $c) } }
  $f.Range('A2:D2').Font.Bold = $true
  $f.Range('A2:D2').Interior.Color = 65535
  $f.Range('A4').Font.Italic = $true
  $f.Range('B3:B5').NumberFormat = '0.00'
  $f.Rows.Item(2).RowHeight = 30
  $f.Rows.Item(4).Hidden = $true
  $f.Rows.Item(5).Interior.Color = 16711680
  $f.Columns.Item(2).ColumnWidth = 20
  $f.Columns.Item(3).Hidden = $true
  $f.Columns.Item(4).ColumnWidth = 5
  $f.Columns.Item(5).Interior.Color = 255
  [void]$f.Range('A3:A4').Rows.Group()
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
  $wb.SaveAs($path, 51)
  $wb.Close($false)
  Scrub-Core $path
  $toml = [IO.Path]::ChangeExtension($path, '.toml')
  $made = "Microsoft Excel $($app.Build) (Windows)"
  $lines = @(
    'feature = "insdel-format"',
    ('made_with = ' + (Json-Str $made)),
    'author = "project member"',
    'expect_dropped = []',
    'notes = "formatting that an inserted row or column inherits: cell styles, row and column styles, heights, widths, hidden, outline"'
  )
  [IO.File]::WriteAllText($toml, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  Write-Host "insdel-oracle: wrote $path"
}

function Sheet-Xml([string]$path, [int]$index) {
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [IO.Compression.ZipFile]::OpenRead($path)
  try {
    $e = $zip.GetEntry("xl/worksheets/sheet$index.xml")
    $r = New-Object IO.StreamReader($e.Open(), (New-Object Text.UTF8Encoding $false))
    $xml = $r.ReadToEnd(); $r.Close()
    return $xml
  } finally { $zip.Dispose() }
}

function Format-Json($app, [string]$path, [string]$op, [int]$at, [int]$count) {
  $wb = $app.Workbooks.Open($path)
  try {
    Apply-Edit $wb.Worksheets.Item('F') $op $at $count
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('o365-insdel-' + [Guid]::NewGuid().ToString('N') + '.xlsx')
    $wb.SaveCopyAs($tmp)
  } finally { $wb.Close($false) }
  $xml = Sheet-Xml $tmp 1
  Remove-Item -LiteralPath $tmp -ErrorAction SilentlyContinue
  $cols = [regex]::Match($xml, '<cols>.*?</cols>').Value
  $rows = ([regex]::Matches($xml, '<row [^>]*>') | ForEach-Object { Json-Str $_.Value }) -join ','
  $cells = ([regex]::Matches($xml, '<c [^>]*>') | ForEach-Object { Json-Str $_.Value }) -join ','
  return '{"op":' + (Json-Str $op) + ',"at":' + $at + ',"count":' + $count + ',"cols":' + (Json-Str $cols) + ',"rows":[' + $rows + '],"cells":[' + $cells + ']}'
}

try {
  Start-Excel
  $path = Join-Path $fixtureDir 'insdel-refs.xlsx'
  $fpath = Join-Path $fixtureDir 'insdel-format.xlsx'
  if ($MakeFixture) { Setup-Fixture $script:xl $path; Setup-Format $script:xl $fpath }
  $fmt = @()
  $fmt += Format-Json $script:xl $fpath 'none' 0 0
  foreach ($n in @(1, 2)) {
    foreach ($at in @(1, 2, 3, 4, 5, 6, 7)) { $fmt += Format-Json $script:xl $fpath 'insert_rows' $at $n }
    foreach ($at in @(1, 2, 3, 4, 5, 6)) { $fmt += Format-Json $script:xl $fpath 'insert_cols' $at $n }
  }
  foreach ($at in @(2, 3, 4, 5)) { $fmt += Format-Json $script:xl $fpath 'delete_rows' $at 1 }
  foreach ($at in @(2, 3, 4, 5)) { $fmt += Format-Json $script:xl $fpath 'delete_cols' $at 1 }
  $rowAt = @(1, 2, 3, 4, 5, 6, 9, 10, 12, 13, 17, 20, 21, 25, 41)
  $colAt = @(1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 14, 16, 17)
  $out = @()
  $out += Scenario-Json $script:xl $path 'none' 0 0
  foreach ($op in @('insert_rows', 'delete_rows')) { foreach ($n in @(1, 3)) { foreach ($at in $rowAt) { $out += Scenario-Json $script:xl $path $op $at $n } } }
  foreach ($op in @('insert_cols', 'delete_cols')) { foreach ($n in @(1, 2)) { foreach ($at in $colAt) { $out += Scenario-Json $script:xl $path $op $at $n } } }
  $lines = @(
    '{',
    '  "source": "fixtures/xlsx/insdel-refs.xlsx",',
    '  "oracle": "Microsoft Excel via COM: Range.Formula and Value2 of every non-empty cell, defined names, tables, conditional formats, validation and merges after Rows/Columns Insert/Delete on sheet S",',
    ('  "excel_build": ' + (Json-Str ([string]$script:xl.Build)) + ','),
    '  "scenarios": ['
  )
  $lines += (($out | ForEach-Object { '    ' + $_ }) -join ",`n")
  $lines += '  ]'
  $lines += '}'
  $outDir = Join-Path $refDir 'insdel-refs'
  New-Item -ItemType Directory -Force -Path $outDir | Out-Null
  [IO.File]::WriteAllText((Join-Path $outDir 'scenarios.json'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  $flines = @(
    '{',
    '  "source": "fixtures/xlsx/insdel-format.xlsx",',
    '  "oracle": "Microsoft Excel via COM: cols, row start tags and cell start tags (r, s) of SaveCopyAs after Rows/Columns Insert/Delete on sheet F",',
    ('  "excel_build": ' + (Json-Str ([string]$script:xl.Build)) + ','),
    '  "scenarios": ['
  )
  $flines += (($fmt | ForEach-Object { '    ' + $_ }) -join ",`n")
  $flines += '  ]'
  $flines += '}'
  [IO.File]::WriteAllText((Join-Path $outDir 'format.json'), (($flines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  Write-Host "insdel-oracle: recorded $($out.Count) scenarios"
}
finally {
  Stop-Excel
}
