//! The shapes the Insert > Shapes gallery offers: category, preset name (an `ST_ShapeType` value, public
//! in ECMA-376 prose) and the label shown in the menu. The geometry itself comes from
//! [`crate::geom::presets`]; every entry here is checked against that table by a test.

/// A gallery group, in menu order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Category {
    pub label: &'static str,
    pub shapes: &'static [(&'static str, &'static str)],
}

/// The gallery. Each shape is `(preset, label)`.
pub const CATEGORIES: &[Category] = &[
    Category {
        label: "Lines",
        shapes: &[("line", "Line")],
    },
    Category {
        label: "Rectangles",
        shapes: &[
            ("rect", "Rectangle"),
            ("roundRect", "Rectangle: Rounded Corners"),
            ("snip1Rect", "Rectangle: Single Corner Snipped"),
            ("snip2SameRect", "Rectangle: Top Corners Snipped"),
            ("snip2DiagRect", "Rectangle: Diagonal Corners Snipped"),
            ("snipRoundRect", "Rectangle: Snipped and Rounded Corner"),
            ("round1Rect", "Rectangle: Single Corner Rounded"),
            ("round2SameRect", "Rectangle: Top Corners Rounded"),
            ("round2DiagRect", "Rectangle: Diagonal Corners Rounded"),
        ],
    },
    Category {
        label: "Basic Shapes",
        shapes: &[
            ("ellipse", "Oval"),
            ("triangle", "Triangle: Isosceles"),
            ("rtTriangle", "Triangle: Right"),
            ("parallelogram", "Parallelogram"),
            ("trapezoid", "Trapezoid"),
            ("diamond", "Diamond"),
            ("pentagon", "Pentagon: Regular"),
            ("hexagon", "Hexagon"),
            ("heptagon", "Heptagon"),
            ("octagon", "Octagon"),
            ("decagon", "Decagon"),
            ("dodecagon", "Dodecagon"),
            ("pie", "Partial Circle"),
            ("chord", "Chord"),
            ("teardrop", "Teardrop"),
            ("frame", "Frame"),
            ("halfFrame", "Half Frame"),
            ("corner", "L-Shape"),
            ("diagStripe", "Diagonal Stripe"),
            ("plus", "Cross"),
            ("plaque", "Plaque"),
            ("can", "Cylinder"),
            ("cube", "Cube"),
            ("bevel", "Bevel"),
            ("donut", "Donut"),
            ("noSmoking", "No Symbol"),
            ("blockArc", "Block Arc"),
            ("foldedCorner", "Folded Corner"),
            ("smileyFace", "Smiley Face"),
            ("heart", "Heart"),
            ("lightningBolt", "Lightning Bolt"),
            ("sun", "Sun"),
            ("moon", "Moon"),
            ("cloud", "Cloud"),
            ("arc", "Arc"),
            ("bracketPair", "Double Bracket"),
            ("bracePair", "Double Brace"),
            ("leftBracket", "Left Bracket"),
            ("rightBracket", "Right Bracket"),
            ("leftBrace", "Left Brace"),
            ("rightBrace", "Right Brace"),
        ],
    },
    Category {
        label: "Block Arrows",
        shapes: &[
            ("rightArrow", "Arrow: Right"),
            ("leftArrow", "Arrow: Left"),
            ("upArrow", "Arrow: Up"),
            ("downArrow", "Arrow: Down"),
            ("leftRightArrow", "Arrow: Left-Right"),
            ("upDownArrow", "Arrow: Up-Down"),
            ("quadArrow", "Arrow: Quad"),
            ("leftRightUpArrow", "Arrow: Left-Right-Up"),
            ("bentArrow", "Arrow: Bent"),
            ("uturnArrow", "Arrow: U-Turn"),
            ("leftUpArrow", "Arrow: Left-Up"),
            ("bentUpArrow", "Arrow: Bent-Up"),
            ("curvedRightArrow", "Arrow: Curved Right"),
            ("curvedLeftArrow", "Arrow: Curved Left"),
            ("curvedUpArrow", "Arrow: Curved Up"),
            ("curvedDownArrow", "Arrow: Curved Down"),
            ("stripedRightArrow", "Arrow: Striped Right"),
            ("notchedRightArrow", "Arrow: Notched Right"),
            ("homePlate", "Arrow: Pentagon"),
            ("chevron", "Arrow: Chevron"),
            ("rightArrowCallout", "Callout: Right Arrow"),
            ("leftArrowCallout", "Callout: Left Arrow"),
            ("upArrowCallout", "Callout: Up Arrow"),
            ("downArrowCallout", "Callout: Down Arrow"),
            ("leftRightArrowCallout", "Callout: Left-Right Arrow"),
            ("upDownArrowCallout", "Callout: Up-Down Arrow"),
            ("quadArrowCallout", "Callout: Quad Arrow"),
            ("circularArrow", "Arrow: Circular"),
        ],
    },
    Category {
        label: "Equation Shapes",
        shapes: &[
            ("mathPlus", "Plus Sign"),
            ("mathMinus", "Minus Sign"),
            ("mathMultiply", "Multiplication Sign"),
            ("mathDivide", "Division Sign"),
            ("mathEqual", "Equal Sign"),
            ("mathNotEqual", "Not Equal"),
        ],
    },
    Category {
        label: "Flowchart",
        shapes: &[
            ("flowChartProcess", "Flowchart: Process"),
            ("flowChartAlternateProcess", "Flowchart: Alternate Process"),
            ("flowChartDecision", "Flowchart: Decision"),
            ("flowChartInputOutput", "Flowchart: Data"),
            (
                "flowChartPredefinedProcess",
                "Flowchart: Predefined Process",
            ),
            ("flowChartInternalStorage", "Flowchart: Internal Storage"),
            ("flowChartDocument", "Flowchart: Document"),
            ("flowChartMultidocument", "Flowchart: Multidocument"),
            ("flowChartTerminator", "Flowchart: Terminator"),
            ("flowChartPreparation", "Flowchart: Preparation"),
            ("flowChartManualInput", "Flowchart: Manual Input"),
            ("flowChartManualOperation", "Flowchart: Manual Operation"),
            ("flowChartConnector", "Flowchart: Connector"),
            ("flowChartOffpageConnector", "Flowchart: Off-page Connector"),
            ("flowChartPunchedCard", "Flowchart: Card"),
            ("flowChartPunchedTape", "Flowchart: Punched Tape"),
            ("flowChartSummingJunction", "Flowchart: Summing Junction"),
            ("flowChartOr", "Flowchart: Or"),
            ("flowChartCollate", "Flowchart: Collate"),
            ("flowChartSort", "Flowchart: Sort"),
            ("flowChartExtract", "Flowchart: Extract"),
            ("flowChartMerge", "Flowchart: Merge"),
            ("flowChartOnlineStorage", "Flowchart: Stored Data"),
            ("flowChartDelay", "Flowchart: Delay"),
            (
                "flowChartMagneticTape",
                "Flowchart: Sequential Access Storage",
            ),
            ("flowChartMagneticDisk", "Flowchart: Magnetic Disk"),
            ("flowChartMagneticDrum", "Flowchart: Direct Access Storage"),
            ("flowChartDisplay", "Flowchart: Display"),
        ],
    },
    Category {
        label: "Stars and Banners",
        shapes: &[
            ("irregularSeal1", "Explosion 1"),
            ("irregularSeal2", "Explosion 2"),
            ("star4", "Star: 4 Points"),
            ("star5", "Star: 5 Points"),
            ("star6", "Star: 6 Points"),
            ("star7", "Star: 7 Points"),
            ("star8", "Star: 8 Points"),
            ("star10", "Star: 10 Points"),
            ("star12", "Star: 12 Points"),
            ("star16", "Star: 16 Points"),
            ("star24", "Star: 24 Points"),
            ("star32", "Star: 32 Points"),
            ("ribbon2", "Ribbon: Tilted Up"),
            ("ribbon", "Ribbon: Tilted Down"),
            ("ellipseRibbon2", "Ribbon: Curved and Tilted Up"),
            ("ellipseRibbon", "Ribbon: Curved and Tilted Down"),
            ("verticalScroll", "Scroll: Vertical"),
            ("horizontalScroll", "Scroll: Horizontal"),
            ("wave", "Wave"),
            ("doubleWave", "Double Wave"),
        ],
    },
    Category {
        label: "Callouts",
        shapes: &[
            ("wedgeRectCallout", "Speech Bubble: Rectangle"),
            (
                "wedgeRoundRectCallout",
                "Speech Bubble: Rectangle with Corners Rounded",
            ),
            ("wedgeEllipseCallout", "Speech Bubble: Oval"),
            ("cloudCallout", "Thought Bubble: Cloud"),
            ("borderCallout1", "Callout: Line"),
            ("borderCallout2", "Callout: Bent Line"),
            ("borderCallout3", "Callout: Double Bent Line"),
            ("accentCallout1", "Callout: Line with Accent Bar"),
            ("accentCallout2", "Callout: Bent Line with Accent Bar"),
            (
                "accentCallout3",
                "Callout: Double Bent Line with Accent Bar",
            ),
            ("callout1", "Callout: Line with No Border"),
            ("callout2", "Callout: Bent Line with No Border"),
            ("callout3", "Callout: Double Bent Line with No Border"),
            (
                "accentBorderCallout1",
                "Callout: Line with Border and Accent Bar",
            ),
            (
                "accentBorderCallout2",
                "Callout: Bent Line with Border and Accent Bar",
            ),
            (
                "accentBorderCallout3",
                "Callout: Double Bent Line with Border and Accent Bar",
            ),
        ],
    },
];

/// The label of a preset in the gallery.
#[must_use]
pub fn label_of(preset: &str) -> Option<&'static str> {
    CATEGORIES
        .iter()
        .flat_map(|c| c.shapes.iter())
        .find(|(p, _)| *p == preset)
        .map(|(_, l)| *l)
}

/// True for a preset that Excel inserts as a connector (`cxnSp`) rather than a shape.
#[must_use]
pub fn is_connector(preset: &str) -> bool {
    matches!(
        preset,
        "line" | "straightConnector1" | "bentConnector3" | "curvedConnector3"
    )
}

/// The name stem Excel gives a new shape: `Rectangle 1`, `Oval 2`, `Straight Connector 3`. Measured
/// with `Shapes.AddShape` and `AddLine`; other presets use their gallery label (Excel names a shape after
/// the gallery entry it came from).
#[must_use]
pub fn default_name_stem(preset: &str) -> &'static str {
    match preset {
        "rect" => "Rectangle",
        "ellipse" => "Oval",
        "line" | "straightConnector1" => "Straight Connector",
        _ => label_of(preset).unwrap_or("Shape"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gallery_preset_is_drawable_or_a_line() {
        let table = crate::geom::presets::table();
        let missing: Vec<&str> = CATEGORIES
            .iter()
            .flat_map(|c| c.shapes.iter())
            .map(|(p, _)| *p)
            .filter(|p| !table.iter().any(|(n, _)| n == p))
            .collect();
        assert!(missing.is_empty(), "not in the preset table: {missing:?}");
    }

    #[test]
    fn presets_are_unique_and_labelled() {
        let mut seen = std::collections::HashSet::new();
        for c in CATEGORIES {
            for (p, l) in c.shapes {
                assert!(seen.insert(*p), "{p} twice");
                assert!(!l.is_empty());
            }
        }
        assert_eq!(label_of("ellipse"), Some("Oval"));
        assert_eq!(default_name_stem("ellipse"), "Oval");
        assert_eq!(default_name_stem("star5"), "Star: 5 Points");
    }
}
