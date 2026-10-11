//! Typed views over the lossless pivot parts. Each part keeps its [`ParsedXml`] tree, so
//! every attribute and element the views do not read round-trips untouched.

use open365_xlsx::{ParsedXml, XmlElement, parse_xml};

use crate::error::PivotError;
use crate::location::Location;
use crate::value::{Value, iso_to_serial};
use crate::xmlutil::{attr_bool, attr_f64, attr_string, attr_u32, children_main};

fn parse_root(bytes: &[u8], local: &str, what: &'static str) -> Result<ParsedXml, PivotError> {
    let xml = parse_xml(bytes).map_err(|e| PivotError::Xml(e.to_string()))?;
    if !xml.root.head.is_main(local) {
        return Err(PivotError::WrongPart(what));
    }
    Ok(xml)
}

/// Reads one `m`/`n`/`b`/`e`/`s`/`d` item element.
fn item_value(e: &XmlElement, date1904: bool) -> Option<Value> {
    let v = e.head.attr("v");
    match e.head.local_name() {
        "m" => Some(Value::Blank),
        "n" => v?.trim().parse().ok().map(Value::Number),
        "b" => Some(Value::Bool(matches!(v?, "1" | "true"))),
        "e" => Some(Value::error_from_literal(v?)),
        "s" => Some(Value::Text(v?.to_owned())),
        "d" => iso_to_serial(v?, date1904).map(Value::Number),
        _ => None,
    }
}

/// Where the cache gets its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheSource {
    /// A worksheet range (`ref` on `sheet`) or a defined name / table (`name`).
    Worksheet {
        sheet: Option<String>,
        reference: Option<String>,
        name: Option<String>,
    },
    /// External connection, consolidation or scenario: not computed here.
    Other(String),
}

/// How a field is grouped (`fieldGroup`), as far as the model reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldGroup {
    pub parent: Option<u32>,
    pub base: Option<u32>,
    pub group_by: Option<String>,
    pub interval: Option<f64>,
    pub start_num: Option<f64>,
    pub end_num: Option<f64>,
    /// `rangePr/@startDate` and `@endDate` as serial numbers.
    pub start_date: Option<f64>,
    pub end_date: Option<f64>,
    /// `rangePr/@autoStart` and `@autoEnd` (default true): the bound follows the data on refresh.
    pub auto_start: bool,
    pub auto_end: bool,
    pub items: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CacheField {
    pub name: String,
    pub num_fmt_id: u32,
    /// `sharedItems` in stored order.
    pub shared: Vec<Value>,
    pub contains_date: bool,
    pub group: Option<FieldGroup>,
    pub formula: Option<String>,
    /// False for derived group fields that have no record column.
    pub database_field: bool,
}

impl CacheField {
    /// The items `x` indices of pivot field items refer to: the group items of a grouped
    /// field, the shared items otherwise.
    pub fn item_values(&self) -> &[Value] {
        match &self.group {
            Some(g) if !g.items.is_empty() => &g.items,
            _ => &self.shared,
        }
    }
}

/// A calculated item (`calculatedItems/calculatedItem`): a formula over the items of one field,
/// shown as a normal item whose shared item (`item`) has no cache records.
#[derive(Debug, Clone, PartialEq)]
pub struct CalculatedItem {
    /// Cache field the item belongs to.
    pub field: usize,
    /// Index into the field's shared items (the `pivotArea` reference).
    pub item: u32,
    /// Formula as stored, e.g. `Product[Apple]+Product[Banana]`.
    pub formula: String,
}

/// `xl/pivotCache/pivotCacheDefinitionN.xml`.
#[derive(Debug, Clone)]
pub struct CacheDefinition {
    pub source: CacheSource,
    pub fields: Vec<CacheField>,
    pub calculated_items: Vec<CalculatedItem>,
    pub record_count: Option<u32>,
    pub refresh_on_load: bool,
    xml: ParsedXml,
}

impl CacheDefinition {
    pub fn parse(bytes: &[u8], date1904: bool) -> Result<Self, PivotError> {
        let xml = parse_root(bytes, "pivotCacheDefinition", "pivot cache definition")?;
        let root = &xml.root;
        let source = match root.child_main("cacheSource") {
            Some(cs) => match cs.head.attr("type").unwrap_or("worksheet") {
                "worksheet" => match cs.child_main("worksheetSource") {
                    Some(ws) => CacheSource::Worksheet {
                        sheet: attr_string(ws, "sheet"),
                        reference: attr_string(ws, "ref"),
                        name: attr_string(ws, "name"),
                    },
                    None => CacheSource::Other("worksheet source missing".into()),
                },
                other => CacheSource::Other(other.to_owned()),
            },
            None => CacheSource::Other("no cacheSource".into()),
        };
        let mut fields = Vec::new();
        if let Some(cf) = root.child_main("cacheFields") {
            for f in children_main(cf, "cacheField") {
                let mut shared = Vec::new();
                let mut contains_date = false;
                if let Some(si) = f.child_main("sharedItems") {
                    contains_date = attr_bool(si, "containsDate", false);
                    shared.extend(si.elements().filter_map(|e| item_value(e, date1904)));
                }
                let group = f.child_main("fieldGroup").map(|g| {
                    let range = g.child_main("rangePr");
                    FieldGroup {
                        parent: attr_u32(g, "par"),
                        base: attr_u32(g, "base"),
                        group_by: range.and_then(|r| attr_string(r, "groupBy")),
                        interval: range.and_then(|r| attr_f64(r, "groupInterval")),
                        start_num: range.and_then(|r| attr_f64(r, "startNum")),
                        end_num: range.and_then(|r| attr_f64(r, "endNum")),
                        start_date: range
                            .and_then(|r| r.head.attr("startDate"))
                            .and_then(|t| iso_to_serial(t, date1904)),
                        end_date: range
                            .and_then(|r| r.head.attr("endDate"))
                            .and_then(|t| iso_to_serial(t, date1904)),
                        auto_start: range.is_none_or(|r| attr_bool(r, "autoStart", true)),
                        auto_end: range.is_none_or(|r| attr_bool(r, "autoEnd", true)),
                        items: g
                            .child_main("groupItems")
                            .map(|gi| {
                                gi.elements()
                                    .filter_map(|e| item_value(e, date1904))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    }
                });
                fields.push(CacheField {
                    name: attr_string(f, "name").unwrap_or_default(),
                    num_fmt_id: attr_u32(f, "numFmtId").unwrap_or(0),
                    shared,
                    contains_date,
                    group,
                    formula: attr_string(f, "formula"),
                    database_field: attr_bool(f, "databaseField", true),
                });
            }
        }
        let calculated_items = root
            .child_main("calculatedItems")
            .map(|ci| {
                children_main(ci, "calculatedItem")
                    .filter_map(|c| {
                        let formula = attr_string(c, "formula")?;
                        let reference = c
                            .child_main("pivotArea")?
                            .child_main("references")?
                            .child_main("reference")?;
                        Some(CalculatedItem {
                            field: attr_u32(reference, "field")? as usize,
                            item: attr_u32(reference.elements().next()?, "v")?,
                            formula,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            source,
            fields,
            calculated_items,
            record_count: attr_u32(root, "recordCount"),
            refresh_on_load: attr_bool(root, "refreshOnLoad", false),
            xml,
        })
    }

    pub fn xml(&self) -> &ParsedXml {
        &self.xml
    }

    /// Sets `refreshOnLoad`: Excel rebuilds the cache (shared items, records) and the pivot
    /// tables from the source range when it opens the file.
    pub fn set_refresh_on_load(&mut self, on: bool) {
        if on {
            self.xml.root.head.set_attr("refreshOnLoad", "1");
        } else {
            self.xml.root.head.remove_attr("refreshOnLoad");
        }
        self.refresh_on_load = on;
    }

    /// The part bytes, unchanged when nothing was edited.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.xml.to_bytes()
    }
}

/// One cell of a cache record.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    /// Index into the field's shared items.
    Shared(u32),
    Value(Value),
}

/// `xl/pivotCache/pivotCacheRecordsN.xml`.
#[derive(Debug, Clone)]
pub struct CacheRecords {
    pub rows: Vec<Vec<Record>>,
    xml: ParsedXml,
}

impl CacheRecords {
    pub fn parse(bytes: &[u8], date1904: bool) -> Result<Self, PivotError> {
        let xml = parse_root(bytes, "pivotCacheRecords", "pivot cache records part")?;
        let rows = children_main(&xml.root, "r")
            .map(|r| {
                r.elements()
                    .map(|e| {
                        if e.head.local_name() == "x" {
                            Record::Shared(attr_u32(e, "v").unwrap_or(0))
                        } else {
                            Record::Value(item_value(e, date1904).unwrap_or(Value::Blank))
                        }
                    })
                    .collect()
            })
            .collect();
        Ok(Self { rows, xml })
    }

    pub fn xml(&self) -> &ParsedXml {
        &self.xml
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.xml.to_bytes()
    }
}

/// Aggregation of a data field or a custom subtotal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    Sum,
    Count,
    Average,
    Max,
    Min,
    Product,
    CountNums,
    StdDev,
    StdDevP,
    Var,
    VarP,
}

impl Func {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "sum" => Self::Sum,
            "count" => Self::Count,
            "average" => Self::Average,
            "max" => Self::Max,
            "min" => Self::Min,
            "product" => Self::Product,
            "countNums" => Self::CountNums,
            "stdDev" => Self::StdDev,
            "stdDevp" => Self::StdDevP,
            "var" => Self::Var,
            "varp" => Self::VarP,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Col,
    Page,
    Values,
}

/// One entry of a pivot field's `items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldItem {
    /// Index into the cache field's shared items; `None` for the subtotal placeholders.
    pub x: Option<u32>,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PivotField {
    pub axis: Option<Axis>,
    pub data_field: bool,
    pub num_fmt_id: u32,
    pub items: Vec<FieldItem>,
    pub sort_type: String,
    pub compact: bool,
    pub outline: bool,
    pub subtotal_top: bool,
    pub insert_blank_row: bool,
    pub default_subtotal: bool,
    /// Custom subtotal functions in Excel's order.
    pub subtotals: Vec<Func>,
    pub subtotal_caption: Option<String>,
    /// `fillDownLabels` (Repeat Item Labels).
    pub fill_down: bool,
    /// Data field index the items are sorted by (`autoSortScope`).
    pub sort_by: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataField {
    pub name: String,
    pub fld: u32,
    pub func: Func,
    pub show_data_as: String,
    /// `x14:dataField/@pivotShowAs` (percent of parent, rank, ...), which supersedes
    /// `showDataAs` when present.
    pub pivot_show_as: Option<String>,
    pub base_field: i32,
    pub base_item: u32,
    pub num_fmt_id: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageField {
    pub fld: u32,
    /// Selected item (index into the field's `items`); `None` shows all.
    pub item: Option<u32>,
}

/// `xl/pivotTables/pivotTableN.xml`.
#[derive(Debug, Clone)]
pub struct PivotTable {
    pub name: String,
    pub cache_id: u32,
    /// `location/@ref`.
    pub reference: String,
    pub first_header_row: u32,
    pub first_data_row: u32,
    pub first_data_col: u32,
    pub row_page_count: u32,
    pub col_page_count: u32,
    pub fields: Vec<PivotField>,
    pub row_fields: Vec<u32>,
    pub col_fields: Vec<u32>,
    pub page_fields: Vec<PageField>,
    pub data_fields: Vec<DataField>,
    pub data_on_rows: bool,
    /// Grand total row at the bottom of the row axis.
    pub row_grand_totals: bool,
    /// Grand total column at the end of the column axis.
    pub col_grand_totals: bool,
    pub compact: bool,
    pub outline: bool,
    pub data_caption: String,
    pub grand_total_caption: Option<String>,
    pub row_header_caption: Option<String>,
    pub col_header_caption: Option<String>,
    pub missing_caption: Option<String>,
    pub error_caption: Option<String>,
    xml: ParsedXml,
}

/// Field index `4294967294` in `rowFields`/`colFields` stands for the values field.
pub const VALUES_FIELD: u32 = 4_294_967_294;

impl PivotTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, PivotError> {
        let xml = parse_root(bytes, "pivotTableDefinition", "pivot table definition")?;
        let r = &xml.root;
        let loc = r.child_main("location");
        let mut fields = Vec::new();
        if let Some(pf) = r.child_main("pivotFields") {
            for f in children_main(pf, "pivotField") {
                let axis = match f.head.attr("axis") {
                    Some("axisRow") => Some(Axis::Row),
                    Some("axisCol") => Some(Axis::Col),
                    Some("axisPage") => Some(Axis::Page),
                    Some("axisValues") => Some(Axis::Values),
                    _ => None,
                };
                let items = f
                    .child_main("items")
                    .map(|its| {
                        children_main(its, "item")
                            .map(|i| FieldItem {
                                x: match i.head.attr("t") {
                                    None | Some("data") => attr_u32(i, "x"),
                                    _ => None,
                                },
                                hidden: attr_bool(i, "h", false),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let flags = [
                    ("sumSubtotal", Func::Sum),
                    ("countASubtotal", Func::Count),
                    ("avgSubtotal", Func::Average),
                    ("maxSubtotal", Func::Max),
                    ("minSubtotal", Func::Min),
                    ("productSubtotal", Func::Product),
                    ("countSubtotal", Func::CountNums),
                    ("stdDevSubtotal", Func::StdDev),
                    ("stdDevPSubtotal", Func::StdDevP),
                    ("varSubtotal", Func::Var),
                    ("varPSubtotal", Func::VarP),
                ];
                fields.push(PivotField {
                    axis,
                    data_field: attr_bool(f, "dataField", false),
                    num_fmt_id: attr_u32(f, "numFmtId").unwrap_or(0),
                    items,
                    sort_type: attr_string(f, "sortType").unwrap_or_else(|| "manual".into()),
                    compact: attr_bool(f, "compact", true),
                    outline: attr_bool(f, "outline", true),
                    subtotal_top: attr_bool(f, "subtotalTop", true),
                    insert_blank_row: attr_bool(f, "insertBlankRow", false),
                    default_subtotal: attr_bool(f, "defaultSubtotal", true),
                    subtotals: flags
                        .iter()
                        .filter(|(n, _)| attr_bool(f, n, false))
                        .map(|(_, func)| *func)
                        .collect(),
                    subtotal_caption: attr_string(f, "subtotalCaption"),
                    fill_down: find_attr(f, "fillDownLabels")
                        .is_some_and(|v| v == "1" || v == "true"),
                    sort_by: f.child_main("autoSortScope").and_then(sort_scope_field),
                });
            }
        }
        let field_list = |name: &str| -> Vec<u32> {
            r.child_main(name)
                .map(|e| {
                    children_main(e, "field")
                        .filter_map(|f| {
                            let x: i64 = f.head.attr("x")?.parse().ok()?;
                            if x == -2 {
                                Some(VALUES_FIELD)
                            } else {
                                u32::try_from(x).ok()
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let data_fields = r
            .child_main("dataFields")
            .map(|d| {
                children_main(d, "dataField")
                    .map(|f| DataField {
                        name: attr_string(f, "name").unwrap_or_default(),
                        fld: attr_u32(f, "fld").unwrap_or(0),
                        func: f
                            .head
                            .attr("subtotal")
                            .and_then(Func::parse)
                            .unwrap_or(Func::Sum),
                        show_data_as: attr_string(f, "showDataAs")
                            .unwrap_or_else(|| "normal".into()),
                        pivot_show_as: x14_show_as(f),
                        base_field: f
                            .head
                            .attr("baseField")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0),
                        base_item: attr_u32(f, "baseItem").unwrap_or(0),
                        num_fmt_id: attr_u32(f, "numFmtId").unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let page_fields = r
            .child_main("pageFields")
            .map(|p| {
                children_main(p, "pageField")
                    .map(|f| PageField {
                        fld: attr_u32(f, "fld").unwrap_or(0),
                        item: attr_u32(f, "item").filter(|i| *i != 32_767),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            name: attr_string(r, "name").unwrap_or_default(),
            cache_id: attr_u32(r, "cacheId").unwrap_or(0),
            reference: loc.and_then(|l| attr_string(l, "ref")).unwrap_or_default(),
            first_header_row: loc.and_then(|l| attr_u32(l, "firstHeaderRow")).unwrap_or(1),
            first_data_row: loc.and_then(|l| attr_u32(l, "firstDataRow")).unwrap_or(1),
            first_data_col: loc.and_then(|l| attr_u32(l, "firstDataCol")).unwrap_or(1),
            row_page_count: loc.and_then(|l| attr_u32(l, "rowPageCount")).unwrap_or(0),
            col_page_count: loc.and_then(|l| attr_u32(l, "colPageCount")).unwrap_or(0),
            fields,
            row_fields: field_list("rowFields"),
            col_fields: field_list("colFields"),
            page_fields,
            data_fields,
            data_on_rows: attr_bool(r, "dataOnRows", false),
            row_grand_totals: attr_bool(r, "rowGrandTotals", true),
            col_grand_totals: attr_bool(r, "colGrandTotals", true),
            compact: attr_bool(r, "compact", true),
            outline: attr_bool(r, "outline", false),
            data_caption: attr_string(r, "dataCaption").unwrap_or_else(|| "Values".into()),
            grand_total_caption: attr_string(r, "grandTotalCaption"),
            row_header_caption: attr_string(r, "rowHeaderCaption"),
            col_header_caption: attr_string(r, "colHeaderCaption"),
            missing_caption: attr_string(r, "missingCaption"),
            error_caption: attr_string(r, "errorCaption"),
            xml,
        })
    }

    pub fn xml(&self) -> &ParsedXml {
        &self.xml
    }

    pub(crate) fn xml_mut(&mut self) -> &mut ParsedXml {
        &mut self.xml
    }

    /// Writes `loc` into the `location` element (and the typed fields). Page counts of 0 are
    /// omitted, as Excel does.
    pub fn set_location(&mut self, loc: &Location) {
        for node in &mut self.xml.root.children {
            let Some(e) = node.as_element_mut().filter(|e| e.head.is_main("location")) else {
                continue;
            };
            e.head.set_attr("ref", &loc.reference);
            e.head
                .set_attr("firstHeaderRow", &loc.first_header_row.to_string());
            e.head
                .set_attr("firstDataRow", &loc.first_data_row.to_string());
            e.head
                .set_attr("firstDataCol", &loc.first_data_col.to_string());
            for (name, n) in [
                ("rowPageCount", loc.row_page_count),
                ("colPageCount", loc.col_page_count),
            ] {
                if n == 0 {
                    e.head.remove_attr(name);
                } else {
                    e.head.set_attr(name, &n.to_string());
                }
            }
        }
        self.reference.clone_from(&loc.reference);
        self.first_header_row = loc.first_header_row;
        self.first_data_row = loc.first_data_row;
        self.first_data_col = loc.first_data_col;
        self.row_page_count = loc.row_page_count;
        self.col_page_count = loc.col_page_count;
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.xml.to_bytes()
    }
}

/// First attribute `name` on `e` or any descendant.
fn find_attr<'a>(e: &'a XmlElement, name: &str) -> Option<&'a str> {
    if let Some(v) = e.head.attr(name) {
        return Some(v);
    }
    e.elements().find_map(|c| find_attr(c, name))
}

/// The data field a sort-by-value scope refers to.
fn sort_scope_field(scope: &XmlElement) -> Option<usize> {
    fn walk(e: &XmlElement) -> Option<usize> {
        if e.head.local_name() == "reference" && e.head.attr("field") == Some("4294967294") {
            return e
                .elements()
                .find(|x| x.head.local_name() == "x")
                .and_then(|x| x.head.attr("v"))
                .and_then(|v| v.parse().ok());
        }
        e.elements().find_map(walk)
    }
    walk(scope)
}

/// `pivotShowAs` of the `x14:dataField` inside `extLst/ext` of a `dataField`.
fn x14_show_as(f: &XmlElement) -> Option<String> {
    let ext_lst = f.elements().find(|e| e.head.local_name() == "extLst")?;
    ext_lst
        .elements()
        .flat_map(|ext| ext.elements())
        .find(|e| e.head.local_name() == "dataField")
        .and_then(|e| e.head.attr("pivotShowAs").map(str::to_owned))
}

/// `pivotTableStyleInfo` of a pivot table definition.
pub(crate) fn read_style_info(xml: &ParsedXml) -> Option<crate::style::StyleInfo> {
    let e = xml.root.child_main("pivotTableStyleInfo")?;
    Some(crate::style::StyleInfo {
        name: attr_string(e, "name")?,
        show_row_headers: attr_bool(e, "showRowHeaders", false),
        show_col_headers: attr_bool(e, "showColHeaders", false),
        show_row_stripes: attr_bool(e, "showRowStripes", false),
        show_col_stripes: attr_bool(e, "showColStripes", false),
        show_last_column: attr_bool(e, "showLastColumn", false),
    })
}
