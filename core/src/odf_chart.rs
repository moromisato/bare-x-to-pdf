use crate::model::*;
use crate::xml::Element;
use std::collections::HashMap;

struct Grid {
    rows: Vec<Vec<Option<(Option<f64>, String)>>>,
    anchors: HashMap<String, (usize, usize)>,
}

impl Grid {
    fn parse(table: &Element) -> Grid {
        let mut rows = Vec::new();
        let mut anchors = HashMap::new();
        collect_rows(table, &mut rows, &mut anchors);
        Grid { rows, anchors }
    }

    fn cells(&self, range: &str) -> Vec<(Option<f64>, String)> {
        let range = range.split_whitespace().next().unwrap_or(range);
        let Some((r1, c1, r2, c2)) = parse_range(range) else { return Vec::new() };
        let (start, down) = if range.starts_with("local-table") {
            ((r1, c1), true)
        } else if let Some(&start) = self.anchors.get(range) {
            (start, c1 == c2)
        } else {
            ((r1, c1), true)
        };
        let count = (r2 - r1 + 1) * (c2 - c1 + 1);
        (0..count)
            .map(|k| {
                let (r, c) = if down { (start.0 + k, start.1) } else { (start.0, start.1 + k) };
                self.rows.get(r).and_then(|row| row.get(c)).cloned().flatten().unwrap_or((None, String::new()))
            })
            .collect()
    }
}

fn collect_rows(el: &Element, out: &mut Vec<Vec<Option<(Option<f64>, String)>>>, anchors: &mut HashMap<String, (usize, usize)>) {
    for child in el.elements() {
        match child.name.as_str() {
            "table-row" => {
                let mut row = Vec::new();
                for cell in child.children("table-cell") {
                    if let Some(desc) = cell.child("g").and_then(|g| g.child("desc")) {
                        for range in desc.text().split_whitespace() {
                            anchors.insert(range.to_string(), (out.len(), row.len()));
                        }
                    }
                    let count = cell.attr("number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(256);
                    let value = cell.attr("value").and_then(|v| v.parse::<f64>().ok());
                    let text: Vec<String> = cell.children("p").map(|p| p.text()).collect();
                    for _ in 0..count {
                        row.push(Some((value, text.join(" "))));
                    }
                }
                out.push(row);
            }
            "table-header-rows" | "table-rows" | "table-row-group" => collect_rows(child, out, anchors),
            _ => {}
        }
    }
}

fn column_number(letters: &str) -> Option<usize> {
    let mut n = 0usize;
    for ch in letters.chars() {
        if !ch.is_ascii_alphabetic() {
            return None;
        }
        n = n * 26 + (ch.to_ascii_uppercase() as usize - 'A' as usize + 1);
    }
    (n > 0).then(|| n - 1)
}

fn cell_address(text: &str) -> Option<(usize, usize)> {
    let text = text.rsplit('.').next()?.replace('$', "");
    let split = text.find(|c: char| c.is_ascii_digit())?;
    let col = column_number(&text[..split])?;
    let row: usize = text[split..].parse().ok()?;
    Some((row.checked_sub(1)?, col))
}

fn parse_range(range: &str) -> Option<(usize, usize, usize, usize)> {
    let range = range.split_whitespace().next()?;
    let (a, b) = range.split_once(':').unwrap_or((range, range));
    let (r1, c1) = cell_address(a)?;
    let (r2, c2) = cell_address(b)?;
    Some((r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)))
}

struct ChartStyles {
    styles: HashMap<String, Element>,
}

impl ChartStyles {
    fn graphic(&self, name: Option<&str>) -> Option<&Element> {
        self.styles.get(name?)?.child("graphic-properties")
    }

    fn fill(&self, name: Option<&str>) -> Option<Color> {
        let g = self.graphic(name)?;
        if g.attr("fill") == Some("none") {
            return None;
        }
        g.attr("fill-color").and_then(Color::parse_hex)
    }

    fn stroke(&self, name: Option<&str>) -> Option<Option<Color>> {
        let g = self.graphic(name)?;
        if g.attr("stroke") == Some("none") {
            return Some(None);
        }
        g.attr("stroke-color").and_then(Color::parse_hex).map(Some)
    }

    fn chart(&self, name: Option<&str>) -> Option<&Element> {
        self.styles.get(name?)?.child("chart-properties")
    }

    fn font(&self, name: Option<&str>) -> Option<String> {
        let t = self.styles.get(name?)?.child("text-properties")?;
        t.attr("font-family").or_else(|| t.attr("font-name")).map(|f| f.trim_matches('\'').to_string())
    }
}

pub fn chart_content(root: &Element) -> DrawingContent {
    match parse(root) {
        Some(chart) => DrawingContent::Chart(chart),
        None => DrawingContent::Placeholder(None),
    }
}

fn parse(root: &Element) -> Option<Chart> {
    let chart = root.child("body")?.child("chart")?.child("chart")?;
    let mut styles = HashMap::new();
    if let Some(auto) = root.child("automatic-styles") {
        for style in auto.children("style") {
            if let Some(name) = style.attr("name") {
                styles.insert(name.to_string(), style.clone());
            }
        }
    }
    let styles = ChartStyles { styles };
    let plot = chart.child("plot-area")?;
    let grid = chart.child("table").map(Grid::parse).unwrap_or(Grid { rows: Vec::new(), anchors: HashMap::new() });
    let vertical = styles.chart(plot.attr("style-name")).and_then(|c| c.attr("vertical")) == Some("true");
    let kind = match chart.attr("class").unwrap_or("").trim_start_matches("chart:") {
        "bar" if vertical => ChartKind::Bar,
        "bar" => ChartKind::Column,
        "line" | "scatter" | "radar" | "filled-radar" | "stock" => ChartKind::Line,
        "circle" | "ring" => ChartKind::Pie,
        "area" => ChartKind::Area,
        _ => return None,
    };
    let title = chart.child("title").map(|t| t.children("p").map(|p| p.text()).collect::<Vec<_>>().join(" ")).filter(|t| !t.trim().is_empty());
    let legend = chart.child("legend").map(|l| match l.attr("legend-position") {
        Some("bottom") => LegendPos::Bottom,
        Some("top") => LegendPos::Top,
        Some("start") => LegendPos::Left,
        _ => LegendPos::Right,
    });
    let mut categories: Vec<String> = Vec::new();
    let mut grid_color = None;
    let mut grid_seen = false;
    for axis in plot.children("axis") {
        if axis.attr("dimension") == Some("x") {
            if let Some(range) = axis.child("categories").and_then(|c| c.attr("cell-range-address")) {
                categories = grid.cells(range).into_iter().map(|(v, t)| if t.is_empty() { v.map(crate::xlsx::format::general).unwrap_or_default() } else { t }).collect();
            }
        }
        if axis.attr("dimension") == Some("y") {
            for g in axis.children("grid") {
                if g.attr("class").unwrap_or("major") == "major" {
                    grid_seen = true;
                    grid_color = styles.stroke(g.attr("style-name")).flatten().or(Some(Color(0xB3, 0xB3, 0xB3)));
                }
            }
        }
    }
    let mut series = Vec::new();
    let mut markers = false;
    for s in plot.children("series") {
        let values: Vec<Option<f64>> = s.attr("values-cell-range-address").map(|r| grid.cells(r).into_iter().map(|(v, _)| v).collect()).unwrap_or_default();
        let name = s
            .attr("label-cell-address")
            .and_then(|a| grid.cells(a).into_iter().next())
            .map(|(_, t)| t)
            .filter(|t| !t.is_empty());
        let style = s.attr("style-name");
        let color = if kind == ChartKind::Line {
            styles.stroke(style).flatten().or_else(|| styles.fill(style))
        } else {
            styles.fill(style)
        };
        let no_line = kind == ChartKind::Line && matches!(styles.stroke(style), Some(None));
        if kind == ChartKind::Line {
            markers |= styles.chart(style).and_then(|c| c.attr("symbol-type")).is_some_and(|t| t != "none");
        }
        let mut point_colors = Vec::new();
        for point in s.children("data-point") {
            let count = point.attr("repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(4096);
            let fill = styles.fill(point.attr("style-name"));
            for _ in 0..count {
                point_colors.push(fill);
            }
        }
        series.push(ChartSeries { name, values, color, point_colors, no_line });
    }
    if series.is_empty() {
        return None;
    }
    let longest = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    while categories.len() < longest {
        categories.push((categories.len() + 1).to_string());
    }
    let gap_width = styles.chart(series_style(plot)).and_then(|c| c.attr("gap-width")).and_then(|g| g.parse::<f64>().ok()).unwrap_or(100.0);
    let wall = plot.child("wall").and_then(|w| w.attr("style-name"));
    let style = ChartStyle {
        font: chart
            .child("title")
            .and_then(|t| {
                t.children("p")
                    .flat_map(|p| p.children("span"))
                    .find_map(|span| styles.font(span.attr("style-name")))
                    .or_else(|| styles.font(t.attr("style-name")))
            })
            .or_else(|| styles.font(chart.attr("style-name")))
            .or_else(|| Some("Liberation Sans".into())),
        border: styles.stroke(chart.attr("style-name")).flatten(),
        plot_fill: styles.fill(wall),
        plot_border: styles.stroke(wall).flatten(),
        grid: grid_color,
        no_grid: !grid_seen,
    };
    Some(Chart { title, kind, categories, series, legend, gap_width, markers, style })
}

fn series_style(plot: &Element) -> Option<&str> {
    plot.child("series").and_then(|s| s.attr("style-name"))
}
