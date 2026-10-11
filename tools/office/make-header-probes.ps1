<#
Generates the empty header and footer probes (fixtures/docx/layout-hdr-*.docx + .toml) by driving the locally installed Microsoft Word over COM.
Question: a section without header or footer part, header distance larger than the top margin (and footer distance larger than the bottom
margin): does Word still reserve the height of an empty header paragraph? The reference JSON comes from tools/office/word-layout-oracle.ps1:

  pwsh -NoProfile -File tools/office/make-header-probes.ps1
  pwsh -NoProfile -File tools/office/word-layout-oracle.ps1 -Path fixtures/docx/layout-hdr-none.docx   # one per probe

LOCAL DEVELOPER TOOL ONLY (Microsoft does not support unattended server-side Office automation).
Starts its own Word instance (AutomationSecurity = force-disable macros), never attaches to or kills another one,
quits it in a finally block, and stops only the WINWORD processes it started. Document author and properties are
generic. All text is original. Nothing outside the repository is read or written (the Word user name is restored).
#>
param([string[]]$Only = @())
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$outDir = Join-Path $root 'fixtures\docx'
New-Item -ItemType Directory -Force $outDir | Out-Null
$created = New-Object System.Collections.Generic.List[string]
$skipped = New-Object System.Collections.Generic.List[string]
function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }

$wordBefore = Get-Pids 'WINWORD'
$word = $null; $oldUser = $null; $oldInit = $null; $oldLocal = $null; $wordPids = @()
try {
  $word = New-Object -ComObject Word.Application
  $wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $wordBefore })
  if ($wordPids.Count -eq 0) { throw 'COM attached to a Word instance this script did not start; refusing to touch it' }
  $word.Visible = $false; $word.DisplayAlerts = 0
  $word.AutomationSecurity = 3   # msoAutomationSecurityForceDisable
  $oldUser = $word.UserName; $oldInit = $word.UserInitials; $oldLocal = $word.Options.UseLocalUserInfo
  $word.Options.UseLocalUserInfo = $true
  $word.UserName = 'project member'; $word.UserInitials = 'PM'

  # Calibri 11, single spacing, 0 before / 0 after, Letter, 36 pt top and bottom margins, 72 pt left and right.
  function Init-Doc($hdist, $fdist) {
    $d = $script:doc
    $n = $d.Styles.Item('Normal')
    $n.Font.Name = 'Calibri'; $n.Font.Size = 11
    $n.ParagraphFormat.SpaceBefore = 0; $n.ParagraphFormat.SpaceAfter = 0; $n.ParagraphFormat.LineSpacingRule = 0
    $ps = $d.PageSetup
    $ps.PageWidth = 612; $ps.PageHeight = 792
    $ps.TopMargin = 36; $ps.BottomMargin = 36; $ps.LeftMargin = 72; $ps.RightMargin = 72
    $ps.HeaderDistance = $hdist; $ps.FooterDistance = $fdist
  }
  function Body($n) {
    $s = $script:word.Selection
    for ($i = 1; $i -le $n; $i++) { [void]$s.EndKey(6); $s.TypeText("Line $i of the plain body text."); $s.TypeParagraph() }
  }

  function C-Fixture($name, $notes, $hdist, $fdist, [scriptblock]$build) {
    $doc = $null; $path = Join-Path $script:outDir "$name.docx"
    if ($script:Only.Count -gt 0 -and $name -notin $script:Only) { return }
    try {
      if (Test-Path $path) { Remove-Item $path -Force }
      $doc = $script:word.Documents.Add()
      $script:doc = $doc
      Init-Doc $hdist $fdist
      & $build
      $doc.RemovePersonalInformation = $true
      $doc.SaveAs2($path, 16)
      Set-Content -Path (Join-Path $script:outDir ("$name.toml")) -Value ("feature = `"layout-hdr`"
made_with = `"$($script:word.Name) $($script:word.Build) (Windows)`"
author = `"project member`"
expect_dropped = []
notes = `"$notes`"
") -Encoding utf8 -NoNewline
      $script:created.Add($name)
    } catch {
      $script:skipped.Add("$name : $($_.Exception.Message) @ $($_.InvocationInfo.PositionMessage -replace '\s+', ' ')")
      Remove-Item $path, (Join-Path $script:outDir "$name.toml") -Force -ErrorAction SilentlyContinue
    } finally {
      if ($doc) { try { $doc.Close(0) } catch {}; Release $doc }
      $script:doc = $null
    }
  }

  C-Fixture 'layout-hdr-none' 'no header or footer part, header and footer distance 72 pt, margins 36 pt, Calibri 11 plain lines' 72 72 { Body 120 }
  C-Fixture 'layout-hdr-none-near' 'no header or footer part, distances 30 pt, margins 36 pt' 30 30 { Body 120 }
  C-Fixture 'layout-hdr-none-style' 'no header or footer part, distances 72 pt, Header and Footer styles 20 pt Calibri' 72 72 {
    foreach ($sn in 'Header', 'Footer') { $st = $script:doc.Styles.Item($sn); $st.Font.Name = 'Calibri'; $st.Font.Size = 20 }
    Body 120
  }
  C-Fixture 'layout-hdr-none-normal' 'no header or footer part, distances 72 pt, Normal 24 pt Calibri' 72 72 {
    $script:doc.Styles.Item('Normal').Font.Size = 24
    Body 60
  }
  C-Fixture 'layout-hdr-none-big' 'no header or footer part, distances 200 pt, margins 36 pt' 200 200 { Body 120 }

} finally {
  if ($word) {
    try { $word.UserName = $oldUser; $word.UserInitials = $oldInit; $word.Options.UseLocalUserInfo = $oldLocal } catch {}
    try { $word.Quit(0) } catch {}; Release $word
  }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($p in $wordPids) { Get-Process -Id $p -ErrorAction SilentlyContinue | Stop-Process -Force }
}
Write-Output "created: $($created -join ', ')"
if ($skipped.Count) { Write-Output 'SKIPPED:'; $skipped | ForEach-Object { Write-Output "  $_" }; exit 1 }
