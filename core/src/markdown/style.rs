use crate::model::*;

const CM: f64 = 72.0 / 2.54;

pub struct Profile {
    pub body: RunProps,
    pub body_line: f64,
    pub body_after: f64,
    pub heading_font: &'static str,
    pub heading_sizes: [f64; 6],
    pub heading_italic: [bool; 6],
    pub heading_before: [f64; 6],
    pub heading_after: [f64; 6],
    pub list_item_after: f64,
    pub quote_after: f64,
    pub quote_line: f64,
    pub code_size: f64,
    pub code_shading: Option<Color>,
    pub code_after: f64,
    pub table_cell_margin: f64,
    pub margins: Margins,
}

pub const MONO: &str = "Liberation Mono";
pub const LINK: Color = Color(0, 0, 0x80);

impl Profile {
    pub fn markdown() -> Profile {
        Profile {
            body: RunProps { font: Some("Liberation Serif".into()), size: Some(12.0), color: Some(Color(0, 0, 0)), ..RunProps::default() },
            body_line: 1.15,
            body_after: 0.247 * CM,
            heading_font: "Liberation Sans",
            heading_sizes: [18.0, 16.0, 14.0, 13.0, 12.0, 12.0],
            heading_italic: [false, false, false, true, false, true],
            heading_before: [0.423 * CM, 0.353 * CM, 0.247 * CM, 0.212 * CM, 0.212 * CM, 0.106 * CM],
            heading_after: [0.212 * CM, 0.212 * CM, 0.212 * CM, 0.212 * CM, 0.106 * CM, 0.106 * CM],
            list_item_after: 0.247 * CM,
            quote_after: 0.5 * CM,
            quote_line: 1.0,
            code_size: 10.0,
            code_shading: Some(Color(0xE1, 0xE1, 0xE1)),
            code_after: 0.5 * CM,
            table_cell_margin: 0.1 * CM,
            margins: Margins { top: 2.0 * CM, bottom: 2.0 * CM, left: 2.0 * CM, right: 2.0 * CM, header: 0.0, footer: 0.0 },
        }
    }

    pub fn text_width(&self) -> f64 {
        612.0 - self.margins.left - self.margins.right
    }

    pub fn heading(&self, level: usize) -> (RunProps, ParagraphProps) {
        let i = level.clamp(1, 6) - 1;
        let run = RunProps {
            font: Some(self.heading_font.into()),
            size: Some(self.heading_sizes[i]),
            bold: Some(true),
            italic: Some(self.heading_italic[i]),
            color: Some(Color(0, 0, 0)),
            ..RunProps::default()
        };
        let props = ParagraphProps {
            space_before: Some(self.heading_before[i]),
            space_after: Some(self.heading_after[i]),
            line_spacing: Some(LineSpacing::Multiple(1.0)),
            keep_next: Some(true),
            ..ParagraphProps::default()
        };
        (run, props)
    }

    pub fn body_props(&self) -> ParagraphProps {
        ParagraphProps {
            space_before: Some(0.0),
            space_after: Some(self.body_after),
            line_spacing: Some(LineSpacing::Multiple(self.body_line)),
            ..ParagraphProps::default()
        }
    }

    pub fn code_run(&self) -> RunProps {
        RunProps { font: Some(MONO.into()), size: Some(self.code_size), ..self.body.clone() }
    }

    pub fn page(&self) -> PageSetup {
        PageSetup { width: 612.0, height: 792.0, margin: self.margins.clone() }
    }
}

pub fn list_indents(depth: usize) -> (f64, f64) {
    let label = 0.75 * CM + depth.saturating_sub(1) as f64 * 1.25 * CM;
    (label, label + 0.5 * CM)
}

pub fn list_label(text: &str, run: &RunProps, depth: usize) -> (ListLabel, f64, f64) {
    let (label_at, text_at) = list_indents(depth);
    let mut props = run.clone();
    props.bold = None;
    props.italic = None;
    props.underline = None;
    props.strike = None;
    (ListLabel { text: text.into(), props, suffix: ListSuffix::Tab, tab_pos: Some(text_at) }, text_at, text_at - label_at)
}

pub fn paragraph(props: ParagraphProps, mark: RunProps, inlines: Vec<Inline>, list: Option<ListLabel>) -> Block {
    Block::Paragraph(Paragraph { props, mark, inlines, anchors: Vec::new(), list })
}

pub fn rule(profile: &Profile) -> Block {
    let mark = RunProps { size: Some(6.0), ..profile.body.clone() };
    let props = ParagraphProps {
        space_before: Some(0.0),
        space_after: Some(0.5 * CM),
        line_spacing: Some(LineSpacing::Multiple(1.0)),
        borders: Borders {
            bottom: BorderSide::Line { width: 0.5, color: Color(0x80, 0x80, 0x80), style: LineStyle::Solid },
            ..Borders::default()
        },
        ..ParagraphProps::default()
    };
    paragraph(props, mark, Vec::new(), None)
}

pub fn empty_body(profile: &Profile) -> Block {
    paragraph(profile.body_props(), profile.body.clone(), Vec::new(), None)
}

pub struct TableCell {
    pub blocks: Vec<Block>,
    pub align: Option<Align>,
}

pub fn table(profile: &Profile, rows: Vec<Vec<TableCell>>) -> Block {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let width = profile.text_width();
    let margin = profile.table_cell_margin;
    let widths = vec![width / columns as f64; columns];
    let line = BorderSide::Line { width: 0.5, color: Color(0, 0, 0), style: LineStyle::Solid };
    let borders = Borders { top: line, left: line, bottom: line, right: line, inside_h: line, inside_v: line };
    let model_rows = rows
        .into_iter()
        .map(|cells| Row {
            cells: cells
                .into_iter()
                .map(|cell| Cell {
                    blocks: if cell.blocks.is_empty() { vec![empty_cell(profile)] } else { cell.blocks },
                    span: 1,
                    borders,
                    halign: cell.align,
                    ..Cell::default()
                })
                .collect(),
            height: None,
            exact_height: false,
        })
        .collect();
    Block::Table(Table {
        columns: widths,
        rows: model_rows,
        borders,
        indent: 0.0,
        cell_margins: CellMargins { top: Some(margin - 0.085), left: Some(margin), bottom: Some(margin - 0.085), right: Some(margin) },
    })
}

fn empty_cell(profile: &Profile) -> Block {
    paragraph(cell_props(None), profile.body.clone(), Vec::new(), None)
}

pub fn cell_props(align: Option<Align>) -> ParagraphProps {
    ParagraphProps {
        align,
        space_before: Some(0.0),
        space_after: Some(0.0),
        line_spacing: Some(LineSpacing::Multiple(1.0)),
        ..ParagraphProps::default()
    }
}

pub fn document(profile: &Profile, blocks: Vec<Block>) -> Document {
    let blocks = if blocks.is_empty() { vec![empty_body(profile)] } else { blocks };
    Document {
        sections: vec![Section { page: profile.page(), blocks, columns: 1, content_scale: 1.0, ..Section::default() }],
        default_tab: 1.25 * CM,
        additive_spacing: true,
        tabs_relative_to_indent: true,
        writer_text_offset: true,
        ..Document::default()
    }
}
