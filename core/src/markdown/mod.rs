mod style;

use crate::error::Error;
use crate::model::*;
use pulldown_cmark::{Alignment, Event, Options, Parser, Tag, TagEnd};
use style::{Profile, TableCell};

pub fn read(bytes: &[u8]) -> Result<Document, Error> {
    let text = String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes));
    let profile = Profile::markdown();
    let mut reader = Reader::new(&profile);
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(&text, options) {
        reader.event(event);
    }
    reader.flush();
    Ok(style::document(&profile, reader.blocks))
}

struct ListState {
    next: Option<u64>,
}

struct ItemState {
    labelled: bool,
    last_paragraph: Option<usize>,
}

struct TableState {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<TableCell>>,
    in_head: bool,
}

#[derive(PartialEq)]
enum Pending {
    None,
    Paragraph,
    Heading(usize),
}

struct Reader<'a> {
    profile: &'a Profile,
    blocks: Vec<Block>,
    inlines: Vec<Inline>,
    pending: Pending,
    bold: usize,
    italic: usize,
    strike: usize,
    link: usize,
    quote: usize,
    lists: Vec<ListState>,
    items: Vec<ItemState>,
    table: Option<TableState>,
    cell: Option<Vec<Block>>,
    code: Option<String>,
    image: usize,
    html: Option<String>,
}

fn align(a: Alignment) -> Option<Align> {
    match a {
        Alignment::Left | Alignment::None => None,
        Alignment::Center => Some(Align::Center),
        Alignment::Right => Some(Align::Right),
    }
}

impl<'a> Reader<'a> {
    fn new(profile: &'a Profile) -> Self {
        Reader {
            profile,
            blocks: Vec::new(),
            inlines: Vec::new(),
            pending: Pending::None,
            bold: 0,
            italic: 0,
            strike: 0,
            link: 0,
            quote: 0,
            lists: Vec::new(),
            items: Vec::new(),
            table: None,
            cell: None,
            code: None,
            image: 0,
            html: None,
        }
    }

    fn run(&self) -> RunProps {
        let mut run = match &self.pending {
            Pending::Heading(level) => self.profile.heading(*level).0,
            _ => self.profile.body.clone(),
        };
        if self.bold > 0 || (self.table.as_ref().is_some_and(|t| t.in_head) && self.cell.is_some()) {
            run.bold = Some(true);
        }
        if self.italic > 0 {
            run.italic = Some(true);
        }
        if self.strike > 0 {
            run.strike = Some(true);
        }
        if self.link > 0 {
            run.color = Some(style::LINK);
            run.underline = Some(true);
        }
        run
    }

    fn quote_indent(&self) -> f64 {
        if self.quote > 0 { 28.346 } else { 0.0 }
    }

    fn text(&mut self, text: &str) {
        if self.image > 0 {
            return;
        }
        if self.pending == Pending::None {
            self.pending = Pending::Paragraph;
        }
        let props = self.run();
        self.inlines.push(Inline::Text { text: text.to_string(), props });
    }

    fn push(&mut self, block: Block) {
        match self.cell.as_mut() {
            Some(cell) => cell.push(block),
            None => self.blocks.push(block),
        }
    }

    fn flush(&mut self) {
        let pending = std::mem::replace(&mut self.pending, Pending::None);
        if pending == Pending::None && self.inlines.is_empty() {
            return;
        }
        let inlines = std::mem::take(&mut self.inlines);
        let mark = inlines
            .iter()
            .rev()
            .find_map(|i| if let Inline::Text { props, .. } = i { Some(props.clone()) } else { None })
            .unwrap_or_else(|| self.profile.body.clone());
        if self.cell.is_some() {
            let alignment = self.table.as_ref().and_then(|t| {
                let column = self.cell_index();
                t.alignments.get(column).copied().and_then(align)
            });
            self.push(style::paragraph(style::cell_props(alignment), mark, inlines, None));
            return;
        }
        if let Pending::Heading(level) = pending {
            let (_, props) = self.profile.heading(level);
            self.push(style::paragraph(props, mark, inlines, None));
            return;
        }
        let mut props = self.profile.body_props();
        let quote_indent = self.quote_indent();
        if self.quote > 0 {
            props.indent_left = Some(quote_indent);
            props.indent_right = Some(quote_indent);
            props.line_spacing = Some(LineSpacing::Multiple(self.profile.quote_line));
            props.space_after = Some(self.profile.quote_after);
        }
        let mut list = None;
        if let Some(item) = self.items.last_mut() {
            let depth = self.lists.len();
            let text = if item.labelled {
                None
            } else {
                item.labelled = true;
                Some(match self.lists.last_mut().and_then(|l| l.next.as_mut()) {
                    Some(n) => {
                        let label = format!("{n}.");
                        *n += 1;
                        label
                    }
                    None => "\u{2022}".to_string(),
                })
            };
            let (_, text_at, hanging) = style::list_label("", &self.profile.body, depth);
            props.indent_left = Some(text_at + quote_indent);
            if self.quote == 0 {
                props.space_after = Some(self.profile.list_item_after);
            }
            if let Some(label) = text {
                props.indent_hanging = Some(hanging);
                props.indent_first_line = Some(0.0);
                list = Some(style::list_label(&label, &self.profile.body, depth).0);
            }
        }
        let index = self.blocks.len();
        self.push(style::paragraph(props, mark, inlines, list));
        if let Some(item) = self.items.last_mut() {
            item.last_paragraph = Some(index);
        }
    }

    fn cell_index(&self) -> usize {
        self.table.as_ref().and_then(|t| t.rows.last()).map_or(0, Vec::len)
    }

    fn event(&mut self, event: Event<'_>) {
        if let Some(html) = self.html.as_mut() {
            match event {
                Event::Html(t) => html.push_str(&t),
                Event::End(TagEnd::HtmlBlock) => self.html_block(),
                _ => {}
            }
            return;
        }
        if let Some(code) = self.code.as_mut() {
            match event {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => self.code_block(),
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                if self.pending == Pending::None {
                    self.pending = Pending::Paragraph;
                }
                let mut props = self.run();
                props.font = Some(style::MONO.into());
                self.inlines.push(Inline::Text { text: t.to_string(), props });
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.inlines.push(Inline::LineBreak),
            Event::Rule => {
                self.flush();
                let rule = style::rule(self.profile);
                self.push(rule);
            }
            Event::TaskListMarker(checked) => {
                if self.pending == Pending::None {
                    self.pending = Pending::Paragraph;
                }
                let props = self.run();
                self.inlines.push(Inline::Text { text: if checked { "\u{2612}".into() } else { "\u{25A1}".into() }, props });
                self.text(" ");
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.flush();
                self.pending = Pending::Paragraph;
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.pending = Pending::Heading(level as usize);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote += 1;
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.code = Some(String::new());
            }
            Tag::List(start) => {
                self.flush();
                if let Some(item) = self.items.last() {
                    if let Some(index) = item.last_paragraph {
                        if let Some(Block::Paragraph(p)) = self.blocks.get_mut(index) {
                            p.props.space_after = Some(0.0);
                        }
                    }
                }
                self.lists.push(ListState { next: start });
            }
            Tag::Item => {
                self.flush();
                self.items.push(ItemState { labelled: false, last_paragraph: None });
            }
            Tag::Table(alignments) => {
                self.flush();
                self.table = Some(TableState { alignments, rows: Vec::new(), in_head: false });
            }
            Tag::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.in_head = true;
                    t.rows.push(Vec::new());
                }
            }
            Tag::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.rows.push(Vec::new());
                }
            }
            Tag::TableCell => self.cell = Some(Vec::new()),
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { .. } => self.link += 1,
            Tag::Image { dest_url, .. } => {
                self.image += 1;
                if let Some(drawing) = data_image(&dest_url, self.profile.text_width()) {
                    if self.pending == Pending::None {
                        self.pending = Pending::Paragraph;
                    }
                    self.inlines.push(Inline::Drawing(drawing));
                }
            }
            Tag::HtmlBlock => {
                self.flush();
                self.html = Some(String::new());
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) => self.flush(),
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush();
                self.items.pop();
            }
            TagEnd::TableCell => {
                self.flush();
                let blocks = self.cell.take().unwrap_or_default();
                let alignment = self.table.as_ref().and_then(|t| t.alignments.get(self.cell_index()).copied().and_then(align));
                if let Some(row) = self.table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(TableCell { blocks, align: alignment });
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.in_head = false;
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    let rows: Vec<Vec<TableCell>> = t.rows.into_iter().filter(|r| !r.is_empty()).collect();
                    let table = style::table(self.profile, rows);
                    self.push(table);
                    let gap = style::empty_body(self.profile);
                    self.push(gap);
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => self.link = self.link.saturating_sub(1),
            TagEnd::Image => self.image = self.image.saturating_sub(1),
            _ => {}
        }
    }

    fn code_block(&mut self) {
        let code = self.code.take().unwrap_or_default();
        let run = self.profile.code_run();
        let mut inlines = Vec::new();
        for (i, line) in code.trim_end_matches('\n').split('\n').enumerate() {
            if i > 0 {
                inlines.push(Inline::LineBreak);
            }
            let text = if line.is_empty() { " " } else { line };
            inlines.push(Inline::Text { text: text.to_string(), props: run.clone() });
        }
        let list_indent = if self.items.is_empty() { 0.0 } else { style::list_indents(self.lists.len()).1 };
        let indent = self.quote_indent() + list_indent;
        let props = ParagraphProps {
            space_before: Some(0.0),
            space_after: Some(self.profile.code_after),
            line_spacing: Some(LineSpacing::Multiple(1.0)),
            shading: self.profile.code_shading,
            indent_left: (indent > 0.0).then_some(indent),
            ..ParagraphProps::default()
        };
        self.push(style::paragraph(props, run, inlines, None));
    }

    fn html_block(&mut self) {
        let html = self.html.take().unwrap_or_default();
        let mut text = String::new();
        let mut in_tag = false;
        for ch in html.chars() {
            match ch {
                '<' => in_tag = true,
                '>' if in_tag => in_tag = false,
                _ if !in_tag => text.push(ch),
                _ => {}
            }
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return;
        }
        let mut props = self.profile.body_props();
        props.space_after = Some(0.0);
        let run = self.profile.body.clone();
        self.push(style::paragraph(props, run.clone(), vec![Inline::Text { text, props: run.clone() }], None));
    }
}

fn data_image(url: &str, max_width: f64) -> Option<Drawing> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    if !meta.ends_with(";base64") {
        return None;
    }
    let data = crate::odt::base64_decode(payload)?;
    let format = ImageFormat::sniff(&data)?;
    let (w, h) = pixel_size(&data)?;
    let (mut width, mut height) = (w as f64 * 0.75, h as f64 * 0.75);
    if width > max_width {
        height *= max_width / width;
        width = max_width;
    }
    Some(Drawing::new(width, height, DrawingContent::Image(ImageData { data, format })))
}

fn pixel_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        let w = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?);
        return Some((w, h));
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        let mut pos = 2;
        while pos + 9 < data.len() {
            if data[pos] != 0xFF {
                return None;
            }
            let marker = data[pos + 1];
            let len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
            if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                let h = u16::from_be_bytes([data[pos + 5], data[pos + 6]]) as u32;
                let w = u16::from_be_bytes([data[pos + 7], data[pos + 8]]) as u32;
                return Some((w, h));
            }
            pos += 2 + len;
        }
    }
    None
}
