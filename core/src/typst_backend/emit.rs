use super::fonts::FontSet;
use crate::model::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write;

const DEFAULT_FONT: &str = "Calibri";
const DEFAULT_SIZE: f64 = 11.0;
const DEFAULT_CELL_MARGIN_X: f64 = 5.4;

pub struct Emitted {
    pub source: String,
    pub files: Vec<(String, Vec<u8>)>,
}

pub fn emit(doc: &Document, fonts: &FontSet) -> Emitted {
    let mut emitter = Emitter {
        doc,
        fonts,
        out: String::new(),
        files: RefCell::new(Vec::new()),
        page: RefCell::new(PageSetup::default()),
    };
    emitter.preamble();
    for (index, section) in doc.sections.iter().enumerate() {
        emitter.section(index, section);
    }
    Emitted {
        source: emitter.out,
        files: emitter.files.into_inner(),
    }
}

struct Emitter<'a> {
    doc: &'a Document,
    fonts: &'a FontSet,
    out: String,
    files: RefCell<Vec<(String, Vec<u8>)>>,
    page: RefCell<PageSetup>,
}

struct CellOut {
    colspan: usize,
    rowspan: usize,
    args: Vec<String>,
    body: String,
}

impl Emitter<'_> {
    fn preamble(&mut self) {
        let default = self.doc.first_paragraph().map(|p| p.mark.clone()).unwrap_or_default();
        let family = self.family(&default);
        let size = default.size.unwrap_or(DEFAULT_SIZE);

        let _ = writeln!(
            self.out,
            "#set text(font: {}, size: {}, top-edge: \"ascender\", bottom-edge: \"descender\", hyphenate: false, fallback: true, lang: \"en\")",
            typst_str(&family),
            pt(size)
        );
        let _ = writeln!(
            self.out,
            "#set par(leading: 0pt, spacing: 0pt, justify: false, linebreaks: \"simple\")"
        );
        let _ = writeln!(self.out, "#set block(above: 0pt, below: 0pt)");
        let _ = writeln!(
            self.out,
            "#let minh(h, body) = layout(size => {{ let m = measure(width: size.width, body); block(width: 100%, height: calc.max(m.height, h), body) }})"
        );
    }

    fn section(&mut self, index: usize, section: &Section) {
        *self.page.borrow_mut() = section.page.clone();
        let page = &section.page;

        let header = self.margin_slot(section, true, index);
        let footer = self.margin_slot(section, false, index);
        let columns = if section.columns > 1 {
            format!(", columns: {}", section.columns)
        } else {
            String::new()
        };
        let _ = writeln!(
            self.out,
            "#set page(width: {}, height: {}, margin: (top: {}, bottom: {}, left: {}, right: {}){columns}, header: {header}, footer: {footer}, header-ascent: 0pt, footer-descent: 0pt)",
            pt(page.width),
            pt(page.height),
            pt(page.margin.top),
            pt(page.margin.bottom),
            pt(page.margin.left),
            pt(page.margin.right)
        );
        if section.columns > 1 {
            let _ = writeln!(self.out, "#set columns(gutter: {})", pt(section.column_gap));
        }
        if let Some(start) = section.page_start {
            let _ = writeln!(self.out, "#counter(page).update({start})");
        }
        let _ = writeln!(self.out, "#metadata(none) <section-{index}>");

        let blocks = self.blocks(&section.blocks, false);
        if section.content_scale > 0.0 && (section.content_scale - 1.0).abs() > 0.001 {
            let percent = trim_num(section.content_scale * 100.0);
            let _ = writeln!(self.out, "#scale(x: {percent}%, y: {percent}%, reflow: true)[{blocks}]");
        } else {
            self.out.push_str(&blocks);
        }
    }

    fn margin_slot(&self, section: &Section, header: bool, index: usize) -> String {
        let (default, first, even) = if header {
            (&section.header_default, &section.header_first, &section.header_even)
        } else {
            (&section.footer_default, &section.footer_first, &section.footer_even)
        };
        let empty: Vec<Block> = Vec::new();
        let first = if section.title_page { Some(first.as_ref().unwrap_or(&empty)) } else { None };
        let even = if self.doc.even_odd_headers { Some(even.as_ref().unwrap_or(&empty)) } else { None };
        if default.is_none() && first.is_none() && even.is_none() {
            return "none".to_string();
        }

        let page = &section.page;
        let render = |blocks: Option<&Vec<Block>>| -> String {
            let inner = blocks.map(|b| self.blocks(b, true)).unwrap_or_default();
            if header {
                format!(
                    "block(width: 100%, height: {}, inset: (top: {}), align(top + left)[{inner}])",
                    pt(page.margin.top),
                    pt(page.margin.header.min(page.margin.top))
                )
            } else {
                format!(
                    "block(width: 100%, height: {}, inset: (bottom: {}), align(bottom + left)[{inner}])",
                    pt(page.margin.bottom),
                    pt(page.margin.footer.min(page.margin.bottom))
                )
            }
        };

        let default_expr = render(default.as_ref());
        let first_expr = first.map(|f| render(Some(f)));
        let even_expr = even.map(|e| render(Some(e)));

        let mut body = String::new();
        let _ = write!(body, "context {{ let n = here().page(); let first = locate(<section-{index}>).page(); ");
        if let Some(first_expr) = first_expr {
            let _ = write!(body, "if n == first {{ {first_expr} }} else ");
        }
        if let Some(even_expr) = even_expr {
            let _ = write!(body, "if calc.even(n) {{ {even_expr} }} else ");
        }
        let _ = write!(body, "{{ {default_expr} }} }}");
        body
    }

    fn blocks(&self, blocks: &[Block], trailing: bool) -> String {
        let mut out = String::new();
        let mut prev_after = 0.0;
        let mut prev_style: Option<&str> = None;
        let mut prev_paragraph = false;
        let mut prev_contextual = false;

        let border_key = |p: &Paragraph| -> Option<(Borders, Option<Color>, i64, i64)> {
            let b = &p.props.borders;
            let has = [b.top, b.left, b.bottom, b.right].iter().any(|s| matches!(s, BorderSide::Line { .. }));
            if has || p.props.shading.is_some() {
                Some((
                    *b,
                    p.props.shading,
                    (p.props.indent_left.unwrap_or(0.0) * 100.0) as i64,
                    (p.props.indent_right.unwrap_or(0.0) * 100.0) as i64,
                ))
            } else {
                None
            }
        };
        let keys: Vec<Option<(Borders, Option<Color>, i64, i64)>> = blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(p) => border_key(p),
                _ => None,
            })
            .collect();

        for (index, block) in blocks.iter().enumerate() {
            match block {
                Block::Paragraph(p) => {
                    let key = &keys[index];
                    let first_in_group = key.is_none() || index == 0 || keys[index - 1] != *key;
                    let last_in_group = key.is_none() || index + 1 == keys.len() || keys[index + 1] != *key;
                    let before = p.props.space_before.unwrap_or(0.0);
                    let same_style = prev_paragraph && p.props.style_id.as_deref() == prev_style;
                    let contextual = p.props.contextual_spacing.unwrap_or(false);
                    let lower = if same_style && prev_contextual { 0.0 } else { prev_after };
                    let upper = if same_style && contextual { 0.0 } else { before };
                    let gap = if self.doc.additive_spacing {
                        lower + upper
                    } else {
                        lower.max(upper)
                    };
                    self.paragraph(p, gap, first_in_group, last_in_group, &mut out);
                    prev_after = p.props.space_after.unwrap_or(0.0);
                    prev_style = p.props.style_id.as_deref();
                    prev_paragraph = true;
                    prev_contextual = contextual;
                }
                Block::Table(t) => {
                    if prev_after > 0.01 {
                        let _ = writeln!(out, "#v({})", pt(prev_after));
                    }
                    self.table(t, &mut out);
                    prev_after = 0.0;
                    prev_style = None;
                    prev_paragraph = false;
                    prev_contextual = false;
                }
            }
        }
        if trailing && prev_after > 0.01 {
            let _ = writeln!(out, "#v({})", pt(prev_after));
        }
        out
    }

    fn paragraph(&self, p: &Paragraph, gap: f64, first_in_group: bool, last_in_group: bool, out: &mut String) {
        if p.props.page_break_before == Some(true) {
            out.push_str("#pagebreak()\n");
        }
        for anchor in &p.anchors {
            self.anchor(anchor, out);
        }

        let mut parts: Vec<Vec<&Inline>> = vec![Vec::new()];
        for inline in &p.inlines {
            if matches!(inline, Inline::PageBreak) {
                parts.push(Vec::new());
            } else {
                parts.last_mut().unwrap().push(inline);
            }
        }

        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                out.push_str("#pagebreak()\n");
            }
            self.paragraph_part(p, part, if index == 0 { gap } else { 0.0 }, first_in_group, last_in_group, out);
        }
    }

    fn paragraph_part(&self, p: &Paragraph, inlines: &[&Inline], gap: f64, first_in_group: bool, last_in_group: bool, out: &mut String) {
        if gap > 0.01 {
            let _ = writeln!(out, "#v({})", pt(gap));
        }

        let spacing = p.props.line_spacing.unwrap_or(LineSpacing::Multiple(1.0));
        let lead_props = inlines
            .iter()
            .find_map(|i| match i {
                Inline::Text { props, .. } => Some(props),
                _ => None,
            })
            .unwrap_or(&p.mark);
        let leading = 0.0;
        let _ = lead_props;

        let left = p.props.indent_left.unwrap_or(0.0);
        let right = p.props.indent_right.unwrap_or(0.0);
        let first = p.props.indent_first_line.unwrap_or(0.0);
        let hanging = p.props.indent_hanging.unwrap_or(0.0);
        let align = p.props.align.unwrap_or_default();
        let stops = p.props.tab_stops();
        let mut x = Some(left - hanging + first);

        let mut body = String::new();
        let mut any_text = false;
        if let Some(label) = &p.list {
            let label_expr = self.text_expr(&label.text, &label.props, spacing);
            match label.suffix {
                ListSuffix::Tab if hanging > 0.01 => {
                    let _ = write!(body, "#box(width: {}, {})", pt(hanging), label_expr.trim_start_matches('#'));
                    x = Some(left);
                }
                ListSuffix::Tab => {
                    let _ = write!(body, "{label_expr}#h({})", pt(self.doc.default_tab));
                    x = None;
                }
                ListSuffix::Space => {
                    body.push_str(&label_expr);
                    body.push_str(&self.text_expr(" ", &label.props, spacing));
                    x = None;
                }
                ListSuffix::Nothing => {
                    body.push_str(&label_expr);
                    x = None;
                }
            }
            any_text = true;
        }
        for inline in inlines {
            match inline {
                Inline::Text { text, props } => {
                    if text.is_empty() {
                        continue;
                    }
                    any_text = true;
                    body.push_str(&self.text_expr(text, props, spacing));
                    x = None;
                }
                Inline::Tab => {
                    let advance = match x {
                        Some(current) => {
                            let next = next_tab_stop(current, &stops, left, hanging, self.doc.default_tab);
                            x = Some(next);
                            next - current
                        }
                        None => self.doc.default_tab,
                    };
                    if advance > 0.01 {
                        let _ = write!(body, "#h({})", pt(advance));
                    }
                }
                Inline::LineBreak => {
                    body.push_str("#linebreak()");
                    x = Some(left);
                }
                Inline::PageBreak => {}
                Inline::Drawing(drawing) => {
                    any_text = true;
                    let _ = write!(body, "#box({})", self.drawing_expr(drawing));
                    x = None;
                }
                Inline::Footnote(blocks) => {
                    let _ = write!(body, "#footnote[{}]", self.blocks(blocks, false));
                    x = None;
                }
                Inline::Field { kind, props } => {
                    any_text = true;
                    let value = match kind {
                        FieldKind::Page => format!("context counter(page).display({})", typst_str(self.page_pattern())),
                        FieldKind::NumPages => "context counter(page).final().first()".to_string(),
                    };
                    body.push_str(&self.text_expr_content(&value, props, spacing));
                    x = None;
                }
            }
        }
        let _ = x;
        if !any_text {
            body.insert_str(0, &self.text_expr("\u{a0}", &p.mark, spacing));
        }

        let mut expr = format!(
            "par(leading: {}, justify: {}, first-line-indent: (amount: {}, all: true), hanging-indent: {})[{}]",
            pt(leading),
            align == Align::Justify,
            pt(first),
            pt(hanging),
            body
        );

        let (wrap_left, wrap_right) = self.wrap_padding(p);
        let pad_left = left - hanging + wrap_left;
        let right = right + wrap_right;
        let b = &p.props.borders;
        let has_border = [b.top, b.left, b.bottom, b.right].iter().any(|s| matches!(s, BorderSide::Line { .. }));
        if has_border || p.props.shading.is_some() {
            let space = p.props.border_space;
            let fill = p
                .props
                .shading
                .map(|c| format!("rgb({})", typst_str(&c.hex())))
                .unwrap_or_else(|| "none".into());
            let top = if first_in_group { b.top } else { BorderSide::None };
            let bottom = if last_in_group { b.bottom } else { BorderSide::None };
            let inset_for = |side: BorderSide| match side {
                BorderSide::Line { width, .. } => space + width,
                _ => 0.0,
            };
            expr = format!(
                "block(width: 100%, fill: {fill}, stroke: (top: {}, bottom: {}, left: {}, right: {}), inset: (top: {}, bottom: {}, left: {}, right: {}), {expr})",
                stroke(top),
                stroke(bottom),
                stroke(b.left),
                stroke(b.right),
                pt(inset_for(top)),
                pt(inset_for(bottom)),
                pt(inset_for(b.left)),
                pt(inset_for(b.right))
            );
        }
        if pad_left.abs() > 0.01 || right.abs() > 0.01 {
            expr = format!("pad(left: {}, right: {}, {})", pt(pad_left), pt(right), expr);
        }
        match align {
            Align::Center => expr = format!("align(center, {expr})"),
            Align::Right => expr = format!("align(right, {expr})"),
            _ => {}
        }

        let _ = writeln!(out, "#{expr}");
    }

    fn edges(&self, family: &str, props: &RunProps, size: f64, spacing: LineSpacing) -> (f64, f64) {
        let m = self
            .fonts
            .metrics(family, props.bold == Some(true), props.italic == Some(true));
        let line = m.line_height();
        let below = m.descender + m.line_gap;
        let above = match spacing {
            LineSpacing::Multiple(mult) => m.ascender + (mult - 1.0) * line,
            LineSpacing::Exact(v) => v / size - below,
            LineSpacing::AtLeast(v) => m.ascender + ((v / size) - line).max(0.0),
        };
        (above.max(0.0) * size, below * size)
    }

    fn text_expr(&self, text: &str, props: &RunProps, spacing: LineSpacing) -> String {
        let content = if props.caps == Some(true) {
            text.to_uppercase()
        } else {
            text.to_owned()
        };
        self.text_expr_content(&typst_str(&content), props, spacing)
    }

    fn text_expr_content(&self, content: &str, props: &RunProps, spacing: LineSpacing) -> String {
        let family = self.family(props);
        let size = props.size.unwrap_or(DEFAULT_SIZE);
        let (top, bottom) = self.edges(&family, props, size, spacing);

        let mut args = vec![
            format!("font: {}", typst_str(&family)),
            format!("size: {}", pt(size)),
            format!("top-edge: {}", pt(top)),
            format!("bottom-edge: -{}", pt(bottom)),
        ];
        if props.bold == Some(true) {
            args.push("weight: \"bold\"".into());
        }
        if props.italic == Some(true) {
            args.push("style: \"italic\"".into());
        }
        if let Some(color) = props.color {
            args.push(format!("fill: rgb({})", typst_str(&color.hex())));
        }

        let mut expr = format!("text({}, {})", args.join(", "), content);
        if props.small_caps == Some(true) {
            expr = format!("smallcaps({expr})");
        }
        if props.underline == Some(true) {
            expr = format!("underline({expr})");
        }
        if props.strike == Some(true) {
            expr = format!("strike({expr})");
        }
        if let Some(fill) = props.highlight {
            expr = format!("highlight(fill: rgb({}), {expr})", typst_str(&fill.hex()));
        }
        match props.vertical {
            Some(VerticalAlign::Superscript) => expr = format!("super({expr})"),
            Some(VerticalAlign::Subscript) => expr = format!("sub({expr})"),
            _ => {}
        }
        format!("#{expr}")
    }

    fn family(&self, props: &RunProps) -> String {
        let requested = props.font.as_deref().unwrap_or(DEFAULT_FONT);
        let generic = self
            .doc
            .generic_families
            .get(&requested.to_lowercase())
            .copied();
        self.fonts.resolve(requested, generic)
    }

    fn page_pattern(&self) -> &'static str {
        let page = self.page.borrow();
        let _ = &*page;
        self.doc
            .sections
            .iter()
            .find(|s| s.page.width == page.width && s.page.height == page.height)
            .map(|s| s.page_format.typst_pattern())
            .unwrap_or("1")
    }

    fn wrap_padding(&self, p: &Paragraph) -> (f64, f64) {
        let page = self.page.borrow();
        let text_width = page.width - page.margin.left - page.margin.right;
        let mut left = 0.0_f64;
        let mut right = 0.0_f64;
        for anchor in &p.anchors {
            if anchor.wrap != Wrap::Square || anchor.behind {
                continue;
            }
            if !matches!(anchor.vertical, VPosition::Offset(VRef::Paragraph | VRef::Line, _) | VPosition::Align(VRef::Paragraph | VRef::Line, _)) {
                continue;
            }
            let x = match anchor.horizontal {
                HPosition::Offset(HRef::Page, x) => x - page.margin.left,
                HPosition::Offset(_, x) => x,
                HPosition::Align(_, HAlign::Left) => 0.0,
                HPosition::Align(_, HAlign::Center) => (text_width - anchor.drawing.width) / 2.0,
                HPosition::Align(_, HAlign::Right) => text_width - anchor.drawing.width,
            };
            let w = anchor.drawing.width;
            if w > text_width * 0.5 {
                continue;
            }
            if x + w / 2.0 < text_width / 2.0 {
                left = left.max(x + w + anchor.dist_right);
            } else {
                right = right.max(text_width - x + anchor.dist_left);
            }
        }
        (left, right)
    }

    fn side_wrapped(&self, anchor: &Anchor) -> bool {
        let page = self.page.borrow();
        let text_width = page.width - page.margin.left - page.margin.right;
        anchor.wrap == Wrap::Square && !anchor.behind && anchor.drawing.width <= text_width * 0.5
    }

    fn register(&self, image: &ImageData) -> String {
        let mut files = self.files.borrow_mut();
        let path = format!("media/img{}.{}", files.len() + 1, image.format.extension());
        files.push((path.clone(), image.data.clone()));
        format!("/{path}")
    }

    fn drawing_expr(&self, drawing: &Drawing) -> String {
        let w = pt(drawing.width.max(0.1));
        let h = pt(drawing.height.max(0.1));
        let mut expr = match &drawing.content {
            DrawingContent::Image(image) => {
                let path = self.register(image);
                format!("image({}, width: {w}, height: {h}, fit: \"stretch\")", typst_str(&path))
            }
            DrawingContent::TextBox(tb) => {
                let fill = tb
                    .fill
                    .map(|c| format!("rgb({})", typst_str(&c.hex())))
                    .unwrap_or_else(|| "none".into());
                let stroke = tb
                    .stroke
                    .map(|(width, c)| format!("{} + rgb({})", pt(width), typst_str(&c.hex())))
                    .unwrap_or_else(|| "none".into());
                if tb.shape == ShapeKind::Line {
                    let (x0, y0, x1, y1) = if drawing.flip_v != drawing.flip_h {
                        ("0pt", h.as_str(), w.as_str(), "0pt")
                    } else {
                        ("0pt", "0pt", w.as_str(), h.as_str())
                    };
                    format!("box(width: {w}, height: {h}, place(top + left, line(start: ({x0}, {y0}), end: ({x1}, {y1}), stroke: {stroke})))")
                } else {
                    let height = if tb.auto_height { "auto".to_string() } else { h.clone() };
                    let valign = match tb.valign {
                        VAlign::Top => "top",
                        VAlign::Center => "horizon",
                        VAlign::Bottom => "bottom",
                    };
                    let body = format!("align({valign} + left)[{}]", self.blocks(&tb.blocks, false));
                    let inset = format!(
                        "inset: (top: {}, left: {}, bottom: {}, right: {})",
                        pt(tb.inset.0),
                        pt(tb.inset.1),
                        pt(tb.inset.2),
                        pt(tb.inset.3)
                    );
                    match &tb.shape {
                        ShapeKind::Ellipse => format!(
                            "ellipse(width: {w}, height: {height}, {inset}, fill: {fill}, stroke: {stroke}, {body})"
                        ),
                        ShapeKind::RoundRect(radius) => format!(
                            "block(width: {w}, height: {height}, {inset}, fill: {fill}, stroke: {stroke}, radius: {}, {body})",
                            pt(*radius)
                        ),
                        ShapeKind::Polygon(points) => {
                            let coords: Vec<String> = points
                                .iter()
                                .map(|(x, y)| {
                                    let px = if drawing.flip_h { 1.0 - x } else { *x };
                                    let py = if drawing.flip_v { 1.0 - y } else { *y };
                                    format!("({}, {})", pt(px * drawing.width), pt(py * drawing.height))
                                })
                                .collect();
                            format!(
                                "box(width: {w}, height: {h}, place(top + left, polygon(fill: {fill}, stroke: {stroke}, {})) + place(top + left, block(width: {w}, height: {height}, {inset}, {body})))",
                                coords.join(", ")
                            )
                        }
                        ShapeKind::Path(commands) => {
                            let map = |x: f64, y: f64| {
                                let px = if drawing.flip_h { 1.0 - x } else { x };
                                let py = if drawing.flip_v { 1.0 - y } else { y };
                                format!("({}, {})", pt(px * drawing.width), pt(py * drawing.height))
                            };
                            let parts: Vec<String> = commands
                                .iter()
                                .map(|c| match c {
                                    PathCommand::Move(x, y) => format!("curve.move({})", map(*x, *y)),
                                    PathCommand::Line(x, y) => format!("curve.line({})", map(*x, *y)),
                                    PathCommand::Quad(cx, cy, x, y) => format!("curve.quad({}, {})", map(*cx, *cy), map(*x, *y)),
                                    PathCommand::Cubic(x1, y1, x2, y2, x, y) => {
                                        format!("curve.cubic({}, {}, {})", map(*x1, *y1), map(*x2, *y2), map(*x, *y))
                                    }
                                    PathCommand::Close => "curve.close(mode: \"straight\")".to_string(),
                                })
                                .collect();
                            format!(
                                "box(width: {w}, height: {h}, place(top + left, curve(fill: {fill}, stroke: {stroke}, {})) + place(top + left, block(width: {w}, height: {height}, {inset}, {body})))",
                                parts.join(", ")
                            )
                        }
                        _ => format!(
                            "block(width: {w}, height: {height}, {inset}, fill: {fill}, stroke: {stroke}, {body})"
                        ),
                    }
                }
            }
            DrawingContent::Table(table) => {
                format!("block(width: {w}, {})", self.table_expr(table))
            }
            DrawingContent::Placeholder => format!("box(width: {w}, height: {h})"),
        };
        if drawing.rotation.abs() > 0.01 {
            expr = format!("rotate({}deg, reflow: false, {expr})", trim_num(drawing.rotation));
        }
        expr
    }

    fn anchor(&self, anchor: &Anchor, out: &mut String) {
        let expr = self.drawing_expr(&anchor.drawing);
        let page = self.page.borrow().clone();
        let margin = &page.margin;
        let (h_align, dx) = match anchor.horizontal {
            HPosition::Offset(HRef::Page, x) => ("left", x - margin.left),
            HPosition::Offset(_, x) => ("left", x),
            HPosition::Align(_, HAlign::Left) => ("left", 0.0),
            HPosition::Align(_, HAlign::Center) => ("center", 0.0),
            HPosition::Align(_, HAlign::Right) => ("right", 0.0),
        };
        match anchor.vertical {
            VPosition::Offset(VRef::Page, y) => {
                let _ = writeln!(out, "#place(top + {h_align}, dx: {}, dy: {}, {expr})", pt(dx), pt(y - margin.top));
            }
            VPosition::Offset(VRef::Margin, y) => {
                let _ = writeln!(out, "#place(top + {h_align}, dx: {}, dy: {}, {expr})", pt(dx), pt(y));
            }
            VPosition::Align(VRef::Page | VRef::Margin, align) => {
                let v = match align {
                    VAnchorAlign::Top => "top",
                    VAnchorAlign::Center => "horizon",
                    VAnchorAlign::Bottom => "bottom",
                };
                let _ = writeln!(out, "#place({v} + {h_align}, dx: {}, {expr})", pt(dx));
            }
            VPosition::Offset(_, _) | VPosition::Align(_, _) => {
                let y = match anchor.vertical {
                    VPosition::Offset(_, y) => y,
                    _ => 0.0,
                };
                let _ = writeln!(
                    out,
                    "#block(width: 100%, height: 0pt, place(top + {h_align}, dx: {}, dy: {}, {expr}))",
                    pt(dx),
                    pt(y)
                );
                if anchor.wrap != Wrap::None && !self.side_wrapped(anchor) {
                    let reserve = y + anchor.drawing.height + anchor.dist_bottom;
                    if reserve > 0.01 {
                        let _ = writeln!(out, "#v({})", pt(reserve));
                    }
                }
            }
        }
    }

    fn table(&self, table: &Table, out: &mut String) {
        if table.rows.is_empty() {
            return;
        }
        let _ = writeln!(out, "#{}", self.table_expr(table));
    }

    fn table_expr(&self, table: &Table) -> String {
        if table.rows.is_empty() {
            return "box()".to_string();
        }
        let grid = table
            .rows
            .iter()
            .map(|r| r.cells.iter().map(|c| c.span).sum::<usize>())
            .max()
            .unwrap_or(0)
            .max(1);
        let fixed_columns = table.columns.len() >= grid;

        let mut defaults = CellMargins {
            top: Some(0.0),
            left: Some(DEFAULT_CELL_MARGIN_X),
            bottom: Some(0.0),
            right: Some(DEFAULT_CELL_MARGIN_X),
        };
        defaults.merge(&table.cell_margins);

        let mut cells: Vec<CellOut> = Vec::new();
        let mut open: HashMap<usize, usize> = HashMap::new();
        let last_row = table.rows.len() - 1;

        for (ri, row) in table.rows.iter().enumerate() {
            let mut column = 0;
            for cell in &row.cells {
                match cell.vertical_merge {
                    Some(VerticalMerge::Continue) => {
                        if let Some(&anchor) = open.get(&column) {
                            cells[anchor].rowspan += 1;
                        }
                        column += cell.span;
                        continue;
                    }
                    Some(VerticalMerge::Restart) => {
                        open.insert(column, cells.len());
                    }
                    None => {
                        open.remove(&column);
                    }
                }

                let mut margins = defaults;
                margins.merge(&cell.margins);
                let top = margins.top.unwrap_or(0.0);
                let bottom = margins.bottom.unwrap_or(0.0);

                let mut body = self.blocks(&cell.blocks, true);
                if cell.no_wrap && !body.trim().is_empty() {
                    body = match (cell.halign, cell.overflow_width) {
                        (Some(Align::Right) | Some(Align::Center), _) => format!("#box[{body}]"),
                        (_, Some(width)) => format!(
                            "#block(width: {}, clip: true, breakable: false, block(width: 20000pt, breakable: false)[{body}])",
                            pt(width)
                        ),
                        _ => format!("#block(width: 20000pt, breakable: false)[{body}]"),
                    };
                }
                if let (Some(height), false, true) = (row.height, row.exact_height, fixed_columns) {
                    body = format!("#minh({}, [{}])", pt(height), body);
                }

                let pick = |own: BorderSide, edge: bool, outer: BorderSide, inside: BorderSide| {
                    if own != BorderSide::Unset {
                        own
                    } else if edge {
                        outer
                    } else {
                        inside
                    }
                };
                let borders = [
                    ("top", pick(cell.borders.top, ri == 0, table.borders.top, table.borders.inside_h)),
                    ("bottom", pick(cell.borders.bottom, ri == last_row, table.borders.bottom, table.borders.inside_h)),
                    ("left", pick(cell.borders.left, column == 0, table.borders.left, table.borders.inside_v)),
                    ("right", pick(cell.borders.right, column + cell.span >= grid, table.borders.right, table.borders.inside_v)),
                ];

                let border_top = match borders[0].1 {
                    BorderSide::Line { width, .. } => width,
                    _ => 0.0,
                };

                let mut args = Vec::new();
                if let Some(fill) = cell.shading {
                    args.push(format!("fill: rgb({})", typst_str(&fill.hex())));
                }
                args.push(format!(
                    "stroke: (top: {}, bottom: {}, left: {}, right: {})",
                    stroke(borders[0].1),
                    stroke(borders[1].1),
                    stroke(borders[2].1),
                    stroke(borders[3].1)
                ));
                args.push(format!(
                    "inset: (top: {}, bottom: {}, left: {}, right: {})",
                    pt(top + border_top),
                    pt(bottom),
                    pt(margins.left.unwrap_or(0.0)),
                    pt(margins.right.unwrap_or(0.0))
                ));
                let h = match cell.halign {
                    Some(Align::Center) => "center",
                    Some(Align::Right) => "right",
                    _ => "left",
                };
                match cell.valign {
                    VAlign::Center => args.push(format!("align: {h} + horizon")),
                    VAlign::Bottom => args.push(format!("align: {h} + bottom")),
                    VAlign::Top if h != "left" => args.push(format!("align: {h} + top")),
                    VAlign::Top => {}
                }

                cells.push(CellOut {
                    colspan: cell.span,
                    rowspan: 1,
                    args,
                    body,
                });
                column += cell.span;
            }
        }

        let columns: Vec<String> = if fixed_columns {
            table.columns.iter().take(grid).map(|w| pt(*w)).collect()
        } else {
            (0..grid).map(|_| "auto".to_string()).collect()
        };

        let mut rows_arg = String::new();
        if table.rows.iter().any(|r| r.height.is_some() && r.exact_height) {
            let rows: Vec<String> = table
                .rows
                .iter()
                .map(|r| match (r.height, r.exact_height) {
                    (Some(h), true) => pt(h),
                    _ => "auto".to_string(),
                })
                .collect();
            rows_arg = format!("rows: ({}), ", tuple(&rows));
        }

        let rendered: Vec<String> = cells
            .iter()
            .map(|c| {
                let mut args = Vec::new();
                if c.colspan > 1 {
                    args.push(format!("colspan: {}", c.colspan));
                }
                if c.rowspan > 1 {
                    args.push(format!("rowspan: {}", c.rowspan));
                }
                args.extend(c.args.iter().cloned());
                format!("table.cell({})[{}]", args.join(", "), c.body)
            })
            .collect();

        let mut expr = format!(
            "table(columns: ({}), {}stroke: none, inset: 0pt, align: left + top,\n{}\n)",
            tuple(&columns),
            rows_arg,
            rendered.join(",\n")
        );
        if table.indent.abs() > 0.01 {
            expr = format!("pad(left: {}, {})", pt(table.indent), expr);
        }
        expr
    }
}

fn trim_num(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    format!("{rounded}")
}

fn next_tab_stop(x: f64, stops: &[f64], left: f64, hanging: f64, default_tab: f64) -> f64 {
    let mut candidates: Vec<f64> = stops.iter().copied().filter(|s| *s > x + 0.01).collect();
    if hanging > 0.01 && left > x + 0.01 {
        candidates.push(left);
    }
    if let Some(min) = candidates.iter().copied().fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |a| a.min(v)))) {
        return min;
    }
    if default_tab <= 0.01 {
        return x + 36.0;
    }
    ((x / default_tab).floor() + 1.0) * default_tab
}

fn stroke(side: BorderSide) -> String {
    match side {
        BorderSide::Line { width, color } => format!("{} + rgb({})", pt(width), typst_str(&color.hex())),
        _ => "none".to_string(),
    }
}

fn tuple(items: &[String]) -> String {
    if items.len() == 1 {
        format!("{},", items[0])
    } else {
        items.join(", ")
    }
}

pub fn pt(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded == 0.0 {
        return "0pt".to_string();
    }
    format!("{rounded}pt")
}

pub fn typst_str(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
