//! Chart families beyond the Excel-made fixtures: combo with a secondary axis, trendlines,
//! error bars, bubble, radar and stock, style parts, builder output and typed edits. Chart XML
//! here is written by hand from the public ECMA-376 schema.

mod common;

use open365_chart::{
    BuildKind, BuildSeries, ChartBuilder, ChartSpace, ChartStyle, ColorStyle, GroupKind, LegendPos,
    NoCells, RenderOptions, ScaleEdit, StyleColor, TrendlineKind, render,
};

use common::CarlitoMeasure;

const HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:x=\"urn:vendor\">";

fn num(f: &str, v: &[f64]) -> String {
    let pts: String = v
        .iter()
        .enumerate()
        .map(|(i, n)| format!("<c:pt idx=\"{i}\"><c:v>{n}</c:v></c:pt>"))
        .collect();
    format!(
        "<c:numRef><c:f>{f}</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"{}\"/>{pts}</c:numCache></c:numRef>",
        v.len()
    )
}

fn combo() -> String {
    format!(
        "{HEAD}<c:chart><c:plotArea><c:layout/>\
<c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/>\
<c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:val>{a}</c:val>\
<c:trendline><c:name>Fit</c:name><c:trendlineType val=\"poly\"/><c:order val=\"3\"/><c:forward val=\"1.5\"/><c:dispRSqr val=\"1\"/><c:dispEq val=\"0\"/></c:trendline>\
<c:errBars><c:errDir val=\"y\"/><c:errBarType val=\"both\"/><c:errValType val=\"percentage\"/><c:noEndCap val=\"0\"/><c:val val=\"5\"/></c:errBars>\
<x:unknown keep=\"me\"/></c:ser>\
<c:axId val=\"1\"/><c:axId val=\"2\"/></c:barChart>\
<c:lineChart><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/>\
<c:ser><c:idx val=\"1\"/><c:order val=\"1\"/><c:val>{b}</c:val></c:ser>\
<c:marker val=\"1\"/><c:axId val=\"3\"/><c:axId val=\"4\"/></c:lineChart>\
<c:catAx><c:axId val=\"1\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/><c:crossAx val=\"2\"/></c:catAx>\
<c:valAx><c:axId val=\"2\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"l\"/><c:crossAx val=\"1\"/></c:valAx>\
<c:valAx><c:axId val=\"4\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"r\"/><c:crossAx val=\"3\"/><c:crosses val=\"max\"/></c:valAx>\
<c:catAx><c:axId val=\"3\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"1\"/><c:axPos val=\"b\"/><c:crossAx val=\"4\"/></c:catAx>\
</c:plotArea></c:chart></c:chartSpace>",
        a = num("S!$B$2:$B$4", &[1.0, 2.0, 3.0]),
        b = num("S!$C$2:$C$4", &[10.0, 40.0, 20.0]),
    )
}

#[test]
fn combo_with_secondary_axis_trendline_and_error_bars() {
    let xml = combo();
    let space = ChartSpace::parse(xml.as_bytes()).expect("parses");
    let plot = &space.chart.plot_area;
    assert!(plot.is_combo());
    assert_eq!(plot.secondary_groups().count(), 1);
    assert_eq!(plot.groups[1].kind, GroupKind::Line);
    assert_eq!(
        plot.axis(4).expect("axis").position,
        open365_chart::AxisPos::Right
    );
    let s = &plot.groups[0].series[0];
    let t = &s.trendlines[0];
    assert_eq!(t.kind, TrendlineKind::Poly);
    assert_eq!(t.order, Some(3));
    assert_eq!(t.forward, Some(1.5));
    assert!(t.display_r_squared && !t.display_equation);
    assert_eq!(t.name.as_deref(), Some("Fit"));
    let e = &s.error_bars[0];
    assert_eq!(
        (e.direction, e.value_type.as_str(), e.value),
        (Some('y'), "percentage", Some(5.0))
    );
    // Byte-stable: unedited output is the input.
    assert_eq!(space.to_bytes(), xml.as_bytes());
}

#[test]
fn bubble_radar_and_stock_groups_are_typed() {
    let ser = |extra: &str| {
        format!(
            "<c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:xVal>{x}</c:xVal><c:yVal>{y}</c:yVal>{extra}</c:ser>",
            x = num("S!$A$2:$A$3", &[1.0, 2.0]),
            y = num("S!$B$2:$B$3", &[3.0, 4.0]),
        )
    };
    let bubble = format!(
        "{HEAD}<c:chart><c:plotArea><c:bubbleChart><c:varyColors val=\"0\"/>{}<c:bubbleScale val=\"60\"/><c:showNegBubbles val=\"0\"/><c:sizeRepresents val=\"w\"/><c:axId val=\"1\"/><c:axId val=\"2\"/></c:bubbleChart></c:plotArea></c:chart></c:chartSpace>",
        ser(&format!(
            "<c:bubbleSize>{}</c:bubbleSize>",
            num("S!$C$2:$C$3", &[5.0, 9.0])
        ))
    );
    let space = ChartSpace::parse(bubble.as_bytes()).expect("bubble");
    let g = &space.chart.plot_area.groups[0];
    assert_eq!(g.kind, GroupKind::Bubble);
    assert_eq!(
        (
            g.bubble_scale,
            g.size_represents.as_str(),
            g.show_negative_bubbles
        ),
        (60, "w", false)
    );
    assert_eq!(
        g.series[0]
            .bubble_sizes
            .as_ref()
            .expect("sizes")
            .cache
            .len(),
        2
    );

    let radar = format!(
        "{HEAD}<c:chart><c:plotArea><c:radarChart><c:radarStyle val=\"filled\"/><c:varyColors val=\"0\"/>\
<c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:val>{}</c:val></c:ser><c:axId val=\"1\"/><c:axId val=\"2\"/></c:radarChart></c:plotArea></c:chart></c:chartSpace>",
        num("S!$B$2:$B$5", &[1.0, 2.0, 3.0, 4.0])
    );
    let space = ChartSpace::parse(radar.as_bytes()).expect("radar");
    assert_eq!(space.chart.plot_area.groups[0].kind, GroupKind::Radar);
    assert_eq!(
        space.chart.plot_area.groups[0].radar_style.as_deref(),
        Some("filled")
    );
    let measure = CarlitoMeasure::new();
    let list = render(
        &space,
        300.0,
        200.0,
        &RenderOptions::new(&NoCells, &measure),
    );
    assert!(list.items.len() > 3, "radar draws a web and a polygon");

    let stock = format!(
        "{HEAD}<c:chart><c:plotArea><c:stockChart>\
<c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:val>{}</c:val></c:ser>\
<c:ser><c:idx val=\"1\"/><c:order val=\"1\"/><c:val>{}</c:val></c:ser>\
<c:hiLowLines/><c:upDownBars><c:gapWidth val=\"150\"/></c:upDownBars><c:axId val=\"1\"/><c:axId val=\"2\"/></c:stockChart></c:plotArea></c:chart></c:chartSpace>",
        num("S!$B$2:$B$3", &[5.0, 6.0]),
        num("S!$C$2:$C$3", &[1.0, 2.0])
    );
    let space = ChartSpace::parse(stock.as_bytes()).expect("stock");
    let g = &space.chart.plot_area.groups[0];
    assert_eq!(g.kind, GroupKind::Stock);
    assert!(g.hi_low_lines && g.up_down_bars);
}

#[test]
fn style_and_colour_parts_parse_and_round_trip() {
    let colors = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cs:colorStyle xmlns:cs=\"http://schemas.microsoft.com/office/drawing/2012/chartStyle\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" meth=\"cycle\" id=\"10\"><a:schemeClr val=\"accent1\"/><a:schemeClr val=\"accent2\"/><a:srgbClr val=\"FF0000\"/><cs:variation/><cs:variation><a:lumMod val=\"60000\"/></cs:variation></cs:colorStyle>";
    let c = ColorStyle::parse(colors.as_bytes()).expect("colour style");
    assert_eq!(
        (c.method.as_str(), c.id, c.colors.len(), c.variations),
        ("cycle", 10, 3, 2)
    );
    assert_eq!(c.colors[0], StyleColor::Scheme("accent1".into()));
    let theme = |n: &str| (n == "accent2").then_some(open365_chart::Rgb(0, 128, 0));
    assert_eq!(
        c.series_color(1, theme),
        Some(open365_chart::Rgb(0, 128, 0))
    );
    assert_eq!(
        c.series_color(2, theme),
        Some(open365_chart::Rgb(255, 0, 0))
    );
    assert_eq!(c.series_color(0, theme), None);
    assert_eq!(c.to_bytes(), colors.as_bytes());

    let style = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cs:chartStyle xmlns:cs=\"http://schemas.microsoft.com/office/drawing/2012/chartStyle\" id=\"201\"><cs:axisTitle/><cs:dataPoint/></cs:chartStyle>";
    let s = ChartStyle::parse(style.as_bytes()).expect("chart style");
    assert_eq!(
        (s.id, s.entries.clone()),
        (201, vec!["axisTitle".to_owned(), "dataPoint".to_owned()])
    );
    assert_eq!(s.to_bytes(), style.as_bytes());
    assert!(ColorStyle::parse(style.as_bytes()).is_err());
}

#[test]
fn fixture_chart_style_number_is_read() {
    let wb = common::open("chart-column-clustered");
    let dc = wb.sheet_charts(0).remove(0);
    let bytes = wb
        .package()
        .part(dc.chart_part.as_str())
        .expect("part")
        .data()
        .to_vec();
    let space = ChartSpace::parse(&bytes).expect("parses");
    assert_eq!(space.style_id, Some(102));
    // The whole fixture part is byte-stable through parse and write.
    assert_eq!(space.to_bytes(), bytes);
}

#[test]
fn builder_output_reads_back_and_renders_for_every_kind() {
    use BuildKind as K;
    let kinds = [
        K::Column,
        K::ColumnStacked,
        K::ColumnPercent,
        K::Bar,
        K::BarStacked,
        K::BarPercent,
        K::Line,
        K::LineMarker,
        K::Area,
        K::AreaStacked,
        K::AreaPercent,
        K::Scatter,
        K::Bubble,
        K::Pie,
        K::Doughnut,
        K::Radar,
        K::Stock,
    ];
    let measure = CarlitoMeasure::new();
    for kind in kinds {
        let mut b = ChartBuilder::new(kind);
        b.categories = vec!["A & B".into(), "C".into(), "D".into()];
        b.series = vec![
            BuildSeries {
                name: "One".into(),
                values: vec![1.0, 2.5, 3.0],
                xs: vec![1.0, 2.0, 4.0],
                sizes: vec![3.0, 1.0, 2.0],
            },
            BuildSeries {
                name: "Two".into(),
                values: vec![2.0, 1.0, 4.0],
                xs: vec![1.0, 3.0, 4.0],
                sizes: vec![1.0, 1.0, 5.0],
            },
        ];
        b.title = Some("T".into());
        let space = b.build().unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        assert_eq!(
            space.chart.title.as_ref().map(|t| t.text.as_str()),
            Some("T"),
            "{kind:?}"
        );
        assert_eq!(space.chart.plot_area.groups[0].series.len(), 2, "{kind:?}");
        assert_eq!(space.to_bytes(), b.to_bytes(), "{kind:?}");
        let list = render(
            &space,
            360.0,
            220.0,
            &RenderOptions::new(&NoCells, &measure),
        );
        assert!(list.items.len() > 4, "{kind:?} draws");
        assert!(list.title.is_some() && list.legend.is_some(), "{kind:?}");
    }
}

#[test]
fn typed_edits_change_only_what_they_name() {
    let mut b = ChartBuilder::new(BuildKind::Column);
    b.categories = vec!["A".into(), "B".into()];
    b.series = vec![BuildSeries {
        name: "S".into(),
        values: vec![3.0, 9.0],
        ..BuildSeries::default()
    }];
    b.legend = None;
    let mut space = b.build().expect("builds");
    let before = space.to_bytes();
    assert!(space.chart.title.is_none() && space.chart.legend.is_none());

    space.set_title_text("Edited & <ok>");
    assert_eq!(
        space.chart.title.as_ref().map(|t| t.text.as_str()),
        Some("Edited & <ok>")
    );
    space.set_legend_position(LegendPos::Bottom);
    assert_eq!(
        space.chart.legend.as_ref().map(|l| l.position),
        Some(LegendPos::Bottom)
    );
    let val_axis = space
        .chart
        .plot_area
        .axes
        .iter()
        .find(|a| a.kind == open365_chart::AxisKind::Value)
        .expect("value axis")
        .id;
    assert!(space.set_axis_scale(
        val_axis,
        ScaleEdit {
            min: Some(0.0),
            max: Some(20.0),
            major_unit: Some(5.0),
            minor_unit: None
        }
    ));
    let ax = space.chart.plot_area.axis(val_axis).expect("axis");
    assert_eq!(
        (ax.min, ax.max, ax.major_unit),
        (Some(0.0), Some(20.0), Some(5.0))
    );
    assert!(!space.set_axis_scale(999, ScaleEdit::default()));

    // Schema order is kept, so the part re-parses and the edits survive a write and read.
    let again = ChartSpace::parse(&space.to_bytes()).expect("re-parses");
    assert_eq!(
        again.chart.title.as_ref().map(|t| t.text.as_str()),
        Some("Edited & <ok>")
    );
    assert_eq!(
        again.chart.plot_area.axis(val_axis).expect("axis").max,
        Some(20.0)
    );
    let text = String::from_utf8(space.to_bytes()).expect("utf8");
    let pos = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle}"));
    assert!(pos("<c:title>") < pos("<c:plotArea>") && pos("</c:plotArea>") < pos("<c:legend>"));
    assert!(
        pos("<c:max ") < pos("<c:min "),
        "CT_Scaling order: max before min"
    );
    assert!(pos("<c:crossBetween") < pos("<c:majorUnit"));

    space.clear_axis_scale(val_axis);
    space.remove_legend();
    space.remove_title();
    let cleared = space.chart.plot_area.axis(val_axis).expect("axis");
    assert_eq!(
        (cleared.min, cleared.max, cleared.major_unit),
        (None, None, None)
    );
    assert!(space.chart.legend.is_none() && space.chart.title.is_none());
    assert_eq!(
        before,
        space.to_bytes(),
        "undoing every edit restores the original bytes"
    );
}
