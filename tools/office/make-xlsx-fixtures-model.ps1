<#
Creates the workbook-model fixtures fixtures/xlsx/model-*.xlsx (tables, defined names, legacy array
formulas, shared formulas) by driving the locally installed Microsoft Excel over COM. Excel writes the
files; nothing is hand-edited. No personal data: RemovePersonalInformation is set and the user name is
replaced while the script runs.

LOCAL DEVELOPER TOOL ONLY (not CI). Starts its own hidden Excel and only stops that process.
After running, regenerate the references the crates/xlsx tests require:
  pwsh -NoProfile -File tools/office/grid-geometry-oracle.ps1 -Only '^model-'
  pwsh -NoProfile -File tools/office/office-oracle.ps1 -Mode reference -Path fixtures/xlsx/model-tables.xlsx,fixtures/xlsx/model-arrays.xlsx
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$dir = Join-Path $root 'fixtures\xlsx'
New-Item -ItemType Directory -Force $dir | Out-Null
function Get-Pids { @(Get-Process -Name EXCEL -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }
function Write-Manifest($name, $feature, $app, $notes) {
  $t = "feature = `"$feature`"`nmade_with = `"$($app.Name) $($app.Build) (Windows)`"`nauthor = `"project member`"`nexpect_dropped = []`nnotes = `"$notes`"`n"
  Set-Content -Path (Join-Path $dir "$name.toml") -Value $t -Encoding utf8 -NoNewline
}
$before = Get-Pids; $xl = $null; $old = $null; $mine = @()
try {
  $xl = New-Object -ComObject Excel.Application
  $xl.Visible = $false; $xl.DisplayAlerts = $false
  $old = $xl.UserName; $xl.UserName = 'project member'
  $mine = @(Get-Pids | Where-Object { $_ -notin $before })

  # ---- model-tables: two tables, calculated column, totals row, defined names ----
  $name = 'model-tables'; $path = Join-Path $dir "$name.xlsx"
  if (Test-Path $path) { Remove-Item $path -Force }
  $wb = $xl.Workbooks.Add(); $ws = $wb.Worksheets.Item(1); $ws.Name = 'Data'
  $ws.Range('A1').Value2 = 'Item'; $ws.Range('B1').Value2 = 'Qty'; $ws.Range('C1').Value2 = 'Price'; $ws.Range('D1').Value2 = 'Total'
  $items = 'pen', 'ink', 'pad', 'cap', 'tab'
  for ($i = 0; $i -lt 5; $i++) {
    $r = $i + 2
    $ws.Cells.Item($r, 1).Value2 = $items[$i]; $ws.Cells.Item($r, 2).Value2 = [double]($i + 1); $ws.Cells.Item($r, 3).Value2 = [double](($i + 1) * 1.5)
  }
  $lo = $ws.ListObjects.Add(1, $ws.Range('A1:D6'), [Type]::Missing, 1)
  $lo.Name = 'SalesTbl'; $lo.TableStyle = 'TableStyleMedium2'
  $ws.Range('D2').Formula = '=[@Qty]*[@Price]'
  $lo.ShowTotals = $true
  $lo.ListColumns.Item(2).TotalsCalculation = 1   # xlTotalsCalculationSum
  $lo.ListColumns.Item(4).TotalsCalculation = 1
  $lo.ListColumns.Item(1).TotalsCalculation = 0   # none (label cell stays empty)
  $s2 = $wb.Worksheets.Add([Type]::Missing, $ws); $s2.Name = 'Other'
  $s2.Range('A1').Value2 = 'Key'; $s2.Range('B1').Value2 = 'Val'
  $s2.Range('A2').Value2 = 'a'; $s2.Range('B2').Value2 = 10.0; $s2.Range('A3').Value2 = 'b'; $s2.Range('B3').Value2 = 20.0
  $lo2 = $s2.ListObjects.Add(1, $s2.Range('A1:B3'), [Type]::Missing, 1)
  $lo2.Name = 'Lookup'; $lo2.TableStyle = 'TableStyleLight9'; $lo2.ShowTableStyleRowStripes = $false; $lo2.ShowTableStyleFirstColumn = $true
  $s2.Range('D1').Formula = '=SUM(SalesTbl[Total])+SUM(Lookup[Val])'
  [void]$wb.Names.Add('TaxRate', '=0.2')
  $n = $wb.Names.Add('SecretBase', '=Data!$B$2'); $n.Visible = $false
  $n2 = $s2.Names.Add('LocalBase', '=Other!$B$2'); $n2.Comment = 'sheet scoped note'
  $wb.RemovePersonalInformation = $true
  $wb.SaveAs($path, 51); Write-Manifest $name $name $xl 'two tables, calculated column, totals row, global/hidden/sheet-scoped names with comment'
  $wb.Close($false); Release $wb

  # ---- model-arrays: legacy array formulas and shared formulas ----
  $name = 'model-arrays'; $path = Join-Path $dir "$name.xlsx"
  if (Test-Path $path) { Remove-Item $path -Force }
  $wb = $xl.Workbooks.Add(); $ws = $wb.Worksheets.Item(1); $ws.Name = 'Calc'
  1..3 | ForEach-Object { $ws.Cells.Item(1, $_).Value2 = [double]$_; $ws.Cells.Item(2, $_).Value2 = [double]($_ * 10) }
  $ws.Range('D4:F4').FormulaArray = '=A1:C1*2'
  $ws.Range('A6:A8').FormulaArray = '=TRANSPOSE(A1:C1)'
  $ws.Range('B10').FormulaArray = '=SUM(A1:C1*A2:C2)'
  1..6 | ForEach-Object { $ws.Cells.Item($_ + 11, 1).Value2 = [double]$_ }
  $ws.Range('B12:B17').Formula = '=A12*2+1'
  $wb.RemovePersonalInformation = $true
  $wb.SaveAs($path, 51); Write-Manifest $name $name $xl 'row array D4:F4, column array A6:A8, single-cell array B10, shared formula B12:B17'
  $wb.Close($false); Release $wb
  Write-Output 'created model-tables, model-arrays'
} finally {
  if ($xl) { try { $xl.UserName = $old } catch {}; try { $xl.Quit() } catch {}; Release $xl }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($p in $mine) { Get-Process -Id $p -ErrorAction SilentlyContinue | Stop-Process -Force }
}
