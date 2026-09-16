use crate::error::Error;
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.name == name)
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        for node in &self.children {
            match node {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) => out.push_str(&e.text()),
            }
        }
        out
    }
}

pub fn parse(bytes: &[u8]) -> Result<Element, Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::new("xml is not valid UTF-8"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut reader = Reader::from_str(text);
    let config = reader.config_mut();
    config.trim_text_start = false;
    config.trim_text_end = false;

    let mut stack: Vec<Element> = vec![Element {
        name: String::from("#document"),
        ..Element::default()
    }];

    loop {
        match reader.read_event()? {
            Event::Start(start) => stack.push(element(&start)?),
            Event::Empty(start) => {
                let el = element(&start)?;
                push_child(&mut stack, Node::Element(el));
            }
            Event::End(_) => {
                let el = stack.pop().ok_or_else(|| Error::new("xml: unbalanced end tag"))?;
                if stack.is_empty() {
                    return Err(Error::new("xml: unbalanced end tag"));
                }
                push_child(&mut stack, Node::Element(el));
            }
            Event::Text(t) => {
                let raw = t.into_inner();
                let value = unescape(&raw).map_err(|e| Error::new(format!("xml: {e}")))?;
                push_text(&mut stack, &value);
            }
            Event::CData(c) => {
                let value = c.into_inner();
                push_text(&mut stack, &value);
            }
            Event::GeneralRef(r) => {
                let value = r.xml10_content();
                push_text(&mut stack, &value);
            }
            Event::Eof => break,
            _ => {}
        }
    }

    let mut root = stack.pop().ok_or_else(|| Error::new("xml: empty document"))?;
    if !stack.is_empty() {
        return Err(Error::new("xml: unclosed element"));
    }
    std::mem::take(&mut root.children)
        .into_iter()
        .find_map(|node| match node {
            Node::Element(el) => Some(el),
            Node::Text(_) => None,
        })
        .ok_or_else(|| Error::new("xml: no root element"))
}

fn element(start: &quick_xml::events::BytesStart<'_>) -> Result<Element, Error> {
    let name = local(start.name().as_ref());
    let mut attrs: Vec<(String, String)> = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| Error::new(format!("xml attribute: {e}")))?;
        let qualified: &str = attr.key.as_ref();
        let key = local(qualified);
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| Error::new(format!("xml attribute: {e}")))?
            .into_owned();
        if qualified.contains(':') {
            attrs.push((qualified.to_string(), value.clone()));
        }
        if !attrs.iter().any(|(k, _)| *k == key) {
            attrs.push((key, value));
        }
    }
    Ok(Element {
        name,
        attrs,
        children: Vec::new(),
    })
}

fn local(qualified: &str) -> String {
    qualified
        .rsplit(':')
        .next()
        .unwrap_or(qualified)
        .to_owned()
}

fn push_child(stack: &mut [Element], node: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    }
}

fn push_text(stack: &mut [Element], value: &str) {
    if value.is_empty() {
        return;
    }
    if let Some(parent) = stack.last_mut() {
        if let Some(Node::Text(existing)) = parent.children.last_mut() {
            existing.push_str(value);
        } else {
            parent.children.push(Node::Text(value.to_owned()));
        }
    }
}
