<#
Reads the definitions of Excel's built-in table styles (TableStyleLight1-21, Medium1-28, Dark1-11) through the TableStyles
object model and writes table_styles.txt next to this script (reference data for a table-style layer; not yet read by the crate).
LOCAL DEVELOPER TOOL ONLY (not CI).

Line format (UTF-8, LF):  <style>|<element>|<stripe>|<bold>|<italic>|<font colour>|<fill pattern>|<fill colour>|<border>...
  element  XlTableStyleElementType 0-12 (0 whole table, 1 header row, 2 total row, 3 first column, 4 last column,
           5/6 row stripes, 7/8 column stripes, 9-12 first/last header/total cell)
  colour   t<ooxml theme index>:<tint> or r<RRGGBB>, empty when unset
  border   six fields left,top,bottom,right,insideV,insideH: <line style>:<weight>:<colour>, empty when unset
Only elements with at least one property are written.

Process safety: own hidden Excel; aborts if COM returned a running instance; quits it and stops only its PID.
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)))
$out = Join-Path $PSScriptRoot 'table_styles.txt'
$inv = [Globalization.CultureInfo]::InvariantCulture
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
$themeMap = @{ 1 = 0; 2 = 1; 3 = 2; 4 = 3; 5 = 4; 6 = 5; 7 = 6; 8 = 7; 9 = 8; 10 = 9; 11 = 10; 12 = 11 }
function Nn($x) { if ($x -is [DBNull]) { return $null }; return $x }
function Col($o) {
  $t = $null; try { $t = Nn $o.ThemeColor } catch {}
  if ($t -is [int] -and $t -gt 0) { return 't' + $themeMap[$t] + ':' + ([double](Nn $o.TintAndShade)).ToString('R', $inv) }
  $c = $null; try { $c = Nn $o.Color } catch {}
  if ($null -ne $c) { $v = [int]$c; return 'r' + ('{0:X2}{1:X2}{2:X2}' -f ($v -band 255), (($v -shr 8) -band 255), (($v -shr 16) -band 255)) }
  return ''
}
try {
  $app.Visible = $false; $app.AutomationSecurity = 3; $app.DisplayAlerts = $false
  $wb = $app.Workbooks.Add()
  $lines = New-Object Collections.ArrayList
  $names = @()
  foreach ($s in $wb.TableStyles) { if ($s.Name -like 'TableStyle*') { $names += $s.Name } }
  foreach ($n in $names) {
    $s = $wb.TableStyles.Item($n)
    for ($i = 0; $i -le 12; $i++) {
      $e = $s.TableStyleElements.Item($i)
      $bold = Nn $e.Font.Bold; $ital = Nn $e.Font.Italic
      $fc = ''; if ($null -ne $bold -or $null -ne $ital -or (Nn $e.Font.ThemeColor)) { $fc = Col $e.Font }
      $pat = Nn $e.Interior.Pattern; $fill = ''
      if ($pat) { $fill = Col $e.Interior }
      $bs = @()
      foreach ($bi in 7, 8, 9, 10, 11, 12) {
        $b = $e.Borders.Item($bi); $ls = Nn $b.LineStyle
        if ($null -ne $ls -and $ls -ne -4142) { $bs += ("$ls" + ':' + $b.Weight + ':' + (Col $b)) } else { $bs += '' }
      }
      $stripe = 1; if ($i -in 5, 6, 7, 8) { $stripe = Nn $e.StripeSize }
      $b2 = ''; if ($null -ne $bold) { $b2 = [int][bool]$bold }
      $i2 = ''; if ($null -ne $ital) { $i2 = [int][bool]$ital }
      $any = ($b2 -ne '') -or ($i2 -ne '') -or $pat -or ($bs -join '') -or ($fc -ne '')
      if ($any) { [void]$lines.Add(($n, $i, $stripe, $b2, $i2, $fc, $pat, $fill, ($bs -join '|')) -join '|') }
    }
  }
  [IO.File]::WriteAllText($out, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding($false)))
  Write-Host "$($names.Count) styles, $($lines.Count) element lines -> $out"
} finally {
  try { foreach ($w in @($app.Workbooks)) { $w.Saved = $true; $w.Close($false) } } catch {}
  try { $app.Quit() } catch {}
  try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) } catch {}
  $deadline = (Get-Date).AddSeconds(10)
  while ((Get-Date) -lt $deadline -and (Get-Process -Id $procId -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 200 }
  $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
  if ($p -and $p.ProcessName -eq 'EXCEL') { Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue }
}
