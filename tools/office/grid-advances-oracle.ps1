<#
Measures Excel's per-character advance widths through COM AutoFit (clean-room: only Excel's public
object model is used). LOCAL DEVELOPER TOOL ONLY; never run in CI.

For each font/size and each printable ASCII character c it autofits a column holding "H" and then
"H" + c*20. Range.Width is in points; pixels = points * dpi / 72 (exact, whole pixels at 120 dpi).
advance_px = (W(H + c*20) - W(H)) / 20. Output: fixtures/reference/xlsx/grid-advances/advances.json
(ASCII, LF, no user data).

  pwsh -NoProfile -File tools/office/grid-advances-oracle.ps1

Process safety: own hidden Excel instance, AutomationSecurity=3, only that process is stopped (finally).
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$refDir = Join-Path $root 'fixtures/reference/xlsx/grid-advances'
$inv = [Globalization.CultureInfo]::InvariantCulture
Add-Type -Namespace Native -Name Win32b -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(System.IntPtr hWnd, out uint processId);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern uint GetDpiForWindow(System.IntPtr hWnd);
'@
$xl = $null; $xlPid = 0
try {
  $xl = New-Object -ComObject Excel.Application
  $xl.AutomationSecurity = 3
  $xl.Visible = $false; $xl.DisplayAlerts = $false; $xl.ScreenUpdating = $false; $xl.EnableEvents = $false
  $p = [uint32]0
  [void][Native.Win32b]::GetWindowThreadProcessId([IntPtr]$xl.Hwnd, [ref]$p)
  $xlPid = [int]$p
  $dpi = [int][Native.Win32b]::GetDpiForWindow([IntPtr]$xl.Hwnd)
  $wb = $xl.Workbooks.Add()
  $ws = $wb.Worksheets.Item(1)
  $ws.Cells.NumberFormat = '@'
  $cases = @(@('Calibri',9),@('Calibri',11),@('Calibri',14),@('Calibri',20),@('Cambria',11),@('Cambria',14))
  $out = New-Object System.Collections.Generic.List[string]
  foreach ($case in $cases) {
    $ws.Cells.Font.Name = $case[0]; $ws.Cells.Font.Size = $case[1]
    $cell = $ws.Cells.Item(1,1)
    $fit = { param($t) $cell.Value2 = $t; [void]$ws.Columns.Item(1).AutoFit(); [math]::Round($ws.Range('A1').Width * $dpi / 72) }
    $base = & $fit 'H'
    $zero20 = & $fit ('0' * 20)
    $zero1 = & $fit '0'
    $hh = & $fit 'HH'
    $rows = New-Object System.Collections.Generic.List[string]
    for ($code = 32; $code -le 126; $code++) {
      $c = [string][char]$code
      $w = & $fit ('H' + ($c * 20))
      $rows.Add("{`"c`":$code,`"w20`":$w}")
    }
    $out.Add("    {`"font`":`"$($case[0])`",`"size`":$($case[1]),`"base_px`":$base,`"zero1_px`":$zero1,`"zero20_px`":$zero20,`"hh_px`":$hh,`"chars`":[" + ($rows -join ',') + "]}")
  }
  $json = "{`n  `"oracle`": `"Microsoft Excel via COM: Columns(1).AutoFit() on H + c*20 vs H`",`n  `"dpi`": $dpi,`n  `"cases`": [`n" + ($out -join ",`n") + "`n  ]`n}`n"
  [IO.File]::WriteAllText((Join-Path $refDir 'advances.json'), $json.Replace("`r`n","`n"), (New-Object Text.UTF8Encoding $false))
}
finally {
  if ($xl) {
    try { foreach ($b in @($xl.Workbooks)) { $b.Saved = $true; $b.Close($false) } } catch {}
    try { $xl.Quit() } catch {}
    try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($xl) } catch {}
  }
  if ($xlPid -gt 0) {
    Start-Sleep -Seconds 2
    $pr = Get-Process -Id $xlPid -ErrorAction SilentlyContinue
    if ($pr -and $pr.ProcessName -eq 'EXCEL') { Stop-Process -Id $xlPid -Force -ErrorAction SilentlyContinue }
  }
}
