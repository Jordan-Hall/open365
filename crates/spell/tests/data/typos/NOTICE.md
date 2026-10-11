# Natural typo corpus

`wikipedia-en.tsv` holds `misspelling<TAB>correction` pairs (about 790 rows, every fifth single-word
entry) taken from the machine-readable list
<https://en.wikipedia.org/wiki/Wikipedia:Lists_of_common_misspellings/For_machines>, fetched
2026-10-11. Only entries with one lower-case alphabetic misspelling and one lower-case alphabetic
correction were kept; rows were not edited.

Licence: text on Wikipedia is available under CC BY-SA 4.0
(<https://creativecommons.org/licenses/by-sa/4.0/>), contributors credited by the page history.
The file is test data only: it is not compiled into the crate, is excluded from the product
binary, and keeps this licence apart from the MIT code. Do not copy it into MIT-only artifacts.

`tests/typos.rs` measures the rate at which the expected correction is the first, one of the first
three and one of the ten suggestions from the bundled `en_US` dictionary, on every second row whose
misspelling the dictionary rejects and whose correction it accepts. Floors (per mille): top-1 840,
top-3 910, top-10 925. Measured on that subset at the time of writing: about 876, 939 and 948 on
the full file (debug build, bundled SCOWL 2020.12.07 en_US).
