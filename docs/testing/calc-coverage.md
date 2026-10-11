# Calc function coverage

Registered functions in `crates/calc/src/functions/mod.rs` compared with Microsoft's public function list.

**Source check:** the list was compared on 2026-10-11 with "Excel functions (alphabetical)",
<https://support.microsoft.com/en-us/office/excel-functions-alphabetical-b3944572-255d-4efb-bb96-c6d90033e188>.
That page has 514 table rows naming 521 distinct functions (seven rows pair a function with its `B` variant). Corrections to the
earlier, recalled list: `COPILOT` is not on the page (removed); `CALL`, `DETECTLANGUAGE`, `EUROCONVERT`, `JIS`, `REGISTER.ID` and
`TRANSLATE` are on it (added under "Web, cube, other"). No registered function is missing from the page. In the table below a name that is
also in another category counts in both (the legacy names repeat), so the category total is larger than 521. Distinct names: 521 on the page,
367 registered before the seventh batch, 451 after, 70 missing (after the seventh batch; the sixth batch had 154 missing; the table below is the earlier count). Before the previous changes: 236, then 301, then 312, then 316 registered.

| Category | Excel | Registered | Missing |
|---|---:|---:|---:|
| Math and trigonometry | 82 | 69 | 13 |
| Statistical | 111 | 30 | 81 |
| Financial | 55 | 23 | 32 |
| Date and time | 25 | 25 | 0 |
| Text | 44 | 40 | 4 |
| Lookup and reference | 36 | 34 | 2 |
| Logical | 18 | 18 | 0 |
| Information | 21 | 21 | 0 |
| Engineering | 54 | 36 | 18 |
| Database | 12 | 12 | 0 |
| Compatibility (legacy names) | 41 | 10 | 31 |
| Web, cube, other | 25 | 0 | 25 |
| **Total** | 524 | 318 | 206 |

## Sixth batch (statistics and securities): 51 functions

Added and checked against Excel (2,409 cases, groups `avedev` ... `yieldmat`, plus argument-handling cases `*.e_*`): AVEDEV, DEVSQ,
GEOMEAN, HARMEAN, SKEW, SKEW.P, KURT, TRIMMEAN, STDEVA, STDEVPA, VARA, VARPA, CORREL, PEARSON, RSQ, SLOPE, INTERCEPT, STEYX, COVAR,
COVARIANCE.P, COVARIANCE.S, FORECAST, FORECAST.LINEAR; COUPDAYBS, COUPDAYS, COUPDAYSNC, COUPNCD, COUPNUM, COUPPCD, PRICE, YIELD,
DURATION, MDURATION, DISC, PRICEDISC, YIELDDISC, INTRATE, RECEIVED, PRICEMAT, YIELDMAT, TBILLPRICE, TBILLYIELD, TBILLEQ, ACCRINT,
ACCRINTM, AMORLINC, AMORDEGRC, PDURATION, RRI, ISPMT, FVSCHEDULE.

Findings (the algorithms were fitted to Excel's last digits on random data sets):

* `CORREL` is `(Sxy/n) / (sqrt(Sxx/n) * sqrt(Syy/n))`, `SLOPE` is `Sxy/Sxx`, `INTERCEPT` is `mean_y - slope * mean_x`, `STEYX` is
  `sqrt((Syy - Sxy*Sxy/Sxx) / (n-2))`, `FORECAST` is `(mean_y - b*mean_x) + b*x`, with means as `sum/n` and deviations from the mean.
  Pairs with a non-number on either side are skipped, different lengths are `#N/A`, errors propagate by position.
* `VARA`/`VARPA`/`STDEV*A` use `(Σx² - (Σx)²/n)/(n-1)` and `(nΣx² - (Σx)²)/n²` with a two-pass fallback (not the `VAR` form);
  array constants contribute only their numbers there, ranges count text as 0 and booleans as 1/0.
* `HARMEAN` is `1/(Σ(1/x)/n)`; `GEOMEAN` carries the log sum in more than double precision (Open365 uses double-double and rounds once:
  33 of 40 random sets match, the rest differ by 1 ulp).
* `YIELD` solves with `DSC = E - A` for every basis (not the actual day count of `COUPDAYSNC` for bases 2 and 3); Excel then stops
  its iteration at a loose tolerance (result 1e-16 to 1e-10 from the exact root). Open365 iterates to convergence, so multi-coupon
  `YIELD` differs in the last digits (six sample cases listed). `PRICEMAT`/`YIELDMAT` take the year length from issue to settlement,
  `DISC` is `(1 - pr/red) * (B/DSM)`.
* Coupon dates follow the maturity backwards in `12/frequency` month steps, with the end-of-month rule when the maturity is a month end
  (all 240 `COUP*` cases match).

Not done: the distribution functions (NORM/T/CHISQ/F/BETA/GAMMA/BINOM/POISSON families and their legacy names), CONFIDENCE.*, Z.TEST,
T.TEST, F.TEST, CHISQ.TEST, LINEST/LOGEST/TREND/GROWTH, PERCENTRANK, MODE.MULT, FREQUENCY, PROB, STANDARDIZE, FISHER, FORECAST.ETS.*,
the `ODDF*`/`ODDL*` bond functions and the exact `ACCRINT`/`AMOR*` rounding rules. `crates/calc/src/functions/distribdd.rs` holds the
double-double arithmetic (exp, ln) the gamma/beta work will need.

## Seventh batch (distributions, tests and array statistics): 84 functions

Added (groups `norm_dist` ... `growth` in the corpus, about 14,200 cases including argument-handling cases `*.e*`): NORM.DIST/INV,
NORM.S.DIST/INV, PHI, GAUSS, STANDARDIZE, LOGNORM.DIST/INV, T.DIST(.RT/.2T), T.INV(.2T), CHISQ.DIST(.RT)/INV(.RT), F.DIST(.RT)/INV(.RT),
BETA.DIST/INV, GAMMA.DIST/INV, GAMMA, GAMMALN(.PRECISE), WEIBULL.DIST, EXPON.DIST, BINOM.DIST(.RANGE)/INV, POISSON.DIST,
NEGBINOM.DIST, HYPGEOM.DIST, CONFIDENCE.NORM/T, Z.TEST, T.TEST, F.TEST, CHISQ.TEST, FISHER, FISHERINV, PERMUTATIONA, FREQUENCY,
MODE.MULT, PROB, PERCENTRANK(.INC/.EXC), LINEST, LOGEST, TREND, GROWTH, and every legacy name (NORMDIST, TDIST, CRITBINOM, ...).

Method: probabilities are evaluated in double-double (`distribdd.rs`: erfc, ln-gamma, incomplete gamma and beta, Newton/false-position
inverses) and rounded once, so Open365 returns the exact value. Excel does not: its results are typically 1 to a few ulp off the exact value
(for the normal tail the error grows like z^2 * 1e-16, e.g. `NORM.S.DIST(-10)` is 6.7e-15 low), so about 40% of the new cases differ in
the last digits and are listed in `fixtures/calc/known-differences.toml` with the measured ulp distance. The cases where the difference is
large (extreme parameters, huge degrees of freedom, `T.DIST` at |t| > 1e8) and those where Excel and Open365 disagree on a domain boundary
are listed with their own reason; they have not been analysed one by one.

Excel hangs (does not return) on `BETA.DIST`/`BETADIST` with a 1E+300 shape parameter and `NEGBINOM.DIST` with 1E+300 arguments; those six
cases were removed from the corpus. Open365 caps its series and fraction loops and returns `#NUM!` instead.

Follow-up (domain and limit rules measured against Excel, 83 known differences removed, 6191 to 6108): degrees of freedom up to 1E10 inclusive
for T/F/CHISQ (`T.DIST(...,FALSE)` and `CHISQ.INV.RT` have no upper limit; the T density gives #DIV/0! for df 0), T functions give #NUM! when
t*t overflows (|t| > 1.34E154), continuous results below 2.2250738585072014E-308 are 0 (the discrete ones keep subnormals), the BETA density
at an end point is #NUM! for exponent <= 1 and 0 above, `GAMMA.DIST` density at 0 is #NUM! for shape <= 1, `WEIBULL.DIST` density at 0 is 0,
`BETA.INV(1,...)` is #NUM!, `GAMMA.INV(0,...)` is 0. Findings on the normal CDF: it is `0.5*erfc(-z*0.7071067811865476)` (central region
`0.5+0.5*erf`) with an unsplit `exp(-x*x)`, which explains the z^2*1e-16 tail error; Cephes `ndtr` has that shape but is not bit-identical
(about 18% of tail points equal), West/Hart and Numerical Recipes `erfccheb` do not match, so the exact coefficients are not identified.
Excel's EXP and LN are correctly rounded; BINOM.DIST is `exp` of a sum of logarithms (`exp(log(p))` reproduces `BINOM.DIST(1,1,0.01,FALSE)` =
0.010000000000000004) but the log-binomial-coefficient term is not identified. T tail below `df + t^2 == df` returns the t=0 value.

Follow-up 2 (edge rules measured against Excel; known differences 6108 to 5971, 381 `edge.*` cases added, `tools/office/calc-oracle.ps1` watchdog fixed):

* `HYPGEOM.DIST`/`HYPGEOMDIST`: any argument below 0 before truncation or at or above 2147483647 is #NUM!; after truncation `n > N` or `M > N` is #NUM!;
  `N = 0` is allowed; outside the support Excel answers instead of #NUM!: `s < max(0, n-(N-M))` gives 0, `s > min(n, M)` gives 0 (pdf) or 1 (cdf).
* `NEGBINOM.DIST`/`NEGBINOMDIST`: p = 0 or p = 1 is #NUM!, a failure count below 0 before truncation is #NUM!, r below 1 is #NUM!, and the pdf with f >= 1 and r >= 1E+16 is #NUM! (r = 9E+15 gives 0).
* `BINOM.INV`/`CRITBINOM`: p and alpha must be strictly inside (0, 1); n below 0 before truncation or >= 2147483647 is #NUM!.
  A tie between alpha and the cdf is resolved upwards at some points (n = 1, alpha = 0.5 gives 1; n = 5 gives 3) that look like rounding in Excel's own cdf; not reproduced (6 cases remain).
* `BINOM.DIST.RANGE`: p = 1 and p = 0 are allowed, n >= 2147483647 is #NUM!, negative counts are refused before truncation.
* `Z.TEST`: one value is always #DIV/0! (even with sigma), sigma <= 0 is #NUM!, no numbers is #N/A; the first argument is not array-evaluated (`Z.TEST(A1:A5*2,4)` is #VALUE!, same for `PERCENTRANK`).
* `CHISQ.TEST`: the two arrays need the same number of cells (paired in row-major order), the degrees of freedom come from the first array's shape, a 1x1 array is #N/A,
  expected 0 is #DIV/0!, negative expected is #NUM!, non-numeric pairs are skipped, no numeric pair is #DIV/0!.
* `PROB`: probabilities are not range-checked; a blank probability or a zero one drops the pair; a text/boolean probability, or a non-number value beside a non-zero probability, is #NUM!;
  the sum (as a double) must lie in [1 - 4.5E-16, 1 + 5E-15] or the result is #NUM!.
* `FREQUENCY`: text and error bins are dropped; an error in the data fills every element; with no numeric bin the result is {0; count}.
* `PERCENTRANK(.INC/.EXC)`: a single value ranks 1 in both; the rank is rounded to 15 significant digits before truncation to the significance.
* `LINEST`/`LOGEST`/`TREND`/`GROWTH`: an error cell in any argument is #VALUE!; when a column is redundant Excel pivots in the order constant, last x column, ..., first x column
  (the earlier of two collinear columns reads 0, a constant column reads 0 beside the intercept, one point gives slope 0 and intercept y); r2 is 1 when the fit is exact and F is #NUM! when the residual sum is 0.
  Open365 keeps the natural order for full-rank data (it matches Excel's last digits more often) and uses Excel's order only after a column was found redundant.

Still open: `CHISQ.INV.RT` is #NUM! for small p depending on df (largest accepted -log10 p for df 1, 3, 4, 6, 10, 20, 30: 12, 18, 24, 42, 78, 132, 198; df 2 and df >= 60 never fail in the grid p = 1E-6k) and
`CHISQ.INV` / `CHISQ.INV.RT` accept df beyond 1E10; the T.INV stopping tolerance; the NEGBINOM.DIST cdf #NUM! region for p >= 0.9 with f = 1E+100; `HYPGEOM.DIST(500,1000,5000,10000,TRUE)`;
centred least squares (Excel's exact zeros for constant y and its 1E-11 better accuracy for `linest.big`); the NORM.S.DIST tail algorithm (not started).

Not done: reproducing Excel's own approximation of the normal CDF/quantile (the largest single source of residue), the multi-variable
`LINEST` pivoting rules for collinear columns, `FORECAST.ETS.*`, and per-case analysis of the large differences.

## Missing by category

**Math and trigonometry** (13): CEILING.PRECISE, COMBINA, FLOOR.PRECISE, ISO.CEILING, MDETERM, MINVERSE, MMULT, MULTINOMIAL, MUNIT, RANDARRAY, SUMX2MY2, SUMX2PY2, SUMXMY2

**Statistical** (4): FORECAST.ETS, FORECAST.ETS.CONFINT, FORECAST.ETS.SEASONALITY, FORECAST.ETS.STAT

**Financial** (4): ODDFPRICE, ODDFYIELD, ODDLPRICE, ODDLYIELD

**Date and time** (0): none

**Text** (0): none

**Lookup and reference** (2): GETPIVOTDATA, RTD

**Logical** (0): none

**Information** (0): none

**Engineering** (18): BESSELI, BESSELJ, BESSELK, BESSELY, CONVERT, ERF, ERF.PRECISE, ERFC, ERFC.PRECISE, IMABS, IMCOT, IMCSC, IMCSCH, IMPOWER, IMSEC, IMSECH, IMSQRT, IMTAN

**Database** (0): none

**Compatibility (legacy names)** (0): 

**Web, cube, other** (25): FILTERXML, WEBSERVICE, CUBEKPIMEMBER, CUBEMEMBER, CUBEMEMBERPROPERTY, CUBERANKEDMEMBER, CUBESET, CUBESETCOUNT, CUBEVALUE, IMAGE, STOCKHISTORY, TRIMRANGE, GROUPBY, PIVOTBY, PERCENTOF, CALL, DETECTLANGUAGE, EUROCONVERT, JIS, REGISTER.ID, TRANSLATE

## Fifth batch (this change): CELL, INFO, SHEET, SHEETS

Added and verified against Excel (about 1,400 cases, groups `cell`, `cell2`, `cellfmt`, `info`, `sheet`): CELL (all info types), INFO,
SHEET, SHEETS. Every information and logical function of the page is now registered. `CELL` needs cell presentation, so the engine gained
`CellStyle` (number format, horizontal alignment, locked), column widths and `CellSource::{cell_style, column_width, sheet_position,
sheet_count}`; the oracle and the conformance test accept a case field `styles`. Findings and residue are in `calc-conformance.md`.

Done since: ASC, DBCS, BAHTTEXT, PHONETIC, ENCODEURL, REGEXTEST, REGEXEXTRACT, REGEXREPLACE (see `calc-regex.md`; JIS is `#NAME?` in the oracle Excel). Not done: the statistical group 3. `XMATCH`/`XLOOKUP` `match_mode` 3 (regular expression)
is unchanged.

## Fourth batch: lambdas, LET, XMATCH, FORMULATEXT

Added and verified against Excel (about 1,940 cases; residue in `fixtures/calc/known-differences.toml`): LET, LAMBDA, MAP, REDUCE, SCAN,
BYROW, BYCOL, MAKEARRAY, ISOMITTED, XMATCH, FORMULATEXT. `LET` and the lambda family live in `eval/lambda.rs` (they need unevaluated
arguments); `Arg::Lambda` is the closure value.

Findings carried forward: this Excel build has REGEX functions and `XMATCH`/`XLOOKUP` `match_mode` 3 is a regular-expression match
(still open, see the fifth batch).

## Third batch: engineering, database, arrays, small lookups

Added and verified against Excel (bit for bit unless listed in `fixtures/calc/known-differences.toml`): BIN2DEC, BIN2OCT, BIN2HEX, OCT2BIN,
OCT2DEC, OCT2HEX, HEX2BIN, HEX2DEC, HEX2OCT, DEC2BIN, DEC2OCT, DEC2HEX, BITAND, BITOR, BITXOR, BITLSHIFT, BITRSHIFT, DELTA, GESTEP,
COMPLEX, IMREAL, IMAGINARY, IMARGUMENT, IMCONJUGATE, IMSUM, IMSUB, IMPRODUCT, IMDIV, IMEXP, IMLN, IMLOG10, IMLOG2, IMSIN, IMCOS, IMSINH,
IMCOSH (15th digit residue of 1 to 8 cases each is listed), DSUM, DAVERAGE, DCOUNT, DCOUNTA, DGET, DMAX, DMIN, DPRODUCT, DSTDEV, DSTDEVP,
DVAR, DVARP (criteria cells holding formulas are not supported and their cases were removed), SEQUENCE, TRANSPOSE, SORT, SORTBY, UNIQUE,
FILTER, TAKE, DROP, CHOOSECOLS, CHOOSEROWS, EXPAND, HSTACK, VSTACK, TOCOL, TOROW, WRAPCOLS, WRAPROWS, AREAS, HYPERLINK.

Probed and not kept (cases removed from the corpus): IMABS (Excel's result is not `hypot`; 22 of 130 differ), IMSQRT, IMTAN, IMCOT, IMSEC,
IMCSC, IMSECH, IMCSCH, IMPOWER (Excel evaluates them with its own operation order; 15 to 60% of random cases differ in the 15th digit).
Not attempted: ERF, ERFC (and .PRECISE), BESSEL*, CONVERT (the unit table needs per-unit measurement), LAMBDA, LET, MAP, REDUCE, SCAN, BYROW,
BYCOL, MAKEARRAY, ISOMITTED, CELL, INFO, SHEET, SHEETS, ASC, DBCS, BAHTTEXT, PHONETIC, XMATCH, FORMULATEXT, RANDARRAY.

## Second math batch (this change)

Added and verified against Excel: ASINH, ACOSH, ATANH, ACOT, ACOTH, COT, COTH, CSC, CSCH, SEC, SECH, SQRTPI, FACTDOUBLE,
SERIESSUM, BASE, DECIMAL, ARABIC, ROMAN. Already registered before (the brief listed them): PI, RADIANS, DEGREES, FACT, GCD, LCM.

Probed against Excel, not kept (cases removed from the corpus; Excel's answers for about 6,100 cases are kept outside the
repository and can be regenerated with the generators `gen_b1.py`/`gen_b2.py`):

* MULTINOMIAL: Excel's results carry noise (`MULTINOMIAL(170)` is `1.0000000000001974`, `MULTINOMIAL(1.9,2.5)` is `3.0000000000000004`), so it is
  not a ratio of exact factorials; the algorithm (probably gamma based) is not identified.
* NORM.S.DIST/NORM.DIST (cumulative), NORM.S.INV/NORM.INV, GAMMALN(.PRECISE), GAMMA, GAMMA.DIST, BETA.DIST, BINOM.DIST,
  POISSON.DIST, T/CHISQ/F distributions and their legacy names, GAUSS, PHI: not implemented. Findings that a later attempt can start from:
  * The normal density is `exp(-x*x/2) * 0.3989422804014327` (138 of 140 exact with a correctly rounded `exp`), `EXPON.DIST` is `l * exp(-l*x)` and
    `-expm1(-l*x)` (183 of 200 exact).
  * The normal CDF is not a correctly rounded value (up to 19 ulps off in the tail at x = -7.4). It is within 4 ulps of
    `0.5 * erfc(-x/sqrt 2)` with W. J. Cody's rational approximations (constants a, b, c, d, p, q of `calerf`) and a plain `exp(-y*y)`; West's double-precision
    Hart (1968) algorithm is 75 of 160 and not it. The CDF constants are probably Cody's `pnorm` set (not the `erf` set).
  * `GAMMALN` and `GAMMA` are not correctly rounded either (154 and 70 of 250 match a correct rounding), so every gamma, beta,
    Poisson, binomial and t/F/chi-square value inherits that unknown routine.
