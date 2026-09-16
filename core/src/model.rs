use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub fn parse_hex(value: &str) -> Option<Color> {
        let hex = value.trim().trim_start_matches('#');
        if hex.len() != 6 {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Color(channel(0)?, channel(2)?, channel(4)?))
    }

    pub fn hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Generic {
    #[default]
    Sans,
    Serif,
    Mono,
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    pub sections: Vec<Section>,
    pub default_tab: f64,
    pub generic_families: HashMap<String, Generic>,
    pub even_odd_headers: bool,
    pub additive_spacing: bool,
}

impl Document {
    pub fn first_paragraph(&self) -> Option<&Paragraph> {
        self.sections.iter().flat_map(|s| s.blocks.iter()).find_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Section {
    pub page: PageSetup,
    pub blocks: Vec<Block>,
    pub columns: usize,
    pub column_gap: f64,
    pub header_default: Option<Vec<Block>>,
    pub header_first: Option<Vec<Block>>,
    pub header_even: Option<Vec<Block>>,
    pub footer_default: Option<Vec<Block>>,
    pub footer_first: Option<Vec<Block>>,
    pub footer_even: Option<Vec<Block>>,
    pub title_page: bool,
    pub page_start: Option<i64>,
    pub page_format: PageNumberFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageNumberFormat {
    #[default]
    Decimal,
    LowerRoman,
    UpperRoman,
    LowerLetter,
    UpperLetter,
}

impl PageNumberFormat {
    pub fn typst_pattern(&self) -> &'static str {
        match self {
            PageNumberFormat::Decimal => "1",
            PageNumberFormat::LowerRoman => "i",
            PageNumberFormat::UpperRoman => "I",
            PageNumberFormat::LowerLetter => "a",
            PageNumberFormat::UpperLetter => "A",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Page,
    NumPages,
}

#[derive(Debug, Clone)]
pub struct PageSetup {
    pub width: f64,
    pub height: f64,
    pub margin: Margins,
}

#[derive(Debug, Clone)]
pub struct Margins {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
    pub header: f64,
    pub footer: f64,
}

impl Default for PageSetup {
    fn default() -> Self {
        PageSetup {
            width: 612.0,
            height: 792.0,
            margin: Margins {
                top: 72.0,
                right: 72.0,
                bottom: 72.0,
                left: 72.0,
                header: 36.0,
                footer: 36.0,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

#[derive(Debug, Clone)]
pub struct Paragraph {
    pub props: ParagraphProps,
    pub mark: RunProps,
    pub inlines: Vec<Inline>,
    pub anchors: Vec<Anchor>,
    pub list: Option<ListLabel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListSuffix {
    #[default]
    Tab,
    Space,
    Nothing,
}

#[derive(Debug, Clone)]
pub struct ListLabel {
    pub text: String,
    pub props: RunProps,
    pub suffix: ListSuffix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineSpacing {
    Multiple(f64),
    Exact(f64),
    AtLeast(f64),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParagraphProps {
    pub style_id: Option<String>,
    pub align: Option<Align>,
    pub indent_left: Option<f64>,
    pub indent_right: Option<f64>,
    pub indent_first_line: Option<f64>,
    pub indent_hanging: Option<f64>,
    pub space_before: Option<f64>,
    pub space_after: Option<f64>,
    pub line_spacing: Option<LineSpacing>,
    pub page_break_before: Option<bool>,
    pub keep_next: Option<bool>,
    pub contextual_spacing: Option<bool>,
    pub numbering: Option<(String, usize)>,
    pub tabs: Vec<TabStop>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabStop {
    pub pos: f64,
    pub clear: bool,
}

impl ParagraphProps {
    pub fn tab_stops(&self) -> Vec<f64> {
        let mut stops: Vec<f64> = Vec::new();
        for tab in &self.tabs {
            stops.retain(|s| (s - tab.pos).abs() > 0.01);
            if !tab.clear {
                stops.push(tab.pos);
            }
        }
        stops.sort_by(|a, b| a.partial_cmp(b).unwrap());
        stops
    }

    pub fn merge(&mut self, other: &ParagraphProps) {
        if other.style_id.is_some() {
            self.style_id = other.style_id.clone();
        }
        merge(&mut self.align, other.align);
        merge(&mut self.indent_left, other.indent_left);
        merge(&mut self.indent_right, other.indent_right);
        merge(&mut self.indent_first_line, other.indent_first_line);
        merge(&mut self.indent_hanging, other.indent_hanging);
        merge(&mut self.space_before, other.space_before);
        merge(&mut self.space_after, other.space_after);
        merge(&mut self.line_spacing, other.line_spacing);
        merge(&mut self.page_break_before, other.page_break_before);
        merge(&mut self.keep_next, other.keep_next);
        merge(&mut self.contextual_spacing, other.contextual_spacing);
        if other.numbering.is_some() {
            self.numbering = other.numbering.clone();
        }
        self.tabs.extend(other.tabs.iter().copied());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    Baseline,
    Superscript,
    Subscript,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunProps {
    pub font: Option<String>,
    pub size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<Color>,
    pub highlight: Option<Color>,
    pub vertical: Option<VerticalAlign>,
    pub caps: Option<bool>,
    pub small_caps: Option<bool>,
    pub hidden: Option<bool>,
}

impl RunProps {
    pub fn merge(&mut self, other: &RunProps) {
        if other.font.is_some() {
            self.font = other.font.clone();
        }
        merge(&mut self.size, other.size);
        merge(&mut self.bold, other.bold);
        merge(&mut self.italic, other.italic);
        merge(&mut self.underline, other.underline);
        merge(&mut self.strike, other.strike);
        merge(&mut self.color, other.color);
        merge(&mut self.highlight, other.highlight);
        merge(&mut self.vertical, other.vertical);
        merge(&mut self.caps, other.caps);
        merge(&mut self.small_caps, other.small_caps);
        merge(&mut self.hidden, other.hidden);
    }
}

fn merge<T: Copy>(target: &mut Option<T>, other: Option<T>) {
    if other.is_some() {
        *target = other;
    }
}

#[derive(Debug, Clone)]
pub enum Inline {
    Text { text: String, props: RunProps },
    Tab,
    LineBreak,
    PageBreak,
    Drawing(Drawing),
    Field { kind: FieldKind, props: RunProps },
    Footnote(Vec<Block>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalMerge {
    Restart,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BorderSide {
    #[default]
    Unset,
    None,
    Line { width: f64, color: Color },
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Borders {
    pub top: BorderSide,
    pub left: BorderSide,
    pub bottom: BorderSide,
    pub right: BorderSide,
    pub inside_h: BorderSide,
    pub inside_v: BorderSide,
}

impl Borders {
    pub fn merge(&mut self, other: &Borders) {
        for (target, source) in [
            (&mut self.top, other.top),
            (&mut self.left, other.left),
            (&mut self.bottom, other.bottom),
            (&mut self.right, other.right),
            (&mut self.inside_h, other.inside_h),
            (&mut self.inside_v, other.inside_v),
        ] {
            if source != BorderSide::Unset {
                *target = source;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CellMargins {
    pub top: Option<f64>,
    pub left: Option<f64>,
    pub bottom: Option<f64>,
    pub right: Option<f64>,
}

impl CellMargins {
    pub fn merge(&mut self, other: &CellMargins) {
        merge(&mut self.top, other.top);
        merge(&mut self.left, other.left);
        merge(&mut self.bottom, other.bottom);
        merge(&mut self.right, other.right);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

#[derive(Debug, Clone, Default)]
pub struct Table {
    pub columns: Vec<f64>,
    pub rows: Vec<Row>,
    pub borders: Borders,
    pub indent: f64,
    pub cell_margins: CellMargins,
}

#[derive(Debug, Clone, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub height: Option<f64>,
    pub exact_height: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub blocks: Vec<Block>,
    pub span: usize,
    pub width: Option<f64>,
    pub shading: Option<Color>,
    pub vertical_merge: Option<VerticalMerge>,
    pub borders: Borders,
    pub margins: CellMargins,
    pub valign: VAlign,
}

#[derive(Debug, Clone, Default)]
pub struct FixedDocument {
    pub pages: Vec<FixedPage>,
}

#[derive(Debug, Clone, Default)]
pub struct FixedPage {
    pub width: f64,
    pub height: f64,
    pub background: Option<RasterImage>,
    pub lines: Vec<TextLine>,
}

#[derive(Debug, Clone)]
pub struct RasterImage {
    pub png: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, Default)]
pub struct TextLine {
    pub x: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
    pub ascent: f64,
    pub runs: Vec<TextRun>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub font: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Svg,
}

impl ImageFormat {
    pub fn sniff(data: &[u8]) -> Option<ImageFormat> {
        if data.starts_with(&[0x89, b'P', b'N', b'G']) {
            Some(ImageFormat::Png)
        } else if data.starts_with(&[0xff, 0xd8]) {
            Some(ImageFormat::Jpeg)
        } else if data.starts_with(b"GIF8") {
            Some(ImageFormat::Gif)
        } else if data.iter().take(512).any(|&b| b == b'<')
            && std::str::from_utf8(&data[..data.len().min(2048)])
                .map(|s| s.contains("<svg"))
                .unwrap_or(false)
        {
            Some(ImageFormat::Svg)
        } else {
            None
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Gif => "gif",
            ImageFormat::Svg => "svg",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImageData {
    pub data: Vec<u8>,
    pub format: ImageFormat,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShapeKind {
    Rect,
    RoundRect(f64),
    Ellipse,
    Line,
    Polygon(Vec<(f64, f64)>),
}

#[derive(Debug, Clone)]
pub struct TextBox {
    pub blocks: Vec<Block>,
    pub fill: Option<Color>,
    pub stroke: Option<(f64, Color)>,
    pub inset: (f64, f64, f64, f64),
    pub auto_height: bool,
    pub valign: VAlign,
    pub shape: ShapeKind,
}

impl Default for TextBox {
    fn default() -> Self {
        TextBox {
            blocks: Vec::new(),
            fill: None,
            stroke: None,
            inset: (3.6, 7.2, 3.6, 7.2),
            auto_height: false,
            valign: VAlign::Top,
            shape: ShapeKind::Rect,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DrawingContent {
    Image(ImageData),
    TextBox(TextBox),
    Table(Table),
    Placeholder,
}

#[derive(Debug, Clone)]
pub struct Drawing {
    pub width: f64,
    pub height: f64,
    pub content: DrawingContent,
    pub rotation: f64,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Drawing {
    pub fn new(width: f64, height: f64, content: DrawingContent) -> Drawing {
        Drawing {
            width,
            height,
            content,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HRef {
    Page,
    Margin,
    Column,
    Character,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VRef {
    Page,
    Margin,
    Paragraph,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAnchorAlign {
    Top,
    Center,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HPosition {
    Offset(HRef, f64),
    Align(HRef, HAlign),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VPosition {
    Offset(VRef, f64),
    Align(VRef, VAnchorAlign),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    None,
    TopAndBottom,
    Square,
}

#[derive(Debug, Clone)]
pub struct Anchor {
    pub drawing: Drawing,
    pub horizontal: HPosition,
    pub vertical: VPosition,
    pub wrap: Wrap,
    pub behind: bool,
    pub dist_top: f64,
    pub dist_bottom: f64,
    pub dist_left: f64,
    pub dist_right: f64,
}
