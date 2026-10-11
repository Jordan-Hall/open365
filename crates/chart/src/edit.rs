//! Typed edits that write straight into the lossless XML tree and then refresh the typed
//! model, so everything a caller did not touch stays exactly as it was read.

use std::sync::Arc;

use open365_xlsx::{Attr, ElementHead, RawXml, XmlElement};

use crate::model::{ChartSpace, LegendPos, is_chart_ns, parse_chart};

const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const DML_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

/// Child order of `c:scaling` (CT_Scaling).
const SCALING_ORDER: [&str; 5] = ["logBase", "orientation", "max", "min", "extLst"];
/// Child order of `c:chart` up to the legend (CT_Chart).
const CHART_ORDER: [&str; 11] = [
    "title",
    "autoTitleDeleted",
    "pivotFmts",
    "view3D",
    "floor",
    "sideWall",
    "backWall",
    "plotArea",
    "legend",
    "plotVisOnly",
    "dispBlanksAs",
];
/// Child order of `c:valAx` (CT_ValAx) from `crossAx` on.
const VAL_AX_TAIL: [&str; 8] = [
    "crossAx",
    "crosses",
    "crossesAt",
    "crossBetween",
    "majorUnit",
    "minorUnit",
    "dispUnits",
    "extLst",
];

fn chart_el(local: &str, val: Option<&str>) -> XmlElement {
    let mut e = XmlElement {
        head: ElementHead {
            name: format!("c:{local}"),
            ns: Some(Arc::from(CHART_NS)),
            attrs: Vec::new(),
        },
        children: Vec::new(),
    };
    if let Some(v) = val {
        e.head.attrs.push(Attr::plain("val", v));
    }
    e
}

fn dml_el(local: &str) -> XmlElement {
    XmlElement {
        head: ElementHead {
            name: format!("a:{local}"),
            ns: Some(Arc::from(DML_NS)),
            attrs: Vec::new(),
        },
        children: Vec::new(),
    }
}

fn index_of(e: &XmlElement, local: &str) -> Option<usize> {
    e.children.iter().position(|c| {
        c.as_element()
            .is_some_and(|x| is_chart_ns(x) && x.head.local_name() == local)
    })
}

/// Inserts `new` where `order` says it belongs, replacing an element of the same name.
fn put(parent: &mut XmlElement, order: &[&str], new: XmlElement) {
    let local = new.head.local_name().to_owned();
    if let Some(i) = index_of(parent, &local) {
        parent.children[i] = RawXml::Element(new);
        return;
    }
    let rank = order
        .iter()
        .position(|n| *n == local)
        .unwrap_or(order.len());
    let at = parent
        .children
        .iter()
        .position(|c| {
            c.as_element().is_some_and(|x| {
                is_chart_ns(x)
                    && order
                        .iter()
                        .position(|n| *n == x.head.local_name())
                        .is_some_and(|r| r > rank)
            })
        })
        .unwrap_or(parent.children.len());
    parent.children.insert(at, RawXml::Element(new));
}

fn remove(parent: &mut XmlElement, local: &str) {
    if let Some(i) = index_of(parent, local) {
        parent.children.remove(i);
    }
}

fn ensure_dml_prefix(root: &mut XmlElement) {
    if !root.head.attrs.iter().any(|a| a.name == "xmlns:a") {
        root.head.attrs.push(Attr::plain("xmlns:a", DML_NS));
    }
}

fn number(v: f64) -> String {
    let mut s = String::new();
    open365_xlsx::format_number(v, &mut s);
    s
}

/// Fixed values for a value axis; `None` leaves the field as it is, `Some(None)` is not
/// expressible: use [`ChartSpace::clear_axis_scale`] to return to automatic.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScaleEdit {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub major_unit: Option<f64>,
    pub minor_unit: Option<f64>,
}

impl ChartSpace {
    fn refresh(&mut self) {
        let chart = self
            .xml_mut()
            .root
            .elements()
            .find(|c| is_chart_ns(c) && c.head.local_name() == "chart")
            .map(parse_chart);
        if let Some(c) = chart {
            self.chart = c;
        }
    }

    fn chart_element_mut(&mut self) -> Option<&mut XmlElement> {
        self.xml_mut().root.children.iter_mut().find_map(|c| {
            c.as_element_mut()
                .filter(|x| is_chart_ns(x) && x.head.local_name() == "chart")
        })
    }

    fn axis_element_mut(&mut self, axis_id: u32) -> Option<&mut XmlElement> {
        let chart = self.chart_element_mut()?;
        let plot = chart.children.iter_mut().find_map(|c| {
            c.as_element_mut()
                .filter(|x| is_chart_ns(x) && x.head.local_name() == "plotArea")
        })?;
        plot.children.iter_mut().find_map(|c| {
            c.as_element_mut().filter(|x| {
                is_chart_ns(x)
                    && matches!(x.head.local_name(), "catAx" | "valAx" | "dateAx" | "serAx")
                    && x.children.iter().any(|a| {
                        a.as_element().is_some_and(|a| {
                            a.head.local_name() == "axId"
                                && a.head.attr("val") == Some(&axis_id.to_string())
                        })
                    })
            })
        })
    }

    /// Fixes the given scale values of axis `axis_id`. Returns false when the axis is absent.
    pub fn set_axis_scale(&mut self, axis_id: u32, edit: ScaleEdit) -> bool {
        let Some(ax) = self.axis_element_mut(axis_id) else {
            return false;
        };
        if edit.min.is_some() || edit.max.is_some() {
            if index_of(ax, "scaling").is_none() {
                let at = index_of(ax, "axId").map_or(0, |i| i + 1);
                ax.children
                    .insert(at, RawXml::Element(chart_el("scaling", None)));
            }
            let i = index_of(ax, "scaling").unwrap_or(0);
            if let Some(sc) = ax.children[i].as_element_mut() {
                if let Some(v) = edit.max {
                    put(sc, &SCALING_ORDER, chart_el("max", Some(&number(v))));
                }
                if let Some(v) = edit.min {
                    put(sc, &SCALING_ORDER, chart_el("min", Some(&number(v))));
                }
            }
        }
        if let Some(v) = edit.major_unit {
            put(ax, &VAL_AX_TAIL, chart_el("majorUnit", Some(&number(v))));
        }
        if let Some(v) = edit.minor_unit {
            put(ax, &VAL_AX_TAIL, chart_el("minorUnit", Some(&number(v))));
        }
        self.refresh();
        true
    }

    /// Returns axis `axis_id` to automatic scaling.
    pub fn clear_axis_scale(&mut self, axis_id: u32) -> bool {
        let Some(ax) = self.axis_element_mut(axis_id) else {
            return false;
        };
        remove(ax, "majorUnit");
        remove(ax, "minorUnit");
        if let Some(i) = index_of(ax, "scaling")
            && let Some(sc) = ax.children[i].as_element_mut()
        {
            remove(sc, "min");
            remove(sc, "max");
        }
        self.refresh();
        true
    }

    /// Sets the chart title to plain text, creating the title when there is none.
    pub fn set_title_text(&mut self, text: &str) {
        ensure_dml_prefix(&mut self.xml_mut().root);
        let Some(chart) = self.chart_element_mut() else {
            return;
        };
        let mut run = dml_el("r");
        let mut t = dml_el("t");
        t.children.push(RawXml::Text(text.to_owned()));
        run.children.push(RawXml::Element(t));
        let mut para = dml_el("p");
        para.children.push(RawXml::Element(run));
        let mut rich = chart_el("rich", None);
        rich.children.push(RawXml::Element(dml_el("bodyPr")));
        rich.children.push(RawXml::Element(para));
        if let Some(i) = index_of(chart, "title")
            && let Some(title) = chart.children[i].as_element_mut()
        {
            let mut tx = chart_el("tx", None);
            tx.children.push(RawXml::Element(rich));
            put(
                title,
                &["tx", "layout", "overlay", "spPr", "txPr", "extLst"],
                tx,
            );
        } else {
            let mut title = chart_el("title", None);
            let mut tx = chart_el("tx", None);
            tx.children.push(RawXml::Element(rich));
            title.children.push(RawXml::Element(tx));
            title
                .children
                .push(RawXml::Element(chart_el("overlay", Some("0"))));
            put(chart, &CHART_ORDER, title);
            put(chart, &CHART_ORDER, chart_el("autoTitleDeleted", Some("0")));
        }
        self.refresh();
    }

    /// Removes the chart title.
    pub fn remove_title(&mut self) {
        if let Some(chart) = self.chart_element_mut() {
            remove(chart, "title");
            put(chart, &CHART_ORDER, chart_el("autoTitleDeleted", Some("1")));
        }
        self.refresh();
    }

    /// Shows the legend at `position`, creating it when there is none.
    pub fn set_legend_position(&mut self, position: LegendPos) {
        let code = match position {
            LegendPos::Right => "r",
            LegendPos::Left => "l",
            LegendPos::Top => "t",
            LegendPos::Bottom => "b",
            LegendPos::TopRight => "tr",
        };
        let Some(chart) = self.chart_element_mut() else {
            return;
        };
        if let Some(i) = index_of(chart, "legend")
            && let Some(legend) = chart.children[i].as_element_mut()
        {
            put(
                legend,
                &["legendPos", "legendEntry", "layout", "overlay"],
                chart_el("legendPos", Some(code)),
            );
        } else {
            let mut legend = chart_el("legend", None);
            legend
                .children
                .push(RawXml::Element(chart_el("legendPos", Some(code))));
            legend
                .children
                .push(RawXml::Element(chart_el("overlay", Some("0"))));
            put(chart, &CHART_ORDER, legend);
        }
        self.refresh();
    }

    /// Removes the legend.
    pub fn remove_legend(&mut self) {
        if let Some(chart) = self.chart_element_mut() {
            remove(chart, "legend");
        }
        self.refresh();
    }
}
