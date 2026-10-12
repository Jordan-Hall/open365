//! Argument names and one-line descriptions of the common worksheet functions, for the Function
//! Arguments and Insert Function dialogs. Functions the table does not know get generic argument
//! boxes (`Value1`..) so the dialog still works.

/// What the dialog shows for one function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    /// Argument names; optional ones are marked.
    pub args: Vec<Arg>,
    /// The last argument repeats (`number1, number2, ...`).
    pub repeats: bool,
    pub help: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arg {
    pub name: String,
    pub optional: bool,
}

/// `(name, "arg, arg, [optional], ...", description)`. A trailing `...` repeats the last argument.
const TABLE: &[(&str, &str, &str)] = &[
    (
        "SUM",
        "number1, [number2], ...",
        "Adds all the numbers in a range of cells.",
    ),
    (
        "AVERAGE",
        "number1, [number2], ...",
        "Returns the average (arithmetic mean) of its arguments.",
    ),
    (
        "COUNT",
        "value1, [value2], ...",
        "Counts the number of cells in a range that contain numbers.",
    ),
    (
        "COUNTA",
        "value1, [value2], ...",
        "Counts the number of cells in a range that are not empty.",
    ),
    (
        "MAX",
        "number1, [number2], ...",
        "Returns the largest value in a set of values.",
    ),
    (
        "MIN",
        "number1, [number2], ...",
        "Returns the smallest number in a set of values.",
    ),
    (
        "IF",
        "logical_test, [value_if_true], [value_if_false]",
        "Checks whether a condition is met, and returns one value if TRUE, and another value if FALSE.",
    ),
    (
        "IFERROR",
        "value, value_if_error",
        "Returns value_if_error if the expression is an error and the value of the expression itself otherwise.",
    ),
    (
        "IFS",
        "logical_test1, value_if_true1, ...",
        "Checks whether one or more conditions are met and returns a value that corresponds to the first TRUE condition.",
    ),
    (
        "AND",
        "logical1, [logical2], ...",
        "Checks whether all arguments are TRUE, and returns TRUE if all arguments are TRUE.",
    ),
    (
        "OR",
        "logical1, [logical2], ...",
        "Checks whether any of the arguments are TRUE, and returns TRUE or FALSE.",
    ),
    ("NOT", "logical", "Changes FALSE to TRUE, or TRUE to FALSE."),
    (
        "VLOOKUP",
        "lookup_value, table_array, col_index_num, [range_lookup]",
        "Looks for a value in the leftmost column of a table, and then returns a value in the same row from a column you specify.",
    ),
    (
        "HLOOKUP",
        "lookup_value, table_array, row_index_num, [range_lookup]",
        "Looks for a value in the top row of a table of values and returns the value in the same column from a row you specify.",
    ),
    (
        "XLOOKUP",
        "lookup_value, lookup_array, return_array, [if_not_found], [match_mode], [search_mode]",
        "Searches a range or an array for a match and returns the corresponding item from a second range or array.",
    ),
    (
        "INDEX",
        "array, row_num, [column_num]",
        "Returns a value or reference of the cell at the intersection of a particular row and column, in a given range.",
    ),
    (
        "MATCH",
        "lookup_value, lookup_array, [match_type]",
        "Returns the relative position of an item in an array that matches a specified value in a specified order.",
    ),
    (
        "CHOOSE",
        "index_num, value1, [value2], ...",
        "Chooses a value or action to perform from a list of values, based on an index number.",
    ),
    (
        "SUMIF",
        "range, criteria, [sum_range]",
        "Adds the cells specified by a given condition or criteria.",
    ),
    (
        "SUMIFS",
        "sum_range, criteria_range1, criteria1, ...",
        "Adds the cells specified by a given set of conditions or criteria.",
    ),
    (
        "COUNTIF",
        "range, criteria",
        "Counts the number of cells within a range that meet the given condition.",
    ),
    (
        "COUNTIFS",
        "criteria_range1, criteria1, ...",
        "Counts the number of cells specified by a given set of conditions or criteria.",
    ),
    (
        "AVERAGEIF",
        "range, criteria, [average_range]",
        "Finds average (arithmetic mean) for the cells specified by a given condition or criteria.",
    ),
    (
        "SUMPRODUCT",
        "array1, [array2], ...",
        "Returns the sum of the products of corresponding ranges or arrays.",
    ),
    (
        "ROUND",
        "number, num_digits",
        "Rounds a number to a specified number of digits.",
    ),
    (
        "ROUNDUP",
        "number, num_digits",
        "Rounds a number up, away from zero.",
    ),
    (
        "ROUNDDOWN",
        "number, num_digits",
        "Rounds a number down, toward zero.",
    ),
    (
        "INT",
        "number",
        "Rounds a number down to the nearest integer.",
    ),
    (
        "MOD",
        "number, divisor",
        "Returns the remainder after a number is divided by a divisor.",
    ),
    (
        "ABS",
        "number",
        "Returns the absolute value of a number, a number without its sign.",
    ),
    (
        "POWER",
        "number, power",
        "Returns the result of a number raised to a power.",
    ),
    ("SQRT", "number", "Returns a positive square root."),
    (
        "RAND",
        "",
        "Returns a random number greater than or equal to 0 and less than 1, evenly distributed (changes on recalculation).",
    ),
    (
        "RANDBETWEEN",
        "bottom, top",
        "Returns a random number between the numbers you specify.",
    ),
    (
        "PI",
        "",
        "Returns the value of Pi, 3.14159265358979, accurate to 15 digits.",
    ),
    (
        "LEFT",
        "text, [num_chars]",
        "Returns the specified number of characters from the start of a text string.",
    ),
    (
        "RIGHT",
        "text, [num_chars]",
        "Returns the specified number of characters from the end of a text string.",
    ),
    (
        "MID",
        "text, start_num, num_chars",
        "Returns the characters from the middle of a text string, given a starting position and length.",
    ),
    (
        "LEN",
        "text",
        "Returns the number of characters in a text string.",
    ),
    ("UPPER", "text", "Converts text to uppercase."),
    (
        "LOWER",
        "text",
        "Converts all letters in a text string to lowercase.",
    ),
    (
        "PROPER",
        "text",
        "Converts a text string to proper case; the first letter in each word in uppercase, and all other letters to lowercase.",
    ),
    (
        "TRIM",
        "text",
        "Removes all spaces from a text string except for single spaces between words.",
    ),
    (
        "CONCAT",
        "text1, [text2], ...",
        "Combines the text from multiple ranges and/or strings.",
    ),
    (
        "CONCATENATE",
        "text1, [text2], ...",
        "Joins several text strings into one text string.",
    ),
    (
        "TEXTJOIN",
        "delimiter, ignore_empty, text1, [text2], ...",
        "Combines the text from multiple ranges and/or strings, and includes a delimiter you specify between each text value.",
    ),
    (
        "TEXT",
        "value, format_text",
        "Converts a value to text in a specific number format.",
    ),
    (
        "VALUE",
        "text",
        "Converts a text string that represents a number to a number.",
    ),
    (
        "FIND",
        "find_text, within_text, [start_num]",
        "Returns the starting position of one text string within another text string. FIND is case-sensitive.",
    ),
    (
        "SEARCH",
        "find_text, within_text, [start_num]",
        "Returns the number of the character at which a specific character or text string is first found.",
    ),
    (
        "SUBSTITUTE",
        "text, old_text, new_text, [instance_num]",
        "Replaces existing text with new text in a text string.",
    ),
    (
        "REPLACE",
        "old_text, start_num, num_chars, new_text",
        "Replaces part of a text string with a different text string.",
    ),
    (
        "REPT",
        "text, number_times",
        "Repeats text a given number of times.",
    ),
    ("TODAY", "", "Returns the current date formatted as a date."),
    (
        "NOW",
        "",
        "Returns the current date and time formatted as a date and time.",
    ),
    (
        "DATE",
        "year, month, day",
        "Returns the number that represents the date in Microsoft Excel date-time code.",
    ),
    (
        "YEAR",
        "serial_number",
        "Returns the year, a number from 1900-9999.",
    ),
    (
        "MONTH",
        "serial_number",
        "Returns the month, a number from 1 (January) to 12 (December).",
    ),
    (
        "DAY",
        "serial_number",
        "Returns the day of the month, a number from 1 to 31.",
    ),
    (
        "WEEKDAY",
        "serial_number, [return_type]",
        "Returns a number from 1 to 7 identifying the day of the week of a date.",
    ),
    (
        "EDATE",
        "start_date, months",
        "Returns the serial number of the date that is the indicated number of months before or after the start date.",
    ),
    (
        "EOMONTH",
        "start_date, months",
        "Returns the serial number of the last day of the month before or after a specified number of months.",
    ),
    (
        "DATEDIF",
        "start_date, end_date, unit",
        "Calculates the number of days, months, or years between two dates.",
    ),
    (
        "PMT",
        "rate, nper, pv, [fv], [type]",
        "Calculates the payment for a loan based on constant payments and a constant interest rate.",
    ),
    (
        "FV",
        "rate, nper, pmt, [pv], [type]",
        "Returns the future value of an investment based on periodic, constant payments and a constant interest rate.",
    ),
    (
        "PV",
        "rate, nper, pmt, [fv], [type]",
        "Returns the present value of an investment: the total amount that a series of future payments is worth now.",
    ),
    (
        "NPV",
        "rate, value1, [value2], ...",
        "Returns the net present value of an investment based on a discount rate and a series of future payments and income.",
    ),
    (
        "IRR",
        "values, [guess]",
        "Returns the internal rate of return for a series of cash flows.",
    ),
    (
        "ISBLANK",
        "value",
        "Checks whether a reference is to an empty cell, and returns TRUE or FALSE.",
    ),
    (
        "ISNUMBER",
        "value",
        "Checks whether a value is a number, and returns TRUE or FALSE.",
    ),
    (
        "ISTEXT",
        "value",
        "Checks whether a value is text, and returns TRUE or FALSE.",
    ),
    (
        "ISERROR",
        "value",
        "Checks whether a value is an error, and returns TRUE or FALSE.",
    ),
    (
        "MEDIAN",
        "number1, [number2], ...",
        "Returns the median, or the number in the middle of the set of given numbers.",
    ),
    (
        "STDEV.S",
        "number1, [number2], ...",
        "Estimates standard deviation based on a sample (ignores logical values and text in the sample).",
    ),
    (
        "LARGE",
        "array, k",
        "Returns the k-th largest value in a data set.",
    ),
    (
        "SMALL",
        "array, k",
        "Returns the k-th smallest value in a data set.",
    ),
    (
        "RANK",
        "number, ref, [order]",
        "Returns the rank of a number in a list of numbers.",
    ),
    (
        "ROW",
        "[reference]",
        "Returns the row number of a reference.",
    ),
    (
        "COLUMN",
        "[reference]",
        "Returns the column number of a reference.",
    ),
    (
        "OFFSET",
        "reference, rows, cols, [height], [width]",
        "Returns a reference to a range that is a specified number of rows and columns from a cell or range of cells.",
    ),
    (
        "INDIRECT",
        "ref_text, [a1]",
        "Returns a reference specified by a text string.",
    ),
    (
        "SORT",
        "array, [sort_index], [sort_order], [by_col]",
        "Sorts a range or array.",
    ),
    (
        "FILTER",
        "array, include, [if_empty]",
        "Filters a range or array.",
    ),
    (
        "UNIQUE",
        "array, [by_col], [exactly_once]",
        "Returns a list of unique values in a list or range.",
    ),
];

fn parse_args(spec: &str) -> (Vec<Arg>, bool) {
    let mut repeats = false;
    let mut args = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if part == "..." {
            repeats = true;
            continue;
        }
        let (name, optional) = match part.strip_prefix('[').and_then(|p| p.strip_suffix(']')) {
            Some(n) => (n, true),
            None => (part, false),
        };
        args.push(Arg {
            name: name.to_owned(),
            optional,
        });
    }
    (args, repeats)
}

/// The signature of `name`, generic when the table does not list it.
#[must_use]
pub fn signature(name: &str) -> Signature {
    let up = name.to_ascii_uppercase();
    match TABLE.iter().find(|(n, _, _)| *n == up) {
        Some((n, spec, help)) => {
            let (args, repeats) = parse_args(spec);
            Signature {
                name: (*n).to_owned(),
                args,
                repeats,
                help: (*help).to_owned(),
            }
        }
        None => Signature {
            name: up,
            args: (1..=3)
                .map(|i| Arg {
                    name: format!("value{i}"),
                    optional: i > 1,
                })
                .collect(),
            repeats: true,
            help: String::new(),
        },
    }
}

impl Signature {
    /// `SUM(number1,[number2],...)` as Insert Function's description shows it.
    #[must_use]
    pub fn syntax(&self) -> String {
        let args: Vec<String> = self
            .args
            .iter()
            .map(|a| {
                if a.optional {
                    format!("[{}]", a.name)
                } else {
                    a.name.clone()
                }
            })
            .collect();
        let tail = if self.repeats { ",..." } else { "" };
        format!("{}({}{tail})", self.name, args.join(","))
    }

    /// How many argument boxes Function Arguments shows: every named argument, and for a
    /// repeating one a spare box after the last (Excel adds one as the last is filled).
    #[must_use]
    pub fn boxes(&self) -> usize {
        self.args.len() + usize::from(self.repeats)
    }

    /// The name of box `i` (repeating arguments continue the numbering).
    #[must_use]
    pub fn box_name(&self, i: usize) -> String {
        match self.args.get(i) {
            Some(a) => a.name.clone(),
            None => {
                let last = self.args.last().map_or("value", |a| a.name.as_str());
                let stem = last.trim_end_matches(|c: char| c.is_ascii_digit());
                format!("{stem}{}", i + 1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown_functions() {
        let s = signature("sum");
        assert_eq!(s.syntax(), "SUM(number1,[number2],...)");
        assert_eq!(s.boxes(), 3);
        assert_eq!(s.box_name(2), "number3");
        let v = signature("VLOOKUP");
        assert_eq!(v.args.len(), 4);
        assert!(v.args[3].optional && !v.args[0].optional);
        let g = signature("ZZZ");
        assert_eq!(g.args.len(), 3);
        assert!(g.help.is_empty());
    }
}
