//! The drawing object model. Coordinates are EMU (914 400 per inch, 12 700 per point);
//! angles are 60000ths of a degree, as in DrawingML.

use crate::geom::ShapeDef;
use crate::paint::{Effect, Fill, Line, StyleRef};
use crate::text::TextBody;

/// EMU per typographic point.
pub const EMU_PER_PT: f64 = 12_700.0;

/// Verbatim source text of an object or of a part of it. Writers patch edited fields and
/// copy everything else, so untouched objects round trip byte for byte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawXml(pub String);

impl RawXml {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Position and size (`a:xfrm`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Xfrm {
    pub off_x: i64,
    pub off_y: i64,
    pub ext_cx: i64,
    pub ext_cy: i64,
    /// Rotation, 60000ths of a degree, clockwise.
    pub rot: i64,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// How a shape's outline is defined.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// `a:prstGeom`: preset name plus the `a:avLst` overrides `(name, formula)`.
    Preset {
        name: String,
        adjust: Vec<(String, String)>,
    },
    /// `a:custGeom`.
    Custom(Box<ShapeDef>),
}

impl Geometry {
    /// A plain rectangle.
    pub fn rect() -> Self {
        Self::Preset {
            name: "rect".into(),
            adjust: Vec::new(),
        }
    }
}

/// A shape, text box or connector.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeData {
    pub geometry: Geometry,
    pub fill: Option<Fill>,
    pub line: Option<Line>,
    pub effects: Vec<Effect>,
    pub style: Option<StyleRef>,
    pub text: Option<TextBody>,
    /// `txBox="1"` on the non-visual shape properties.
    pub is_text_box: bool,
    pub is_connector: bool,
}

/// Source rectangle of a picture crop, in 1/1000 percent of the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SrcRect {
    pub l: i64,
    pub t: i64,
    pub r: i64,
    pub b: i64,
}

/// A picture.
#[derive(Debug, Clone, PartialEq)]
pub struct PictureData {
    /// Relationship id (`r:embed`) of the image part.
    pub embed: Option<String>,
    /// Relationship id (`r:link`) for a linked picture.
    pub link: Option<String>,
    pub src_rect: Option<SrcRect>,
    pub geometry: Geometry,
    pub line: Option<Line>,
    pub effects: Vec<Effect>,
}

/// A group; children live in the child coordinate space `ch_off`/`ch_ext`.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupData {
    pub ch_off_x: i64,
    pub ch_off_y: i64,
    pub ch_ext_cx: i64,
    pub ch_ext_cy: i64,
    pub children: Vec<DrawingObject>,
}

/// The payload of an object.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Shape(Box<ShapeData>),
    Picture(Box<PictureData>),
    Group(GroupData),
    /// A chart reference (`c:chart r:id`).
    Chart {
        rid: String,
    },
    /// Anything else in `a:graphicData` (SmartArt, ink, OLE, ...): preserved, not drawn.
    Unknown {
        uri: String,
    },
}

/// One drawing object with the source it was read from.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingObject {
    pub id: u32,
    pub name: String,
    pub descr: Option<String>,
    pub hidden: bool,
    pub xfrm: Xfrm,
    pub kind: Kind,
    pub raw: RawXml,
}

impl DrawingObject {
    /// Number of objects in this tree, groups included.
    pub fn count(&self) -> usize {
        match &self.kind {
            Kind::Group(g) => 1 + g.children.iter().map(Self::count).sum::<usize>(),
            _ => 1,
        }
    }
}
