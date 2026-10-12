//! Function catalogue, part 3 (O to Z). See `catalogue_a` for the format.

pub(super) const ROWS: &[(&str, &str, &str)] = &[
    (
        "OCT2BIN",
        "number, [places]",
        "Turns an octal number into a binary one.",
    ),
    (
        "OCT2DEC",
        "number",
        "Turns an octal number into a decimal one.",
    ),
    (
        "OCT2HEX",
        "number, [places]",
        "Turns an octal number into a hexadecimal one.",
    ),
    (
        "ODD",
        "number",
        "Rounds a number away from zero to the next odd integer.",
    ),
    (
        "ODDFPRICE",
        "settlement, maturity, issue, first_coupon, rate, yld, redemption, frequency, [basis]",
        "Gives the price of a security with an odd first period.",
    ),
    (
        "ODDFYIELD",
        "settlement, maturity, issue, first_coupon, rate, pr, redemption, frequency, [basis]",
        "Gives the yield of a security with an odd first period.",
    ),
    (
        "ODDLPRICE",
        "settlement, maturity, last_interest, rate, yld, redemption, frequency, [basis]",
        "Gives the price of a security with an odd last period.",
    ),
    (
        "ODDLYIELD",
        "settlement, maturity, last_interest, rate, pr, redemption, frequency, [basis]",
        "Gives the yield of a security with an odd last period.",
    ),
    (
        "OFFSET",
        "reference, rows, cols, [height], [width]",
        "Gives a reference a number of rows and columns away from a starting reference.",
    ),
    (
        "OR",
        "logical1, [logical2], ...",
        "Gives TRUE when any condition is TRUE.",
    ),
    (
        "PDURATION",
        "rate, pv, fv",
        "Gives the periods an investment needs to reach a value.",
    ),
    (
        "PEARSON",
        "array1, array2",
        "Gives the Pearson correlation coefficient of two data sets.",
    ),
    (
        "PERCENTILE",
        "array, k",
        "Gives the k-th percentile of a data set.",
    ),
    (
        "PERCENTILE.EXC",
        "array, k",
        "Gives the k-th percentile of a data set, leaving out 0 and 1.",
    ),
    (
        "PERCENTILE.INC",
        "array, k",
        "Gives the k-th percentile of a data set, including 0 and 1.",
    ),
    (
        "PERCENTRANK",
        "array, x, [significance]",
        "Gives the rank of a value in a data set as a percentage.",
    ),
    (
        "PERCENTRANK.EXC",
        "array, x, [significance]",
        "Gives the rank of a value as a percentage, leaving out 0 and 1.",
    ),
    (
        "PERCENTRANK.INC",
        "array, x, [significance]",
        "Gives the rank of a value as a percentage, including 0 and 1.",
    ),
    (
        "PERMUT",
        "number, number_chosen",
        "Counts the ordered ways to choose items.",
    ),
    (
        "PERMUTATIONA",
        "number, number_chosen",
        "Counts the ordered ways to choose items, repeats allowed.",
    ),
    (
        "PHI",
        "x",
        "Gives the density of the standard normal distribution at x.",
    ),
    (
        "PHONETIC",
        "reference",
        "Gives the phonetic characters of text.",
    ),
    ("PI", "", "Gives the value of pi."),
    (
        "PMT",
        "rate, nper, pv, [fv], [type]",
        "Gives the regular payment of a loan with a fixed rate.",
    ),
    (
        "POISSON",
        "x, mean, cumulative",
        "Gives the Poisson distribution.",
    ),
    (
        "POISSON.DIST",
        "x, mean, cumulative",
        "Gives the Poisson distribution.",
    ),
    ("POWER", "number, power", "Raises a number to a power."),
    (
        "PPMT",
        "rate, per, nper, pv, [fv], [type]",
        "Gives the principal part of a loan payment.",
    ),
    (
        "PRICE",
        "settlement, maturity, rate, yld, redemption, frequency, [basis]",
        "Gives the price of a security that pays interest regularly.",
    ),
    (
        "PRICEDISC",
        "settlement, maturity, discount, redemption, [basis]",
        "Gives the price of a discounted security.",
    ),
    (
        "PRICEMAT",
        "settlement, maturity, issue, rate, yld, [basis]",
        "Gives the price of a security that pays interest at maturity.",
    ),
    (
        "PROB",
        "x_range, prob_range, lower_limit, [upper_limit]",
        "Gives the chance that values fall between two limits.",
    ),
    (
        "PRODUCT",
        "number1, [number2], ...",
        "Multiplies the numbers.",
    ),
    (
        "PROPER",
        "text",
        "Changes the first letter of each word to upper case and the rest to lower case.",
    ),
    (
        "PV",
        "rate, nper, pmt, [fv], [type]",
        "Gives the present value of an investment.",
    ),
    (
        "QUARTILE",
        "array, quart",
        "Gives a quartile of a data set.",
    ),
    (
        "QUARTILE.EXC",
        "array, quart",
        "Gives a quartile of a data set, leaving out 0 and 1.",
    ),
    (
        "QUARTILE.INC",
        "array, quart",
        "Gives a quartile of a data set, including 0 and 1.",
    ),
    (
        "QUOTIENT",
        "numerator, denominator",
        "Gives the whole part of a division.",
    ),
    ("RADIANS", "angle", "Changes degrees to radians."),
    (
        "RAND",
        "",
        "Gives a random number from 0 up to but not including 1.",
    ),
    (
        "RANDARRAY",
        "[rows], [columns], [min], [max], [whole_number]",
        "Gives an array of random numbers.",
    ),
    (
        "RANDBETWEEN",
        "bottom, top",
        "Gives a random whole number between two numbers.",
    ),
    (
        "RANK",
        "number, ref, [order]",
        "Gives the rank of a number in a list.",
    ),
    (
        "RANK.AVG",
        "number, ref, [order]",
        "Gives the rank of a number in a list, averaging the rank of ties.",
    ),
    (
        "RANK.EQ",
        "number, ref, [order]",
        "Gives the rank of a number in a list, giving ties the top rank.",
    ),
    (
        "RATE",
        "nper, pmt, pv, [fv], [type], [guess]",
        "Gives the interest rate per period of a loan or investment.",
    ),
    (
        "RECEIVED",
        "settlement, maturity, investment, discount, [basis]",
        "Gives the amount received at maturity for a fully invested security.",
    ),
    (
        "REDUCE",
        "initial_value, array, lambda",
        "Folds an array into one value by applying a function to each value in turn.",
    ),
    (
        "REGEXEXTRACT",
        "text, pattern, [return_mode], [case_sensitivity]",
        "Gives the parts of text that match a pattern.",
    ),
    (
        "REGEXREPLACE",
        "text, pattern, replacement, [occurrence], [case_sensitivity]",
        "Replaces the parts of text that match a pattern.",
    ),
    (
        "REGEXTEST",
        "text, pattern, [case_sensitivity]",
        "Tells whether text matches a pattern.",
    ),
    (
        "REPLACE",
        "old_text, start_num, num_chars, new_text",
        "Replaces part of a string, by position, with other text.",
    ),
    (
        "REPLACEB",
        "old_text, start_num, num_bytes, new_text",
        "Replaces part of a string, by byte position, with other text.",
    ),
    (
        "REPT",
        "text, number_times",
        "Repeats text a number of times.",
    ),
    (
        "RIGHT",
        "text, [num_chars]",
        "Gives the last characters of a string.",
    ),
    (
        "RIGHTB",
        "text, [num_bytes]",
        "Gives the last bytes of a string.",
    ),
    (
        "ROMAN",
        "number, [form]",
        "Writes a number as a Roman numeral.",
    ),
    (
        "ROUND",
        "number, num_digits",
        "Rounds a number to a number of digits.",
    ),
    (
        "ROUNDDOWN",
        "number, num_digits",
        "Rounds a number toward zero to a number of digits.",
    ),
    (
        "ROUNDUP",
        "number, num_digits",
        "Rounds a number away from zero to a number of digits.",
    ),
    ("ROW", "[reference]", "Gives the row number of a reference."),
    ("ROWS", "array", "Counts the rows in a reference or array."),
    (
        "RRI",
        "nper, pv, fv",
        "Gives the interest rate that makes an investment grow to a value.",
    ),
    (
        "RSQ",
        "known_y's, known_x's",
        "Gives the square of the Pearson correlation coefficient.",
    ),
    (
        "SCAN",
        "initial_value, array, lambda",
        "Folds an array by applying a function to each value in turn and keeps every step.",
    ),
    (
        "SEARCH",
        "find_text, within_text, [start_num]",
        "Gives the position of one string inside another, ignoring case.",
    ),
    (
        "SEARCHB",
        "find_text, within_text, [start_num]",
        "Gives the byte position of one string inside another, ignoring case.",
    ),
    ("SEC", "number", "Gives the secant of an angle in radians."),
    ("SECH", "number", "Gives the hyperbolic secant of a number."),
    (
        "SECOND",
        "serial_number",
        "Gives the second, 0 to 59, of a time.",
    ),
    (
        "SEQUENCE",
        "rows, [columns], [start], [step]",
        "Gives an array of consecutive numbers.",
    ),
    (
        "SERIESSUM",
        "x, n, m, coefficients",
        "Gives the sum of a power series.",
    ),
    (
        "SHEET",
        "[value]",
        "Gives the sheet number of a reference or sheet name.",
    ),
    (
        "SHEETS",
        "[reference]",
        "Counts the sheets in a reference or workbook.",
    ),
    (
        "SIGN",
        "number",
        "Gives 1 for a positive number, 0 for zero and -1 for a negative number.",
    ),
    ("SIN", "number", "Gives the sine of an angle in radians."),
    ("SINH", "number", "Gives the hyperbolic sine of a number."),
    (
        "SKEW",
        "number1, [number2], ...",
        "Gives the skewness of a data set.",
    ),
    (
        "SKEW.P",
        "number1, [number2], ...",
        "Gives the skewness of a whole population.",
    ),
    (
        "SLN",
        "cost, salvage, life",
        "Gives straight-line depreciation for one period.",
    ),
    (
        "SLOPE",
        "known_y's, known_x's",
        "Gives the slope of the best-fit line.",
    ),
    (
        "SMALL",
        "array, k",
        "Gives the k-th smallest value in a data set.",
    ),
    (
        "SORT",
        "array, [sort_index], [sort_order], [by_col]",
        "Sorts the rows or columns of an array.",
    ),
    (
        "SORTBY",
        "array, by_array1, sort_order1, [by_array2], [sort_order2], ...2",
        "Sorts an array by the values in other arrays.",
    ),
    ("SQRT", "number", "Gives the square root of a number."),
    (
        "SQRTPI",
        "number",
        "Gives the square root of a number times pi.",
    ),
    (
        "STANDARDIZE",
        "x, mean, standard_dev",
        "Gives a value as a distance from the mean in standard deviations.",
    ),
    (
        "STDEV",
        "number1, [number2], ...",
        "Estimates the standard deviation from a sample.",
    ),
    (
        "STDEV.P",
        "number1, [number2], ...",
        "Gives the standard deviation of a whole population.",
    ),
    (
        "STDEV.S",
        "number1, [number2], ...",
        "Estimates the standard deviation from a sample.",
    ),
    (
        "STDEVA",
        "value1, [value2], ...",
        "Estimates the standard deviation from a sample, counting text and logical values.",
    ),
    (
        "STDEVP",
        "number1, [number2], ...",
        "Gives the standard deviation of a whole population.",
    ),
    (
        "STDEVPA",
        "value1, [value2], ...",
        "Gives the standard deviation of a population, counting text and logical values.",
    ),
    (
        "STEYX",
        "known_y's, known_x's",
        "Gives the standard error of the predicted y for each x.",
    ),
    (
        "SUBSTITUTE",
        "text, old_text, new_text, [instance_num]",
        "Replaces matching text in a string with new text.",
    ),
    (
        "SUBTOTAL",
        "function_num, ref1, [ref2], ...",
        "Gives a subtotal for a list, leaving out other subtotals.",
    ),
    ("SUM", "number1, [number2], ...", "Adds the numbers."),
    (
        "SUMIF",
        "range, criteria, [sum_range]",
        "Adds the cells that meet one condition.",
    ),
    (
        "SUMIFS",
        "sum_range, criteria_range1, criteria1, [criteria_range2], [criteria2], ...2",
        "Adds the cells that meet all the conditions.",
    ),
    (
        "SUMPRODUCT",
        "array1, [array2], ...",
        "Multiplies arrays item by item and adds the products.",
    ),
    (
        "SUMSQ",
        "number1, [number2], ...",
        "Adds the squares of the numbers.",
    ),
    (
        "SUMX2MY2",
        "array_x, array_y",
        "Adds the differences of the squares of two arrays.",
    ),
    (
        "SUMX2PY2",
        "array_x, array_y",
        "Adds the sums of the squares of two arrays.",
    ),
    (
        "SUMXMY2",
        "array_x, array_y",
        "Adds the squares of the differences of two arrays.",
    ),
    (
        "SWITCH",
        "expression, value1, result1, [default_or_value2], [result2], ...2",
        "Compares an expression with a list of values and gives the result of the first match.",
    ),
    (
        "SYD",
        "cost, salvage, life, per",
        "Gives sum-of-years'-digits depreciation for a period.",
    ),
    (
        "T",
        "value",
        "Gives the text a value holds, or empty text for anything else.",
    ),
    (
        "T.DIST",
        "x, deg_freedom, cumulative",
        "Gives the left-tail Student's t distribution.",
    ),
    (
        "T.DIST.2T",
        "x, deg_freedom",
        "Gives the two-tail Student's t distribution.",
    ),
    (
        "T.DIST.RT",
        "x, deg_freedom",
        "Gives the right-tail Student's t distribution.",
    ),
    (
        "T.INV",
        "probability, deg_freedom",
        "Gives the left-tail inverse of Student's t distribution.",
    ),
    (
        "T.INV.2T",
        "probability, deg_freedom",
        "Gives the two-tail inverse of Student's t distribution.",
    ),
    (
        "T.TEST",
        "array1, array2, tails, type",
        "Gives the probability from a Student's t-test.",
    ),
    (
        "TAKE",
        "array, rows, [columns]",
        "Keeps rows or columns from the start or end of an array.",
    ),
    ("TAN", "number", "Gives the tangent of an angle in radians."),
    (
        "TANH",
        "number",
        "Gives the hyperbolic tangent of a number.",
    ),
    (
        "TBILLEQ",
        "settlement, maturity, discount",
        "Gives the bond-equivalent yield of a treasury bill.",
    ),
    (
        "TBILLPRICE",
        "settlement, maturity, discount",
        "Gives the price of a treasury bill per 100 of face value.",
    ),
    (
        "TBILLYIELD",
        "settlement, maturity, pr",
        "Gives the yield of a treasury bill.",
    ),
    (
        "TDIST",
        "x, deg_freedom, tails",
        "Gives Student's t distribution for one or two tails.",
    ),
    (
        "TEXT",
        "value, format_text",
        "Writes a value as text in a number format you give.",
    ),
    (
        "TEXTAFTER",
        "text, delimiter, [instance_num], [match_mode], [match_end], [if_not_found]",
        "Gives the text that comes after a delimiter.",
    ),
    (
        "TEXTBEFORE",
        "text, delimiter, [instance_num], [match_mode], [match_end], [if_not_found]",
        "Gives the text that comes before a delimiter.",
    ),
    (
        "TEXTJOIN",
        "delimiter, ignore_empty, text1, [text2], ...",
        "Joins text with a delimiter between the pieces.",
    ),
    (
        "TEXTSPLIT",
        "text, col_delimiter, [row_delimiter], [ignore_empty], [match_mode], [pad_with]",
        "Splits text into columns and rows at delimiters.",
    ),
    (
        "TIME",
        "hour, minute, second",
        "Builds a time from an hour, a minute and a second.",
    ),
    (
        "TIMEVALUE",
        "time_text",
        "Turns a time written as text into a time number.",
    ),
    (
        "TINV",
        "probability, deg_freedom",
        "Gives the two-tail inverse of Student's t distribution.",
    ),
    (
        "TOCOL",
        "array, [ignore], [scan_by_column]",
        "Turns an array into one column.",
    ),
    ("TODAY", "", "Gives today's date."),
    (
        "TOROW",
        "array, [ignore], [scan_by_column]",
        "Turns an array into one row.",
    ),
    (
        "TRANSPOSE",
        "array",
        "Swaps the rows and columns of an array.",
    ),
    (
        "TREND",
        "known_y's, [known_x's], [new_x's], [const]",
        "Fits a straight line to known points and gives values along it.",
    ),
    (
        "TRIM",
        "text",
        "Removes extra spaces from text, leaving single spaces between words.",
    ),
    (
        "TRIMMEAN",
        "array, percent",
        "Gives the mean of a data set after cutting off the highest and lowest values.",
    ),
    ("TRUE", "", "Gives the logical value TRUE."),
    (
        "TRUNC",
        "number, [num_digits]",
        "Cuts a number to a number of digits without rounding.",
    ),
    (
        "TTEST",
        "array1, array2, tails, type",
        "Gives the probability from a Student's t-test.",
    ),
    (
        "TYPE",
        "value",
        "Gives a number for the kind of value: number, text, logical, error or array.",
    ),
    (
        "UNICHAR",
        "number",
        "Gives the character for a Unicode number.",
    ),
    (
        "UNICODE",
        "text",
        "Gives the Unicode number of the first character of text.",
    ),
    (
        "UNIQUE",
        "array, [by_col], [exactly_once]",
        "Gives the distinct rows or columns of an array.",
    ),
    ("UPPER", "text", "Changes text to upper case."),
    (
        "VALUE",
        "text",
        "Turns text that looks like a number into a number.",
    ),
    (
        "VALUETOTEXT",
        "value, [format]",
        "Turns a value into text, either for reading or in a form that can be typed back.",
    ),
    (
        "VAR",
        "number1, [number2], ...",
        "Estimates the variance from a sample.",
    ),
    (
        "VAR.P",
        "number1, [number2], ...",
        "Gives the variance of a whole population.",
    ),
    (
        "VAR.S",
        "number1, [number2], ...",
        "Estimates the variance from a sample.",
    ),
    (
        "VARA",
        "value1, [value2], ...",
        "Estimates the variance from a sample, counting text and logical values.",
    ),
    (
        "VARP",
        "number1, [number2], ...",
        "Gives the variance of a whole population.",
    ),
    (
        "VARPA",
        "value1, [value2], ...",
        "Gives the variance of a population, counting text and logical values.",
    ),
    (
        "VDB",
        "cost, salvage, life, start_period, end_period, [factor], [no_switch]",
        "Gives declining-balance depreciation between two periods.",
    ),
    (
        "VLOOKUP",
        "lookup_value, table_array, col_index_num, [range_lookup]",
        "Looks down the first column of a table and gives a value from a column to its right.",
    ),
    (
        "VSTACK",
        "array1, [array2], ...",
        "Joins arrays one below the other into one taller array.",
    ),
    (
        "WEEKDAY",
        "serial_number, [return_type]",
        "Gives the day of the week of a date as a number.",
    ),
    (
        "WEEKNUM",
        "serial_number, [return_type]",
        "Gives the week of the year that a date falls in.",
    ),
    (
        "WEIBULL",
        "x, alpha, beta, cumulative",
        "Gives the Weibull distribution.",
    ),
    (
        "WEIBULL.DIST",
        "x, alpha, beta, cumulative",
        "Gives the Weibull distribution.",
    ),
    (
        "WORKDAY",
        "start_date, days, [holidays]",
        "Gives the date a number of working days from a date.",
    ),
    (
        "WORKDAY.INTL",
        "start_date, days, [weekend], [holidays]",
        "Gives the date a number of working days from a date, with your own weekend days.",
    ),
    (
        "WRAPCOLS",
        "vector, wrap_count, [pad_with]",
        "Wraps a row or column into columns of a given length.",
    ),
    (
        "WRAPROWS",
        "vector, wrap_count, [pad_with]",
        "Wraps a row or column into rows of a given length.",
    ),
    (
        "XIRR",
        "values, dates, [guess]",
        "Gives the internal rate of return of cash flows that fall on given dates.",
    ),
    (
        "XLOOKUP",
        "lookup_value, lookup_array, return_array, [if_not_found], [match_mode], [search_mode]",
        "Looks a value up in one range and gives the item at the same place in another.",
    ),
    (
        "XMATCH",
        "lookup_value, lookup_array, [match_mode], [search_mode]",
        "Gives the position of a value in a list.",
    ),
    (
        "XNPV",
        "rate, values, dates",
        "Gives the present value of cash flows that fall on given dates.",
    ),
    (
        "XOR",
        "logical1, [logical2], ...",
        "Gives TRUE when an odd number of conditions are TRUE.",
    ),
    ("YEAR", "serial_number", "Gives the year of a date."),
    (
        "YEARFRAC",
        "start_date, end_date, [basis]",
        "Gives the part of a year between two dates.",
    ),
    (
        "YIELD",
        "settlement, maturity, rate, pr, redemption, frequency, [basis]",
        "Gives the yield of a security that pays interest regularly.",
    ),
    (
        "YIELDDISC",
        "settlement, maturity, pr, redemption, [basis]",
        "Gives the yearly yield of a discounted security.",
    ),
    (
        "YIELDMAT",
        "settlement, maturity, issue, rate, pr, [basis]",
        "Gives the yearly yield of a security that pays interest at maturity.",
    ),
    (
        "Z.TEST",
        "array, x, [sigma]",
        "Gives the one-tail probability from a z-test.",
    ),
    (
        "ZTEST",
        "array, x, [sigma]",
        "Gives the one-tail probability from a z-test.",
    ),
];
