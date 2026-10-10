# Microsoft template corpus: access method and licensing

Researched 2026-10-10. Facts below were read from the public pages on that date. This is engineering notes, not legal advice.

## Decision

The template files are **not committed**. They are downloaded on demand into `fixtures/external/ms-templates/`
(gitignored). The repository commits only:

- `fixtures/external/ms-templates.toml`: one entry per file (id, title, category, kind, url, sha256, size, licence note).
- `fixtures/reference/ms-templates/<id>/`: derived reference outputs from the local Office oracle
  (Word: `<id>.pdf`, `pages.json`, `layout.json`; Excel: `values.json`).
- `tools/corpus/fetch-ms-templates.ps1`: the fetcher.

## Confirmed facts

### How the site exposes downloads
- `https://create.microsoft.com/en-us/templates/<slug>` answers with plain HTTP (some slugs 301 to
  `word.cloud.microsoft/create/...` or `excel.cloud.microsoft/create/...`). The server-rendered HTML lists template
  cards without login or JavaScript.
- Each card links to `view.officeapps.live.com/op/view.aspx?src=<url-encoded file URL>`. The `src` value is a direct,
  anonymous file URL of the form
  `https://createcatalog.public.onecdn.static.microsoft/catalog-assets/en-us/<guid>/<name>.<docx|xlsx>`.
  A plain HTTPS GET returns the file with the OOXML content type. No cookie, token or login is involved or sent.
- The catalog serves `.docx` and `.xlsx` (documents that behave as templates), not `.dotx`/`.xltx`. The fetcher
  still allows `.dotx`/`.xltx` if a future page links them.
- Category pages cap at a few dozen cards. There is no pagination or sitemap used here. `robots.txt` at
  `create.microsoft.com` returned an HTML page instead of a robots file, so no crawl rules were found. The fetcher
  makes only a small number of page requests plus one request per file, sequentially.
- Not downloadable with plain HTTP: anything behind "Open in Word/Excel" sign-in, Copilot-generated templates, and
  slugs that return no cards (for example `planners`, `reports`, `checklists`). These were skipped.

### Terms found
1. Microsoft Terms of Use (the site footer "Terms of use" links to `https://go.microsoft.com/fwlink/?linkid=206977`;
   text read at <https://www.microsoft.com/en-us/legal/terms-of-use>, "Last Updated: February 7, 2022"):
   - "Unless otherwise specified, the Services are for your personal and non-commercial use."
   - Documents may be used for informational or personal use on condition that "no modifications of any Documents are made".
   - "All rights not expressly granted to you in this TOU are reserved and retained by Microsoft or its licensors."
   - All content on the Services is the property of Microsoft or its suppliers.
   - The page does not mention templates.
2. Microsoft Services Agreement, <https://www.microsoft.com/en-us/servicesagreement> (Published July 30, 2026,
   effective September 30, 2026): Section 8.b bars you from "publish, copy, rent, lease, sell, export, import,
   distribute, or lend the software or the Services" unless Microsoft expressly authorizes it. Section 14.h.i says Word and Excel
   use is governed by supplemental terms at <https://aka.ms/useterms>. The agreement text was read only to character
   100,000 of about 131,000.
3. The template pages themselves state only "©Microsoft 2026" (checked on the budgets page,
   <https://excel.cloud.microsoft/create/en/budget-templates/>). No template-specific licence, CC licence or open
   licence was found on any page read.

### Not confirmed
- No document found that grants a right to redistribute these templates. Treat them as all rights reserved.
- Whether private local testing of interoperability on lawfully downloaded files is covered is a legal question
  that was not resolved. The decision to download, not commit and not redistribute is the conservative reading.
- `aka.ms/useterms` (Microsoft 365 supplemental terms) and the product software licence terms were not read.

## Consequences
- CI cannot download the corpus without a reviewed decision; CI consumes only the committed manifest and reference outputs.
  Tests that need the files must skip when `fixtures/external/ms-templates/` is absent.
- Reference outputs (PDF renderings, extracted values) are derived from Microsoft's content. Word PDFs reproduce
  template text and layout. If counsel judges these derivative, drop the PDFs and keep `pages.json`/`layout.json`/`values.json`.
- Do not copy template text or styling into product assets, defaults or documentation.
- Evidence that would change this: an explicit Microsoft statement that these catalog files may be redistributed.

## Corpus summary

| Category | docx | xlsx | Total |
|---|---|---|---|
| budgets | 0 | 14 | 14 |
| calendars | 0 | 5 | 5 |
| charts | 0 | 13 | 13 |
| invoices | 0 | 6 | 6 |
| letters | 14 | 0 | 14 |
| newsletters | 14 | 0 | 14 |
| planners | 14 | 0 | 14 |
| reports | 14 | 0 | 14 |
| resumes | 14 | 0 | 14 |
| schedules | 4 | 10 | 14 |
| trackers | 0 | 14 | 14 |
| **Total** | 74 | 62 | **136** |

The 'letters' category includes cover letters; 'reports' includes business plans; 'planners' includes wedding planners (timelines pages added nothing within the per-category cap); 'schedules' includes agendas; 'charts' includes Gantt charts. Calendars are limited by what the public page lists (5).

Excluded: 1 workbook ("Graph paper", id a3fde570-770a-470d-ab91-2478070cdb9e) was dropped because the Excel oracle hung on it (90 s timeout, repeated twice).

Reference outputs: ~20 MB under fixtures/reference/ms-templates/ (Word: PDF + pages.json + layout.json; Excel: values.json). Word layout baselines are null for most pages because templates contain tables/shapes (the oracle prints a WARN and leaves them null by design).
