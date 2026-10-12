//! Argument names and descriptions of every worksheet function the calculation engine has, for
//! the Function Arguments and Insert Function dialogs. The rows live in `catalogue_a`,
//! `catalogue_b` and `catalogue_c`; a function that is not there (a name typed by hand) gets
//! generic boxes so the dialog still works.

use super::{catalogue_a, catalogue_b, catalogue_c};

/// What the dialog shows for one function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    /// Argument names; optional ones are marked.
    pub args: Vec<Arg>,
    /// The last `period` arguments repeat (`number1, number2, ...`).
    pub repeats: bool,
    /// How many trailing arguments repeat together: 1, or 2 for pairs such as
    /// `criteria_range2, criteria2`.
    pub period: usize,
    pub help: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arg {
    pub name: String,
    pub optional: bool,
}

/// Every row of the catalogue: `(name, arguments, description)`.
fn rows() -> impl Iterator<Item = &'static (&'static str, &'static str, &'static str)> {
    catalogue_a::ROWS
        .iter()
        .chain(catalogue_b::ROWS)
        .chain(catalogue_c::ROWS)
}

/// The names of the functions the catalogue describes.
pub fn catalogue_names() -> impl Iterator<Item = &'static str> {
    rows().map(|r| r.0)
}

fn parse_args(spec: &str) -> (Vec<Arg>, bool, usize) {
    let mut repeats = false;
    let mut period = 1;
    let mut args = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(n) = part.strip_prefix("...") {
            repeats = true;
            period = n.parse().unwrap_or(1);
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
    let period = period.min(args.len().max(1));
    (args, repeats, period)
}

/// The signature of `name`, generic when the catalogue does not list it.
#[must_use]
pub fn signature(name: &str) -> Signature {
    let up = name.to_ascii_uppercase();
    match rows().find(|(n, _, _)| *n == up) {
        Some((n, spec, help)) => {
            let (args, repeats, period) = parse_args(spec);
            Signature {
                name: (*n).to_owned(),
                args,
                repeats,
                period,
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
            period: 1,
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
    /// repeating one a spare set after the last (Excel adds one as the last is filled).
    #[must_use]
    pub fn boxes(&self) -> usize {
        self.args.len() + if self.repeats { self.period } else { 0 }
    }

    /// The name of box `i` (repeating arguments continue the numbering).
    #[must_use]
    pub fn box_name(&self, i: usize) -> String {
        if let Some(a) = self.args.get(i) {
            return a.name.clone();
        }
        let n = self.args.len();
        let period = self.period.max(1).min(n.max(1));
        let round = (i - n) / period + 1;
        let base = self
            .args
            .get(n.saturating_sub(period) + (i - n) % period)
            .map_or("value", |a| a.name.as_str());
        let stem = base.trim_end_matches(|c: char| c.is_ascii_digit());
        let first: usize = base[stem.len()..].parse().unwrap_or(1);
        format!("{stem}{}", first + round)
    }

    /// True when box `i` can be left empty.
    #[must_use]
    pub fn is_optional(&self, i: usize) -> bool {
        self.args.get(i).is_none_or(|a| a.optional)
    }

    /// What argument box `i` wants, one sentence for the line under the boxes.
    #[must_use]
    pub fn arg_help(&self, i: usize) -> String {
        let name = self.box_name(i);
        let body = capitalise(&describe_arg(&self.name, &name));
        if self.is_optional(i) {
            format!("{name}: Optional. {body}")
        } else {
            format!("{name}: {body}")
        }
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// A sentence for an argument, by what it is called.
fn describe_arg(function: &str, name: &str) -> String {
    let stem = name
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_ascii_lowercase();
    let text: &str = match stem.as_str() {
        "number" | "num" => "the number to work on.",
        "value" => "the value to use.",
        "text" => "the text to work on.",
        "array" | "array_x" | "array_y" => "the array or range to use.",
        "range" => "the range of cells to look at.",
        "criteria" => "the condition, as a number, expression or text, that picks the cells.",
        "criteria_range" => "the range of cells the condition is tested on.",
        "sum_range" => "the cells to add, when they differ from the range tested.",
        "average_range" => "the cells to average, when they differ from the range tested.",
        "max_range" | "min_range" => "the cells to take the value from.",
        "logical" => "a condition that is TRUE or FALSE.",
        "logical_test" => "any value or expression that can be TRUE or FALSE.",
        "value_if_true" => "the result when the condition is TRUE.",
        "value_if_false" => "the result when the condition is FALSE.",
        "value_if_error" => "the result when the value is an error.",
        "value_if_na" => "the result when the value is #N/A.",
        "lookup_value" => "the value to look for.",
        "lookup_array" | "lookup_vector" => "the range or array to search.",
        "return_array" | "result_vector" => "the range or array to take the result from.",
        "table_array" => "the table of cells to search.",
        "col_index_num" => "the column of the table to take the result from, counting from 1.",
        "row_index_num" => "the row of the table to take the result from, counting from 1.",
        "range_lookup" => "TRUE for the nearest match in sorted data, FALSE for an exact match.",
        "match_type" | "match_mode" => "how values are matched.",
        "search_mode" => "the direction and method of the search.",
        "if_not_found" => "the result when nothing matches.",
        "index_num" => "which value to pick, counting from 1.",
        "row_num" | "rows" => "the row, or number of rows, to use.",
        "column_num" | "columns" | "cols" => "the column, or number of columns, to use.",
        "reference" | "ref" => "the cell or range to use.",
        "ref_text" => "text that reads like a cell address.",
        "a1" => "TRUE for A1-style addresses, FALSE for R1C1-style.",
        "sheet_text" => "the name of the sheet to put in the address.",
        "abs_num" => "the kind of reference: 1 absolute, 2 mixed row, 3 mixed column, 4 relative.",
        "start_num" => "where in the text to start, counting from 1.",
        "num_chars" => "how many characters to take.",
        "num_bytes" => "how many bytes to take.",
        "find_text" => "the text to find.",
        "within_text" => "the text to search in.",
        "old_text" => "the text to be changed.",
        "new_text" => "the text to put in.",
        "instance_num" => "which occurrence to change or find.",
        "delimiter" | "col_delimiter" | "row_delimiter" => "the text that marks the split.",
        "ignore_empty" => "TRUE to skip empty items.",
        "format_text" => "a number format, such as 0.00 or dd/mm/yyyy.",
        "decimals" | "num_digits" => "how many digits to keep after the decimal point.",
        "no_commas" => "TRUE to leave out the thousands separators.",
        "significance" | "multiple" => "the multiple to round to.",
        "mode" => "how negative numbers are treated.",
        "divisor" => "the number to divide by.",
        "power" => "the exponent.",
        "base" | "radix" => "the base of the number system.",
        "x" => "the value at which to evaluate the function.",
        "y" => "the value to transform.",
        "z" => "the value from the standard normal distribution.",
        "k" => "the position or fraction to look at.",
        "n" | "m" => "a number that sets the order or step.",
        "alpha" | "beta" => "a shape parameter of the distribution.",
        "mean" => "the mean of the distribution.",
        "standard_dev" => "the standard deviation of the distribution.",
        "probability" | "probability_s" => "a probability between 0 and 1.",
        "cumulative" => "TRUE for the cumulative distribution, FALSE for the density.",
        "deg_freedom" => "the degrees of freedom.",
        "trials" => "the number of independent trials.",
        "size" => "the sample size.",
        "tails" => "1 for a one-tailed test, 2 for a two-tailed test.",
        "type" => "which kind of calculation to do.",
        "known_y's" => "the known y-values.",
        "known_x's" | "new_x's" => "the x-values to use.",
        "const" => "TRUE to calculate the constant normally, FALSE to force it to a fixed value.",
        "stats" => "TRUE to return extra regression statistics.",
        "rate" | "nominal_rate" | "effect_rate" => "the interest rate per period.",
        "nper" | "npery" => "the number of periods.",
        "pmt" => "the payment made each period.",
        "pv" => "the present value.",
        "fv" => "the future value.",
        "per" | "period" => "the period to look at.",
        "guess" => "a first guess at the answer.",
        "values" => "the series of cash flows or values.",
        "dates" => "the dates that go with the cash flows.",
        "settlement" => "the date the security is bought.",
        "maturity" => "the date the security expires.",
        "issue" => "the date the security was issued.",
        "redemption" => "the amount received at maturity per 100 of face value.",
        "frequency" => "payments per year: 1 yearly, 2 half-yearly, 4 quarterly.",
        "basis" => "the day-count method to use.",
        "cost" => "the starting cost of the asset.",
        "salvage" => "the value of the asset at the end of its life.",
        "life" => "the number of periods over which the asset loses value.",
        "factor" => "the rate at which the balance declines.",
        "serial_number" | "date" => "a date or time.",
        "start_date" | "end_date" => "a date.",
        "months" | "days" => "how many to add; negative goes back.",
        "holidays" => "dates to leave out of the working days.",
        "weekend" => "which days of the week are weekend days.",
        "year" | "month" | "day" | "hour" | "minute" | "second" => {
            "the number for that part of the date or time."
        }
        "return_type" => "how the result is numbered.",
        "unit" | "from_unit" | "to_unit" => "the unit of measure.",
        "database" => "the range that holds the list, with its headings.",
        "field" => "the column to use, by heading or position.",
        "inumber" => "a complex number.",
        "places" => "how many characters to pad the result to.",
        "lambda" => "a function to apply.",
        "include" => "the TRUE and FALSE values that say what to keep.",
        "sort_index" | "sort_order" | "by_col" | "by_array" => "how to sort.",
        "link_location" => "where the link goes.",
        "friendly_name" => "the text to show in the cell.",
        "info_type" | "type_text" => "which fact to give.",
        "error_val" => "the error to look at.",
        "percent" => "the share of values to cut off.",
        "quart" => "which quartile to give.",
        "bins_array" => "the intervals to count into.",
        "data_array" => "the values to count.",
        "initial_value" => "the starting value.",
        "name" | "name_value" => "a name and the value it stands for.",
        "calculation_or_name" | "parameter_or_calculation" => {
            "a calculation, or a name to use in it."
        }
        _ => "",
    };
    if text.is_empty() {
        let readable = name.replace('_', " ");
        format!("the {readable} for {function}.")
    } else {
        text.to_owned()
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

    #[test]
    fn pair_repeats_count_both_arguments() {
        let s = signature("SUMIFS");
        assert_eq!(s.period, 2);
        assert_eq!(s.args.len(), 5);
        assert_eq!(s.boxes(), 7);
        assert_eq!(s.box_name(5), "criteria_range3");
        assert_eq!(s.box_name(6), "criteria3");
        assert_eq!(signature("IFS").box_name(4), "logical_test3");
    }

    #[test]
    fn the_catalogue_is_the_engine_function_list_and_matches_its_argument_counts() {
        let mut engine: Vec<&str> = open365_calc::functions::function_names();
        let mut ours: Vec<&str> = catalogue_names().collect();
        engine.sort_unstable();
        ours.sort_unstable();
        ours.dedup();
        let missing: Vec<&&str> = engine.iter().filter(|n| !ours.contains(n)).collect();
        let extra: Vec<&&str> = ours.iter().filter(|n| !engine.contains(n)).collect();
        assert!(missing.is_empty(), "not described: {missing:?}");
        assert!(extra.is_empty(), "not in the engine: {extra:?}");
        // The engine's parser knows how many arguments Excel accepts: the required ones must
        // be enough, and every named one must be allowed.
        let call = |name: &str, n: usize| {
            let args = vec!["1"; n].join(",");
            open365_calc::parse_formula(&format!("{name}({args})")).is_ok()
        };
        let mut wrong = Vec::new();
        for name in &ours {
            let s = signature(name);
            let required = s.args.iter().filter(|a| !a.optional).count();
            let named = s.args.len();
            if !call(name, required) {
                wrong.push(format!("{name}: {required} required arguments are refused"));
            }
            if !call(name, named) {
                wrong.push(format!("{name}: {named} arguments are refused"));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn every_argument_has_a_description() {
        for name in catalogue_names() {
            let s = signature(name);
            assert!(!s.help.is_empty(), "{name} has no description");
            for i in 0..s.boxes() {
                assert!(!s.arg_help(i).is_empty(), "{name} argument {i}");
            }
        }
        assert_eq!(
            signature("SUM").arg_help(1),
            "number2: Optional. The number to work on."
        );
    }
}
