<#
Word PDF reference exporter: writes what Word's own "Save as PDF" makes of each fixture, so that the
glyph positions of our PDF export (crates/pdf) can be compared with Word's.

LOCAL DEVELOPER TOOL ONLY. Microsoft does not support unattended server-side Office automation, so this is never run
in CI. CI consumes the committed output: fixtures/reference/pdf/<name>.pdf (read by crates/pdf/tests/word_geometry.rs).

  pwsh -NoProfile -File tools/office/make-pdf-refs.ps1                              # every fixture with a layout reference
  pwsh -NoProfile -File tools/office/make-pdf-refs.ps1 -Name layout-align,fields   # chosen fixtures
  pwsh -NoProfile -File tools/office/make-pdf-refs.ps1 -DocxDir fixtures/reference/pdf/probes -OutDir fixtures/reference/pdf/probes
                                                                                    # the tagging probes (every .docx in the folder)

Only generated fixtures (fixtures/docx) are exported, never third-party documents. Each document is opened read-only
(never saved) in a Word instance this script starts; the script refuses to run if COM attaches to an existing
instance, forces macros off (AutomationSecurity = 3), quits its own instance in `finally` and kills only its own PID.
Export options are Word's defaults for "Save as PDF": print quality, headings as bookmarks, document structure tags,
no document properties, no ISO 19005-1.
#>
param(
  [string[]]$Name,
  [string]$OutDir,
  # Export every .docx of this folder instead of the fixtures that have a layout reference.
  [string]$DocxDir
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$docxDir = if ($DocxDir) { [IO.Path]::GetFullPath($DocxDir) } else { Join-Path $root 'fixtures\docx' }
$layoutDir = Join-Path $root 'fixtures\reference\layout'
$outDir = if ($OutDir) { [IO.Path]::GetFullPath($OutDir) } else { Join-Path $root 'fixtures\reference\pdf' }
New-Item -ItemType Directory -Force $outDir | Out-Null

function Get-Pids($proc) { @(Get-Process -Name $proc -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }

if ($Name) { $names = @($Name | ForEach-Object { $_ -split ',' } | Where-Object { $_ }) }
elseif ($DocxDir) {
  $names = Get-ChildItem -LiteralPath $docxDir -Filter *.docx -File | Sort-Object Name | ForEach-Object { $_.BaseName }
}
else {
  $names = Get-ChildItem -LiteralPath $layoutDir -Filter *.json -File |
    Where-Object { $_.BaseName -notlike '*.lists' } | Sort-Object Name | ForEach-Object { $_.BaseName }
}

$wordBefore = Get-Pids 'WINWORD'
$word = $null; $wordPids = @()
$done = New-Object System.Collections.Generic.List[string]
$skipped = New-Object System.Collections.Generic.List[string]
try {
  $word = New-Object -ComObject Word.Application
  $wordPids = @(Get-Pids 'WINWORD' | Where-Object { $_ -notin $wordBefore })
  if ($wordPids.Count -eq 0) {
    $word = $null  # not ours: the `finally` below must neither quit nor kill it
    throw 'COM attached to a Word instance this script did not start; refusing to touch it'
  }
  $word.Visible = $false; $word.DisplayAlerts = 0
  $word.AutomationSecurity = 3  # msoAutomationSecurityForceDisable
  foreach ($n in $names) {
    $docx = Join-Path $docxDir "$n.docx"
    if (-not (Test-Path -LiteralPath $docx)) { $skipped.Add("$n : no $docx"); continue }
    $doc = $null
    try {
      # FileName, ConfirmConversions = false, ReadOnly = true, AddToRecentFiles = false
      $doc = $word.Documents.Open([string]$docx, [ref]$false, [ref]$true, [ref]$false)
      $pdf = Join-Path $outDir "$n.pdf"
      # OutputFileName, ExportFormat 17 = PDF, OpenAfterPublish, OptimizeFor 0 = print, Range 0 = whole document,
      # From, To, Item 0 = content only, IncludeDocProperties, KeepIRM, CreateBookmarks 1 = headings,
      # DocStructureTags, BitmapMissingFonts, UseISO19005_1
      $doc.ExportAsFixedFormat($pdf, 17, $false, 0, 0, 1, 1, 0, $false, $true, 1, $true, $true, $false)
      $done.Add($n)
    } catch { $skipped.Add("$n : $($_.Exception.Message.Trim())") }
    finally { if ($doc) { try { $doc.Close(0) } catch {}; Release $doc } }
  }
} finally {
  if ($word) { try { $word.Quit(0) } catch {}; Release $word }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  foreach ($id in $wordPids) { Get-Process -Id $id -ErrorAction SilentlyContinue | Stop-Process -Force }
}
Write-Output ("exported: {0} file(s) to {1}" -f $done.Count, $outDir)
if ($skipped.Count) { Write-Output 'SKIPPED:'; $skipped | ForEach-Object { Write-Output "  $_" }; exit 1 }
