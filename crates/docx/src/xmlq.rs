//! Small read-only helpers over the generic XML tree, shared by the typed readers of
//! numbering and tables.

use crate::xml::{RawXml, XmlElement};

/// First child element `w:local`.
pub(crate) fn child<'a>(e: &'a XmlElement, local: &'a str) -> Option<&'a XmlElement> {
    children(e, local).next()
}

/// Child elements `w:local`, in order.
pub(crate) fn children<'a>(
    e: &'a XmlElement,
    local: &'a str,
) -> impl Iterator<Item = &'a XmlElement> {
    e.children
        .iter()
        .filter_map(RawXml::as_element)
        .filter(move |c| c.head.is_w(local))
}

/// `w:val` of the child `w:local`.
pub(crate) fn val<'a>(e: &'a XmlElement, local: &'a str) -> Option<&'a str> {
    child(e, local).and_then(|c| c.head.attr_w("val"))
}

/// Integer attribute or value.
pub(crate) fn int(v: Option<&str>) -> Option<i64> {
    v.and_then(|v| v.trim().parse().ok())
}

/// An on/off element (`w:b`, `w:isLgl`): present means on unless `w:val` switches it off.
pub(crate) fn on_off(e: &XmlElement) -> bool {
    !matches!(e.head.attr_w("val"), Some("0" | "false" | "off" | "none"))
}

/// Twips attribute `w:name` of `e` as points.
pub(crate) fn twips_attr(e: &XmlElement, name: &str) -> Option<f64> {
    e.head
        .attr_w(name)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|t| t / 20.0)
}
