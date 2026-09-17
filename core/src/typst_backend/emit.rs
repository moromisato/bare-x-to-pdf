use super::fonts::FontSet;
use crate::model::*;
use std::cell::{Cell, RefCell};
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
        para_line: Cell::new(None),
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
    para_line: Cell<Option<f64>>,
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
            "#set text(font: {}, size: {}, top-edge: \"ascender\", bottom-edge: \"descender\", hyphenate: false, fallback: true, kerning: false, lang: \"en\")",
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
            "#set footnote.entry(separator: line(length: {}, stroke: 0.5pt), clearance: 2.83pt, gap: 5.1pt, indent: 0pt)",
            self.doc.footnote_separator_width.map(pt).unwrap_or_else(|| "25%".to_string())
        );
        let _ = writeln!(
            self.out,
            "#let minh(h, body) = layout(size => {{ let m = measure(width: size.width, body); block(width: 100%, height: calc.max(m.height, h), body) }})\n#let minhw(w, h, body) = context {{ let m = measure(width: w, body); block(width: 100%, height: calc.max(m.height, h), body) }}\n#let lbl(label, x0, lft, stops, tab, rel) = context {{ let end = x0 + measure(label).width.pt(); let target = {{ let c = (stops + (lft,)).filter(s => s > end + 0.01); if c.len() > 0 {{ calc.min(..c) }} else if rel and end >= lft {{ lft + (calc.floor((end - lft) / tab) + 1) * tab }} else {{ (calc.floor(end / tab) + 1) * tab }} }}; box(width: (target - x0) * 1pt, align(left, label)) }}"
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
        for anchor in &section.anchors {
            let mut placed = String::new();
            self.anchor(anchor, &mut placed);
            self.out.push_str(&placed);
        }

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
                Block::Columns(c) => {
                    if prev_after > 0.01 {
                        let _ = writeln!(out, "#v({})", pt(prev_after));
                    }
                    self.columns_block(c, &mut out);
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

    fn columns_block(&self, c: &ColumnsBlock, out: &mut String) {
        let body = self.blocks(&c.blocks, false);
        let n = c.count.max(1);
        let gap = pt(c.gap);
        if !c.balanced {
            let _ = writeln!(out, "#columns({n}, gutter: {gap})[{body}]");
            return;
        }
        let size = c
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Paragraph(p) => p.inlines.iter().find_map(|i| match i {
                    Inline::Text { props, .. } => Some(props.size.unwrap_or(DEFAULT_SIZE)),
                    _ => None,
                }),
                _ => None,
            })
            .unwrap_or(DEFAULT_SIZE);
        let mut separators = String::new();
        if let Some(sep) = c.separator {
            let offset = match sep.valign {
                VAlign::Top => "0".to_string(),
                VAlign::Center => format!("h * {} / 2", trim_num(1.0 - sep.height)),
                VAlign::Bottom => format!("h * {}", trim_num(1.0 - sep.height)),
            };
            let dash = if sep.dotted { ", dash: \"dotted\"" } else { "" };
            for i in 1..n {
                let _ = write!(
                    separators,
                    "place(top + left, dx: colw * {i} + {gap} * {} + {gap} / 2 - {w} / 2, dy: {offset}, line(angle: 90deg, length: h * {}, stroke: (paint: rgb({}), thickness: {w}{dash}))); ",
                    i - 1,
                    trim_num(sep.height),
                    typst_str(&sep.color.hex()),
                    w = pt(sep.width)
                );
            }
        }
        let _ = writeln!(
            out,
            "#layout(size => {{ let body = [{body}]; let colw = (size.width - {gap} * {}) / {n}; let h = measure(width: colw, body).height / {n} + {}; block(width: 100%, height: h, {{ {separators}columns({n}, gutter: {gap}, body) }}) }})",
            n - 1,
            pt(size * 0.6)
        );
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
        let outer_line = self.para_line.replace(self.paragraph_line_height(p, inlines));
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
        let (wrap_left, wrap_right) = self.wrap_padding(p);
        let wrap_limit = self.wrap_limit(p);
        let split_words = (wrap_left > 0.01 || wrap_right > 0.01) && wrap_limit > 0.0;

        let mut items: Vec<String> = Vec::new();
        let mut any_text = false;
        if let Some(label) = &p.list {
            let label_expr = self.text_expr(&label.text, &label.props, spacing);
            match label.suffix {
                ListSuffix::Tab if hanging > 0.01 => {
                    let mut label_stops = stops.clone();
                    label_stops.extend(label.tab_pos);
                    let tab = if self.doc.default_tab > 0.01 { self.doc.default_tab } else { 36.0 };
                    items.push(format!(
                        "#lbl({}, {}, {}, {}, {}, {})",
                        label_expr.trim_start_matches('#'),
                        trim_num(left - hanging + first),
                        trim_num(left),
                        array(&label_stops),
                        trim_num(tab),
                        self.doc.tabs_relative_to_indent
                    ));
                    x = Some(left);
                }
                ListSuffix::Tab => {
                    items.push(format!("{label_expr}#h({})", pt(self.doc.default_tab)));
                    x = None;
                }
                ListSuffix::Space => {
                    items.push(label_expr.to_string());
                    items.push(self.text_expr(" ", &label.props, spacing).to_string());
                    x = None;
                }
                ListSuffix::Nothing => {
                    items.push(label_expr.to_string());
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
                    if split_words {
                        for token in split_tokens(text) {
                            items.push(self.text_expr(&token, props, spacing));
                        }
                    } else {
                        items.push(self.text_expr(text, props, spacing));
                    }
                    x = None;
                }
                Inline::Tab => {
                    let advance = match x {
                        Some(current) => {
                            let next = next_tab_stop(current, &stops, left, hanging, self.doc.default_tab, self.doc.tabs_relative_to_indent);
                            x = Some(next);
                            next - current
                        }
                        None => self.doc.default_tab,
                    };
                    if advance > 0.01 {
                        items.push(format!("#h({})", pt(advance)));
                    }
                }
                Inline::LineBreak => {
                    items.push("#linebreak()".to_string());
                    x = Some(left);
                }
                Inline::PageBreak => {}
                Inline::Drawing(drawing) => {
                    any_text = true;
                    items.push(format!("#box({})", self.drawing_expr(drawing)));
                    x = None;
                }
                Inline::Footnote(blocks) => {
                    items.push(format!("#footnote[{}]", self.footnote_body(blocks)));
                    x = None;
                }
                Inline::Field { kind, props } => {
                    any_text = true;
                    let value = match kind {
                        FieldKind::Page => format!("context counter(page).display({})", typst_str(self.page_pattern())),
                        FieldKind::NumPages => "context counter(page).final().first()".to_string(),
                    };
                    items.push(self.text_expr_content(&value, props, spacing).to_string());
                    x = None;
                }
            }
        }
        let _ = x;
        if !any_text {
            items.insert(0, self.text_expr_line("\u{a0}", &p.mark, spacing, true));
        }

        let body: String = items.concat();
        let mut expr = format!(
            "par(leading: {}, justify: {}, first-line-indent: (amount: {}, all: true), hanging-indent: {})[{}]",
            pt(leading),
            align == Align::Justify,
            pt(first),
            pt(hanging),
            body
        );

        let right_indent = right;
        let mut pad_left = left - hanging + wrap_left;
        let mut right = right + wrap_right;
        let b = &p.props.borders;
        let has_border = [b.top, b.left, b.bottom, b.right].iter().any(|s| matches!(s, BorderSide::Line { .. }));
        let mut padded = false;
        if split_words && !has_border && p.props.shading.is_none() && !items.is_empty() {
            let text_width = {
                let page = self.page.borrow();
                page.width - page.margin.left - page.margin.right
            };
            let narrow = (text_width - pad_left - right).max(10.0);
            let code: Vec<String> = items.iter().map(|i| i.trim_start_matches('#').to_string()).collect();
            expr = format!(
                "context {{ let items = ({},); let n = items.len(); let mk(sel, fi, hg) = par(leading: 0pt, justify: {}, first-line-indent: (amount: fi, all: true), hanging-indent: hg)[#sel.join()]; let narrow = {}; let limit = {}; let lo = 0; let up = n; while lo < up {{ let mid = calc.ceil((lo + up) / 2); if measure(width: narrow, mk(items.slice(0, mid), {fi}, {hg})).height <= limit + 0.5pt {{ lo = mid }} else {{ up = mid - 1 }} }}; let k = if lo == 0 {{ n }} else {{ lo }}; let head = mk(items.slice(0, k), {fi}, {hg}); pad(left: {}, right: {}, head); if k < n {{ pad(left: {}, right: {}, mk(items.slice(k, n), 0pt, 0pt)) }} else {{ let used = measure(width: narrow, head).height; if used < limit {{ v(limit - used) }} }} }}",
                code.join(", "),
                align == Align::Justify,
                pt(narrow),
                pt(wrap_limit),
                pt(pad_left),
                pt(right),
                pt(left),
                pt(right_indent),
                fi = pt(first),
                hg = pt(hanging)
            );
            padded = true;
        }
        if has_border || p.props.shading.is_some() {
            let space = p.props.border_space;
            let fill = p
                .props
                .shading
                .map(|c| format!("rgb({})", typst_str(&c.hex())))
                .unwrap_or_else(|| "none".into());
            let top = if first_in_group { b.top } else { BorderSide::None };
            let bottom = if last_in_group { b.bottom } else { BorderSide::None };
            let inset_for = |side: BorderSide, gap: f64| match side {
                BorderSide::Line { width, .. } => gap + width,
                _ => 0.0,
            };
            let inset_left = inset_for(b.left, space.left);
            let inset_right = inset_for(b.right, space.right);
            if self.doc.borders_outside_indent {
                pad_left -= inset_left;
                right -= inset_right;
            }
            expr = format!(
                "block(width: 100%, fill: {fill}, stroke: (top: {}, bottom: {}, left: {}, right: {}), inset: (top: {}, bottom: {}, left: {}, right: {}), {expr})",
                stroke(top),
                stroke(bottom),
                stroke(b.left),
                stroke(b.right),
                pt(inset_for(top, space.top)),
                pt(inset_for(bottom, space.bottom)),
                pt(inset_left),
                pt(inset_right)
            );
        }
        if !padded && (pad_left.abs() > 0.01 || right.abs() > 0.01) {
            expr = format!("pad(left: {}, right: {}, {})", pt(pad_left), pt(right), expr);
        }
        match align {
            Align::Center => expr = format!("align(center, {expr})"),
            Align::Right => expr = format!("align(right, {expr})"),
            _ => {}
        }

        let _ = writeln!(out, "#{expr}");
        self.para_line.set(outer_line);
    }

    fn footnote_body(&self, blocks: &[Block]) -> String {
        match blocks {
            [Block::Paragraph(p)] => {
                let spacing = p.props.line_spacing.unwrap_or(LineSpacing::Multiple(1.0));
                let mut out = String::new();
                for inline in &p.inlines {
                    match inline {
                        Inline::Text { text, props } if !text.is_empty() => out.push_str(&self.text_expr(text, props, spacing)),
                        Inline::Tab => out.push_str("#h(1em)"),
                        Inline::LineBreak => out.push_str("#linebreak()"),
                        Inline::Drawing(drawing) => {
                            let _ = write!(out, "#box({})", self.drawing_expr(drawing));
                        }
                        _ => {}
                    }
                }
                if out.is_empty() {
                    self.text_expr("\u{a0}", &p.mark, spacing)
                } else {
                    out
                }
            }
            _ => self.blocks(blocks, false),
        }
    }

    fn first_line_height(&self, blocks: &[Block]) -> Option<f64> {
        let props = blocks.iter().find_map(|b| match b {
            Block::Paragraph(p) => p.inlines.iter().find_map(|i| match i {
                Inline::Text { props, text } if !text.is_empty() => Some(props),
                _ => None,
            }),
            _ => None,
        })?;
        let size = props.size.unwrap_or(DEFAULT_SIZE);
        let m = self.fonts.metrics(&self.family(props), props.bold == Some(true), props.italic == Some(true));
        let below = if self.doc.cell_metrics { m.descender } else { m.descender + m.line_gap };
        Some((m.ascender + below) * size)
    }

    fn paragraph_line_height(&self, p: &Paragraph, inlines: &[&Inline]) -> Option<f64> {
        if self.doc.fixed_line_metrics {
            return None;
        }
        let runs = inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Text { props, text } if !text.is_empty() => Some(props),
                _ => None,
            })
            .chain(p.list.as_ref().map(|l| &l.props));
        let mut above = 0.0f64;
        let mut below = 0.0f64;
        let mut any = false;
        for props in runs {
            any = true;
            let size = props.size.unwrap_or(DEFAULT_SIZE);
            let m = self
                .fonts
                .metrics(&self.family(props), props.bold == Some(true), props.italic == Some(true));
            above = above.max(m.ascender * size);
            below = below.max((m.descender + m.line_gap) * size);
        }
        any.then_some(above + below)
    }

    fn edges(&self, family: &str, props: &RunProps, size: f64, spacing: LineSpacing, empty_line: bool) -> (f64, f64) {
        if self.doc.fixed_line_metrics {
            let height = match spacing {
                LineSpacing::Multiple(mult) => 1.2 * mult * size,
                LineSpacing::Exact(v) => v,
                LineSpacing::AtLeast(v) => v.max(1.2 * size),
            };
            let below = 0.2 * size;
            return ((height - below).max(0.0), below);
        }
        let m = self
            .fonts
            .metrics(family, props.bold == Some(true), props.italic == Some(true));
        let line = m.line_height();
        let natural_below = if self.doc.cell_metrics { m.descender } else { m.descender + m.line_gap };
        let _ = empty_line;
        let (above, below) = match spacing {
            LineSpacing::Multiple(mult) if mult >= 1.0 => {
                let line = self.para_line.get().map_or(line, |v| v / size);
                (m.ascender, natural_below + (mult - 1.0) * line)
            }
            LineSpacing::Multiple(mult) => (m.ascender + (mult - 1.0) * line, natural_below),
            LineSpacing::Exact(v) => (v / size - natural_below, natural_below),
            LineSpacing::AtLeast(v) => (m.ascender + ((v / size) - line).max(0.0), natural_below),
        };
        (above.max(0.0) * size, below.max(0.0) * size)
    }

    fn text_expr(&self, text: &str, props: &RunProps, spacing: LineSpacing) -> String {
        self.text_expr_line(text, props, spacing, false)
    }

    fn text_expr_line(&self, text: &str, props: &RunProps, spacing: LineSpacing, empty_line: bool) -> String {
        if props.caps == Some(true) {
            return self.text_expr_content_line(&typst_str(&text.to_uppercase()), props, spacing, empty_line);
        }
        if props.small_caps == Some(true) && text.chars().any(char::is_lowercase) {
            let family = self.family(props);
            let size = props.size.unwrap_or(DEFAULT_SIZE);
            let (top, bottom) = self.edges(&family, props, size, spacing, empty_line);
            let mut small = props.clone();
            small.size = Some(size * 0.8);
            let mut out = String::new();
            let mut segment = String::new();
            let mut segment_lower = false;
            let flush = |segment: &mut String, lower: bool, out: &mut String| {
                if segment.is_empty() {
                    return;
                }
                if lower {
                    out.push_str(&self.text_with_edges(&typst_str(&segment.to_uppercase()), &small, &family, size * 0.8, top, bottom));
                } else {
                    out.push_str(&self.text_with_edges(&typst_str(segment), props, &family, size, top, bottom));
                }
                segment.clear();
            };
            for ch in text.chars() {
                let lower = ch.is_lowercase();
                if lower != segment_lower {
                    flush(&mut segment, segment_lower, &mut out);
                    segment_lower = lower;
                }
                segment.push(ch);
            }
            flush(&mut segment, segment_lower, &mut out);
            return out;
        }
        self.text_expr_content_line(&typst_str(text), props, spacing, empty_line)
    }

    fn text_expr_content(&self, content: &str, props: &RunProps, spacing: LineSpacing) -> String {
        self.text_expr_content_line(content, props, spacing, false)
    }

    fn text_expr_content_line(&self, content: &str, props: &RunProps, spacing: LineSpacing, empty_line: bool) -> String {
        let family = self.family(props);
        let size = props.size.unwrap_or(DEFAULT_SIZE);
        let (top, bottom) = self.edges(&family, props, size, spacing, empty_line);
        self.text_with_edges(content, props, &family, size, top, bottom)
    }

    fn text_with_edges(&self, content: &str, props: &RunProps, family: &str, size: f64, top: f64, bottom: f64) -> String {
        let mut args = vec![
            format!("font: {}", typst_str(family)),
            format!("size: {}", pt(size)),
            format!("top-edge: {}", pt(top)),
            format!("bottom-edge: -{}", pt(bottom)),
        ];
        if let Some(spacing) = props.letter_spacing.filter(|s| s.abs() > 0.001) {
            args.push(format!("tracking: {}", pt(spacing)));
        }
        if props.kerning == Some(true) {
            args.push("kerning: true".into());
        }
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
        if let Some((side @ BorderSide::Line { width, .. }, space)) = props.border {
            expr = format!("box(stroke: {}, inset: {}, {expr})", stroke(side), pt(space + width));
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

    fn wrap_limit(&self, p: &Paragraph) -> f64 {
        let mut limit = 0.0_f64;
        for anchor in &p.anchors {
            if !self.side_wrapped(anchor) {
                continue;
            }
            let offset = match anchor.vertical {
                VPosition::Offset(VRef::Paragraph | VRef::Line, y) => y.max(0.0),
                VPosition::Align(VRef::Paragraph | VRef::Line, _) => 0.0,
                _ => continue,
            };
            limit = limit.max(offset + anchor.drawing.height + anchor.dist_bottom);
        }
        limit
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
                let alpha = |c: &Color| format!("rgb({})", typst_str(&format!("{}{:02x}", c.hex(), (tb.opacity.clamp(0.0, 1.0) * 255.0).round() as u8)));
                let mut backdrop = String::new();
                let fill = match &tb.fill_style {
                    Some(FillStyle::Gradient { start, end, angle, radial }) => {
                        if *radial {
                            format!("gradient.radial({}, {})", alpha(start), alpha(end))
                        } else {
                            format!("gradient.linear({}, {}, angle: {}deg)", alpha(start), alpha(end), trim_num(90.0 - angle))
                        }
                    }
                    Some(FillStyle::Hatch { color, distance, angle, background }) => {
                        let d = pt(distance.max(0.75));
                        let a = angle.rem_euclid(180.0);
                        let line = if (a - 90.0).abs() < 22.5 {
                            format!("line(start: ({d} / 2, 0pt), end: ({d} / 2, {d}), stroke: 0.4pt + {})", alpha(color))
                        } else if (a - 45.0).abs() < 22.5 {
                            format!("line(start: (0pt, {d}), end: ({d}, 0pt), stroke: 0.4pt + {})", alpha(color))
                        } else if (a - 135.0).abs() < 22.5 {
                            format!("line(start: (0pt, 0pt), end: ({d}, {d}), stroke: 0.4pt + {})", alpha(color))
                        } else {
                            format!("line(start: (0pt, {d} / 2), end: ({d}, {d} / 2), stroke: 0.4pt + {})", alpha(color))
                        };
                        let bg = background.map(|c| format!("rect(width: {d}, height: {d}, fill: {}); ", alpha(&c))).unwrap_or_default();
                        format!("tiling(size: ({d}, {d}), {{ {bg}place(top + left, {line}) }})")
                    }
                    Some(FillStyle::Image { data, repeat }) => {
                        let path = self.register(data);
                        match (repeat, png_size(&data.data)) {
                            (true, Some((pw, ph))) => {
                                let tw = pt(pw as f64 * 0.75);
                                let th = pt(ph as f64 * 0.75);
                                format!("tiling(size: ({tw}, {th}), image({}, width: {tw}, height: {th}))", typst_str(&path))
                            }
                            _ => {
                                backdrop = format!(
                                    "place(top + left, dx: -{}, dy: -{}, image({}, width: {w}, height: {h})); ",
                                    pt(tb.inset.1),
                                    pt(tb.inset.0),
                                    typst_str(&path)
                                );
                                "none".to_string()
                            }
                        }
                    }
                    None => tb.fill.map(|c| alpha(&c)).unwrap_or_else(|| "none".into()),
                };
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
                    let mut body = format!("align({valign} + left)[{}]", self.blocks(&tb.blocks, false));
                    if !backdrop.is_empty() {
                        body = format!("{{ {backdrop}{body} }}");
                    }
                    if let Some(min) = tb.min_height.filter(|_| tb.auto_height) {
                        body = format!(
                            "minhw({}, {}, {body})",
                            pt((drawing.width - tb.inset.1 - tb.inset.3).max(0.0)),
                            pt((min - tb.inset.0 - tb.inset.2).max(0.0))
                        );
                    }
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
                    "#context {{ let inner = {expr}; let m = measure(inner); block(width: 100%, height: 0pt, place(top + {h_align}, dx: {}, dy: {}, box(width: m.width, height: m.height, inner))) }}",
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
                let mut top = margins.top.unwrap_or(0.0);
                let bottom = margins.bottom.unwrap_or(0.0);
                if row.exact_height {
                    if let (Some(height), Some(line)) = (row.height, self.first_line_height(&cell.blocks)) {
                        top = top.min((height - bottom - line).max(0.0));
                    }
                }

                let visible: Vec<Block> = cell.blocks.iter().filter(|b| !matches!(b, Block::Paragraph(p) if paragraph_hidden(p))).cloned().collect();
                let mut body = self.blocks(&visible, true);
                if let (Some(shift), false) = (cell.overflow_shift, body.trim().is_empty()) {
                    let clip = cell.overflow_width.unwrap_or(1.0);
                    body = format!(
                        "#context {{ let body = [{body}]; if measure(body).width > {shift} {{ block(width: {clip}, clip: true, breakable: false, pad(left: -{shift}, block(width: 20000pt, breakable: false, body))) }} }}",
                        shift = pt(shift),
                        clip = pt(clip)
                    );
                } else if cell.no_wrap && !body.trim().is_empty() {
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

fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 24 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    (width > 0 && height > 0).then_some((width, height))
}

fn paragraph_hidden(p: &Paragraph) -> bool {
    p.mark.hidden == Some(true)
        && p.anchors.is_empty()
        && p.list.is_none()
        && p.inlines.iter().all(|i| match i {
            Inline::Text { text, props } => text.trim().is_empty() || props.hidden == Some(true),
            Inline::Tab | Inline::LineBreak => true,
            _ => false,
        })
}

fn split_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if ch == ' ' {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn trim_num(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    format!("{rounded}")
}

fn next_tab_stop(x: f64, stops: &[f64], left: f64, hanging: f64, default_tab: f64, relative_to_indent: bool) -> f64 {
    let mut candidates: Vec<f64> = stops.iter().copied().filter(|s| *s > x + 0.01).collect();
    if (hanging > 0.01 || relative_to_indent) && left > x + 0.01 {
        candidates.push(left);
    }
    if let Some(min) = candidates.iter().copied().fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |a| a.min(v)))) {
        return min;
    }
    if default_tab <= 0.01 {
        return x + 36.0;
    }
    let origin = if relative_to_indent { left } else { 0.0 };
    origin + (((x - origin) / default_tab).floor() + 1.0) * default_tab
}

fn array(items: &[f64]) -> String {
    let parts: Vec<String> = items.iter().map(|v| trim_num(*v)).collect();
    if parts.len() == 1 {
        format!("({},)", parts[0])
    } else {
        format!("({})", parts.join(", "))
    }
}

fn stroke(side: BorderSide) -> String {
    match side {
        BorderSide::Line { width, color, style } => match style {
            LineStyle::Dotted => format!("(paint: rgb({}), thickness: {}, dash: \"dotted\")", typst_str(&color.hex()), pt(width)),
            LineStyle::Dashed => format!("(paint: rgb({}), thickness: {}, dash: \"dashed\")", typst_str(&color.hex()), pt(width)),
            _ => format!("{} + rgb({})", pt(width), typst_str(&color.hex())),
        },
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
