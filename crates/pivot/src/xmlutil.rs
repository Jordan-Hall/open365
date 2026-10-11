//! Small helpers over the lossless XML tree of `open365-xlsx`.

use open365_xlsx::XmlElement;

/// `xsd:boolean` attribute with a default.
pub(crate) fn attr_bool(e: &XmlElement, name: &str, default: bool) -> bool {
    match e.head.attr(name).map(str::trim) {
        Some("1" | "true") => true,
        Some("0" | "false") => false,
        _ => default,
    }
}

pub(crate) fn attr_u32(e: &XmlElement, name: &str) -> Option<u32> {
    e.head.attr(name).and_then(|v| v.trim().parse().ok())
}

pub(crate) fn attr_f64(e: &XmlElement, name: &str) -> Option<f64> {
    e.head
        .attr(name)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

pub(crate) fn attr_string(e: &XmlElement, name: &str) -> Option<String> {
    e.head.attr(name).map(str::to_owned)
}

/// Child elements named `local` in the SpreadsheetML main namespace.
pub(crate) fn children_main<'a>(
    e: &'a XmlElement,
    local: &'a str,
) -> impl Iterator<Item = &'a XmlElement> {
    e.elements().filter(move |c| c.head.is_main(local))
}
