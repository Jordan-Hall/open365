//! Named cell styles (Excel's Cell Styles gallery): applying a built-in or workbook style to a
//! cell format, adding the built-in definition to the style sheet the first time it is used.

use crate::XlsxError;
use crate::styles::Styles;
use crate::styles_builtin::BUILTIN_STYLES;
use crate::styles_edit::{
    attr_u32, canon, el, fix_count, intern, item, remove_child, section, section_index, set_apply,
    set_font_child,
};
use crate::xml::{Attr, RawXml, XmlElement};

/// Copies `src` (parsed without the SpreadsheetML namespace) into the namespace of `tpl`.
fn adopt(tpl: &XmlElement, src: &XmlElement) -> XmlElement {
    let mut e = tpl.sibling(src.head.local_name());
    e.head.attrs = src
        .head
        .attrs
        .iter()
        .map(|a| Attr::plain(&a.name, &a.value))
        .collect();
    e.children = src
        .elements()
        .map(|c| RawXml::Element(adopt(tpl, c)))
        .collect();
    e
}

fn snippet(tpl: &XmlElement, xml: &str) -> Result<XmlElement, XlsxError> {
    Ok(adopt(tpl, &crate::xml::parse(xml.as_bytes())?.root))
}

/// Index of `item` among the items of `name` at or after `from`, adding it when none matches.
fn intern_from(root: &mut XmlElement, name: &str, item: XmlElement, from: usize) -> usize {
    let at = section_index(root, name);
    let Some(sec) = root.children[at].as_element_mut() else {
        return 0;
    };
    let key = canon(&item);
    if let Some(i) = sec
        .elements()
        .enumerate()
        .position(|(i, e)| i >= from && canon(e) == key)
    {
        return i;
    }
    let n = sec.elements().count();
    sec.children.push(RawXml::Element(item));
    fix_count(sec);
    n
}

impl Styles {
    /// The cell style (`cellStyleXfs` index) called `name`, a built-in one added to the sheet
    /// when it is not there yet. `None` when there is no such style.
    fn style_xf_for(&mut self, name: &str) -> Result<Option<u32>, XlsxError> {
        let builtin = BUILTIN_STYLES
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(name));
        let root = self.root_mut()?;
        if let Some(cs) = section(root, "cellStyles").and_then(|s| {
            s.elements().find(|c| {
                c.head
                    .attr("name")
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
                    || builtin.is_some_and(|b| attr_u32(c, "builtinId") == Some(b.builtin_id))
            })
        }) {
            return Ok(attr_u32(cs, "xfId"));
        }
        let Some(b) = builtin else { return Ok(None) };
        let tpl = root.sibling("x");
        // The font: the workbook's Normal font for the theme's minor font. Excel keeps a
        // separate record for cell styles, so the search starts at the second entry.
        let normal = item(root, "fonts", 0);
        let mut font = snippet(&tpl, b.font)?;
        let minor = font
            .child_main("name")
            .and_then(|n| n.head.attr("val"))
            .is_some_and(|n| n == "@minor");
        if minor && let Some(normal) = &normal {
            for local in ["name", "family", "scheme"] {
                remove_child(&mut font, local);
                if let Some(c) = normal.child_main(local) {
                    set_font_child(&mut font, c.clone());
                }
            }
        }
        let font_id = intern_from(root, "fonts", font, 1);
        let fill_id = intern(root, "fills", snippet(&tpl, b.fill)?);
        let border_id = intern(root, "borders", snippet(&tpl, b.border)?);
        let mut xf = el(
            &tpl,
            "xf",
            &[
                ("numFmtId", &b.num_fmt_id.to_string()),
                ("fontId", &font_id.to_string()),
                ("fillId", &fill_id.to_string()),
                ("borderId", &border_id.to_string()),
            ],
        );
        for (n, v) in b.apply {
            xf.head.attrs.push(Attr::plain(n, v));
        }
        let at = section_index(root, "cellStyleXfs");
        let Some(sec) = root.children[at].as_element_mut() else {
            return Ok(None);
        };
        let xf_id = sec.elements().count() as u32;
        sec.children.push(RawXml::Element(xf));
        fix_count(sec);
        let at = section_index(root, "cellStyles");
        let Some(sec) = root.children[at].as_element_mut() else {
            return Ok(None);
        };
        let new = el(
            &tpl,
            "cellStyle",
            &[
                ("name", b.name),
                ("xfId", &xf_id.to_string()),
                ("builtinId", &b.builtin_id.to_string()),
            ],
        );
        // Excel keeps the list sorted by name.
        let lower = b.name.to_ascii_lowercase();
        let pos = sec
            .children
            .iter()
            .position(|c| {
                c.as_element().is_some_and(|c| {
                    c.head
                        .attr("name")
                        .is_some_and(|n| n.to_ascii_lowercase() > lower)
                })
            })
            .unwrap_or(sec.children.len());
        sec.children.insert(pos, RawXml::Element(new));
        fix_count(sec);
        Ok(Some(xf_id))
    }

    /// The cell format of named style `name`: every aspect comes from the style, so earlier
    /// formatting of the cell is dropped. `Ok(None)` when the workbook has no such style and it
    /// is not a built-in one.
    pub(crate) fn derive_named_style(&mut self, name: &str) -> Result<Option<u32>, XlsxError> {
        if name.eq_ignore_ascii_case("Normal") {
            return Ok(Some(0));
        }
        let Some(style_xf) = self.style_xf_for(name)? else {
            return Ok(None);
        };
        let root = self.root_mut()?;
        let Some(sx) = item(root, "cellStyleXfs", style_xf as usize) else {
            return Ok(None);
        };
        let mut xf = el(&sx.sibling("x"), "xf", &[]);
        let style_font = attr_u32(&sx, "fontId").unwrap_or(0);
        let style_fill = attr_u32(&sx, "fillId").unwrap_or(0);
        let style_border = attr_u32(&sx, "borderId").unwrap_or(0);
        // A style that does not set a group (`applyX="0"`) leaves the cell on the Normal record
        // of it, flagged as applied; otherwise the cell uses the style's own record.
        let leaves = |name: &str| sx.head.attr(name) == Some("0");
        let pick = |apply: &str, id: u32| if leaves(apply) { 0 } else { id };
        let font_id = pick("applyFont", style_font);
        let fill_id = pick("applyFill", style_fill);
        let border_id = pick("applyBorder", style_border);
        let num_fmt = if leaves("applyNumberFormat") {
            "0"
        } else {
            sx.head.attr("numFmtId").unwrap_or("0")
        };
        for (n, v) in [
            ("numFmtId", num_fmt.to_owned()),
            ("fontId", font_id.to_string()),
            ("fillId", fill_id.to_string()),
            ("borderId", border_id.to_string()),
            ("xfId", style_xf.to_string()),
        ] {
            xf.head.attrs.push(Attr::plain(n, &v));
        }
        if font_id != style_font {
            set_apply(&mut xf, "applyFont");
        }
        if fill_id != style_fill {
            set_apply(&mut xf, "applyFill");
        }
        if border_id != style_border {
            set_apply(&mut xf, "applyBorder");
        }
        for c in sx.elements() {
            if matches!(c.head.local_name(), "alignment" | "protection") {
                xf.children.push(RawXml::Element(c.clone()));
            }
        }
        let id = intern(root, "cellXfs", xf);
        self.reindex();
        Ok(Some(id as u32))
    }

    /// The names of the cell styles that can be applied: Normal, the built-in ones and the
    /// workbook's own.
    pub fn cell_style_names(&self) -> Vec<String> {
        let mut names: Vec<String> = BUILTIN_STYLES.iter().map(|b| b.name.to_owned()).collect();
        names.insert(0, "Normal".to_owned());
        if let Ok(root) = self.root()
            && let Some(sec) = section(root, "cellStyles")
        {
            for c in sec.elements() {
                if let Some(n) = c.head.attr("name")
                    && !names.iter().any(|x| x.eq_ignore_ascii_case(n))
                {
                    names.push(n.to_owned());
                }
            }
        }
        names
    }

    /// The name of the cell style the format `xf` is based on.
    pub fn cell_style_name(&self, xf: u32) -> Option<String> {
        let root = self.root().ok()?;
        let x = section(root, "cellXfs")?.elements().nth(xf as usize)?;
        let id = attr_u32(x, "xfId").unwrap_or(0);
        section(root, "cellStyles")?
            .elements()
            .find(|c| attr_u32(c, "xfId") == Some(id))
            .and_then(|c| c.head.attr("name"))
            .map(str::to_owned)
    }
}
