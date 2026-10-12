<#
Creates input.xlsx for the table-style probe through Excel: for each built-in style in $styles a 9 x 5 table at
B(2+12i) and the same table at J(2+12i) whose column L carries borders of its own (red, all four edges of one cell) at
data rows 2, 4, 6, 8 with thin, medium, hair and double. Then run ../styles-border-colors/measure.ps1 -Name table-styles.
LOCAL DEVELOPER TOOL ONLY. Own hidden Excel; aborts if COM returned a running instance; stops only its own PID.
#>
$ErrorActionPreference = 'Stop'
$styles = 'TableStyleLight1', 'TableStyleLight9', 'TableStyleLight15', 'TableStyleMedium2', 'TableStyleMedium9', 'TableStyleDark1'
$out = Join-Path $PSScriptRoot 'input.xlsx'
Add-Type -Namespace Native -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
'@
$existing = @(Get-Process -Name EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$app = New-Object -ComObject Excel.Application
$procId = [uint32]0
[void][Native.Win32]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
if ($existing -contains [int]$procId) { throw "COM returned the already running Excel $procId; aborting." }
Write-Host "excel pid $procId"
try {
  $app.Visible = $false; $app.DisplayAlerts = $false; $app.AutomationSecurity = 3
  $wb = $app.Workbooks.Add(); $ws = $wb.Worksheets.Item(1)
  $own = @{ 2 = 1; 4 = -4138; 6 = 1; 8 = -4119 }   # data row -> weight/line marker below
  for ($i = 0; $i -lt $styles.Count; $i++) {
    $top = 2 + 12 * $i
    foreach ($c0 in 2, 10) {
      for ($j = 0; $j -lt 5; $j++) { $ws.Cells.Item($top, $c0 + $j).Value2 = 'H' + ($j + 1) }
      for ($r = 1; $r -le 8; $r++) { for ($j = 0; $j -lt 5; $j++) { $ws.Cells.Item($top + $r, $c0 + $j).Value2 = [double]($r * 10 + $j) } }
      $rng = $ws.Range($ws.Cells.Item($top, $c0), $ws.Cells.Item($top + 8, $c0 + 4))
      $lo = $ws.ListObjects.Add(1, $rng, $null, 1)
      $lo.TableStyle = $styles[$i]
    }
    # own borders in the second table, column L (data rows 2, 4, 6, 8)
    $spec = @{ 2 = @(1, 2); 4 = @(1, -4138); 6 = @(1, 1); 8 = @(-4119, 4) }
    foreach ($r in 2, 4, 6, 8) {
      $cell = $ws.Cells.Item($top + $r, 12)
      foreach ($b in 7, 8, 9, 10) {
        $cell.Borders.Item($b).LineStyle = $spec[$r][0]
        if ($spec[$r][0] -eq 1) { $cell.Borders.Item($b).Weight = $spec[$r][1] }
        $cell.Borders.Item($b).Color = 255
      }
    }
  }
  if (Test-Path $out) { Remove-Item $out }
  $wb.SaveAs($out, 51); $wb.Close($false)
} finally {
  try { $app.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
  $deadline = (Get-Date).AddSeconds(10)
  while ((Get-Date) -lt $deadline -and (Get-Process -Id $procId -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
  $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
  if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue }
}
