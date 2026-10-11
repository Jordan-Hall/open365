<#
Uses the locally installed Microsoft Excel (16.0, COM) as the oracle for hiding and unhiding columns and rows and for
select-all size edits (Cells.RowHeight / Cells.ColumnWidth).

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this script is
never run in CI. CI consumes the committed output (fixtures/reference/xlsx/grid-hide/xml.json) through
crates/commands/tests/hide.rs.

-MakeFixture (re)creates fixtures/xlsx/grid-hide.xlsx, the "before" state, through COM: one sheet per scenario with
custom, default, hidden and zero-sized columns and rows. docProps/core.xml dc:creator and cp:lastModifiedBy are
scrubbed. The oracle then opens the fixture, applies the scenario's edit through COM on a copy, saves the copy to a
temporary file and records, per sheet, the XML Excel wrote for sheetFormatPr, cols and every row start tag (attributes
only; no cell content). Nothing is hand-edited.

Scenarios (sheet names): Hide (EntireColumn/EntireRow.Hidden = True), Zero (ColumnWidth / RowHeight = 0), SetVisible (ColumnWidth / RowHeight on hidden ranges), Unhide
(Hidden = False on hidden and zero-sized ranges), AllRows (Cells.RowHeight = 25), AllCols (Cells.ColumnWidth = 15).

  pwsh -NoProfile -File tools/office/grid-hide-oracle.ps1 [-MakeFixture]

Process safety: starts its own hidden Excel instance, remembers its process id and only ever stops that process
(in a finally block). Other EXCEL/WINWORD processes are never touched.
#>
param(
  [switch]$MakeFixture
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureDir = Join-Path $root 'fixtures/xlsx'
$refDir = Join-Path $root 'fixtures/reference/xlsx'
$inv = [Globalization.CultureInfo]::InvariantCulture

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

function Rgb([int]$r, [int]$g, [int]$b) { return $r + 256 * $g + 65536 * $b }

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


$scenarios = @('Hide', 'Zero', 'Unhide', 'AllRows', 'AllCols', 'SetVisible')

function Setup-Sheet($ws, [string]$name) {
  $ws.Name = $name
  $ws.Range('A1').Value2 = $name
  $ws.Range('A3').Value2 = 'r3'
  $ws.Range('A5').Value2 = 'r5'
  $ws.Range('A7').Value2 = 'r7'
  $ws.Range('E1').Value2 = 'e1'
  switch ($name) {
    'Hide' {
      $ws.Columns.Item(2).ColumnWidth = 20
      $ws.Columns.Item(4).ColumnWidth = 12
      $ws.Rows.Item(4).RowHeight = 30
    }
    'Zero' {
      $ws.Columns.Item(2).ColumnWidth = 20
      $ws.Rows.Item(4).RowHeight = 30
    }
    'Unhide' {
      $ws.Columns.Item(2).ColumnWidth = 20
      $ws.Columns.Item(2).Hidden = $true
      $ws.Columns.Item(3).Hidden = $true
      $ws.Columns.Item(4).ColumnWidth = 0
      $ws.Rows.Item(4).RowHeight = 30
      $ws.Rows.Item(3).Hidden = $true
      $ws.Rows.Item(4).Hidden = $true
      $ws.Rows.Item(5).RowHeight = 0
      $ws.Rows.Item(6).Hidden = $true
    }
    'AllRows' {
      $ws.Rows.Item(4).RowHeight = 30
      $ws.Rows.Item(5).Hidden = $true
    }
    'AllCols' {
      $ws.Columns.Item(2).ColumnWidth = 20
      $ws.Columns.Item(3).Hidden = $true
      $ws.Columns.Item(4).ColumnWidth = 12
    }
    'SetVisible' {
      $ws.Columns.Item(2).ColumnWidth = 20
      $ws.Columns.Item(2).Hidden = $true
      $ws.Columns.Item(3).Hidden = $true
      $ws.Rows.Item(3).Hidden = $true
      $ws.Rows.Item(4).RowHeight = 30
      $ws.Rows.Item(4).Hidden = $true
    }
  }
}

function Apply-Edit($ws, [string]$name) {
  switch ($name) {
    'Hide' {
      $ws.Columns.Item(2).Hidden = $true
      $ws.Columns.Item(3).Hidden = $true
      $ws.Range('D1:E1').EntireColumn.Hidden = $true
      $ws.Rows.Item(3).Hidden = $true
      $ws.Rows.Item(4).Hidden = $true
      $ws.Rows.Item(6).Hidden = $true
    }
    'Zero' {
      $ws.Columns.Item(2).ColumnWidth = 0
      $ws.Columns.Item(6).ColumnWidth = 0
      $ws.Rows.Item(3).RowHeight = 0
      $ws.Rows.Item(4).RowHeight = 0
      $ws.Rows.Item(6).RowHeight = 0
    }
    'Unhide' {
      $ws.Range('B1:D1').EntireColumn.Hidden = $false
      $ws.Range('A3:A6').EntireRow.Hidden = $false
    }
    'AllRows' { $ws.Cells.RowHeight = 25 }
    'AllCols' { $ws.Cells.ColumnWidth = 15 }
    'SetVisible' {
      $ws.Columns.Item(2).ColumnWidth = 12
      $ws.Columns.Item(3).ColumnWidth = 12
      $ws.Rows.Item(3).RowHeight = 20
      $ws.Rows.Item(4).RowHeight = 20
    }
  }
}

function Write-Fixture($app, [string]$path) {
  $wb = $app.Workbooks.Add()
  while ($wb.Worksheets.Count -lt $scenarios.Count) { [void]$wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count)) }
  for ($i = 0; $i -lt $scenarios.Count; $i++) { Setup-Sheet $wb.Worksheets.Item($i + 1) $scenarios[$i] }
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
  $wb.SaveAs($path, 51)
  $wb.Close($false)
  Scrub-Core $path
  $toml = [IO.Path]::ChangeExtension($path, '.toml')
  $made = "Microsoft Excel $($app.Build) (Windows)"
  $lines = @(
    'feature = "grid-hide"',
    ('made_with = ' + (Json-Str $made)),
    'author = "project member"',
    'expect_dropped = []',
    'notes = "before-state for hide, unhide and select-all size edits: custom, default, hidden and zero-sized columns and rows"'
  )
  [IO.File]::WriteAllText($toml, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  Write-Host "grid-hide-oracle: wrote $path"
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

function Read-Edits($app, [string]$path) {
  $out = @()
  for ($i = 0; $i -lt $scenarios.Count; $i++) {
    $wb = $app.Workbooks.Open($path)
    try {
      Apply-Edit $wb.Worksheets.Item($i + 1) $scenarios[$i]
      $tmp = Join-Path ([IO.Path]::GetTempPath()) ('o365-hide-' + [Guid]::NewGuid().ToString('N') + '.xlsx')
      $wb.SaveCopyAs($tmp)
    } finally { $wb.Close($false) }
    $xml = Sheet-Xml $tmp ($i + 1)
    Remove-Item -LiteralPath $tmp -ErrorAction SilentlyContinue
    $fmt = [regex]::Match($xml, '<sheetFormatPr[^>]*/?>').Value
    $cols = [regex]::Match($xml, '<cols>.*?</cols>').Value
    $rows = ([regex]::Matches($xml, '<row [^>]*>') | ForEach-Object { $_.Value })
    $rowJson = ($rows | ForEach-Object { '      ' + (Json-Str $_) }) -join ",`n"
    $out += ('    ' + (Json-Str $scenarios[$i]) + ': {' + "`n" +
      '      "sheetFormatPr": ' + (Json-Str $fmt) + ",`n" +
      '      "cols": ' + (Json-Str $cols) + ",`n" +
      '      "rows": [' + "`n" + $rowJson + "`n      ]`n    }")
  }
  return $out
}

try {
  Start-Excel
  $path = Join-Path $fixtureDir 'grid-hide.xlsx'
  if ($MakeFixture) { Write-Fixture $script:xl $path }
  $edits = Read-Edits $script:xl $path
  $lines = @(
    '{',
    '  "source": "fixtures/xlsx/grid-hide.xlsx",',
    '  "oracle": "Microsoft Excel via COM: XML of sheetFormatPr, cols and row start tags after the scenario edit, SaveCopyAs",',
    ('  "excel_build": ' + (Json-Str ([string]$script:xl.Build)) + ','),
    '  "scenarios": {'
  )
  $lines += $edits -join ",`n"
  $lines += '  }'
  $lines += '}'
  $outDir = Join-Path $refDir 'grid-hide'
  New-Item -ItemType Directory -Force -Path $outDir | Out-Null
  [IO.File]::WriteAllText((Join-Path $outDir 'xml.json'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
  Write-Host "grid-hide-oracle: recorded $($scenarios.Count) scenarios"
}
finally {
  Stop-Excel
}
