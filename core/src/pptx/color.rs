use crate::model::Color;
use crate::xml::Element;
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct Theme {
    pub colors: HashMap<String, Color>,
    pub major: String,
    pub minor: String,
}

impl Theme {
    pub fn parse(root: &Element) -> Theme {
        let mut theme = Theme {
            major: "Calibri Light".into(),
            minor: "Calibri".into(),
            ..Theme::default()
        };
        let Some(elements) = root.child("themeElements") else { return theme };
        if let Some(scheme) = elements.child("clrScheme") {
            for entry in scheme.elements() {
                if let Some(color) = entry.elements().next().and_then(raw_color) {
                    theme.colors.insert(entry.name.clone(), color);
                }
            }
        }
        if let Some(fonts) = elements.child("fontScheme") {
            let latin = |name: &str| {
                fonts
                    .child(name)
                    .and_then(|f| f.child("latin"))
                    .and_then(|l| l.attr("typeface"))
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned)
            };
            if let Some(major) = latin("majorFont") {
                theme.major = major;
            }
            if let Some(minor) = latin("minorFont") {
                theme.minor = minor;
            }
        }
        theme
    }

    pub fn font(&self, name: &str) -> String {
        match name {
            "+mj-lt" | "+mj-ea" | "+mj-cs" => self.major.clone(),
            "+mn-lt" | "+mn-ea" | "+mn-cs" => self.minor.clone(),
            other => other.to_string(),
        }
    }
}

pub struct ColorContext<'a> {
    pub theme: &'a Theme,
    pub clr_map: &'a HashMap<String, String>,
    pub placeholder: Option<Color>,
}

pub fn default_clr_map() -> HashMap<String, String> {
    [
        ("bg1", "lt1"),
        ("tx1", "dk1"),
        ("bg2", "lt2"),
        ("tx2", "dk2"),
        ("accent1", "accent1"),
        ("accent2", "accent2"),
        ("accent3", "accent3"),
        ("accent4", "accent4"),
        ("accent5", "accent5"),
        ("accent6", "accent6"),
        ("hlink", "hlink"),
        ("folHlink", "folHlink"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

pub fn parse_clr_map(el: &Element) -> HashMap<String, String> {
    let mut map = default_clr_map();
    for (key, value) in &el.attrs {
        map.insert(key.clone(), value.clone());
    }
    map
}

fn raw_color(el: &Element) -> Option<Color> {
    match el.name.as_str() {
        "srgbClr" => el.attr("val").and_then(Color::parse_hex),
        "sysClr" => el
            .attr("lastClr")
            .and_then(Color::parse_hex)
            .or_else(|| preset(el.attr("val").unwrap_or(""))),
        "prstClr" => preset(el.attr("val").unwrap_or("")),
        "scrgbClr" => {
            let channel = |name: &str| {
                el.attr(name)
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(|v| (v / 100000.0 * 255.0).round().clamp(0.0, 255.0) as u8)
            };
            Some(Color(channel("r")?, channel("g")?, channel("b")?))
        }
        _ => None,
    }
}

pub fn resolve(el: &Element, ctx: &ColorContext) -> Option<Color> {
    let base = match el.name.as_str() {
        "schemeClr" => {
            let name = el.attr("val")?;
            if name == "phClr" {
                ctx.placeholder?
            } else {
                let mapped = ctx.clr_map.get(name).map(String::as_str).unwrap_or(name);
                *ctx.theme.colors.get(mapped).or_else(|| ctx.theme.colors.get(name))?
            }
        }
        _ => raw_color(el)?,
    };
    Some(apply_modifiers(base, el))
}

pub fn resolve_child(parent: &Element, ctx: &ColorContext) -> Option<Color> {
    parent.elements().find_map(|c| resolve(c, ctx))
}

fn apply_modifiers(color: Color, el: &Element) -> Color {
    let (mut h, mut s, mut l) = to_hsl(color);
    let mut rgb: Option<Color> = None;
    for m in el.elements() {
        let value = m
            .attr("val")
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v / 100000.0);
        match (m.name.as_str(), value) {
            ("lumMod", Some(v)) => l = (l * v).clamp(0.0, 1.0),
            ("lumOff", Some(v)) => l = (l + v).clamp(0.0, 1.0),
            ("satMod", Some(v)) => s = (s * v).clamp(0.0, 1.0),
            ("hueMod", Some(v)) => h = (h * v) % 360.0,
            ("tint", Some(v)) => {
                let c = rgb.unwrap_or_else(|| from_hsl(h, s, l));
                let mix = |x: u8| to_srgb(1.0 - (1.0 - to_linear(x)) * v);
                rgb = Some(Color(mix(c.0), mix(c.1), mix(c.2)));
            }
            ("shade", Some(v)) => {
                let c = rgb.unwrap_or_else(|| from_hsl(h, s, l));
                let mix = |x: u8| to_srgb(to_linear(x) * v);
                rgb = Some(Color(mix(c.0), mix(c.1), mix(c.2)));
            }
            _ => {}
        }
    }
    rgb.unwrap_or_else(|| from_hsl(h, s, l))
}

fn to_linear(x: u8) -> f64 {
    let c = x as f64 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(v: f64) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let c = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (c * 255.0).round() as u8
}

fn preset(name: &str) -> Option<Color> {
    Some(match name.to_lowercase().as_str() {
        "black" | "windowtext" => Color(0, 0, 0),
        "white" | "window" => Color(255, 255, 255),
        "red" => Color(255, 0, 0),
        "green" => Color(0, 128, 0),
        "blue" => Color(0, 0, 255),
        "yellow" => Color(255, 255, 0),
        "gray" | "grey" => Color(128, 128, 128),
        "ltgray" | "lightgray" => Color(211, 211, 211),
        "dkgray" | "darkgray" => Color(169, 169, 169),
        "orange" => Color(255, 165, 0),
        "purple" => Color(128, 0, 128),
        _ => return None,
    })
}

pub(crate) fn apply_tint(color: Color, tint: f64) -> Color {
    if tint.abs() < 1e-6 {
        return color;
    }
    let (h, s, l) = to_hsl(color);
    let l = if tint > 0.0 { l * (1.0 - tint) + tint } else { l * (1.0 + tint) };
    from_hsl(h, s, l.clamp(0.0, 1.0))
}

fn to_hsl(c: Color) -> (f64, f64, f64) {
    let r = c.0 as f64 / 255.0;
    let g = c.1 as f64 / 255.0;
    let b = c.2 as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-9 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if (max - r).abs() < 1e-9 {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) * 60.0
    } else if (max - g).abs() < 1e-9 {
        ((b - r) / d + 2.0) * 60.0
    } else {
        ((r - g) / d + 4.0) * 60.0
    };
    (h, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Color {
    if s <= 0.0 {
        let v = (l * 255.0).round() as u8;
        return Color(v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    let hk = h / 360.0;
    Color(hue(hk + 1.0 / 3.0), hue(hk), hue(hk - 1.0 / 3.0))
}
