//! A reader of the tagged-PDF structure tree for the comparison with Word's PDFs: the catalog
//! language, the role map, the elements (role, depth, selected attributes) in reading order and the
//! link annotations of every page.
#![allow(dead_code)]

use std::collections::BTreeMap;

use super::Obj;
use super::scan::Scan;

#[derive(Debug, Clone, PartialEq)]
pub struct Elem {
    pub depth: usize,
    pub role: String,
    pub lang: Option<String>,
    pub alt: Option<String>,
    /// Number of marked-content ids and of annotation references directly inside.
    pub mcids: usize,
    pub objrs: usize,
    /// Page index of `/Pg`.
    pub page: Option<usize>,
    /// Attribute dictionaries as `Key=value` pairs (owner `Table`, `List`, ...).
    pub attrs: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Annot {
    pub page: usize,
    pub subtype: String,
    pub rect: [f64; 4],
    pub has_struct_parent: bool,
    pub dest: bool,
    pub uri: Option<String>,
}

#[derive(Debug, Default)]
pub struct Structure {
    pub lang: Option<String>,
    pub marked: bool,
    pub role_map: BTreeMap<String, String>,
    pub elems: Vec<Elem>,
    pub annots: Vec<Annot>,
    pub outline_titles: Vec<(usize, String)>,
}

fn text(o: &Obj) -> String {
    match o {
        Obj::Str(b) => {
            if b.starts_with(&[0xfe, 0xff]) {
                let u: Vec<u16> = b[2..]
                    .chunks(2)
                    .filter(|c| c.len() == 2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                String::from_utf16_lossy(&u)
            } else {
                String::from_utf8_lossy(b).into_owned()
            }
        }
        Obj::Name(n) => format!("/{n}"),
        Obj::Int(i) => i.to_string(),
        Obj::Real(r) => format!("{r:.2}"),
        Obj::Bool(b) => b.to_string(),
        Obj::Arr(a) => format!("[{}]", a.iter().map(text).collect::<Vec<_>>().join(" ")),
        _ => String::new(),
    }
}

pub fn read(data: &[u8]) -> Structure {
    let s = Scan::parse(data);
    let mut out = Structure::default();
    let Some(cat) = s.catalog() else {
        return out;
    };
    out.lang = s.key(&cat, "Lang").map(|l| text(&l));
    out.marked = matches!(
        s.key(&cat, "MarkInfo").and_then(|m| s.key(&m, "Marked")),
        Some(Obj::Bool(true))
    );
    let pages = s.pages();
    // Page objects are resolved copies, so pages are matched by their Contents reference.
    let page_key = |o: &Obj| -> String { format!("{:?}", o.dict().get("Contents")) };
    let page_keys: Vec<String> = pages.iter().map(page_key).collect();
    if let Some(root) = s.key(&cat, "StructTreeRoot") {
        if let Some(Obj::Dict(rm)) = s.key(&root, "RoleMap") {
            for (k, v) in rm {
                out.role_map
                    .insert(k, text(&s.resolve(&v)).trim_start_matches('/').to_owned());
            }
        }
        if let Some(k) = root.dict().get("K") {
            walk(&s, k, 0, &page_keys, &page_key, &mut out.elems);
        }
    }
    for (pi, p) in pages.iter().enumerate() {
        if let Some(Obj::Arr(an)) = s.key(p, "Annots") {
            for a in an {
                let a = s.resolve(&a);
                let sub = s.key(&a, "Subtype").map(|x| text(&x)).unwrap_or_default();
                let mut r = [0.0; 4];
                if let Some(Obj::Arr(v)) = s.key(&a, "Rect") {
                    for (i, x) in v.iter().take(4).enumerate() {
                        r[i] = s.resolve(x).num();
                    }
                }
                let uri = s
                    .key(&a, "A")
                    .and_then(|ac| s.key(&ac, "URI"))
                    .map(|u| text(&u));
                let dest = s.key(&a, "Dest").is_some()
                    || s.key(&a, "A").and_then(|ac| s.key(&ac, "D")).is_some();
                out.annots.push(Annot {
                    page: pi,
                    subtype: sub,
                    rect: r,
                    has_struct_parent: s.key(&a, "StructParent").is_some(),
                    dest,
                    uri,
                });
            }
        }
    }
    if let Some(o) = s.key(&cat, "Outlines") {
        outline(&s, &o, 0, &mut out.outline_titles);
    }
    out
}

fn outline(s: &Scan, node: &Obj, depth: usize, out: &mut Vec<(usize, String)>) {
    let mut cur = node.dict().get("First").map(|f| s.resolve(f));
    let mut guard = 0;
    while let Some(c) = cur {
        if !matches!(c, Obj::Dict(_)) || guard > 5000 {
            break;
        }
        guard += 1;
        out.push((
            depth,
            s.key(&c, "Title").map(|t| text(&t)).unwrap_or_default(),
        ));
        outline(s, &c, depth + 1, out);
        cur = c.dict().get("Next").map(|n| s.resolve(n));
    }
}

fn walk(
    s: &Scan,
    k: &Obj,
    depth: usize,
    page_keys: &[String],
    page_key: &dyn Fn(&Obj) -> String,
    out: &mut Vec<Elem>,
) {
    let node = s.resolve(k);
    match &node {
        Obj::Arr(items) => {
            for i in items {
                walk(s, i, depth, page_keys, page_key, out);
            }
        }
        Obj::Dict(d) => {
            if matches!(d.get("Type"), Some(Obj::Name(n)) if n == "OBJR" || n == "MCR") {
                return;
            }
            let Some(role) = s.key(&node, "S") else {
                return;
            };
            let page = s.key(&node, "Pg").and_then(|p| {
                let key = page_key(&p);
                page_keys.iter().position(|k| *k == key)
            });
            let kids: Vec<Obj> = match d.get("K").map(|x| s.resolve(x)) {
                Some(Obj::Arr(a)) => a,
                Some(Obj::Null) | None => Vec::new(),
                Some(o) => vec![o],
            };
            let (mut mc, mut ob) = (0, 0);
            for kid in &kids {
                match s.resolve(kid) {
                    Obj::Int(_) => mc += 1,
                    Obj::Dict(kd) => match kd.get("Type") {
                        Some(Obj::Name(n)) if n == "OBJR" => ob += 1,
                        Some(Obj::Name(n)) if n == "MCR" => mc += 1,
                        _ => {}
                    },
                    _ => {}
                }
            }
            let mut attrs = Vec::new();
            if let Some(a) = s.key(&node, "A") {
                let list = match a {
                    Obj::Arr(v) => v.iter().map(|x| s.resolve(x)).collect(),
                    o => vec![o],
                };
                for ad in list {
                    if let Obj::Dict(m) = ad {
                        for (key, v) in m {
                            if key != "O" {
                                attrs.push(format!("{key}={}", text(&s.resolve(&v))));
                            }
                        }
                    }
                }
            }
            out.push(Elem {
                depth,
                role: text(&role).trim_start_matches('/').to_owned(),
                lang: s.key(&node, "Lang").map(|l| text(&l)),
                alt: s.key(&node, "Alt").map(|l| text(&l)),
                mcids: mc,
                objrs: ob,
                page,
                attrs,
            });
            for kid in &kids {
                if matches!(s.resolve(kid), Obj::Dict(_)) {
                    walk(s, kid, depth + 1, page_keys, page_key, out);
                }
            }
        }
        _ => {}
    }
}
