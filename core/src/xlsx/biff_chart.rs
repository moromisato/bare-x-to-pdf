use crate::model::{Chart, ChartKind, ChartSeries, ChartStyle, Color, LegendPos};
use std::collections::BTreeMap;

const SERIES: u16 = 0x1003;
const DATAFORMAT: u16 = 0x1006;
const LINEFORMAT: u16 = 0x1007;
const MARKERFORMAT: u16 = 0x1009;
const AREAFORMAT: u16 = 0x100A;
const SERIESTEXT: u16 = 0x100D;
const CHARTFORMAT: u16 = 0x1014;
const LEGEND: u16 = 0x1015;
const BAR: u16 = 0x1017;
const LINE: u16 = 0x1018;
const PIE: u16 = 0x1019;
const AREA: u16 = 0x101A;
const SCATTER: u16 = 0x101B;
const AXISLINEFORMAT: u16 = 0x1021;
const TEXT: u16 = 0x1025;
const OBJECTLINK: u16 = 0x1027;
const FRAME: u16 = 0x1032;
const BEGIN: u16 = 0x1033;
const END: u16 = 0x1034;
const PLOTAREA: u16 = 0x1035;
const RADAR: u16 = 0x103E;
const BRAI: u16 = 0x1051;
const FONTX: u16 = 0x1026;

#[derive(Clone, Copy)]
pub struct Area {
    pub sheet: u16,
    pub row1: u32,
    pub col1: u32,
    pub row2: u32,
    pub col2: u32,
}

enum Name {
    Literal(String),
    Cells(Area),
}

#[derive(Default)]
struct SeriesSource {
    name: Option<Name>,
    values: Option<Area>,
    categories: Option<Area>,
    fill: Option<Color>,
    line: Option<Color>,
    no_line: bool,
    marker: bool,
    points: BTreeMap<usize, Color>,
}

pub struct ChartSource {
    kind: Option<ChartKind>,
    title: Option<String>,
    legend: Option<LegendPos>,
    gap_width: f64,
    series: Vec<SeriesSource>,
    style: ChartStyle,
    pub font: Option<usize>,
}

pub enum Cell {
    Number(f64, String),
    Text(String),
    Empty,
}

fn le16(d: &[u8], at: usize) -> u16 {
    d.get(at..at + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
}

fn rgb(d: &[u8]) -> Option<Color> {
    d.get(0..3).map(|c| Color(c[0], c[1], c[2]))
}

enum Stroke {
    None,
    Auto,
    Solid(Color),
}

fn stroke(d: &[u8]) -> Stroke {
    if le16(d, 4) == 5 {
        Stroke::None
    } else if le16(d, 8) & 1 != 0 {
        Stroke::Auto
    } else {
        rgb(d).map_or(Stroke::Auto, Stroke::Solid)
    }
}

fn area_fill(d: &[u8]) -> Option<Color> {
    (le16(d, 8) != 0 && le16(d, 10) & 1 == 0).then(|| rgb(d)).flatten()
}

fn series_text(d: &[u8]) -> String {
    let count = d.get(2).copied().unwrap_or(0) as usize;
    let wide = d.get(3).copied().unwrap_or(0) & 1 == 1;
    let body = d.get(4..).unwrap_or(&[]);
    if wide {
        let units: Vec<u16> = body.chunks(2).take(count).filter(|c| c.len() == 2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        body.iter().take(count).map(|&b| b as char).collect()
    }
}

fn area(d: &[u8]) -> Option<Area> {
    if d.get(1).copied()? != 2 {
        return None;
    }
    let len = le16(d, 6) as usize;
    let formula = d.get(8..8 + len)?;
    let token = *formula.first()?;
    match token {
        0x3A | 0x5A | 0x7A => {
            let (row, col) = (le16(formula, 3) as u32, (le16(formula, 5) & 0x3FFF) as u32);
            Some(Area { sheet: le16(formula, 1), row1: row, col1: col, row2: row, col2: col })
        }
        0x3B | 0x5B | 0x7B => Some(Area {
            sheet: le16(formula, 1),
            row1: le16(formula, 3) as u32,
            row2: le16(formula, 5) as u32,
            col1: (le16(formula, 7) & 0x3FFF) as u32,
            col2: (le16(formula, 9) & 0x3FFF) as u32,
        }),
        _ => None,
    }
}

pub fn parse(records: &[(u16, &[u8])]) -> Option<ChartSource> {
    let mut chart = ChartSource { kind: None, title: None, legend: None, gap_width: 150.0, series: Vec::new(), style: ChartStyle::default(), font: None };
    let mut stack: Vec<u16> = Vec::new();
    let mut last = 0u16;
    let mut point: Option<usize> = None;
    let mut text_value: Option<String> = None;
    let mut text_link = 0u16;
    let mut axis_line = None;
    let mut plot_frame = false;
    let mut grid_seen = false;
    let mut border = Stroke::Auto;
    let mut plot_border = Stroke::Auto;
    let mut grid = Stroke::Auto;
    for &(kind, data) in records {
        let context = stack.last().copied().unwrap_or(0);
        let in_series = stack.contains(&SERIES);
        let in_format = stack.last() == Some(&DATAFORMAT);
        match kind {
            BEGIN => {
                stack.push(last);
                if last == TEXT {
                    text_value = None;
                    text_link = 0;
                }
            }
            END => {
                let closed = stack.pop();
                if closed == Some(TEXT) && text_link == 1 {
                    chart.title = text_value.take().filter(|t| !t.trim().is_empty());
                }
                if closed == Some(DATAFORMAT) {
                    point = None;
                }
                if closed == Some(FRAME) {
                    plot_frame = false;
                }
            }
            SERIES => chart.series.push(SeriesSource::default()),
            BRAI if in_series && context == SERIES => {
                let series = chart.series.last_mut()?;
                match data.first().copied() {
                    Some(0) => series.name = area(data).map(Name::Cells).or(series.name.take()),
                    Some(1) => series.values = area(data),
                    Some(2) => series.categories = area(data),
                    _ => {}
                }
            }
            SERIESTEXT if context == TEXT => text_value = Some(series_text(data)),
            SERIESTEXT if in_series => {
                let series = chart.series.last_mut()?;
                if series.name.is_none() {
                    series.name = Some(Name::Literal(series_text(data)));
                }
            }
            OBJECTLINK if context == TEXT => text_link = le16(data, 0),
            DATAFORMAT if in_series => {
                let index = le16(data, 0);
                point = (index != 0xFFFF).then_some(index as usize);
            }
            AREAFORMAT if in_format => {
                let series = chart.series.last_mut()?;
                if let Some(color) = area_fill(data) {
                    match point {
                        Some(i) => {
                            series.points.insert(i, color);
                        }
                        None => series.fill = Some(color),
                    }
                }
            }
            LINEFORMAT if in_format && point.is_none() => {
                let series = chart.series.last_mut()?;
                match stroke(data) {
                    Stroke::None => series.no_line = true,
                    Stroke::Solid(color) => series.line = Some(color),
                    Stroke::Auto => {}
                }
            }
            MARKERFORMAT if in_format && point.is_none() => chart.series.last_mut()?.marker = le16(data, 8) != 0,
            FRAME => {
                plot_frame = last == PLOTAREA;
                last = FRAME;
                continue;
            }
            AREAFORMAT if context == FRAME && plot_frame => chart.style.plot_fill = area_fill(data),
            LINEFORMAT if context == FRAME && plot_frame => plot_border = stroke(data),
            LINEFORMAT if context == FRAME && !stack.contains(&CHARTFORMAT) && stack.len() <= 2 => border = stroke(data),
            AXISLINEFORMAT => axis_line = Some(le16(data, 0)),
            FONTX if chart.font.is_none() => chart.font = Some(le16(data, 0) as usize),
            LINEFORMAT if axis_line == Some(1) => {
                grid_seen = true;
                grid = stroke(data);
                axis_line = None;
            }
            LINEFORMAT => axis_line = None,
            BAR if chart.kind.is_none() => {
                chart.kind = Some(if le16(data, 4) & 1 != 0 { ChartKind::Bar } else { ChartKind::Column });
                chart.gap_width = le16(data, 2) as f64;
            }
            LINE | SCATTER | RADAR if chart.kind.is_none() => chart.kind = Some(ChartKind::Line),
            PIE if chart.kind.is_none() => chart.kind = Some(ChartKind::Pie),
            AREA if chart.kind.is_none() => chart.kind = Some(ChartKind::Area),
            LEGEND if chart.legend.is_none() => {
                chart.legend = Some(match data.get(16).copied().unwrap_or(3) {
                    0 => LegendPos::Bottom,
                    2 => LegendPos::Top,
                    4 => LegendPos::Left,
                    _ => LegendPos::Right,
                });
            }
            _ => {}
        }
        last = kind;
    }
    let black = Color(0, 0, 0);
    let pick = |stroke: Stroke, auto: Option<Color>| match stroke {
        Stroke::None => None,
        Stroke::Auto => auto,
        Stroke::Solid(color) => Some(color),
    };
    chart.style.border = pick(border, Some(black));
    chart.style.plot_border = pick(plot_border, None);
    let grid_color = pick(grid, Some(black));
    chart.style.no_grid = !grid_seen || grid_color.is_none();
    chart.style.grid = grid_color;
    chart.kind.is_some().then_some(chart)
}

pub fn resolve(source: &ChartSource, cells: &dyn Fn(&Area) -> Vec<Cell>, fills: &[Color], lines: &[Color], font: Option<String>) -> Chart {
    let kind = source.kind.clone().unwrap_or(ChartKind::Column);
    let mut categories: Vec<String> = Vec::new();
    let mut series = Vec::new();
    for (i, s) in source.series.iter().enumerate() {
        let values: Vec<Option<f64>> = s
            .values
            .as_ref()
            .map(|a| cells(a).into_iter().map(|c| if let Cell::Number(v, _) = c { Some(v) } else { None }).collect())
            .unwrap_or_default();
        if categories.is_empty() {
            if let Some(a) = &s.categories {
                categories = cells(a)
                    .into_iter()
                    .map(|c| match c {
                        Cell::Number(_, text) | Cell::Text(text) => text,
                        Cell::Empty => String::new(),
                    })
                    .collect();
            }
        }
        let name = match &s.name {
            Some(Name::Literal(t)) => Some(t.clone()),
            Some(Name::Cells(a)) => cells(a).into_iter().find_map(|c| match c {
                Cell::Number(_, t) | Cell::Text(t) => Some(t),
                Cell::Empty => None,
            }),
            None => None,
        };
        let default_fill = fills.get(i % fills.len().max(1)).copied();
        let default_line = lines.get(i % lines.len().max(1)).copied();
        let color = if kind == ChartKind::Line { s.line.or(s.fill).or(default_line) } else { s.fill.or(default_fill) };
        let point_colors = if kind == ChartKind::Pie {
            (0..values.len()).map(|p| s.points.get(&p).copied().or_else(|| fills.get(p % fills.len().max(1)).copied())).collect()
        } else {
            Vec::new()
        };
        series.push(ChartSeries { name, values, color, point_colors, no_line: s.no_line });
    }
    let longest = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    while categories.len() < longest {
        categories.push((categories.len() + 1).to_string());
    }
    let markers = kind == ChartKind::Line && source.series.iter().any(|s| s.marker);
    let mut style = source.style.clone();
    style.font = Some(font.unwrap_or_else(|| "Liberation Sans".into()));
    Chart { title: source.title.clone(), kind, categories, series, legend: source.legend, gap_width: source.gap_width, markers, style }
}
