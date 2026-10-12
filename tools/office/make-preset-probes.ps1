<#
Preset-geometry probes: one DrawingML preset shape per page of a .docx, at several sizes and adjust values, with no
fill, a 0.75 pt black line and no text. Word then exports the document to PDF (tools/office/make-pdf-refs.ps1), and the
vector paths in that PDF are the measurement the preset formulas are written from (clean-room method,
docs/architecture/drawing.md). The Rust side is crates/drawing/tests/preset_probes.rs.

  pwsh -NoProfile -File tools/office/make-preset-probes.ps1 -Batch pilot        # rect roundRect ellipse triangle rightArrow
  pwsh -NoProfile -File tools/office/make-preset-probes.ps1 -Batch b03          # a batch of manifest.json
  pwsh -NoProfile -File tools/office/make-preset-probes.ps1 -Presets star5,heart -Batch mine
  pwsh -NoProfile -File tools/office/make-preset-probes.ps1 -Batch b03 -NoExport   # write the .docx and .probes.json only

Outputs, in fixtures/reference/drawing/presets (or -OutDir):
  <batch>.docx         the probe document (generic metadata, no docProps part)
  <batch>.probes.json  one entry per page: preset, kind, shape size in EMU, adjust values set
  <batch>.pdf          Word's export
  <batch>.handles.json Word's adjust count and default values per preset
  <batch>.run.json     start time, elapsed seconds, dropped pages, the Word PIDs this run started

If Word refuses the document, every page is opened on its own and the refused pages are dropped (and listed in
probes.json `dropped`), then the export runs again.

Stage 1 (discover): Word opens a one-shape-per-page document with default adjusts and the script reads
Shape.Adjustments (count and default values, in Word's own units) into <batch>.handles.json. Word REFUSES to open a
document whose avLst names an adjust the preset does not have, so the names for stage 2 come from the count: one adjust
is `adj`, n adjusts are adj1 .. adjn. Presets that name theirs otherwise (adj, hf, vf) are found by trying those names
when their pages are refused, and remembered in <OutDir>/adjnames.json (`{"preset": ["name", ...]}`; edit it by hand for
any other case). An existing handles.json that covers the batch is reused unless -Rediscover.

Pages per preset (4 + 5 + n for n adjusts): four sizes at default adjusts (sq 144x144 pt, wide 288x96, tall 96x288,
box 216x144); on the 216x144 box all adjusts set together to 0, 30000, 100000, -50000 and 400000 (out of range); and
one "scan" page per adjust setting that one to 0 and the rest to Word's default, which shows what each adjust does on its own. A 24-preset
batch is a few hundred pages; the pilot batch of 5 shows the run time.

The shape sits at (72 pt, 72 pt) from the page corner (an anchored, wrap-none shape), so the box origin is known and the
PDF coordinates can be normalised to the box.

LOCAL DEVELOPER TOOL ONLY (Microsoft does not support unattended server-side Office automation); never run in CI.
Word is started by make-pdf-refs.ps1, which refuses to attach to a running instance and kills only its own PID. This
script records the WINWORD PIDs that appear while the export runs and never touches any other WINWORD process.
-Fill draws a solid fill too (paths with stroke="0" are only visible that way).
#>
param(
  [string]$Batch = 'pilot',
  [string[]]$Presets,
  [string]$OutDir,
  [switch]$NoExport,
  [switch]$Rediscover,
  [switch]$Fill
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$outDir = if ($OutDir) { [IO.Path]::GetFullPath($OutDir) } else { Join-Path $root 'fixtures\reference\drawing\presets' }
New-Item -ItemType Directory -Force $outDir | Out-Null

if ($Presets) { $names = @($Presets | ForEach-Object { $_ -split ',' } | Where-Object { $_ }) }
else {
  $man = Get-Content -Raw (Join-Path $outDir 'manifest.json') | ConvertFrom-Json
  $names = if ($Batch -eq 'pilot') { @($man.pilot) } else { @($man.batches.$Batch) }
  if ($names.Count -eq 0) { throw "no presets for batch '$Batch'" }
}

function Get-Pids { @(Get-Process -Name 'WINWORD' -ErrorAction SilentlyContinue | ForEach-Object Id) }
function Release($o) { if ($o) { try { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) } catch {} } }

$PT = 12700
$ns = 'xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" ' +
  'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" ' +
  'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" ' +
  'xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"'
$fillXml = if ($Fill) { '<a:solidFill><a:srgbClr val="C0C0C0"/></a:solidFill>' } else { '<a:noFill/>' }

# Writes a one-shape-per-page document; $pages holds @{ page; preset; kind; w_emu; h_emu; adj (ordered name -> value) }.
function Write-ProbeDocx($path, $pages) {
  $body = New-Object System.Text.StringBuilder
  foreach ($pg in $pages) {
    $av = ($pg.adj.Keys | ForEach-Object { '<a:gd name="{0}" fmla="val {1}"/>' -f $_, $pg.adj[$_] }) -join ''
    $brk = if ($pg.page -gt 1) { '<w:pageBreakBefore/>' } else { '' }
    $frag = ('<w:p><w:pPr>{0}<w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing>' +
      '<wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="251659264" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">' +
      '<wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionH>' +
      '<wp:positionV relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionV>' +
      '<wp:extent cx="{1}" cy="{2}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/>' +
      '<wp:docPr id="{3}" name="probe {3}"/><wp:cNvGraphicFramePr/>' +
      '<a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr/>' +
      '<wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{1}" cy="{2}"/></a:xfrm><a:prstGeom prst="{4}"><a:avLst>{5}</a:avLst></a:prstGeom>' +
      '{6}<a:ln w="9525"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic>' +
      '</wp:anchor></w:drawing></mc:Choice></mc:AlternateContent></w:r></w:p>') -f $brk, $pg.w_emu, $pg.h_emu, $pg.page, $pg.preset, $av, $fillXml
    [void]$body.Append($frag)
  }
  $doc = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document ' + $ns + '><w:body>' + $body.ToString() +
    '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>'
  $ct = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>' +
    '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>'
  $rels = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>'
  if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
  $fs = [IO.File]::Open($path, 'CreateNew')
  $zip = New-Object IO.Compression.ZipArchive($fs, [IO.Compression.ZipArchiveMode]::Create)
  foreach ($e in @(@('[Content_Types].xml', $ct), @('_rels/.rels', $rels), @('word/document.xml', $doc))) {
    $en = $zip.CreateEntry($e[0], [IO.Compression.CompressionLevel]::Optimal)
    $w = New-Object IO.StreamWriter($en.Open(), (New-Object Text.UTF8Encoding($false)))
    $w.Write($e[1]); $w.Dispose()
  }
  $zip.Dispose(); $fs.Dispose()
}

Add-Type -AssemblyName System.IO.Compression
$allPids = New-Object System.Collections.Generic.HashSet[int]
$t0 = Get-Date

# Stage 1: adjust counts and default values, read from Word's object model.
$handlesPath = Join-Path $outDir "$Batch.handles.json"
$handles = $null
if ((Test-Path -LiteralPath $handlesPath) -and -not $Rediscover) {
  $h = Get-Content -Raw $handlesPath | ConvertFrom-Json
  if (@($names | Where-Object { $null -eq $h.presets.$_ }).Count -eq 0) { $handles = $h }
}
if (-not $handles) {
  $dpages = @(); $i = 0
  foreach ($p in $names) { $i++; $dpages += @{ page = $i; preset = $p; kind = 'discover'; w_emu = 144 * $PT; h_emu = 144 * $PT; adj = [ordered]@{} } }
  $ddocx = Join-Path $outDir "$Batch.discover.docx"
  Write-ProbeDocx $ddocx $dpages
  $before = Get-Pids; $word = $null; $wp = @(); $found = [ordered]@{}
  try {
    $word = New-Object -ComObject Word.Application
    $wp = @(Get-Pids | Where-Object { $_ -notin $before })
    if ($wp.Count -eq 0) { $word = $null; throw 'COM attached to a Word instance this script did not start; refusing to touch it' }
    foreach ($x in $wp) { [void]$allPids.Add($x) }
    $word.Visible = $false; $word.DisplayAlerts = 0; $word.AutomationSecurity = 3
    $doc = $word.Documents.Open([string]$ddocx, [ref]$false, [ref]$true, [ref]$false)
    try {
      foreach ($sh in $doc.Shapes) {
        $idx = [int](($sh.Name -replace '\D', ''))
        if ($idx -lt 1 -or $idx -gt $names.Count) { continue }
        $vals = @()
        $n = $sh.Adjustments.Count
        for ($k = 1; $k -le $n; $k++) { $vals += [math]::Round([double]$sh.Adjustments.Item($k), 6) }
        $found[$names[$idx - 1]] = [ordered]@{ count = $n; values = $vals; auto_shape_type = [int]$sh.AutoShapeType }
      }
    } finally { try { $doc.Close(0) } catch {}; Release $doc }
  } finally {
    if ($word) { try { $word.Quit(0) } catch {}; Release $word }
    [GC]::Collect(); [GC]::WaitForPendingFinalizers()
    foreach ($x in $wp) { Get-Process -Id $x -ErrorAction SilentlyContinue | Stop-Process -Force }
  }
  Remove-Item -LiteralPath $ddocx
  foreach ($p in $names) { if (-not $found.Contains($p)) { throw "Word reported no shape for preset '$p' (does Word open it?)" } }
  [ordered]@{ batch = $Batch; note = 'Shape.Adjustments from Word with default adjusts: count and values in Word units (fractions of 100000, angles in degrees)'; presets = $found } |
    ConvertTo-Json -Depth 5 | Out-File -Encoding utf8NoBOM $handlesPath
  $handles = Get-Content -Raw $handlesPath | ConvertFrom-Json
}

# Stage 2: the probe document.
$override = $null
$ovPath = Join-Path $outDir 'adjnames.json'
if (Test-Path -LiteralPath $ovPath) { $override = Get-Content -Raw $ovPath | ConvertFrom-Json }
# Convention for the adjust names of an n-adjust preset: adj for one, adj1 .. adjn for several. Some presets name theirs
# differently (adj, hf, vf), which only Word's refusal of the document shows; see Resolve-AdjNames.
function Get-AdjNames($p) {
  $n = [int]$handles.presets.$p.count
  @(if ($override -and $override.$p) { $override.$p } elseif ($n -eq 1) { 'adj' } else { 1..$n | ForEach-Object { "adj$_" } })
}

function New-PresetPages($p, $adjNames) {
  $adjNames = @($adjNames)
  $n = [int]$handles.presets.$p.count
  $list = New-Object System.Collections.Generic.List[object]
  $add = {
    param($kind, $w, $h, $adj)
    $list.Add(@{ page = 0; preset = $p; kind = $kind; w_emu = [int64]$w; h_emu = [int64]$h; adj = $adj })
  }
  & $add 'size-sq'   (144 * $PT) (144 * $PT) ([ordered]@{})
  & $add 'size-wide' (288 * $PT) (96 * $PT)  ([ordered]@{})
  & $add 'size-tall' (96 * $PT)  (288 * $PT) ([ordered]@{})
  & $add 'size-box'  (216 * $PT) (144 * $PT) ([ordered]@{})
  if ($n -gt 0) {
    foreach ($v in 0, 30000, 100000, -50000, 400000) {
      $a = [ordered]@{}; foreach ($nm in $adjNames) { $a[$nm] = $v }
      & $add "adj-all-$v" (216 * $PT) (144 * $PT) $a
    }
    # Word refuses a partial avLst on a preset with several adjusts, so a scan page sets one adjust to 0 and keeps the
    # others at Word's default (fractions are x100000; a value of 2 or more is an angle in degrees, x60000).
    for ($k = 0; $k -lt $adjNames.Count; $k++) {
      $a = [ordered]@{}
      for ($j = 0; $j -lt $adjNames.Count; $j++) {
        $d = [double]$handles.presets.$p.values[$j]
        $a[$adjNames[$j]] = if ($j -eq $k) { 0 } elseif ([math]::Abs($d) -ge 2) { [int64][math]::Round($d * 60000) } else { [int64][math]::Round($d * 100000) }
      }
      & $add ("scan-" + $adjNames[$k]) (216 * $PT) (144 * $PT) $a
    }
  }
  $list
}

$pages = New-Object System.Collections.Generic.List[object]
foreach ($p in $names) { foreach ($pg in (New-PresetPages $p (Get-AdjNames $p))) { $pages.Add($pg) } }
$docx = Join-Path $outDir "$Batch.docx"
$dropped = New-Object System.Collections.Generic.List[string]

function Save-Probes($pages) {
  for ($n = 0; $n -lt $pages.Count; $n++) { $pages[$n].page = $n + 1 }
  Write-ProbeDocx $docx $pages
  $jp = New-Object System.Collections.Generic.List[object]
  foreach ($pg in $pages) {
    $o = [ordered]@{}
    foreach ($key in $pg.adj.Keys) { $o[[string]$key] = $pg.adj[$key] }
    $jp.Add([ordered]@{ page = $pg.page; preset = $pg.preset; kind = $pg.kind; w_emu = $pg.w_emu; h_emu = $pg.h_emu; adj = $o })
  }
  [ordered]@{ batch = $Batch; fill = [bool]$Fill; origin_emu = 914400; page_pt = @(612, 792); dropped = @($dropped); pages = $jp } |
    ConvertTo-Json -Depth 6 -Compress | Out-File -Encoding utf8NoBOM (Join-Path $outDir "$Batch.probes.json")
}

# Opens every page as its own document (Word rejects a whole file for one bad page) and returns the pages Word refuses.
function Find-BadPages($pages) {
  $bad = New-Object System.Collections.Generic.List[object]
  $before = Get-Pids; $word = $null; $wp = @()
  $tmp = Join-Path $outDir "$Batch.single.docx"
  try {
    $word = New-Object -ComObject Word.Application
    $wp = @(Get-Pids | Where-Object { $_ -notin $before })
    if ($wp.Count -eq 0) { $word = $null; throw 'COM attached to a Word instance this script did not start; refusing to touch it' }
    foreach ($x in $wp) { [void]$script:allPids.Add($x) }
    $word.Visible = $false; $word.DisplayAlerts = 0; $word.AutomationSecurity = 3
    foreach ($pg in $pages) {
      Write-ProbeDocx $tmp @(@{ page = 1; preset = $pg.preset; kind = $pg.kind; w_emu = $pg.w_emu; h_emu = $pg.h_emu; adj = $pg.adj })
      try { $d = $word.Documents.Open([string]$tmp, [ref]$false, [ref]$true, [ref]$false); $d.Close(0); Release $d }
      catch { $bad.Add($pg) }
    }
  } finally {
    if ($word) { try { $word.Quit(0) } catch {}; Release $word }
    [GC]::Collect(); [GC]::WaitForPendingFinalizers()
    foreach ($x in $wp) { Get-Process -Id $x -ErrorAction SilentlyContinue | Stop-Process -Force }
    if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp }
  }
  $bad
}

# Export through the shared exporter, in a child process so the Word PIDs it starts can be recorded here.
function Export-Pdf {
  $before = Get-Pids
  $exporter = Join-Path $PSScriptRoot 'make-pdf-refs.ps1'
  $child = Start-Process pwsh -PassThru -NoNewWindow -ArgumentList @('-NoProfile', '-File', $exporter,
    '-DocxDir', $outDir, '-OutDir', $outDir, '-Name', $Batch)
  while (-not $child.HasExited) {
    foreach ($x in Get-Pids) { if ($x -notin $before) { [void]$script:allPids.Add($x) } }
    Start-Sleep -Milliseconds 250
  }
  $child.WaitForExit()
  $child.ExitCode
}

Save-Probes $pages
Write-Output ("probes: {0} presets, {1} pages -> {2}" -f $names.Count, $pages.Count, $docx)
if ($NoExport) { return }
# Presets that name their adjusts unusually (adj, hf, vf): try the subsets of that pool for the presets whose
# all-adjusts pages were refused, keep the first one Word opens and remember it in adjnames.json.
function Resolve-AdjNames($bad) {
  $found = [ordered]@{}
  $suspects = @($bad | Where-Object { $_.kind -like 'adj-all-*' } | ForEach-Object { $_.preset } | Select-Object -Unique)
  if ($suspects.Count -eq 0) { return $found }
  $pool = @('adj', 'hf', 'vf')
  $cands = New-Object System.Collections.Generic.List[object]
  foreach ($p in $suspects) {
    $n = [int]$handles.presets.$p.count
    $sets = @()
    if ($n -eq 1) { $sets = @(, @('hf'); , @('vf')) }
    elseif ($n -eq 2) { $sets = @(, @('adj', 'hf'); , @('adj', 'vf'); , @('hf', 'vf')) }
    elseif ($n -eq 3) { $sets = @(, @('adj', 'hf', 'vf')) }
    foreach ($set in $sets) {
      $a = [ordered]@{}; foreach ($nm in $set) { $a[$nm] = 30000 }
      $cands.Add(@{ page = 0; preset = $p; kind = ($set -join ','); w_emu = [int64](216 * $PT); h_emu = [int64](144 * $PT); adj = $a; set = $set })
    }
  }
  $refused = @(Find-BadPages $cands)
  foreach ($c in $cands) {
    if ($found.Contains($c.preset)) { continue }
    if (@($refused | Where-Object { $_.preset -eq $c.preset -and $_.kind -eq $c.kind }).Count -eq 0) { $found[$c.preset] = $c.set }
  }
  $found
}

$code = Export-Pdf
if ($code -ne 0) {
  Write-Output 'export failed: opening each page on its own to find the pages Word refuses'
  $bad = @(Find-BadPages $pages)
  $renamed = Resolve-AdjNames $bad
  if ($renamed.Count -gt 0) {
    $merged = [ordered]@{}
    if ($override) { foreach ($pr in $override.PSObject.Properties) { $merged[$pr.Name] = @($pr.Value) } }
    foreach ($k in $renamed.Keys) { $merged[$k] = @($renamed[$k]); Write-Output ("  adjust names for {0}: {1}" -f $k, ($renamed[$k] -join ',')) }
    $merged | ConvertTo-Json -Compress | Out-File -Encoding utf8NoBOM $ovPath
    $override = Get-Content -Raw $ovPath | ConvertFrom-Json
    $pages = New-Object System.Collections.Generic.List[object]
    foreach ($p in $names) { foreach ($pg in (New-PresetPages $p (Get-AdjNames $p))) { $pages.Add($pg) } }
    $bad = @(Find-BadPages $pages)
  }
  if ($bad.Count -eq 0 -and $renamed.Count -eq 0) { Write-Output 'no single page fails; the document as a whole is refused' }
  else {
    foreach ($b in $bad) { $dropped.Add(("{0} {1}" -f $b.preset, $b.kind)); Write-Output ("  dropped: {0} {1}" -f $b.preset, $b.kind) }
    $keep = New-Object System.Collections.Generic.List[object]
    foreach ($pg in $pages) { if ($pg -notin $bad) { $keep.Add($pg) } }
    $pages = $keep
    Save-Probes $pages
    $code = Export-Pdf
  }
}
$elapsed = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
[ordered]@{ batch = $Batch; started = $t0.ToString('s'); elapsed_s = $elapsed; exporter_exit = $code; dropped = @($dropped); word_pids_started_by_run = @($allPids) } |
  ConvertTo-Json -Compress | Out-File -Encoding utf8NoBOM (Join-Path $outDir "$Batch.run.json")
Write-Output ("export: exit {0}, {1} s total, Word PIDs started by this run: {2}" -f $code, $elapsed, ($allPids -join ','))
if ($code -ne 0) { exit 1 }
