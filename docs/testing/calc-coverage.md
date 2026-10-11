# Calc function coverage

Registered functions in `crates/calc/src/functions/mod.rs` compared with Microsoft's public function list.

**Source check:** the list was compared on 2026-10-11 with "Excel functions (alphabetical)",
<https://support.microsoft.com/en-us/office/excel-functions-alphabetical-b3944572-255d-4efb-bb96-c6d90033e188>.
That page has 514 table rows naming 521 distinct functions (seven rows pair a function with its `B` variant). Corrections to the
earlier, recalled list: `COPILOT` is not on the page (removed); `CALL`, `DETECTLANGUAGE`, `EUROCONVERT`, `JIS`, `REGISTER.ID` and
`TRANSLATE` are on it (added under "Web, cube, other"). No registered function is missing from the page. In the table below a name that is
also in another category counts in both (the legacy names repeat), so the category total is larger than 521. Distinct names: 521 on the page,
312 registered, 209 missing. Before the previous change: 236 registered; before this one: 301.

| Category | Excel | Registered | Missing |
|---|---:|---:|---:|
| Math and trigonometry | 82 | 69 | 13 |
| Statistical | 111 | 30 | 81 |
| Financial | 55 | 23 | 32 |
| Date and time | 25 | 25 | 0 |
| Text | 44 | 40 | 4 |
| Lookup and reference | 36 | 34 | 2 |
| Logical | 18 | 18 | 0 |
| Information | 21 | 17 | 4 |
| Engineering | 54 | 36 | 18 |
| Database | 12 | 12 | 0 |
| Compatibility (legacy names) | 41 | 10 | 31 |
| Web, cube, other | 25 | 0 | 25 |
| **Total** | 524 | 314 | 210 |

## Missing by category

**Math and trigonometry** (13): CEILING.PRECISE, COMBINA, FLOOR.PRECISE, ISO.CEILING, MDETERM, MINVERSE, MMULT, MULTINOMIAL, MUNIT, RANDARRAY, SUMX2MY2, SUMX2PY2, SUMXMY2

**Statistical** (81): AVEDEV, BETA.DIST, BETA.INV, BINOM.DIST, BINOM.DIST.RANGE, BINOM.INV, CHISQ.DIST, CHISQ.DIST.RT, CHISQ.INV, CHISQ.INV.RT, CHISQ.TEST, CONFIDENCE.NORM, CONFIDENCE.T, CORREL, COVARIANCE.P, COVARIANCE.S, DEVSQ, EXPON.DIST, F.DIST, F.DIST.RT, F.INV, F.INV.RT, F.TEST, FISHER, FISHERINV, FORECAST, FORECAST.ETS, FORECAST.ETS.CONFINT, FORECAST.ETS.SEASONALITY, FORECAST.ETS.STAT, FORECAST.LINEAR, FREQUENCY, GAMMA, GAMMA.DIST, GAMMA.INV, GAMMALN, GAMMALN.PRECISE, GAUSS, GEOMEAN, GROWTH, HARMEAN, HYPGEOM.DIST, INTERCEPT, KURT, LINEST, LOGEST, LOGNORM.DIST, LOGNORM.INV, MODE.MULT, NEGBINOM.DIST, NORM.DIST, NORM.INV, NORM.S.DIST, NORM.S.INV, PEARSON, PERCENTRANK.EXC, PERCENTRANK.INC, PERMUTATIONA, PHI, POISSON.DIST, PROB, RSQ, SKEW, SKEW.P, SLOPE, STANDARDIZE, STDEVA, STDEVPA, STEYX, T.DIST, T.DIST.2T, T.DIST.RT, T.INV, T.INV.2T, T.TEST, TREND, TRIMMEAN, VARA, VARPA, WEIBULL.DIST, Z.TEST

**Financial** (32): ACCRINT, ACCRINTM, AMORDEGRC, AMORLINC, COUPDAYBS, COUPDAYS, COUPDAYSNC, COUPNCD, COUPNUM, COUPPCD, DISC, DURATION, FVSCHEDULE, INTRATE, ISPMT, MDURATION, ODDFPRICE, ODDFYIELD, ODDLPRICE, ODDLYIELD, PDURATION, PRICE, PRICEDISC, PRICEMAT, RECEIVED, RRI, TBILLEQ, TBILLPRICE, TBILLYIELD, YIELD, YIELDDISC, YIELDMAT

**Date and time** (0): none

**Text** (4): ASC, BAHTTEXT, DBCS, PHONETIC

**Lookup and reference** (2): GETPIVOTDATA, RTD

**Logical** (0): none

**Information** (4): CELL, INFO, SHEET, SHEETS

**Engineering** (18): BESSELI, BESSELJ, BESSELK, BESSELY, CONVERT, ERF, ERF.PRECISE, ERFC, ERFC.PRECISE, IMABS, IMCOT, IMCSC, IMCSCH, IMPOWER, IMSEC, IMSECH, IMSQRT, IMTAN

**Database** (0): none

**Compatibility (legacy names)** (31): BETADIST, BETAINV, BINOMDIST, CHIDIST, CHIINV, CHITEST, CONFIDENCE, COVAR, CRITBINOM, EXPONDIST, FDIST, FINV, FORECAST, FTEST, GAMMADIST, GAMMAINV, HYPGEOMDIST, LOGINV, LOGNORMDIST, NEGBINOMDIST, NORMDIST, NORMINV, NORMSDIST, NORMSINV, PERCENTRANK, POISSON, TDIST, TINV, TTEST, WEIBULL, ZTEST

**Web, cube, other** (25): ENCODEURL, FILTERXML, WEBSERVICE, CUBEKPIMEMBER, CUBEMEMBER, CUBEMEMBERPROPERTY, CUBERANKEDMEMBER, CUBESET, CUBESETCOUNT, CUBEVALUE, IMAGE, STOCKHISTORY, TRIMRANGE, GROUPBY, PIVOTBY, PERCENTOF, REGEXTEST, REGEXEXTRACT, REGEXREPLACE, CALL, DETECTLANGUAGE, EUROCONVERT, JIS, REGISTER.ID, TRANSLATE

## Fourth batch (this change): lambdas, LET, XMATCH, FORMULATEXT

Added and verified against Excel (about 1,940 cases; residue in `fixtures/calc/known-differences.toml`): LET, LAMBDA, MAP, REDUCE, SCAN,
BYROW, BYCOL, MAKEARRAY, ISOMITTED, XMATCH, FORMULATEXT. `LET` and the lambda family live in `eval/lambda.rs` (they need unevaluated
arguments); `Arg::Lambda` is the closure value.

Not done in this change (no code, no cases): CELL, INFO, SHEET, SHEETS (group 2) and ASC, DBCS, BAHTTEXT, PHONETIC, REGEXTEST,
REGEXEXTRACT, REGEXREPLACE (group 3). Findings for whoever continues: this Excel build has REGEX functions and `XMATCH`/`XLOOKUP`
`match_mode` 3 is a regular-expression match; `CELL("filename")` and `INFO("osversion")` must be tested for type and shape only
(for example `ISTEXT(INFO("osversion"))`) because they depend on the machine; the oracle's columns are 60 wide, so `CELL("width")`
cannot match an engine without column widths.

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
