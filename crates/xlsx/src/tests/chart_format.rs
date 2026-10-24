//! Chart formatting Excel saves beyond the chart type: markers-only and line-only scatter series,
//! marker symbols and fills, preset colours, fixed axis bounds and units, vertical gridlines,
//! deleted legend entries, a legend placed by hand, chart and plot area fill and border.

use std::sync::Arc;

use gridcraft_model::{Anchor, AxisScale, Chart, ChartKind, Color, LegendPos, MarkerSymbol, Series, SeriesFormat, Sheet, Workbook};

use crate::chart::read_chart;
use crate::xml::parse;
use crate::{read_xlsx, write_xlsx};

const XML: &str = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea>
<c:scatterChart><c:scatterStyle val="lineMarker"/>
<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>Data</c:v></c:tx><c:spPr><a:ln w="28575"><a:noFill/></a:ln></c:spPr><c:marker><c:symbol val="circle"/><c:size val="6"/><c:spPr><a:solidFill><a:srgbClr val="000080"/></a:solidFill></c:spPr></c:marker><c:xVal><c:numRef><c:f>D!$A$1:$A$9</c:f></c:numRef></c:xVal><c:yVal><c:numRef><c:f>D!$B$1:$B$9</c:f></c:numRef></c:yVal></c:ser>
<c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:v>Fit</c:v></c:tx><c:spPr><a:ln><a:solidFill><a:prstClr val="black"/></a:solidFill></a:ln></c:spPr><c:marker><c:symbol val="none"/></c:marker><c:xVal><c:numRef><c:f>D!$A$1:$A$9</c:f></c:numRef></c:xVal><c:yVal><c:numRef><c:f>D!$C$1:$C$9</c:f></c:numRef></c:yVal></c:ser>
<c:ser><c:idx val="2"/><c:order val="2"/><c:tx><c:v>Points</c:v></c:tx><c:spPr><a:ln><a:noFill/></a:ln></c:spPr><c:marker><c:symbol val="square"/><c:spPr><a:solidFill><a:sysClr val="windowText"/></a:solidFill></c:spPr></c:marker><c:xVal><c:numRef><c:f>D!$A$1:$A$9</c:f></c:numRef></c:xVal><c:yVal><c:numRef><c:f>D!$D$1:$D$9</c:f></c:numRef></c:yVal></c:ser>
<c:axId val="1"/><c:axId val="2"/></c:scatterChart>
<c:valAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/><c:max val="240"/><c:min val="0"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:majorGridlines/><c:crossAx val="2"/><c:majorUnit val="20"/></c:valAx>
<c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/><c:max val="-8"/><c:min val="-10"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/><c:crossAx val="1"/><c:majorUnit val="0.5"/></c:valAx>
<c:spPr><a:noFill/><a:ln w="12700"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln></c:spPr>
</c:plotArea>
<c:legend><c:legendPos val="r"/><c:legendEntry><c:idx val="1"/><c:delete val="1"/></c:legendEntry><c:layout><c:manualLayout><c:xMode val="edge"/><c:yMode val="edge"/><c:x val="0.12"/><c:y val="0.11"/><c:w val="0.2"/><c:h val="0.2"/></c:manualLayout></c:layout><c:overlay val="0"/></c:legend>
</c:chart><c:spPr><a:solidFill><a:srgbClr val="FFFF99"/></a:solidFill><a:ln w="3175"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln></c:spPr></c:chartSpace>"#;

fn check(ch: &Chart) {
    assert_eq!(ch.kind, ChartKind::ScatterLines);
    let f = |i: usize| ch.series[i].format;
    // Markers only, in navy circles; a black line without markers; black squares without a line.
    assert_eq!(f(0), SeriesFormat { no_line: true, marker: MarkerSymbol::Circle, marker_fill: None });
    assert_eq!(ch.series[0].color, Some(Color::Rgb(0x000080)));
    assert_eq!(f(1), SeriesFormat { no_line: false, marker: MarkerSymbol::None, marker_fill: None });
    assert_eq!(ch.series[1].color, Some(Color::Rgb(0x000000)));
    assert_eq!((f(2).no_line, f(2).marker), (true, MarkerSymbol::Square));
    assert_eq!(ch.series[2].color, Some(Color::Rgb(0x000000)));
    let fm = &ch.format;
    assert_eq!(fm.x_axis, AxisScale { min: Some(0.0), max: Some(240.0), major: Some(20.0) });
    assert_eq!(fm.y_axis, AxisScale { min: Some(-10.0), max: Some(-8.0), major: Some(0.5) });
    assert!(fm.x_gridlines && ch.gridlines);
    assert_eq!(fm.legend_deleted, vec![1]);
    assert_eq!(fm.legend_layout, Some([0.12, 0.11, 0.2, 0.2]));
    assert_eq!((fm.chart_fill, fm.chart_border), (Some(Color::Rgb(0xFFFF99)), Some(Color::Rgb(0x000000))));
    assert_eq!((fm.plot_fill, fm.plot_border), (None, Some(Color::Rgb(0x000000))));
}

#[test]
fn excel_chart_formatting_is_read() {
    let ch = read_chart(&parse(XML.as_bytes()).unwrap()).unwrap();
    assert_eq!(ch.legend, LegendPos::Right);
    check(&ch);
}

fn book(ch: Chart) -> Workbook {
    let mut s = Sheet::new("D");
    s.charts.push(Chart { id: 1, anchor: Anchor::default(), ..ch });
    Workbook { sheets: vec![Arc::new(s)], ..Workbook::default() }
}

fn chart_xml(bytes: &[u8]) -> String {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut s = String::new();
    z.by_name("xl/charts/chart1.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

#[test]
fn chart_formatting_round_trips_without_drift() {
    let ch = read_chart(&parse(XML.as_bytes()).unwrap()).unwrap();
    let bytes = write_xlsx(&book(ch)).unwrap();
    let (back, _) = read_xlsx(&bytes).unwrap();
    check(&back.sheets[0].charts[0]);
    // A second save writes the same chart.
    assert_eq!(chart_xml(&write_xlsx(&back).unwrap()), chart_xml(&bytes));
}

#[test]
fn scatter_groups_on_two_axes_stay_a_scatter_chart() {
    let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea>
<c:scatterChart><c:scatterStyle val="lineMarker"/><c:ser><c:idx val="0"/><c:order val="0"/><c:xVal><c:numRef><c:f>D!$A$1:$A$3</c:f></c:numRef></c:xVal><c:yVal><c:numRef><c:f>D!$B$1:$B$3</c:f></c:numRef></c:yVal></c:ser><c:axId val="1"/><c:axId val="2"/></c:scatterChart>
<c:scatterChart><c:scatterStyle val="lineMarker"/><c:ser><c:idx val="1"/><c:order val="1"/><c:xVal><c:numRef><c:f>D!$A$1:$A$3</c:f></c:numRef></c:xVal><c:yVal><c:numRef><c:f>D!$C$1:$C$3</c:f></c:numRef></c:yVal></c:ser><c:axId val="3"/><c:axId val="4"/></c:scatterChart>
<c:valAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/><c:max val="30"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="2"/></c:valAx>
<c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/><c:min val="5"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:crossAx val="1"/></c:valAx>
<c:valAx><c:axId val="4"/><c:scaling><c:orientation val="minMax"/><c:max val="1000"/></c:scaling><c:delete val="0"/><c:axPos val="r"/><c:crossAx val="3"/></c:valAx>
<c:valAx><c:axId val="3"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="1"/><c:axPos val="b"/><c:crossAx val="4"/></c:valAx>
</c:plotArea></c:chart></c:chartSpace>"#;
    let ch = read_chart(&parse(xml.as_bytes()).unwrap()).unwrap();
    assert_eq!(ch.kind, ChartKind::ScatterLines);
    assert!(!ch.series[0].secondary && ch.series[1].secondary);
    assert_eq!(ch.format.x_axis.max, Some(30.0));
    assert_eq!(ch.format.y_axis.min, Some(5.0));
    assert_eq!(ch.format.y2_axis.max, Some(1000.0));
    // Saved and read back: still a scatter chart, the secondary axis bound kept.
    let mut wb = book(ch);
    let s = Arc::make_mut(&mut wb.sheets[0]);
    s.charts[0].series.iter_mut().for_each(|x| x.format = SeriesFormat::default());
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    let ch = &back.sheets[0].charts[0];
    assert_eq!(ch.kind, ChartKind::ScatterLines);
    assert_eq!(ch.format.y2_axis.max, Some(1000.0));
    assert!(ch.series[1].secondary);
}

#[test]
fn hostile_axis_values_are_dropped() {
    let s = AxisScale { min: Some(f64::NAN), max: Some(f64::INFINITY), major: Some(-1.0) }.sanitized();
    assert_eq!(s, AxisScale::default());
    let s = AxisScale { min: Some(5.0), max: Some(1.0), major: Some(0.0) }.sanitized();
    assert_eq!(s, AxisScale::default());
    let _ = Series::default();
}

#[test]
fn review_category_gridlines_markers_only_scatter_and_legend_size_round_trip() {
    let chart_of = |kind: ChartKind, format: SeriesFormat| Chart {
        kind,
        series: vec![Series { values: "D!$B$1:$B$3".into(), categories: Some("D!$A$1:$A$3".into()), format, ..Series::default() }],
        ..read_chart(&parse(XML.as_bytes()).unwrap()).unwrap()
    };
    // Category-axis gridlines on a column chart.
    let mut col = chart_of(ChartKind::ColumnClustered, SeriesFormat::default());
    col.format.x_gridlines = true;
    let (back, _) = read_xlsx(&write_xlsx(&book(col)).unwrap()).unwrap();
    assert!(back.sheets[0].charts[0].format.x_gridlines);
    // A markers-only scatter series with a marker set stays markers only.
    let sq = chart_of(ChartKind::Scatter, SeriesFormat { no_line: false, marker: MarkerSymbol::Square, marker_fill: None });
    let (back, _) = read_xlsx(&write_xlsx(&book(sq)).unwrap()).unwrap();
    assert_eq!(back.sheets[0].charts[0].kind, ChartKind::Scatter);
    // A legend placed without a size is written without one.
    let mut placed = chart_of(ChartKind::ScatterLines, SeriesFormat::default());
    placed.format.legend_layout = Some([0.1, 0.2, 0.0, 0.0]);
    let xml = chart_xml(&write_xlsx(&book(placed)).unwrap());
    assert!(xml.contains("<c:x val=\"0.1\"/>") && !xml.contains("<c:w ") && !xml.contains("<c:h "), "{xml}");
}
