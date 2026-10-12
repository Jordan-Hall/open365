//! Function catalogue, part 1 (A to F): `(name, arguments, description)`. The arguments are a
//! comma list; `[name]` is optional, a trailing `...` repeats the last argument and `...2` the
//! last two. The descriptions are written for Open365.

pub(super) const ROWS: &[(&str, &str, &str)] = &[
    ("ABS", "number", "Gives a number's size without its sign."),
    (
        "ACCRINT",
        "issue, first_interest, settlement, rate, par, frequency, [basis], [calc_method]",
        "Works out the interest accrued on a security that pays interest on a schedule.",
    ),
    (
        "ACCRINTM",
        "issue, settlement, rate, par, [basis]",
        "Works out the interest accrued on a security that pays everything at maturity.",
    ),
    (
        "ACOS",
        "number",
        "Gives the angle, in radians, whose cosine is the number.",
    ),
    (
        "ACOSH",
        "number",
        "Gives the inverse hyperbolic cosine of a number.",
    ),
    (
        "ACOT",
        "number",
        "Gives the angle, in radians, whose cotangent is the number.",
    ),
    (
        "ACOTH",
        "number",
        "Gives the inverse hyperbolic cotangent of a number.",
    ),
    (
        "ADDRESS",
        "row_num, column_num, [abs_num], [a1], [sheet_text]",
        "Builds a cell address as text from a row and a column number.",
    ),
    (
        "AGGREGATE",
        "function_num, options, ref1, [ref2], ...",
        "Summarises a list or database with a chosen function, optionally skipping errors and hidden rows.",
    ),
    (
        "AMORDEGRC",
        "cost, date_purchased, first_period, salvage, period, rate, [basis]",
        "Gives the depreciation of an asset for each period, using a coefficient that depends on its life.",
    ),
    (
        "AMORLINC",
        "cost, date_purchased, first_period, salvage, period, rate, [basis]",
        "Gives the straight-line depreciation of an asset for each period.",
    ),
    (
        "AND",
        "logical1, [logical2], ...",
        "Gives TRUE when every condition is TRUE.",
    ),
    (
        "ARABIC",
        "text",
        "Turns a Roman numeral into an ordinary number.",
    ),
    (
        "AREAS",
        "reference",
        "Counts the separate areas in a reference.",
    ),
    (
        "ARRAYTOTEXT",
        "array, [format]",
        "Turns an array into text, either for reading or in a form that can be typed back.",
    ),
    (
        "ASC",
        "text",
        "Changes full-width characters in text to half-width ones.",
    ),
    (
        "ASIN",
        "number",
        "Gives the angle, in radians, whose sine is the number.",
    ),
    (
        "ASINH",
        "number",
        "Gives the inverse hyperbolic sine of a number.",
    ),
    (
        "ATAN",
        "number",
        "Gives the angle, in radians, whose tangent is the number.",
    ),
    (
        "ATAN2",
        "x_num, y_num",
        "Gives the angle, in radians, of the point with the given coordinates.",
    ),
    (
        "ATANH",
        "number",
        "Gives the inverse hyperbolic tangent of a number.",
    ),
    (
        "AVEDEV",
        "number1, [number2], ...",
        "Gives the average distance of values from their mean.",
    ),
    (
        "AVERAGE",
        "number1, [number2], ...",
        "Gives the mean of the numbers.",
    ),
    (
        "AVERAGEA",
        "value1, [value2], ...",
        "Gives the mean of the values, counting text as 0 and TRUE as 1.",
    ),
    (
        "AVERAGEIF",
        "range, criteria, [average_range]",
        "Gives the mean of the cells that meet one condition.",
    ),
    (
        "AVERAGEIFS",
        "average_range, criteria_range1, criteria1, [criteria_range2], [criteria2], ...2",
        "Gives the mean of the cells that meet all the conditions.",
    ),
    (
        "BAHTTEXT",
        "number",
        "Writes a number out in Thai words, as baht.",
    ),
    (
        "BASE",
        "number, radix, [min_length]",
        "Writes a number in another base, such as 2 or 16, as text.",
    ),
    (
        "BESSELI",
        "x, n",
        "Gives the modified Bessel function In(x).",
    ),
    ("BESSELJ", "x, n", "Gives the Bessel function Jn(x)."),
    (
        "BESSELK",
        "x, n",
        "Gives the modified Bessel function Kn(x).",
    ),
    ("BESSELY", "x, n", "Gives the Bessel function Yn(x)."),
    (
        "BETA.DIST",
        "x, alpha, beta, cumulative, [A], [B]",
        "Gives the beta distribution, cumulative or as a density.",
    ),
    (
        "BETA.INV",
        "probability, alpha, beta, [A], [B]",
        "Gives the value at which the cumulative beta distribution reaches a probability.",
    ),
    (
        "BETADIST",
        "x, alpha, beta, [A], [B]",
        "Gives the cumulative beta distribution.",
    ),
    (
        "BETAINV",
        "probability, alpha, beta, [A], [B]",
        "Gives the value at which the cumulative beta distribution reaches a probability.",
    ),
    (
        "BIN2DEC",
        "number",
        "Turns a binary number into a decimal one.",
    ),
    (
        "BIN2HEX",
        "number, [places]",
        "Turns a binary number into a hexadecimal one.",
    ),
    (
        "BIN2OCT",
        "number, [places]",
        "Turns a binary number into an octal one.",
    ),
    (
        "BINOM.DIST",
        "number_s, trials, probability_s, cumulative",
        "Gives the binomial distribution for a number of successes.",
    ),
    (
        "BINOM.DIST.RANGE",
        "trials, probability_s, number_s, [number_s2]",
        "Gives the chance of a number of successes falling in a range.",
    ),
    (
        "BINOM.INV",
        "trials, probability_s, alpha",
        "Gives the smallest success count whose cumulative binomial chance reaches a limit.",
    ),
    (
        "BINOMDIST",
        "number_s, trials, probability_s, cumulative",
        "Gives the binomial distribution for a number of successes.",
    ),
    (
        "BITAND",
        "number1, number2",
        "Gives the bitwise AND of two numbers.",
    ),
    (
        "BITLSHIFT",
        "number, shift_amount",
        "Shifts the bits of a number to the left.",
    ),
    (
        "BITOR",
        "number1, number2",
        "Gives the bitwise OR of two numbers.",
    ),
    (
        "BITRSHIFT",
        "number, shift_amount",
        "Shifts the bits of a number to the right.",
    ),
    (
        "BITXOR",
        "number1, number2",
        "Gives the bitwise exclusive OR of two numbers.",
    ),
    (
        "BYCOL",
        "array, lambda",
        "Applies a function to each column of an array and returns the results as one row.",
    ),
    (
        "BYROW",
        "array, lambda",
        "Applies a function to each row of an array and returns the results as one column.",
    ),
    (
        "CEILING",
        "number, significance",
        "Rounds a number up to the next multiple of the significance.",
    ),
    (
        "CEILING.MATH",
        "number, [significance], [mode]",
        "Rounds a number up to the next multiple, with a choice of direction for negatives.",
    ),
    (
        "CEILING.PRECISE",
        "number, [significance]",
        "Rounds a number up to the next multiple, whatever its sign.",
    ),
    (
        "CELL",
        "info_type, [reference]",
        "Gives facts about a cell, such as its address, format or contents type.",
    ),
    (
        "CHAR",
        "number",
        "Gives the character for a character code.",
    ),
    (
        "CHIDIST",
        "x, deg_freedom",
        "Gives the right-tail probability of the chi-squared distribution.",
    ),
    (
        "CHIINV",
        "probability, deg_freedom",
        "Gives the value whose right-tail chi-squared probability is the one given.",
    ),
    (
        "CHISQ.DIST",
        "x, deg_freedom, cumulative",
        "Gives the left-tail chi-squared distribution.",
    ),
    (
        "CHISQ.DIST.RT",
        "x, deg_freedom",
        "Gives the right-tail probability of the chi-squared distribution.",
    ),
    (
        "CHISQ.INV",
        "probability, deg_freedom",
        "Gives the value whose left-tail chi-squared probability is the one given.",
    ),
    (
        "CHISQ.INV.RT",
        "probability, deg_freedom",
        "Gives the value whose right-tail chi-squared probability is the one given.",
    ),
    (
        "CHISQ.TEST",
        "actual_range, expected_range",
        "Gives the chi-squared test for independence.",
    ),
    (
        "CHITEST",
        "actual_range, expected_range",
        "Gives the chi-squared test for independence.",
    ),
    (
        "CHOOSE",
        "index_num, value1, [value2], ...",
        "Picks one value from a list by its position.",
    ),
    (
        "CHOOSECOLS",
        "array, col_num1, [col_num2], ...",
        "Returns the chosen columns of an array.",
    ),
    (
        "CHOOSEROWS",
        "array, row_num1, [row_num2], ...",
        "Returns the chosen rows of an array.",
    ),
    (
        "CLEAN",
        "text",
        "Removes characters that cannot be printed from text.",
    ),
    (
        "CODE",
        "text",
        "Gives the character code of the first character of text.",
    ),
    (
        "COLUMN",
        "[reference]",
        "Gives the column number of a reference.",
    ),
    (
        "COLUMNS",
        "array",
        "Counts the columns in a reference or array.",
    ),
    (
        "COMBIN",
        "number, number_chosen",
        "Counts the ways to choose items when order does not matter.",
    ),
    (
        "COMBINA",
        "number, number_chosen",
        "Counts the ways to choose items, repeats allowed, when order does not matter.",
    ),
    (
        "COMPLEX",
        "real_num, i_num, [suffix]",
        "Builds a complex number from its real and imaginary parts.",
    ),
    (
        "CONCAT",
        "text1, [text2], ...",
        "Joins text from several strings or ranges.",
    ),
    (
        "CONCATENATE",
        "text1, [text2], ...",
        "Joins several strings into one.",
    ),
    (
        "CONFIDENCE",
        "alpha, standard_dev, size",
        "Gives the margin of a confidence interval for a mean.",
    ),
    (
        "CONFIDENCE.NORM",
        "alpha, standard_dev, size",
        "Gives the margin of a confidence interval for a mean, using the normal distribution.",
    ),
    (
        "CONFIDENCE.T",
        "alpha, standard_dev, size",
        "Gives the margin of a confidence interval for a mean, using Student's t distribution.",
    ),
    (
        "CONVERT",
        "number, from_unit, to_unit",
        "Converts a number from one unit of measure to another.",
    ),
    (
        "CORREL",
        "array1, array2",
        "Gives the correlation coefficient of two data sets.",
    ),
    ("COS", "number", "Gives the cosine of an angle in radians."),
    ("COSH", "number", "Gives the hyperbolic cosine of a number."),
    (
        "COT",
        "number",
        "Gives the cotangent of an angle in radians.",
    ),
    (
        "COTH",
        "number",
        "Gives the hyperbolic cotangent of a number.",
    ),
    (
        "COUNT",
        "value1, [value2], ...",
        "Counts the cells that hold numbers.",
    ),
    (
        "COUNTA",
        "value1, [value2], ...",
        "Counts the cells that are not empty.",
    ),
    ("COUNTBLANK", "range", "Counts the empty cells in a range."),
    (
        "COUNTIF",
        "range, criteria",
        "Counts the cells that meet one condition.",
    ),
    (
        "COUNTIFS",
        "criteria_range1, criteria1, [criteria_range2], [criteria2], ...2",
        "Counts the cells that meet all the conditions.",
    ),
    (
        "COUPDAYBS",
        "settlement, maturity, frequency, [basis]",
        "Counts the days from the start of the coupon period to the settlement date.",
    ),
    (
        "COUPDAYS",
        "settlement, maturity, frequency, [basis]",
        "Counts the days in the coupon period that holds the settlement date.",
    ),
    (
        "COUPDAYSNC",
        "settlement, maturity, frequency, [basis]",
        "Counts the days from the settlement date to the next coupon.",
    ),
    (
        "COUPNCD",
        "settlement, maturity, frequency, [basis]",
        "Gives the next coupon date after the settlement date.",
    ),
    (
        "COUPNUM",
        "settlement, maturity, frequency, [basis]",
        "Counts the coupons still to be paid between settlement and maturity.",
    ),
    (
        "COUPPCD",
        "settlement, maturity, frequency, [basis]",
        "Gives the last coupon date before the settlement date.",
    ),
    (
        "COVAR",
        "array1, array2",
        "Gives the covariance of two data sets, treated as whole populations.",
    ),
    (
        "COVARIANCE.P",
        "array1, array2",
        "Gives the population covariance of two data sets.",
    ),
    (
        "COVARIANCE.S",
        "array1, array2",
        "Gives the sample covariance of two data sets.",
    ),
    (
        "CRITBINOM",
        "trials, probability_s, alpha",
        "Gives the smallest success count whose cumulative binomial chance reaches a limit.",
    ),
    (
        "CSC",
        "number",
        "Gives the cosecant of an angle in radians.",
    ),
    (
        "CSCH",
        "number",
        "Gives the hyperbolic cosecant of a number.",
    ),
    (
        "CUMIPMT",
        "rate, nper, pv, start_period, end_period, type",
        "Gives the interest paid on a loan between two periods.",
    ),
    (
        "CUMPRINC",
        "rate, nper, pv, start_period, end_period, type",
        "Gives the principal repaid on a loan between two periods.",
    ),
    (
        "DATE",
        "year, month, day",
        "Builds a date from a year, a month and a day.",
    ),
    (
        "DATEDIF",
        "start_date, end_date, unit",
        "Gives the days, months or years between two dates.",
    ),
    (
        "DATEVALUE",
        "date_text",
        "Turns a date written as text into a date number.",
    ),
    (
        "DAVERAGE",
        "database, field, criteria",
        "Gives the mean of a column of a list for the rows that match the criteria.",
    ),
    (
        "DAY",
        "serial_number",
        "Gives the day of the month, 1 to 31, of a date.",
    ),
    (
        "DAYS",
        "end_date, start_date",
        "Counts the days between two dates.",
    ),
    (
        "DAYS360",
        "start_date, end_date, [method]",
        "Counts the days between two dates on a 360-day year.",
    ),
    (
        "DB",
        "cost, salvage, life, period, [month]",
        "Gives depreciation by the fixed-declining-balance method.",
    ),
    (
        "DBCS",
        "text",
        "Changes half-width characters in text to full-width ones.",
    ),
    (
        "DCOUNT",
        "database, field, criteria",
        "Counts the numbers in a column of a list for the rows that match the criteria.",
    ),
    (
        "DCOUNTA",
        "database, field, criteria",
        "Counts the non-empty cells in a column of a list for the rows that match the criteria.",
    ),
    (
        "DDB",
        "cost, salvage, life, period, [factor]",
        "Gives depreciation by the double-declining-balance method or another rate.",
    ),
    (
        "DEC2BIN",
        "number, [places]",
        "Turns a decimal number into a binary one.",
    ),
    (
        "DEC2HEX",
        "number, [places]",
        "Turns a decimal number into a hexadecimal one.",
    ),
    (
        "DEC2OCT",
        "number, [places]",
        "Turns a decimal number into an octal one.",
    ),
    (
        "DECIMAL",
        "text, radix",
        "Turns text that writes a number in another base into a decimal number.",
    ),
    ("DEGREES", "angle", "Changes radians to degrees."),
    (
        "DELTA",
        "number1, [number2]",
        "Tells whether two numbers are equal: 1 if they are, 0 if not.",
    ),
    (
        "DEVSQ",
        "number1, [number2], ...",
        "Gives the sum of the squared distances of values from their mean.",
    ),
    (
        "DGET",
        "database, field, criteria",
        "Gives the single value of a column that matches the criteria.",
    ),
    (
        "DISC",
        "settlement, maturity, pr, redemption, [basis]",
        "Gives the discount rate of a security.",
    ),
    (
        "DMAX",
        "database, field, criteria",
        "Gives the largest number in a column of a list for the rows that match the criteria.",
    ),
    (
        "DMIN",
        "database, field, criteria",
        "Gives the smallest number in a column of a list for the rows that match the criteria.",
    ),
    (
        "DOLLAR",
        "number, [decimals]",
        "Writes a number as currency text.",
    ),
    (
        "DOLLARDE",
        "fractional_dollar, fraction",
        "Turns a price written as a fraction into a decimal price.",
    ),
    (
        "DOLLARFR",
        "decimal_dollar, fraction",
        "Turns a decimal price into a price written as a fraction.",
    ),
    (
        "DPRODUCT",
        "database, field, criteria",
        "Multiplies the values of a column of a list for the rows that match the criteria.",
    ),
    (
        "DROP",
        "array, rows, [columns]",
        "Removes rows or columns from the start or end of an array.",
    ),
    (
        "DSTDEV",
        "database, field, criteria",
        "Estimates the standard deviation of a sample of the matching rows.",
    ),
    (
        "DSTDEVP",
        "database, field, criteria",
        "Gives the standard deviation of the matching rows as a whole population.",
    ),
    (
        "DSUM",
        "database, field, criteria",
        "Adds the values of a column of a list for the rows that match the criteria.",
    ),
    (
        "DURATION",
        "settlement, maturity, coupon, yld, frequency, [basis]",
        "Gives the weighted average time to receive the payments of a security.",
    ),
    (
        "DVAR",
        "database, field, criteria",
        "Estimates the variance of a sample of the matching rows.",
    ),
    (
        "DVARP",
        "database, field, criteria",
        "Gives the variance of the matching rows as a whole population.",
    ),
    (
        "EDATE",
        "start_date, months",
        "Gives the date a number of months before or after a date.",
    ),
    (
        "EFFECT",
        "nominal_rate, npery",
        "Gives the effective yearly interest rate.",
    ),
    (
        "ENCODEURL",
        "text",
        "Encodes text so that it can be used in a web address.",
    ),
    (
        "EOMONTH",
        "start_date, months",
        "Gives the last day of the month a number of months away.",
    ),
    (
        "ERF",
        "lower_limit, [upper_limit]",
        "Gives the error function between two limits.",
    ),
    ("ERF.PRECISE", "x", "Gives the error function from 0 to x."),
    (
        "ERFC",
        "x",
        "Gives the complementary error function from x to infinity.",
    ),
    (
        "ERFC.PRECISE",
        "x",
        "Gives the complementary error function from x to infinity.",
    ),
    (
        "ERROR.TYPE",
        "error_val",
        "Gives a number for the kind of error a value is.",
    ),
    (
        "EVEN",
        "number",
        "Rounds a number away from zero to the next even integer.",
    ),
    (
        "EXACT",
        "text1, text2",
        "Tells whether two strings are exactly the same, including case.",
    ),
    ("EXP", "number", "Gives e raised to a power."),
    (
        "EXPAND",
        "array, rows, [columns], [pad_with]",
        "Makes an array larger, filling the new cells.",
    ),
    (
        "EXPON.DIST",
        "x, lambda, cumulative",
        "Gives the exponential distribution.",
    ),
    (
        "EXPONDIST",
        "x, lambda, cumulative",
        "Gives the exponential distribution.",
    ),
    (
        "F.DIST",
        "x, deg_freedom1, deg_freedom2, cumulative",
        "Gives the left-tail F distribution.",
    ),
    (
        "F.DIST.RT",
        "x, deg_freedom1, deg_freedom2",
        "Gives the right-tail F distribution.",
    ),
    (
        "F.INV",
        "probability, deg_freedom1, deg_freedom2",
        "Gives the value whose left-tail F probability is the one given.",
    ),
    (
        "F.INV.RT",
        "probability, deg_freedom1, deg_freedom2",
        "Gives the value whose right-tail F probability is the one given.",
    ),
    (
        "F.TEST",
        "array1, array2",
        "Gives the result of an F-test on two data sets.",
    ),
    ("FACT", "number", "Gives the factorial of a number."),
    (
        "FACTDOUBLE",
        "number",
        "Gives the double factorial of a number.",
    ),
    ("FALSE", "", "Gives the logical value FALSE."),
    (
        "FDIST",
        "x, deg_freedom1, deg_freedom2",
        "Gives the right-tail F distribution.",
    ),
    (
        "FILTER",
        "array, include, [if_empty]",
        "Keeps the rows or columns of an array that meet a condition.",
    ),
    (
        "FIND",
        "find_text, within_text, [start_num]",
        "Gives the position of one string inside another, matching case.",
    ),
    (
        "FINDB",
        "find_text, within_text, [start_num]",
        "Gives the byte position of one string inside another, matching case.",
    ),
    (
        "FINV",
        "probability, deg_freedom1, deg_freedom2",
        "Gives the value whose right-tail F probability is the one given.",
    ),
    (
        "FISHER",
        "x",
        "Gives the Fisher transformation of a number.",
    ),
    (
        "FISHERINV",
        "y",
        "Gives the inverse of the Fisher transformation.",
    ),
    (
        "FIXED",
        "number, [decimals], [no_commas]",
        "Writes a number as text with a fixed number of decimals.",
    ),
    (
        "FLOOR",
        "number, significance",
        "Rounds a number down to the previous multiple of the significance.",
    ),
    (
        "FLOOR.MATH",
        "number, [significance], [mode]",
        "Rounds a number down to the previous multiple, with a choice of direction for negatives.",
    ),
    (
        "FLOOR.PRECISE",
        "number, [significance]",
        "Rounds a number down to the previous multiple, whatever its sign.",
    ),
    (
        "FORECAST",
        "x, known_y's, known_x's",
        "Predicts a value along a straight line fitted to known points.",
    ),
    (
        "FORECAST.ETS",
        "target_date, values, timeline, [seasonality], [data_completion], [aggregation]",
        "Predicts a future value with exponential smoothing.",
    ),
    (
        "FORECAST.ETS.CONFINT",
        "target_date, values, timeline, [confidence_level], [seasonality], [data_completion], [aggregation]",
        "Gives the confidence interval of a smoothed forecast.",
    ),
    (
        "FORECAST.ETS.SEASONALITY",
        "values, timeline, [data_completion], [aggregation]",
        "Gives the length of the repeating pattern the smoothing model found.",
    ),
    (
        "FORECAST.ETS.STAT",
        "values, timeline, statistic_type, [seasonality], [data_completion], [aggregation]",
        "Gives a statistic of the smoothed forecast model.",
    ),
    (
        "FORECAST.LINEAR",
        "x, known_y's, known_x's",
        "Predicts a value along a straight line fitted to known points.",
    ),
    (
        "FORMULATEXT",
        "reference",
        "Gives the formula in a cell as text.",
    ),
    (
        "FREQUENCY",
        "data_array, bins_array",
        "Counts how many values fall in each of the intervals.",
    ),
    (
        "FTEST",
        "array1, array2",
        "Gives the result of an F-test on two data sets.",
    ),
    (
        "FV",
        "rate, nper, pmt, [pv], [type]",
        "Gives the future value of an investment with regular payments and a fixed rate.",
    ),
    (
        "FVSCHEDULE",
        "principal, schedule",
        "Gives the future value of a sum after a series of interest rates.",
    ),
];
