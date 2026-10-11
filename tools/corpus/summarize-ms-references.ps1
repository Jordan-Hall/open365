<#
Writes fixtures/reference/ms-templates-summary.json: per manifest entry, the kind, category, counts and sha256 of
each local reference output under fixtures/reference/ms-templates/<id>/. No template content (no titles, text or
URLs) is included, so the aggregate is committable. Entries with missing outputs are listed with "complete": false.

  pwsh -NoProfile -File tools/corpus/summarize-ms-references.ps1
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$refDir = Join-Path $root 'fixtures\reference\ms-templates'
$text = Get-Content -LiteralPath (Join-Path $root 'fixtures\external\ms-templates.toml') -Raw
$entries = foreach ($m in [regex]::Matches($text, '(?ms)\[\[template\]\](.*?)(?=\[\[template\]\]|\z)')) {
  $b = $m.Groups[1].Value
  $g = { param($k) [regex]::Match($b, "(?m)^$k = ""([^""]*)""").Groups[1].Value }
  [pscustomobject]@{ id = (& $g 'id'); kind = (& $g 'kind'); category = (& $g 'category'); sha = (& $g 'sha256') }
}
function Hash($f) { (Get-FileHash -LiteralPath $f -Algorithm SHA256).Hash.ToLowerInvariant() }
$out = New-Object System.Collections.Generic.List[object]
foreach ($e in ($entries | Sort-Object id)) {
  $d = Join-Path $refDir $e.id
  $o = [ordered]@{ id = $e.id; kind = $e.kind; category = $e.category; templateSha256 = $e.sha }
  $files = [ordered]@{}
  $complete = $false
  if ($e.kind -in 'docx', 'dotx') {
    $pj = Join-Path $d 'pages.json'; $lj = Join-Path $d 'layout.json'
    if ((Test-Path $pj) -and (Test-Path $lj)) {
      $p = Get-Content -LiteralPath $pj -Raw | ConvertFrom-Json; $l = Get-Content -LiteralPath $lj -Raw | ConvertFrom-Json
      $o.pageCount = [int]$p.pageCount; $o.layoutLines = @($l.lines).Count; $o.layoutCells = @($l.cells).Count
      $files['pages.json'] = Hash $pj; $files['layout.json'] = Hash $lj; $complete = $true
    }
  } else {
    $vj = Join-Path $d 'values.json'
    if (Test-Path $vj) {
      $v = Get-Content -LiteralPath $vj -Raw -Encoding UTF8 | ConvertFrom-Json
      $sheets = @($v.sheets); $cells = 0; $formulas = 0; $trunc = 0
      foreach ($s in $sheets) {
        if ($s.truncatedAt2000) { $trunc++ }
        foreach ($c in $s.cells.PSObject.Properties) { $cells++; if ($null -ne $c.Value.formula -and "$($c.Value.formula)".StartsWith('=')) { $formulas++ } }
      }
      $o.sheets = $sheets.Count; $o.cells = $cells; $o.formulaCells = $formulas; $o.truncatedSheets = $trunc
      $files['values.json'] = Hash $vj; $complete = $true
    }
  }
  $o.complete = $complete; $o.outputSha256 = $files
  $out.Add([pscustomobject]$o)
}
$doc = [ordered]@{
  schema = 1
  note = 'Counts and sha256 of local Office-oracle reference outputs for the Microsoft template corpus. No template content.'
  entries = $out.Count
  complete = @($out | Where-Object complete).Count
  items = $out
}
$json = $doc | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText((Join-Path $root 'fixtures\reference\ms-templates-summary.json'), $json + "`n", (New-Object Text.UTF8Encoding $false))
Write-Output "summary: $($doc.complete)/$($doc.entries) complete"
