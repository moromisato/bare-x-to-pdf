use crate::error::Error;
use crate::model::*;
use std::fmt::Write as _;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

const EMU_PER_PT: f64 = 12700.0;
const BOX_SLACK: f64 = 1.08;

pub fn write_fixed(doc: &FixedDocument) -> Result<Vec<u8>, Error> {
    let mut images: Vec<(String, Vec<u8>)> = Vec::new();
    let mut body = String::new();
    let mut ids = 1u32;

    for (index, page) in doc.pages.iter().enumerate() {
        body.push_str("<w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"20\" w:lineRule=\"exact\"/><w:rPr><w:sz w:val=\"2\"/></w:rPr>");
        if index + 1 < doc.pages.len() {
            body.push_str(&sect_pr(page));
        }
        body.push_str("</w:pPr>");

        if let Some(background) = &page.background {
            let rel = format!("rId{}", 100 + images.len());
            let name = format!("media/page{}.png", index + 1);
            images.push((rel.clone(), background.png.clone()));
            let _ = name;
            body.push_str(&picture_anchor(&rel, ids, page.width, page.height));
            ids += 1;
        }

        for line in &page.lines {
            body.push_str(&text_box(line, ids));
            ids += 1;
        }
        body.push_str("</w:p>");
    }

    if let Some(last) = doc.pages.last() {
        body.push_str(&sect_pr(last));
    }

    let document = format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
            "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" ",
            "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" ",
            "xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" ",
            "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" ",
            "xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\" ",
            "xmlns:wps=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\">",
            "<w:body>{}</w:body></w:document>"
        ),
        body
    );

    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
    );
    for (i, (rel, _)) in images.iter().enumerate() {
        let _ = write!(
            rels,
            "<Relationship Id=\"{rel}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/image{}.png\"/>",
            i + 1
        );
    }
    rels.push_str("</Relationships>");

    let content_types = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
        "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
        "<Default Extension=\"xml\" ContentType=\"application/xml\"/>",
        "<Default Extension=\"png\" ContentType=\"image/png\"/>",
        "<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>",
        "<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>",
        "</Types>"
    );

    let root_rels = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
        "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>",
        "</Relationships>"
    );

    let styles = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
        "<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Liberation Sans\" w:hAnsi=\"Liberation Sans\" w:cs=\"Liberation Sans\"/><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault>",
        "<w:pPrDefault><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>",
        "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>",
        "</w:styles>"
    );

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut add = |name: &str, data: &[u8]| -> Result<(), Error> {
        zip.start_file(name, options)?;
        zip.write_all(data)?;
        Ok(())
    };
    add("[Content_Types].xml", content_types.as_bytes())?;
    add("_rels/.rels", root_rels.as_bytes())?;
    add("word/document.xml", document.as_bytes())?;
    add("word/_rels/document.xml.rels", rels.as_bytes())?;
    add("word/styles.xml", styles.as_bytes())?;
    for (i, (_, png)) in images.iter().enumerate() {
        add(&format!("word/media/image{}.png", i + 1), png)?;
    }
    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}

fn sect_pr(page: &FixedPage) -> String {
    format!(
        "<w:sectPr><w:type w:val=\"nextPage\"/><w:pgSz w:w=\"{}\" w:h=\"{}\"{}/><w:pgMar w:top=\"0\" w:right=\"0\" w:bottom=\"0\" w:left=\"0\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/></w:sectPr>",
        twips(page.width),
        twips(page.height),
        if page.width > page.height { " w:orient=\"landscape\"" } else { "" }
    )
}

fn anchor_open(id: u32, name: &str, x: f64, y: f64, width: f64, height: f64, behind: bool) -> String {
    format!(
        concat!(
            "<w:r><w:drawing><wp:anchor distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\" simplePos=\"0\" relativeHeight=\"{id}\" behindDoc=\"{behind}\" locked=\"0\" layoutInCell=\"1\" allowOverlap=\"1\">",
            "<wp:simplePos x=\"0\" y=\"0\"/>",
            "<wp:positionH relativeFrom=\"page\"><wp:posOffset>{x}</wp:posOffset></wp:positionH>",
            "<wp:positionV relativeFrom=\"page\"><wp:posOffset>{y}</wp:posOffset></wp:positionV>",
            "<wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/><wp:wrapNone/>",
            "<wp:docPr id=\"{id}\" name=\"{name}\"/><wp:cNvGraphicFramePr/>",
        ),
        id = id,
        behind = if behind { 1 } else { 0 },
        x = emu(x),
        y = emu(y),
        cx = emu(width),
        cy = emu(height),
        name = name
    )
}

fn picture_anchor(rel: &str, id: u32, width: f64, height: f64) -> String {
    let mut out = anchor_open(id, &format!("Background {id}"), 0.0, 0.0, width, height, true);
    let _ = write!(
        out,
        concat!(
            "<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">",
            "<pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"Background {id}\"/><pic:cNvPicPr/></pic:nvPicPr>",
            "<pic:blipFill><a:blip r:embed=\"{rel}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>",
            "<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr>",
            "</pic:pic></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"
        ),
        id = id,
        rel = rel,
        cx = emu(width),
        cy = emu(height)
    );
    out
}

fn text_box(line: &TextLine, id: u32) -> String {
    let width = line.width * BOX_SLACK + 4.0;
    let height = line.height.max(1.0);
    let mut out = anchor_open(id, &format!("Text {id}"), line.x, line.top, width, height, false);
    let _ = write!(
        out,
        concat!(
            "<a:graphic><a:graphicData uri=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\">",
            "<wps:wsp><wps:cNvSpPr txBox=\"1\"/>",
            "<wps:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></wps:spPr>",
            "<wps:txbx><w:txbxContent><w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"{line}\" w:lineRule=\"exact\"/></w:pPr>{runs}</w:p></w:txbxContent></wps:txbx>",
            "<wps:bodyPr rot=\"0\" vert=\"horz\" wrap=\"none\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" anchor=\"t\" anchorCtr=\"0\"><a:noAutofit/></wps:bodyPr>",
            "</wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"
        ),
        cx = emu(width),
        cy = emu(height),
        line = twips(height),
        runs = runs_xml(&line.runs)
    );
    out
}

fn runs_xml(runs: &[TextRun]) -> String {
    let mut out = String::new();
    for run in runs {
        let font = escape(&run.font);
        let _ = write!(
            out,
            "<w:r><w:rPr><w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:cs=\"{font}\"/>{bold}{italic}<w:color w:val=\"{color}\"/><w:sz w:val=\"{sz}\"/><w:szCs w:val=\"{sz}\"/></w:rPr><w:t xml:space=\"preserve\">{text}</w:t></w:r>",
            bold = if run.bold { "<w:b/><w:bCs/>" } else { "" },
            italic = if run.italic { "<w:i/><w:iCs/>" } else { "" },
            color = &run.color.hex()[1..],
            sz = (run.size * 2.0).round().max(2.0) as i64,
            text = escape(&run.text)
        );
    }
    out
}

fn emu(pt: f64) -> i64 {
    (pt * EMU_PER_PT).round() as i64
}

fn twips(pt: f64) -> i64 {
    (pt * 20.0).round() as i64
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}
