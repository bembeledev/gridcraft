//! Importing a combination chart keeps each series' own group kind, the first group's included.

use gridcraft_model::ChartKind;

use crate::chart::read_chart;
use crate::xml::parse;

fn ser(i: usize) -> String {
    format!("<c:ser><c:idx val=\"{i}\"/><c:order val=\"{i}\"/><c:val><c:numRef><c:f>Data!$A$1:$A$5</c:f></c:numRef></c:val></c:ser>")
}

const AX: &str = "<c:axId val=\"1\"/><c:axId val=\"2\"/>";

fn bar(sers: &str) -> String {
    format!("<c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/>{sers}{AX}</c:barChart>")
}

fn line(sers: &str) -> String {
    format!("<c:lineChart><c:grouping val=\"standard\"/>{sers}{AX}</c:lineChart>")
}

fn kinds(first: &str, second: &str) -> Vec<Option<ChartKind>> {
    let xml = format!(
        "<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\">\
         <c:chart><c:plotArea>{first}{second}</c:plotArea></c:chart></c:chartSpace>"
    );
    let root = parse(xml.as_bytes()).unwrap();
    let ch = read_chart(&root).unwrap();
    assert_eq!(ch.kind, ChartKind::Combo);
    ch.series.iter().map(|s| s.kind).collect()
}

#[test]
fn first_group_with_two_columns_stays_columns() {
    let k = kinds(&bar(&(ser(0) + &ser(1))), &line(&ser(2)));
    assert_eq!(k[0], Some(ChartKind::ColumnClustered));
    assert_eq!(k[1], Some(ChartKind::ColumnClustered));
    assert!(matches!(k[2], Some(ChartKind::Line | ChartKind::LineMarkers)));
}

#[test]
fn line_first_group_stays_a_line() {
    let k = kinds(&line(&ser(0)), &bar(&ser(1)));
    assert!(matches!(k[0], Some(ChartKind::Line | ChartKind::LineMarkers)));
    assert_eq!(k[1], Some(ChartKind::ColumnClustered));
}

#[test]
fn chart_combo_horizontal_bars_keep_titles_on_category_and_value_axes_after_save() {
    let title = |text: &str| format!("<c:title><c:tx><c:rich><a:p><a:r><a:t>{text}</a:t></a:r></a:p></c:rich></c:tx></c:title>");
    let bars = bar(&ser(0)).replace("val=\"col\"", "val=\"bar\"");
    let xml = format!(
        "<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" \
         xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><c:chart><c:plotArea>\
         {bars}{}<c:catAx><c:axId val=\"1\"/><c:axPos val=\"l\"/>{}</c:catAx>\
         <c:valAx><c:axId val=\"2\"/><c:axPos val=\"b\"/>{}</c:valAx>\
         </c:plotArea></c:chart></c:chartSpace>",
        line(&ser(1)),
        title("Region"),
        title("Sales")
    );
    let mut chart = read_chart(&parse(xml.as_bytes()).unwrap()).unwrap();
    let wb = gridcraft_model::Workbook::default();
    for _ in 0..2 {
        assert_eq!(chart.kind, ChartKind::Combo);
        assert_eq!(chart.x_title.as_deref(), Some("Region"));
        assert_eq!(chart.y_title.as_deref(), Some("Sales"));
        let saved = crate::chart::write_chart(&wb, 0, &chart, "Book.xlsx");
        let root = parse(saved.as_bytes()).unwrap();
        let plot = root.path(&["chart", "plotArea"]).unwrap();
        assert_eq!(plot.path(&["catAx", "title", "tx", "rich", "p", "r", "t"]).unwrap().text, "Region");
        assert_eq!(plot.path(&["valAx", "title", "tx", "rich", "p", "r", "t"]).unwrap().text, "Sales");
        chart = read_chart(&root).unwrap();
    }
}
