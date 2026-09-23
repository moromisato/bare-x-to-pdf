use crate::error::Error;
use crate::model::PageNote;
use std::fmt::Write;

const NOTE_SIZE: f64 = 3.203;

pub fn append_notes(mut pdf: Vec<u8>, pages: &[(f64, &[PageNote])]) -> Result<Vec<u8>, Error> {
    let broken = || Error::new("cannot attach notes: unexpected PDF structure");
    let text = String::from_utf8_lossy(&pdf).into_owned();
    let trailer_at = text.rfind("trailer").ok_or_else(broken)?;
    let trailer = &text[trailer_at..];
    let prev = trailer
        .rfind("startxref")
        .and_then(|i| trailer[i + 9..].split_whitespace().next())
        .and_then(|v| v.parse::<usize>().ok())
        .ok_or_else(broken)?;
    let size = dict_number(trailer, "/Size").ok_or_else(broken)?;
    let root = dict_ref(trailer, "/Root").ok_or_else(broken)?;
    let info = dict_ref(trailer, "/Info");
    let id = trailer.find("/ID").map(|i| {
        let rest = &trailer[i + 3..];
        let end = rest.find(']').map_or(0, |e| e + 1);
        rest[..end].trim().to_string()
    });

    let catalog = object_dict(&text, root).ok_or_else(broken)?;
    let pages_ref = dict_ref(&catalog, "/Pages").ok_or_else(broken)?;
    let pages_dict = object_dict(&text, pages_ref).ok_or_else(broken)?;
    let kids_at = pages_dict.find("/Kids").ok_or_else(broken)?;
    let kids_text = &pages_dict[kids_at + 5..];
    let kids_end = kids_text.find(']').ok_or_else(broken)?;
    let kids: Vec<usize> = kids_text[..kids_end]
        .trim_start_matches(|c: char| c == '[' || c.is_whitespace())
        .split('R')
        .filter_map(|r| r.split_whitespace().next().and_then(|n| n.parse().ok()))
        .collect();
    if kids.len() != pages.len() {
        return Ok(pdf);
    }

    let mut next = size;
    let mut objects: Vec<(usize, String)> = Vec::new();
    for (page_ref, (height, notes)) in kids.iter().zip(pages) {
        if notes.is_empty() {
            continue;
        }
        let mut refs = Vec::new();
        for note in notes.iter() {
            let (annot, popup) = (next, next + 1);
            next += 2;
            let top = height - note.y;
            let size = NOTE_SIZE * note.scale;
            let mut body = String::new();
            let _ = write!(
                body,
                "<</Type/Annot/Subtype/Text/Rect[{} {} {} {}]/Popup {popup} 0 R/Contents {}/T {}>>",
                num(note.x - size),
                num(top - size),
                num(note.x),
                num(top),
                utf16_hex(&note.text),
                utf16_hex(&note.title)
            );
            objects.push((annot, body));
            objects.push((
                popup,
                format!(
                    "<</Type/Annot/Subtype/Popup/Rect[{} {} {} {}]/Parent {annot} 0 R>>",
                    num(note.x + 11.226 * note.scale),
                    num(top - 63.184 * note.scale),
                    num(note.x + 105.535 * note.scale),
                    num(top - 1.502 * note.scale)
                ),
            ));
            refs.push(format!("{annot} 0 R {popup} 0 R"));
        }
        let dict = object_dict(&text, *page_ref).ok_or_else(broken)?;
        let joined = refs.join(" ");
        let updated = if let Some(at) = dict.find("/Annots") {
            let open = dict[at..].find('[').ok_or_else(broken)? + at + 1;
            format!("{}{} {}", &dict[..open], joined, &dict[open..])
        } else {
            let close = dict.rfind(">>").ok_or_else(broken)?;
            format!("{}/Annots[{}]{}", &dict[..close], joined, &dict[close..])
        };
        objects.push((*page_ref, updated));
    }
    if objects.is_empty() {
        return Ok(pdf);
    }

    if !pdf.ends_with(b"\n") {
        pdf.push(b'\n');
    }
    let mut offsets = Vec::new();
    for (number, body) in &objects {
        offsets.push((*number, pdf.len()));
        pdf.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    offsets.sort_by_key(|(n, _)| *n);
    let xref_at = pdf.len();
    let mut xref = String::from("xref\n");
    for (number, offset) in &offsets {
        let _ = write!(xref, "{number} 1\n{offset:010} 00000 n\r\n");
    }
    let _ = write!(xref, "trailer\n<</Size {next}/Root {root} 0 R");
    if let Some(info) = info {
        let _ = write!(xref, "/Info {info} 0 R");
    }
    if let Some(id) = id {
        let _ = write!(xref, "/ID{id}");
    }
    let _ = write!(xref, "/Prev {prev}>>\nstartxref\n{xref_at}\n%%EOF\n");
    pdf.extend_from_slice(xref.as_bytes());
    Ok(pdf)
}

fn num(v: f64) -> String {
    let rounded = (v * 1000.0).round() / 1000.0;
    format!("{rounded}")
}

fn utf16_hex(text: &str) -> String {
    let mut out = String::from("<FEFF");
    for unit in text.encode_utf16() {
        let _ = write!(out, "{unit:04X}");
    }
    out.push('>');
    out
}

fn dict_number(text: &str, key: &str) -> Option<usize> {
    let at = text.find(key)? + key.len();
    text[at..].split(|c: char| !c.is_ascii_digit() && c != ' ').find(|s| !s.trim().is_empty())?.trim().parse().ok()
}

fn dict_ref(text: &str, key: &str) -> Option<usize> {
    let at = text.find(key)? + key.len();
    text[at..].split_whitespace().next()?.parse().ok()
}

fn object_dict(text: &str, number: usize) -> Option<String> {
    let marker = format!("{number} 0 obj");
    let mut search = 0;
    let start = loop {
        let at = text[search..].find(&marker)? + search;
        if at == 0 || matches!(text.as_bytes()[at - 1], b'\n' | b'\r' | b' ') {
            break at + marker.len();
        }
        search = at + marker.len();
    };
    let end = text[start..].find("endobj")? + start;
    let body = text[start..end].trim();
    let body = body.split("stream").next().unwrap_or(body).trim();
    body.starts_with("<<").then(|| body.to_string())
}
