<#
Reads the definitions of Excel's built-in PivotTable styles (PivotStyleLight1-28, Medium1-28, Dark1-28) through the
TableStyles object model and writes crates/pivot/src/builtin_styles.txt, the data table behind crates/pivot/src/style.rs.
With -Kind Table it reads the built-in worksheet table styles (TableStyleLight1-21, Medium1-28, Dark1-11), which a PivotTable
can use too, into crates/pivot/src/builtin_table_styles.txt (same line format; table styles define elements 0-12 only).

LOCAL DEVELOPER TOOL ONLY (not CI). Starts its own Excel instance; aborts when COM attached to an instance that was
already running; AutomationSecurity = 3; Quit in finally. Other EXCEL processes are never touched.

Line format (UTF-8, LF):  <style>|<element>|<stripe>|<bold>|<italic>|<font colour>|<fill pattern>|<fill colour>|<border>...
  element  XlTableStyleElementType 0-27 (0 whole table, 1 header row, 2 total row, 3 first column, 4 last column,
           5/6 row stripes, 7/8 column stripes, 9-12 first/last header/total cell, 13-15 subtotal columns,
           16-18 subtotal rows, 19-21 row subheadings, 22-24 column subheadings, 25 blank row, 26/27 page field
           labels/values)
  colour   t<ooxml theme index>:<tint> or r<RRGGBB>, empty when unset
  border   six fields left,top,bottom,right,insideV,insideH: <line style>:<weight>:<colour>, empty when unset
Only elements with at least one property are written.
#>
param([ValidateSet('Pivot', 'Table')][string]$Kind = 'Pivot')
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$out = Join-Path $root $(if ($Kind -eq 'Table') { 'crates/pivot/src/builtin_table_styles.txt' } else { 'crates/pivot/src/builtin_styles.txt' })
$inv = [Globalization.CultureInfo]::InvariantCulture
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class PvDefWin { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p); }
'@
$before = @(Get-Process EXCEL -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$app = New-Object -ComObject Excel.Application
$procId = [uint32]0
[void][PvDefWin]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$procId)
if ($before -contains [int]$procId) { Write-Host 'attached to a pre-existing Excel instance; aborting without touching it'; exit 1 }
Write-Host "EXCEL PID $procId"
$xlPid = [int]$procId
# COM theme colour index -> OOXML theme index (measured: COM 1 is white = lt1, COM 2 is black = dk1)
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
  $app.AutomationSecurity = 3; $app.DisplayAlerts = $false
  $wb = $app.Workbooks.Add()
  $lines = New-Object Collections.ArrayList
  $names = @()
  foreach ($s in $wb.TableStyles) { if ($s.Name -like "${Kind}Style*") { $names += $s.Name } }
  foreach ($n in $names) {
    $s = $wb.TableStyles.Item($n)
    for ($i = 0; $i -le 27; $i++) {
      try { $e = $s.TableStyleElements.Item($i) } catch { continue }
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
  Start-Sleep -Seconds 2
  if (Get-Process -Id $xlPid -ErrorAction SilentlyContinue) { Stop-Process -Id $xlPid -Force }
}
