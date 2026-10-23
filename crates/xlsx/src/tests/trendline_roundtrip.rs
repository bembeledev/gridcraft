//! Chart trendlines (`<c:trendline>`) round-trip through XLSX in every chart group that has them.

use std::io::Read;
use std::sync::Arc;

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Anchor, Chart, ChartKind, Color, LegendPos, LineDash, Series, Sheet, Trendline, TrendlineKind, Workbook};

use crate::{read_xlsx, write_xlsx};

fn all_trendlines() -> Vec<Trendline> {
    vec![
        Trendline {
            kind: TrendlineKind::Linear,
            forward: 2.0,
            backward: 0.5,
            display_equation: true,
            display_r_squared: true,
            ..Trendline::default()
        },
        Trendline { kind: TrendlineKind::Exponential, name: Some("Growth & decay".into()), intercept: Some(1.5), ..Trendline::default() },
        Trendline {
            kind: TrendlineKind::Logarithmic,
            color: Some(Color::Rgb(0xC00000)),
            width: Some(2.25),
            dash: Some(LineDash::SysDot),
            ..Trendline::default()
        },
        Trendline { kind: TrendlineKind::Polynomial(2), ..Trendline::default() },
        Trendline { kind: TrendlineKind::Polynomial(6), intercept: Some(-3.0), display_equation: true, ..Trendline::default() },
        Trendline { kind: TrendlineKind::Power, display_r_squared: true, dash: Some(LineDash::LgDashDot), ..Trendline::default() },
        Trendline { kind: TrendlineKind::MovingAverage(4), ..Trendline::default() },
    ]
}

fn chart(id: u32, kind: ChartKind, trendlines: Vec<Trendline>) -> Chart {
    Chart {
        id,
        kind,
        anchor: Anchor { cell: CellRef::new(id * 16, 3), width: 480.0, height: 300.0, ..Anchor::default() },
        title: None,
        series: vec![Series {
            name: Some("Data!$B$1".into()),
            categories: Some("Data!$A$2:$A$6".into()),
            values: "Data!$B$2:$B$6".into(),
            bubble_sizes: None,
            color: Some(Color::Rgb(0x4472C4)),
            secondary: false,
            kind: None,
            quartiles: Default::default(),
            smooth: false,
            trendlines,
        }],
        legend: LegendPos::Bottom,
        data_labels: false,
        gridlines: true,
        style: 2,
        x_title: None,
        y_title: None,
        source: None,
        by_rows: false,
        pivot: None,
        format: Default::default(),
    }
}

const KINDS: [ChartKind; 8] = [
    ChartKind::Line,
    ChartKind::LineMarkers,
    ChartKind::ColumnClustered,
    ChartKind::BarClustered,
    ChartKind::Area,
    ChartKind::Scatter,
    ChartKind::ScatterLines,
    ChartKind::Bubble,
];

#[test]
fn hollow_marker_contours_transparency_and_smoothing_are_written_natively() {
    let mut wb = workbook();
    let c = &mut Arc::make_mut(&mut wb.sheets[0]).charts[0];
    let style = gridcraft_model::ChartSeriesFormat {
        marker: Some("plus".into()),
        marker_size: Some(72.0),
        marker_no_fill: true,
        marker_border_color: Some([255, 0, 0, 255]),
        marker_border_width: Some(2.0),
        line_color: Some([10, 20, 30, 128]),
        smooth: Some(true),
        ..Default::default()
    };
    c.format.series.insert(0, style.clone());
    let bytes = write_xlsx(&wb).unwrap();
    let xml = chart_parts(&bytes).remove(0);
    assert_eq!(xml.matches("<c:marker>").count(), 1);
    assert!(xml.contains("<c:symbol val=\"plus\"/>"));
    assert!(xml.contains("<c:size val=\"72\"/>"));
    assert!(xml.contains("<a:noFill/>"));
    assert!(xml.contains("<a:srgbClr val=\"FF0000\">"));
    assert!(xml.contains("<c:smooth val=\"1\"/>"));
    assert!(xml.contains("<a:alpha val=\"50196\"/>"));
    let (back, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(back.sheets[0].charts[0].format.series.get(&0), Some(&style));
}

#[test]
fn axis_and_series_format_roundtrip_and_native_axis_xml() {
    let mut wb = workbook();
    let sheet = Arc::make_mut(&mut wb.sheets[0]);
    let c = &mut sheet.charts[0];
    c.format.y_axis =
        gridcraft_model::ChartAxis { minimum: Some(1.0), maximum: Some(1000.0), logarithmic: true, reversed: true, ..Default::default() };
    c.format.vertical_gridlines = true;
    c.format.series.insert(
        0,
        gridcraft_model::ChartSeriesFormat {
            marker: Some("diamond".into()),
            marker_size: Some(9.0),
            line_width: Some(3.0),
            dashed: true,
            ..Default::default()
        },
    );
    let expected = c.format.clone();
    let bytes = write_xlsx(&wb).unwrap();
    let xml = chart_parts(&bytes).remove(0);
    assert!(xml.contains("<c:logBase val=\"10\"/>"));
    assert!(xml.contains("<c:orientation val=\"maxMin\"/>"));
    assert!(xml.contains("<c:min val=\"1\"/>"));
    assert!(xml.contains("<c:max val=\"1000\"/>"));
    assert!(xml.contains("<c:symbol val=\"diamond\"/>"));
    let (back, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(back.sheets[0].charts[0].format, expected);
}

#[test]
fn independent_marker_fills_surfaces_spacing_and_axis_masks_are_native_and_preserved() {
    let mut wb = workbook();
    let c = &mut Arc::make_mut(&mut wb.sheets[0]).charts[2];
    c.format.y_axis.number_format = Some("0 \"kg\"".into());
    c.format.gap_width = Some(80.0);
    c.format.overlap = Some(25.0);
    c.format.chart_area.fill = Some([240, 240, 240, 255]);
    c.format.plot_area.fill = Some([200, 210, 220, 128]);
    c.format
        .series
        .insert(0, gridcraft_model::ChartSeriesFormat { fill_color: Some([40, 80, 120, 128]), border_width: Some(2.0), ..Default::default() });
    let expected = c.format.clone();
    let bytes = write_xlsx(&wb).unwrap();
    let xml = chart_parts(&bytes).remove(2);
    assert!(xml.contains("<c:gapWidth val=\"80\"/>"));
    assert!(xml.contains("<c:overlap val=\"25\"/>"));
    assert!(xml.contains("formatCode=\"0 &quot;kg&quot;\" sourceLinked=\"0\""));
    assert!(xml.contains("<a:alpha val=\"50196\"/>"));
    let (back, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(back.sheets[0].charts[2].format, expected);
}

fn workbook() -> Workbook {
    let mut s = Sheet::new("Data");
    s.set_value(CellRef::new(0, 1), Value::Text("Sales".into()));
    for r in 1..=5u32 {
        s.set_value(CellRef::new(r, 0), Value::Number(f64::from(r)));
        s.set_value(CellRef::new(r, 1), Value::Number(f64::from(r * r)));
    }
    for (i, k) in KINDS.iter().enumerate() {
        s.charts.push(chart(i as u32 + 1, *k, all_trendlines()));
    }
    Workbook { sheets: vec![Arc::new(s)], ..Workbook::default() }
}

#[test]
fn every_trendline_kind_round_trips_in_every_chart_group() {
    let bytes = write_xlsx(&workbook()).unwrap();
    let (back, _rep) = read_xlsx(&bytes).unwrap();
    let charts = &back.sheets[0].charts;
    assert_eq!(charts.len(), KINDS.len());
    for (c, k) in charts.iter().zip(KINDS) {
        assert_eq!(c.kind, k);
        assert_eq!(c.series[0].trendlines, all_trendlines(), "{k:?}");
    }
    // A second save writes the same parts (nothing drifts on the way round).
    let again = write_xlsx(&back).unwrap();
    let expected = chart_parts(&bytes);
    for (index, (actual, expected)) in chart_parts(&again).iter().zip(&expected).enumerate() {
        if actual != expected {
            let mismatch = actual.chars().zip(expected.chars()).take_while(|(a, b)| a == b).count();
            let start = mismatch.saturating_sub(100);
            let actual_end = (mismatch + 240).min(actual.len());
            let expected_end = (mismatch + 240).min(expected.len());
            panic!(
                "chart part {index} changes on its second save near byte {mismatch}:\nactual: {}\nexpected: {}",
                actual.get(start..actual_end).unwrap_or(""),
                expected.get(start..expected_end).unwrap_or("")
            );
        }
    }
}

fn chart_parts(bytes: &[u8]) -> Vec<String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut names: Vec<String> =
        (0..z.len()).map(|i| z.name_for_index(i).unwrap().to_string()).filter(|n| n.starts_with("xl/charts/chart")).collect();
    names.sort();
    names
        .iter()
        .map(|n| {
            let mut s = String::new();
            z.by_name(n).unwrap().read_to_string(&mut s).unwrap();
            s
        })
        .collect()
}

#[test]
fn trendlines_follow_the_series_schema_sequence() {
    let parts = chart_parts(&write_xlsx(&workbook()).unwrap());
    assert_eq!(parts.len(), KINDS.len());
    for (xml, k) in parts.iter().zip(KINDS) {
        let at = |tag: &str| xml.find(tag).unwrap_or_else(|| panic!("{k:?}: no {tag}"));
        let first = at("<c:trendline>");
        // After spPr / marker / invertIfNegative, before the data references.
        let data = if matches!(k, ChartKind::Scatter | ChartKind::ScatterLines | ChartKind::Bubble) { "<c:xVal>" } else { "<c:cat>" };
        assert!(at("<c:spPr>") < first && first < at(data), "{k:?}");
        if let Some(m) = xml.find("<c:marker>") {
            assert!(m < first, "{k:?}: marker after trendline");
        }
        if let Some(m) = xml.find("<c:invertIfNegative") {
            assert!(m < first, "{k:?}: invertIfNegative after trendline");
        }
        assert_eq!(xml.matches("<c:trendline>").count(), all_trendlines().len(), "{k:?}");
    }
    // Children in CT_Trendline order.
    let xml = &parts[0];
    let lin = &xml[xml.find("<c:trendline>").unwrap()..];
    let lin = &lin[..lin.find("</c:trendline>").unwrap()];
    let pos: Vec<usize> = [
        "<c:trendlineType val=\"linear\"/>",
        "<c:forward val=\"2\"/>",
        "<c:backward val=\"0.5\"/>",
        "<c:dispRSqr val=\"1\"/>",
        "<c:dispEq val=\"1\"/>",
        "<c:trendlineLbl>",
    ]
    .iter()
    .map(|t| lin.find(t).unwrap_or_else(|| panic!("missing {t}")))
    .collect();
    assert!(pos.windows(2).all(|w| w[0] < w[1]), "{lin}");
    let exp = &xml[xml.find("<c:name>Growth &amp; decay</c:name>").expect("escaped name")..];
    assert!(exp.find("<c:trendlineType val=\"exp\"/>").unwrap() < exp.find("<c:intercept val=\"1.5\"/>").unwrap());
}

#[test]
fn excel_style_trendlines_are_read() {
    // As Excel writes them: a theme-coloured dashed line, defaults left out, and an unknown
    // type that is skipped rather than failing the chart.
    let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:lineChart><c:grouping val="standard"/><c:ser><c:idx val="0"/><c:order val="0"/><c:marker><c:symbol val="none"/></c:marker><c:trendline><c:spPr><a:ln w="19050" cap="rnd"><a:solidFill><a:schemeClr val="accent2"/></a:solidFill><a:prstDash val="sysDot"/></a:ln></c:spPr><c:trendlineType val="poly"/><c:order val="3"/><c:dispRSqr val="0"/><c:dispEq/></c:trendline><c:trendline><c:trendlineType val="movingAvg"/><c:period val="1"/></c:trendline><c:trendline><c:trendlineType val="cubicSpline"/></c:trendline><c:val><c:numRef><c:f>Data!$B$2:$B$6</c:f></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:lineChart></c:plotArea></c:chart></c:chartSpace>"#;
    let root = crate::xml::parse(xml.as_bytes()).unwrap();
    let c = crate::chart::read_chart(&root).unwrap();
    let t = &c.series[0].trendlines;
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].kind, TrendlineKind::Polynomial(3));
    assert!(t[0].display_equation && !t[0].display_r_squared, "an empty CT_Boolean means true");
    assert_eq!((t[0].width, t[0].dash), (Some(1.5), Some(LineDash::SysDot)));
    assert_eq!(t[0].color, Some(Color::Theme(5, 0)));
    assert_eq!(t[1].kind, TrendlineKind::MovingAverage(2), "period clamps to 2..255");
}

#[test]
fn a_huge_forecast_in_a_file_is_read_clamped() {
    let t = |f: &str, b: &str| {
        format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/><c:trendline><c:trendlineType val="linear"/><c:forward val="{f}"/><c:backward val="{b}"/></c:trendline><c:val><c:numRef><c:f>Data!$B$2:$B$6</c:f></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart></c:plotArea></c:chart></c:chartSpace>"#
        )
    };
    let read = |f: &str, b: &str| {
        let root = crate::xml::parse(t(f, b).as_bytes()).unwrap();
        crate::chart::read_chart(&root).unwrap().series[0].trendlines[0].clone()
    };
    let big = read("1000000", "1e300");
    assert_eq!((big.forward, big.backward), (Trendline::MAX_FORECAST, Trendline::MAX_FORECAST));
    let small = read("2.5", "-3");
    assert_eq!((small.forward, small.backward), (2.5, 0.0), "ordinary values are kept");
}
