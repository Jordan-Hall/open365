//! Sheets ribbon: the Data tab (`data.*`).
//!
//! Every control is a `DataOp` command applied to the selection (see `sheet_ext::data`), so each
//! is one undo step. A control that is a dialog in Excel asks the shell for it
//! (`SheetModel::ext_data.take_request()`) when clicked bare, and also takes the dialog's answer
//! as a value after `=` (the shell draws no Sort, Text to Columns... dialog yet, so tests and the
//! agent surface use these forms):
//!
//! - `data.sort-filter.sort=B:desc,C;header=yes;case` (columns by letter, `;left` sorts left to right)
//! - `data.sort-filter.filter=B:values=a|b|(Blanks)`, `B:gt=10`, `B:top=10`, `B:above`, `B:clear`
//! - `data.sort-filter.advanced=list=A1:D9;criteria=F1:F2;copy=H1;unique`
//! - `data.data-tools.text-to-columns=delim=comma,space;consecutive;dest=B1` or `fixed=5,10`
//! - `data.data-tools.remove-duplicates=headers=yes;cols=A,C`
//! - `data.data-tools.data-validation=list=a,b,c` / `whole=between,1,10` / `custom==A1>0` / `clear`
//! - `data.data-tools.consolidate=fn=sum;ref=A1:B3;ref=Sheet2!A1:B3;top;left;dest=F1`
//! - `data.forecast.what-if-analysis=goalseek=B2,100,A1`
//! - `data.outline.subtotal=at=A;fn=sum;cols=C,D;above` or `remove`
//!
//! Not wired, each needing a subsystem: Get Data and its From sources, Recent Sources, Existing
//! Connections, Refresh All, Queries & Connections, Properties, Workbook Links (queries and
//! connections), Data Types (linked data), Clean Data, Quick Fill (Flash Fill inference), Data
//! Model, Analyse Data, Forecast Sheet, and What-If's Scenario Manager and Data Table.

use open365_commands::RangeRef;
use open365_commands::data_ops::{
    Aggregate, AllowKind, Cmp, ColumnFilter, DataCmp, ErrorAlert, FilterAction, SortHeader,
    SortLevel, SortRequest, SourceRange, SplitMode, SubtotalRequest, ValidationRule,
};
use open365_xlsx::col_number;

use crate::ribbon::{Handled, RibbonCtx, base};
use crate::sheet::sheet_ext::data::Request;
use crate::sheet::{SheetModel, parse_a1};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "data.get-transform-data.from-text-csv",
    "data.get-transform-data.recent-sources",
    "data.get-transform-data.existing-connections",
    "data.queries-connections.refresh-all",
    "data.queries-connections.queries-connections",
    "data.queries-connections.properties",
    "data.queries-connections.workbook-links",
    "data.data-tools.quick-fill",
    "data.forecast.forecast-sheet",
    "data.sort-filter.sort-a-to-z",
    "data.sort-filter.sort-z-to-a",
    "data.sort-filter.sort",
    "data.sort-filter.filter",
    "data.sort-filter.clear",
    "data.sort-filter.reapply",
    "data.sort-filter.advanced",
    "data.data-tools.text-to-columns",
    "data.data-tools.remove-duplicates",
    "data.data-tools.data-validation",
    "data.data-tools.consolidate",
    "data.forecast.what-if-analysis",
    "data.outline.group",
    "data.outline.ungroup",
    "data.outline.subtotal",
    "data.outline.show-detail",
    "data.outline.hide-detail",
];

type R4 = (u32, u32, u32, u32);

/// `(control id, item after it, value after =)`.
fn split(id: &str) -> Option<(&'static str, &str, Option<&str>)> {
    let b = base(id);
    let ctrl = IDS
        .iter()
        .find(|c| b == **c || b.strip_prefix(**c).is_some_and(|r| r.starts_with('.')))?;
    let sub = b[ctrl.len()..].trim_start_matches('.');
    let arg = id.split_once('=').map(|(_, a)| a);
    Some((ctrl, sub, arg))
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let Some((ctrl, sub, arg)) = split(id) else {
        return Handled::No;
    };
    let m = &mut *ctx.sheet;
    if let Err(e) = run(m, ctrl, sub, arg) {
        m.status = e;
    }
    Handled::Yes
}

fn run(m: &mut SheetModel, ctrl: &str, sub: &str, arg: Option<&str>) -> Result<(), String> {
    let tail = ctrl.rsplit('.').next().unwrap_or(ctrl);
    let ask = |m: &mut SheetModel, r: Request| {
        m.ext_data.request(r);
        Ok(())
    };
    match tail {
        "sort-a-to-z" => m.data_sort_quick(false),
        "sort-z-to-a" => m.data_sort_quick(true),
        "sort" => match arg {
            Some(a) => sort_from(m, a),
            None => ask(m, Request::Sort),
        },
        "filter" => match arg {
            Some(a) => filter_from(m, a),
            None => m.data_toggle_filter(),
        },
        "clear" => m.data_clear_filter(),
        "reapply" => m.data_reapply_filter(),
        "advanced" => match arg {
            Some(a) => advanced_from(m, a),
            None => ask(m, Request::AdvancedFilter),
        },
        "text-to-columns" => match arg {
            Some(a) => text_to_columns_from(m, a),
            None => ask(m, Request::TextToColumns),
        },
        "remove-duplicates" => match arg {
            Some(a) => remove_duplicates_from(m, a),
            None => ask(m, Request::RemoveDuplicates),
        },
        "data-validation" if sub == "circle-invalid-data" => {
            m.data_circle_invalid();
            Ok(())
        }
        "data-validation" if sub == "clear-validation-circles" => {
            m.data_clear_circles();
            Ok(())
        }
        "data-validation" => match arg {
            Some(a) => validation_from(m, a),
            None => ask(m, Request::DataValidation),
        },
        "consolidate" => match arg {
            Some(a) => consolidate_from(m, a),
            None => ask(m, Request::Consolidate),
        },
        "quick-fill" => m.data_quick_fill(),
        "from-text-csv" => match arg {
            Some(a) => import_from(m, a),
            None => {
                m.status = "Choose a text or CSV file to import.".to_owned();
                Ok(())
            }
        },
        "forecast-sheet" => forecast_from(m, arg.unwrap_or("")),
        "refresh-all" => {
            m.data_refresh_all();
            Ok(())
        }
        "queries-connections"
        | "existing-connections"
        | "recent-sources"
        | "properties"
        | "workbook-links" => {
            m.data_connections();
            Ok(())
        }
        "what-if-analysis" => match (sub, arg) {
            (_, Some(a)) if a.starts_with("scenario=") => scenario_from(m, a),
            (_, Some(a)) if a.starts_with("datatable") => data_table_from(m, a),
            ("scenario-manager", None) => {
                m.status = "Scenario Manager needs its dialog.".to_owned();
                Ok(())
            }
            ("data-table", None) => data_table_from(m, "datatable"),
            (_, Some(a)) if a.starts_with("goalseek=") => {
                goal_seek_from(m, &a["goalseek=".len()..])
            }
            ("goal-seek", None) => ask(m, Request::GoalSeek),
            (_, None) if sub.is_empty() || sub == "run" => ask(m, Request::GoalSeek),
            _ => Err("Unknown What-If Analysis command.".to_owned()),
        },
        "group" => match arg {
            Some("columns") => m.data_group_cols(),
            Some("rows") => m.data_group_rows(),
            _ if m.data_selection_is_columns() => m.data_group_cols(),
            _ => m.data_group_rows(),
        },
        "ungroup" => match arg {
            Some("columns") => m.data_ungroup_cols(),
            Some("rows") => m.data_ungroup_rows(),
            _ if m.data_selection_is_columns() => m.data_ungroup_cols(),
            _ => m.data_ungroup_rows(),
        },
        "subtotal" => match arg {
            Some("remove") => m.data_remove_subtotals(),
            Some(a) => subtotal_from(m, a),
            None => ask(m, Request::Subtotal),
        },
        "show-detail" if m.data_selection_is_columns() => m.data_detail_cols(true),
        "hide-detail" if m.data_selection_is_columns() => m.data_detail_cols(false),
        "show-detail" => m.data_detail(true),
        "hide-detail" => m.data_detail(false),
        _ => Ok(()),
    }
}

// ---- value forms -------------------------------------------------------------------------

fn parts(arg: &str) -> Vec<(&str, Option<&str>)> {
    arg.split(';')
        .filter(|p| !p.trim().is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (k.trim(), Some(v)),
            None => (p.trim(), None),
        })
        .collect()
}

fn col(s: &str) -> Result<u32, String> {
    col_number(&s.trim().trim_start_matches('$').to_ascii_uppercase())
        .ok_or_else(|| format!("'{s}' is not a column."))
}

fn cell(s: &str) -> Result<(u32, u32), String> {
    parse_a1(s).ok_or_else(|| format!("'{s}' is not a cell reference."))
}

fn rect(s: &str) -> Result<R4, String> {
    open365_xlsx::parse_range(&s.replace('$', "")).ok_or_else(|| format!("'{s}' is not a range."))
}

fn rr(r: R4) -> RangeRef {
    RangeRef {
        first_row: r.0,
        first_col: r.1,
        last_row: r.2,
        last_col: r.3,
    }
}

fn sort_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let mut levels = Vec::new();
    let mut header = SortHeader::Guess;
    let mut case_sensitive = false;
    let mut left_to_right = false;
    for (i, (k, v)) in parts(arg).into_iter().enumerate() {
        if i == 0 && v.is_none() {
            for item in k.split(',') {
                let (c, order) = item.split_once(':').unwrap_or((item, "asc"));
                levels.push(SortLevel {
                    index: col(c)?,
                    descending: order.trim().eq_ignore_ascii_case("desc"),
                    custom_list: None,
                });
            }
            continue;
        }
        match (k, v) {
            ("header", Some("yes")) => header = SortHeader::Yes,
            ("header", Some("no")) => header = SortHeader::No,
            ("case", _) => case_sensitive = true,
            ("left", _) => left_to_right = true,
            _ => return Err(format!("Unknown sort option '{k}'.")),
        }
    }
    if levels.is_empty() {
        return Err("Add a column to sort by.".to_owned());
    }
    let range = m.data_target();
    m.data_sort(SortRequest {
        range: rr(range),
        header,
        left_to_right,
        case_sensitive,
        levels,
    })
}

fn filter_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let (c, rest) = arg.split_once(':').ok_or("Use <column>:<filter>.")?;
    let column = col(c)?;
    let (kind, value) = match rest.split_once('=') {
        Some((k, v)) => (k, v),
        None => (rest, ""),
    };
    let cmp = |op| ColumnFilter::Custom {
        first: (op, value.to_owned()),
        second: None,
        and: true,
    };
    let filter = match kind {
        "clear" => None,
        "values" => {
            let mut items: Vec<String> = value.split('|').map(str::to_owned).collect();
            let blank = items.iter().any(|i| i == "(Blanks)");
            items.retain(|i| i != "(Blanks)");
            Some(ColumnFilter::Values { items, blank })
        }
        "eq" => Some(cmp(Cmp::Equal)),
        "ne" => Some(cmp(Cmp::NotEqual)),
        "gt" => Some(cmp(Cmp::Greater)),
        "ge" => Some(cmp(Cmp::GreaterOrEqual)),
        "lt" => Some(cmp(Cmp::Less)),
        "le" => Some(cmp(Cmp::LessOrEqual)),
        "top" | "bottom" | "toppct" | "bottompct" => Some(ColumnFilter::Top10 {
            top: kind.starts_with("top"),
            percent: kind.ends_with("pct"),
            count: value
                .trim()
                .parse()
                .map_err(|_| "Enter a number of items.")?,
        }),
        "above" => Some(ColumnFilter::AboveAverage),
        "below" => Some(ColumnFilter::BelowAverage),
        other => return Err(format!("Unknown filter '{other}'.")),
    };
    m.data_filter_column(column, filter)
}

fn advanced_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let (mut list, mut criteria, mut copy, mut unique) = (None, None, None, false);
    for (k, v) in parts(arg) {
        match (k, v) {
            ("list", Some(v)) => list = Some(rect(v)?),
            ("criteria", Some(v)) => criteria = Some(rect(v)?),
            ("copy", Some(v)) => copy = Some(cell(v)?),
            ("unique", _) => unique = true,
            _ => return Err(format!("Unknown option '{k}'.")),
        }
    }
    let list = list.unwrap_or_else(|| m.data_target());
    let criteria = criteria.ok_or("Enter a criteria range.")?;
    let action = match copy {
        Some((row, col)) => FilterAction::CopyTo { row, col },
        None => FilterAction::InPlace,
    };
    m.data_advanced_filter(list, criteria, action, unique)
}

fn text_to_columns_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let mut delimited = (false, false, false, false, None::<char>);
    let (mut consecutive, mut qualifier, mut fixed, mut dest) = (false, Some('"'), None, None);
    for (k, v) in parts(arg) {
        match (k, v) {
            ("delim", Some(v)) => {
                for d in v.split(',') {
                    match d.trim() {
                        "tab" => delimited.0 = true,
                        "semicolon" => delimited.1 = true,
                        "comma" => delimited.2 = true,
                        "space" => delimited.3 = true,
                        other if other.chars().count() == 1 => delimited.4 = other.chars().next(),
                        other => return Err(format!("Unknown delimiter '{other}'.")),
                    }
                }
            }
            ("consecutive", _) => consecutive = true,
            ("qualifier", Some("none")) => qualifier = None,
            ("qualifier", Some(q)) => qualifier = q.chars().next(),
            ("fixed", Some(v)) => {
                fixed = Some(
                    v.split(',')
                        .map(|n| n.trim().parse::<usize>().map_err(|_| "Bad break position."))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            ("dest", Some(v)) => dest = Some(cell(v)?),
            _ => return Err(format!("Unknown option '{k}'.")),
        }
    }
    let split = match fixed {
        Some(breaks) => SplitMode::Fixed { breaks },
        None => SplitMode::Delimited {
            tab: delimited.0,
            semicolon: delimited.1,
            comma: delimited.2,
            space: delimited.3,
            other: delimited.4,
            consecutive,
            qualifier,
        },
    };
    m.data_text_to_columns(split, dest)
}

fn remove_duplicates_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let (mut headers, mut cols) = (false, Vec::new());
    for (k, v) in parts(arg) {
        match (k, v) {
            ("headers", Some(v)) => headers = v == "yes",
            ("cols", Some(v)) => {
                for c in v.split(',') {
                    cols.push(col(c)?);
                }
            }
            _ => return Err(format!("Unknown option '{k}'.")),
        }
    }
    m.data_remove_duplicates(headers, &cols).map(|_| ())
}

fn validation_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    if matches!(arg.trim(), "clear" | "any") {
        return m.data_set_validation(None);
    }
    let (kind, rest) = arg.split_once('=').ok_or("Use <kind>=<settings>.")?;
    let allow = match kind.trim() {
        "whole" => AllowKind::WholeNumber,
        "decimal" => AllowKind::Decimal,
        "list" => AllowKind::List,
        "date" => AllowKind::Date,
        "time" => AllowKind::Time,
        "textlength" => AllowKind::TextLength,
        "custom" => AllowKind::Custom,
        other => return Err(format!("Unknown validation type '{other}'.")),
    };
    let rule = |compare, v1: &str, v2: Option<&str>| ValidationRule {
        allow,
        compare,
        value1: v1.to_owned(),
        value2: v2.map(str::to_owned),
        ignore_blank: true,
        in_cell_dropdown: true,
        input_message: None,
        error_alert: Some(ErrorAlert {
            style: "stop".to_owned(),
            title: String::new(),
            message: String::new(),
        }),
    };
    let r = match allow {
        AllowKind::List | AllowKind::Custom => rule(DataCmp::Between, rest, None),
        _ => {
            let mut it = rest.splitn(3, ',');
            let op = it.next().unwrap_or("between");
            let v1 = it.next().ok_or("Enter a value.")?;
            let v2 = it.next();
            let compare = match op {
                "between" => DataCmp::Between,
                "notbetween" => DataCmp::NotBetween,
                "eq" => DataCmp::Cmp(Cmp::Equal),
                "ne" => DataCmp::Cmp(Cmp::NotEqual),
                "gt" => DataCmp::Cmp(Cmp::Greater),
                "ge" => DataCmp::Cmp(Cmp::GreaterOrEqual),
                "lt" => DataCmp::Cmp(Cmp::Less),
                "le" => DataCmp::Cmp(Cmp::LessOrEqual),
                other => return Err(format!("Unknown operator '{other}'.")),
            };
            rule(compare, v1, v2)
        }
    };
    m.data_set_validation(Some(r))
}

fn aggregate(name: &str) -> Result<Aggregate, String> {
    Ok(match name.trim().to_ascii_lowercase().as_str() {
        "sum" => Aggregate::Sum,
        "count" => Aggregate::Count,
        "average" | "avg" => Aggregate::Average,
        "max" => Aggregate::Max,
        "min" => Aggregate::Min,
        "product" => Aggregate::Product,
        "countnums" => Aggregate::CountNums,
        "stdev" => Aggregate::StdDev,
        "stdevp" => Aggregate::StdDevP,
        "var" => Aggregate::Var,
        "varp" => Aggregate::VarP,
        other => return Err(format!("Unknown function '{other}'.")),
    })
}

fn consolidate_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let (mut function, mut sources, mut top, mut left, mut dest) =
        (Aggregate::Sum, Vec::new(), false, false, None);
    for (k, v) in parts(arg) {
        match (k, v) {
            ("fn", Some(v)) => function = aggregate(v)?,
            ("ref", Some(v)) => {
                let (sheet, r) = match v.rsplit_once('!') {
                    Some((name, r)) => (
                        m.data_sheet_index(name.trim_matches('\''))
                            .ok_or_else(|| format!("There is no sheet '{name}'."))?,
                        r,
                    ),
                    None => (m.sheet, v),
                };
                sources.push(SourceRange {
                    sheet,
                    range: rr(rect(r)?),
                });
            }
            ("top", _) => top = true,
            ("left", _) => left = true,
            ("dest", Some(v)) => dest = Some(cell(v)?),
            _ => return Err(format!("Unknown option '{k}'.")),
        }
    }
    let dest = dest.unwrap_or(m.active);
    m.data_consolidate(function, sources, top, left, dest)
}

fn goal_seek_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let p: Vec<&str> = arg.split(',').collect();
    let [set, to, change] = p[..] else {
        return Err("Use <set cell>,<to value>,<by changing cell>.".to_owned());
    };
    let to: f64 = to
        .trim()
        .parse()
        .map_err(|_| "Enter a number for To value.")?;
    m.data_goal_seek(cell(set)?, to, cell(change)?).map(|_| ())
}

fn subtotal_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let (mut at, mut function, mut cols, mut below, mut keep) =
        (None, Aggregate::Sum, Vec::new(), true, false);
    for (k, v) in parts(arg) {
        match (k, v) {
            ("at", Some(v)) => at = Some(col(v)?),
            ("fn", Some(v)) => function = aggregate(v)?,
            ("cols", Some(v)) => {
                for c in v.split(',') {
                    cols.push(col(c)?);
                }
            }
            ("above", _) => below = false,
            ("keep", _) => keep = true,
            _ => return Err(format!("Unknown option '{k}'.")),
        }
    }
    let range = m.data_target();
    m.data_subtotal(SubtotalRequest {
        range: rr(range),
        at_change: at.ok_or("Choose a column for 'At each change in'.")?,
        function,
        columns: cols,
        replace_current: !keep,
        summary_below: below,
    })
}

// ---- Quick Fill, text import, scenarios, data table, forecast -----------------------------

fn import_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    use crate::sheet_ext::data_more::TextImport;
    // `text=` carries the payload and so is the last option (it may contain `;`).
    let (opts, inline) = match arg.find("text=") {
        Some(i) if i == 0 || arg[..i].ends_with(';') => (&arg[..i], Some(&arg[i + 5..])),
        _ => (arg, None),
    };
    let (mut file, mut delim, mut dest) = (None, None, None);
    let (mut consecutive, mut start, mut qualifier) = (false, 1usize, Some('"'));
    let mut fixed = None;
    for (k, v) in parts(opts) {
        match (k, v) {
            ("file", Some(v)) => file = Some(v.to_owned()),
            ("delim", Some(v)) => delim = Some(v.to_owned()),
            ("consecutive", _) => consecutive = true,
            ("start", Some(v)) => start = v.trim().parse().map_err(|_| "Bad start row.")?,
            ("qualifier", Some("none")) => qualifier = None,
            ("qualifier", Some(q)) => qualifier = q.chars().next(),
            ("fixed", Some(v)) => {
                fixed = Some(
                    v.split(',')
                        .map(|n| n.trim().parse::<usize>().map_err(|_| "Bad break position."))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            ("dest", Some(v)) => dest = Some(cell(v)?),
            _ => return Err(format!("Unknown option '{k}'.")),
        }
    }
    let text = match (inline, file) {
        (Some(t), _) => t.replace("\\n", "\n").replace("\\t", "\t"),
        (None, Some(path)) => {
            let bytes = std::fs::read(&path).map_err(|e| format!("Cannot read the file: {e}"))?;
            if bytes.len() > 256 * 1024 * 1024 {
                return Err("The file is too large to import.".to_owned());
            }
            String::from_utf8_lossy(&bytes).into_owned()
        }
        (None, None) => return Err("Choose a text or CSV file to import.".to_owned()),
    };
    let sample = text
        .lines()
        .skip(start.saturating_sub(1))
        .collect::<Vec<_>>()
        .join("\n");
    let mut opts = TextImport::detect(&sample);
    if let Some(d) = delim.as_deref() {
        opts.delimiters = d
            .split(',')
            .filter_map(|n| match n.trim() {
                "tab" => Some('\t'),
                "semicolon" => Some(';'),
                "comma" => Some(','),
                "space" => Some(' '),
                "pipe" => Some('|'),
                other => other.chars().next().filter(|_| other.chars().count() == 1),
            })
            .collect();
    }
    opts.fixed = fixed;
    opts.consecutive = consecutive;
    opts.qualifier = qualifier;
    opts.start_row = start;
    m.data_import_text(&text, &opts, dest).map(|_| ())
}

fn cells_list(v: &str) -> Result<Vec<(u32, u32)>, String> {
    v.split(',').map(|c| cell(c.trim())).collect()
}

fn scenario_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let (mut action, mut name, mut cells, mut values) = ("", "", Vec::new(), None);
    let (mut comment, mut results) = ("", Vec::new());
    for (k, v) in parts(arg) {
        match (k, v) {
            ("scenario", Some(v)) => action = v,
            ("name", Some(v)) => name = v,
            ("cells", Some(v)) => cells = cells_list(v)?,
            ("values", Some(v)) => values = Some(v.split('|').map(str::to_owned).collect()),
            ("comment", Some(v)) => comment = v,
            ("results", Some(v)) => results = cells_list(v)?,
            _ => return Err(format!("Unknown option '{k}'.")),
        }
    }
    match action {
        "add" => m.data_scenario_add(name, cells, values, comment),
        "show" => m.data_scenario_show(name),
        "delete" => m.data_scenario_delete(name),
        "summary" => m.data_scenario_summary(&results),
        other => Err(format!("Unknown scenario action '{other}'.")),
    }
}

fn data_table_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let (mut row_in, mut col_in) = (None, None);
    for (k, v) in parts(arg) {
        match (k, v) {
            ("datatable", _) => {}
            ("row", Some(v)) => row_in = Some(cell(v)?),
            ("col", Some(v)) => col_in = Some(cell(v)?),
            _ => return Err(format!("Unknown option '{k}'.")),
        }
    }
    if row_in.is_none() && col_in.is_none() {
        m.status = "Data Table needs a row input cell or a column input cell.".to_owned();
        return Ok(());
    }
    m.data_table(row_in, col_in)
}

fn forecast_from(m: &mut SheetModel, arg: &str) -> Result<(), String> {
    let (mut periods, mut confidence) = (10usize, 0.95);
    for (k, v) in parts(arg) {
        match (k, v) {
            ("periods", Some(v)) => periods = v.trim().parse().map_err(|_| "Bad period count.")?,
            ("confidence", Some(v)) => {
                confidence = v.trim().parse::<f64>().map_err(|_| "Bad confidence.")? / 100.0;
            }
            _ => return Err(format!("Unknown option '{k}'.")),
        }
    }
    m.data_forecast_sheet(periods, confidence)
}
