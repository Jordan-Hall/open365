# Bundled dictionaries

These four dictionaries are compiled into `open365-spell` by the `bundled-en` feature (on by default).
They are data, not code, and keep their own licence. The full licence text shipped with each one is in
the matching `*.LICENSE.txt`; keep those files with any redistribution (the UKACD and Ispell clauses
require the text to travel with the data).

| File | Language | Origin | Licence (SPDX as published by the packager) |
|---|---|---|---|
| `en_US.aff/.dic` | English (United States) | SCOWL 2020.12.07 Hunspell build `en_US`, size 60 | MIT AND BSD (SCOWL permissive notice; Ispell BSD-style) |
| `en_GB.aff/.dic` | English (United Kingdom, `-ise`) | SCOWL 2020.12.07 build `en_GB-ise`, size 60 | same |
| `en_CA.aff/.dic` | English (Canada) | SCOWL 2020.12.07 build `en_CA`, size 60 | same |
| `en_AU.aff/.dic` | English (Australia) | SCOWL 2020.12.07 build `en_AU`, size 60 | same |

Source: Kevin Atkinson's SCOWL (http://wordlist.sourceforge.net), as packaged unmodified by
https://github.com/wooorm/dictionaries (`dictionaries/en`, `en-GB`, `en-CA`, `en-AU`; files `index.aff`,
`index.dic`, `license`), fetched 2026-10-11 from raw.githubusercontent.com (branch `main`; the GitHub API
was unreachable, so no commit hash was recorded). The files are renamed only. The four `.aff` files are
identical.

SHA-256 of the fetched files:

| File | SHA-256 |
|---|---|
| `*.aff` (all four) | 8ae1f19d4840d957728ad90555d5a8dff6cc5c046279c95ff0c00fc0a0136c7b |
| `en_US.dic` | f0b1a234bd178bdd01875b2a392a9647f888b8fe879f79c52aae62c2759b3647 |
| `en_GB.dic` | 869fe17ba4ee4b5401c60a666ee2d6a3dcc237f460b7294435df1cc6a799aa57 |
| `en_CA.dic` | 53096689870246a78fa408b1e7a8c49bc140bd466a84a384cfb509445fff7173 |
| `en_AU.dic` | aa07c46571f306b79fc1bc534357ed357af15687381b26f891ba66e8a2caed89 |

Obligations summarised from the licence text (not legal advice): keep the copyright and permission
notices with all copies (SCOWL, VarCon, Princeton WordNet 1.6, UKACD, Ispell); the Ispell conditions
(no endorsement by Geoff Kuenning, modifications marked) apply; no warranty. Size 60 does not include the
UKACD word list (size 80+), but its licence is kept verbatim anyway.

Every other language is NOT bundled. See `docs/architecture/adr-spell.md` for how other dictionaries are
installed at run time and which licences they carry.
