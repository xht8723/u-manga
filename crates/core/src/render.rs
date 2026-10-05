use crate::{cleanup, types::*};
use anyhow::{Context, Result};
#[cfg(test)]
use image::RgbImage;
use image::{DynamicImage, Rgb};
use std::path::Path;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};
use unicode_properties::{
    GeneralCategory as Category, GeneralCategoryGroup as Group, UnicodeGeneralCategory,
};
use unicode_segmentation::UnicodeSegmentation;

type FontData = (std::sync::Arc<Vec<u8>>, u32);
fn cache_font(
    cache: &mut std::collections::HashMap<String, FontData>,
    key: String,
    value: FontData,
) -> FontData {
    if cache.values().map(|(bytes, _)| bytes.len()).sum::<usize>() + value.0.len()
        > 128 * 1024 * 1024
    {
        cache.clear();
    }
    cache.insert(key, value.clone());
    value
}
pub struct Renderer {
    fonts: Vec<FontData>,
    system: fontdb::Database,
    font_cache: parking_lot::Mutex<std::collections::HashMap<String, FontData>>,
    measurements: parking_lot::Mutex<std::collections::HashMap<String, (f32, f32)>>,
}
struct Outline(PathBuilder);
impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, -y)
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, -y)
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, -y1, x, -y)
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, -y1, x2, -y2, x, -y)
    }
    fn close(&mut self) {
        self.0.close()
    }
}
pub fn color(s: &str) -> [u8; 3] {
    let s = s.trim_start_matches('#');
    if s.len() == 6
        && let Ok(v) = u32::from_str_radix(s, 16)
    {
        return [(v >> 16) as u8, (v >> 8) as u8, v as u8];
    }
    [32, 32, 32]
}

/// Caption formatting for newly generated translations only.
pub fn lettering_text(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let chars = normalized.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut pending_break = false;
    let mut double_quote_open = false;
    let mut single_quote_open = false;
    for (i, &c) in chars.iter().enumerate() {
        let previous = i.checked_sub(1).and_then(|n| chars.get(n)).copied();
        let next = chars.get(i + 1).copied();
        let word_apostrophe = c == '\''
            && previous.is_some_and(char::is_alphanumeric)
            && next.is_some_and(char::is_alphanumeric);
        let numeric = matches!(c, '.' | ',' | '．' | '，')
            && previous.is_some_and(char::is_numeric)
            && next.is_some_and(char::is_numeric);
        let ellipsis = matches!(c, '.' | '．') && (previous == Some(c) || next == Some(c));
        if matches!(c, '.' | ',' | '。' | '，' | '．' | '｡' | '،' | '۔') && !numeric && !ellipsis
        {
            while output
                .chars()
                .last()
                .is_some_and(|c| c.is_whitespace() && c != '\n')
            {
                output.pop();
            }
            pending_break = true;
            continue;
        }
        if c == '\n' {
            if !pending_break || !output.ends_with('\n') {
                output.push('\n');
            }
            pending_break = false;
            continue;
        }
        if pending_break {
            if c.is_whitespace() {
                continue;
            }
            // A closing quote/bracket stays with the sentence that it closes.
            let closing = "”’」』）》】〕〉］｝)]}!！?？".contains(c)
                || (c == '"' && double_quote_open)
                || (c == '\'' && single_quote_open && !word_apostrophe);
            if !closing {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                pending_break = false;
            }
        }
        output.push(c);
        if c == '"' {
            double_quote_open = !double_quote_open;
        } else if c == '\'' && !word_apostrophe {
            single_quote_open = !single_quote_open;
        }
    }
    output.trim().to_owned()
}

/// Rendering honors saved punctuation and explicit breaks. CRLF is one break.
pub fn region_lettering_text(region: &Region) -> String {
    region.target.replace("\r\n", "\n").replace('\r', "\n")
}

/// Format a newly accepted machine translation once, before publishing its target.
pub fn format_translation(region: &Region, target: &str) -> String {
    if dialogue_region(region) {
        dialogue_lettering_text(target)
    } else {
        lettering_text(target)
    }
}

fn dialogue_region(region: &Region) -> bool {
    region.bubble.is_some() || !matches!(region.kind.as_str(), "free" | "free_text")
}

pub fn vertical_lettering(region: &Region, target_lang: &str) -> bool {
    region.direction == "vertical"
        || (region.direction == "auto"
            && (target_lang.starts_with("zh") || target_lang == "ja")
            && region.bbox[3] - region.bbox[1] > region.bbox[2] - region.bbox[0])
}

pub fn dialogue_lettering_text(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let units = lettering_units(&normalized);
    let mut words = normalized.unicode_word_indices().peekable();
    let mut at = 0;
    let mut output = String::new();
    let mut pending_break = false;
    let mut double_quote_open = false;
    let mut single_quote_open = false;
    for (i, &unit) in units.iter().enumerate() {
        while words
            .peek()
            .is_some_and(|(start, word)| start + word.len() <= at)
        {
            words.next();
        }
        let inside_word = words
            .peek()
            .is_some_and(|(start, word)| *start < at && at + unit.len() < start + word.len());
        at += unit.len();
        let c = unit.chars().next().unwrap();
        if unit == "\n" {
            if !pending_break || !output.ends_with('\n') {
                output.push('\n');
            }
            pending_break = false;
            continue;
        }
        if unit.chars().all(char::is_whitespace) {
            if !pending_break {
                output.push_str(unit);
            }
            continue;
        }
        let previous = i.checked_sub(1).and_then(|n| units.get(n));
        let next = units.get(i + 1);
        let numeric = ".,．，٫٬:：/／-−".contains(c)
            && previous.is_some_and(|s| s.chars().last().is_some_and(char::is_numeric))
            && next.is_some_and(|s| s.chars().next().is_some_and(char::is_numeric));
        let internal_hyphen = "-‐‑".contains(c)
            && previous.is_some_and(|s| s.chars().any(word_letter))
            && next.is_some_and(|s| s.chars().any(word_letter));
        let kind = punctuation_kind(unit);
        let ellipsis = kind == 1 && (unit.chars().count() > 1 || !".．".contains(c));
        // UAX word boundaries protect apostrophes/initialisms/connector marks,
        // but sentence separators between CJK clauses do not belong to a word.
        let word_punctuation = inside_word && ".'’·_‿⁀．".contains(c);
        let protected = !ellipsis && kind != 2 && (numeric || internal_hyphen || word_punctuation);
        let closing = !protected
            && (matches!(
                c.general_category(),
                Category::ClosePunctuation | Category::FinalPunctuation
            ) || (c == '"' && double_quote_open)
                || (c == '\'' && single_quote_open));
        let opening = !protected
            && (matches!(
                c.general_category(),
                Category::OpenPunctuation | Category::InitialPunctuation
            ) || (c == '"' && !double_quote_open)
                || (c == '\'' && !single_quote_open));
        let enumeration = "、､﹑".contains(c);
        let inline = matches!(c, '·' | '=');
        let punctuation = unit.chars().any(|c| {
            matches!(
                c.general_category_group(),
                Group::Punctuation | Group::Symbol
            )
        }) || unit.contains('\u{20e3}'); // keycap emoji
        let remove = !protected && !ellipsis && ".,。，．｡،۔﹐﹒".contains(c);
        if remove {
            while output
                .chars()
                .last()
                .is_some_and(|c| c.is_whitespace() && c != '\n')
            {
                output.pop();
            }
            pending_break = true;
            continue;
        }
        let boundary = punctuation && !protected && !opening && !closing && !enumeration && !inline;
        // Keep closing brackets and complete punctuation runs attached. An
        // opening bracket begins the following phrase, never an empty column.
        if pending_break && !(closing || boundary || enumeration || inline) {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push('\n');
            }
            pending_break = false;
        }
        output.push_str(unit);
        if boundary {
            pending_break = true;
        }
        if !protected {
            if c == '"' {
                double_quote_open = !double_quote_open;
            } else if c == '\'' {
                single_quote_open = !single_quote_open;
            }
        }
    }
    output.trim().to_owned()
}

fn punctuation_kind(g: &str) -> u8 {
    if g.chars().all(|c| "….．⋯︙⋮".contains(c)) {
        1
    } else if g.chars().all(|c| "—―⸺⸻︱︲".contains(c)) {
        2
    } else {
        0
    }
}

/// Atomic runs are also used when choosing wrapping boundaries. A decimal dot
/// remains a single ordinary glyph; only adjacent dots form an ellipsis run.
fn lettering_units(text: &str) -> Vec<&str> {
    let mut units = Vec::<&str>::new();
    let mut begin = 0;
    let mut previous = 0;
    for (at, g) in text.grapheme_indices(true) {
        let kind = punctuation_kind(g);
        if at > begin && (kind == 0 || kind != previous) {
            units.push(&text[begin..at]);
            begin = at;
        }
        previous = kind;
    }
    if begin < text.len() {
        units.push(&text[begin..]);
    }
    units
}

fn latin_grapheme(g: &str) -> bool {
    g.chars().next().is_some_and(|c| {
        c.is_alphabetic()
            && (c <= '\u{024f}'
                || ('\u{1e00}'..='\u{1eff}').contains(&c)
                || ('\u{2c60}'..='\u{2c7f}').contains(&c)
                || ('\u{ff21}'..='\u{ff5a}').contains(&c))
    })
}

fn word_letter(c: char) -> bool {
    c.is_alphabetic() && !matches!(c as u32, 0x2e80..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af)
}

// Shaping still sees individual graphemes and vertical forms. Wrapping must
// treat adjacent punctuation as one unit and attach closing brackets.
fn wrapping_units(text: &str, dialogue: bool) -> Vec<&str> {
    let units = lettering_units(text);
    if !dialogue {
        return units;
    }
    let mut result = Vec::new();
    let mut start = 0;
    let mut end = 0;
    let mut previous_punctuation = false;
    let mut double_open = false;
    let mut single_open = false;
    for (i, &unit) in units.iter().enumerate() {
        let c = unit.chars().next().unwrap();
        let apostrophe = c == '\''
            && i > 0
            && units[i - 1].chars().any(word_letter)
            && units.get(i + 1).is_some_and(|s| s.chars().any(word_letter));
        let closing_quote = c == '"' && double_open || c == '\'' && single_open && !apostrophe;
        let closing = matches!(
            c.general_category(),
            Category::ClosePunctuation | Category::FinalPunctuation
        ) || closing_quote;
        let punctuation = !"、､﹑·=".contains(c)
            && !apostrophe
            && (!(c == '"' || c == '\'') || closing_quote)
            && !matches!(
                c.general_category(),
                Category::OpenPunctuation | Category::InitialPunctuation
            )
            && (matches!(
                c.general_category_group(),
                Group::Punctuation | Group::Symbol
            ) || unit.contains('\u{20e3}'));
        if end > start && !(closing || punctuation && previous_punctuation) || unit == "\n" {
            if end > start {
                result.push(&text[start..end]);
            }
            start = end;
        }
        end += unit.len();
        if unit == "\n" {
            result.push(&text[start..end]);
            start = end;
        }
        previous_punctuation = punctuation;
        if c == '"' {
            double_open = !double_open;
        }
        if c == '\'' && !apostrophe {
            single_open = !single_open;
        }
    }
    if end > start {
        result.push(&text[start..end]);
    }
    result
}

fn path_bounds(paths: &[tiny_skia::Path]) -> (f32, f32, f32, f32) {
    paths
        .iter()
        .fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |b, p| {
            let r = p.bounds();
            (
                b.0.min(r.left()),
                b.1.min(r.top()),
                b.2.max(r.right()),
                b.3.max(r.bottom()),
            )
        })
}

impl Renderer {
    pub fn new(folder: &Path) -> Result<Self> {
        let mut fonts = Vec::new();
        for (name, index) in [
            ("NotoSansCJK-Regular.ttc", 2),
            ("NotoSansArabic.ttf", 0),
            ("NotoSansDevanagari.ttf", 0),
        ] {
            fonts.push((
                std::sync::Arc::new(std::fs::read(folder.join(name))?),
                index,
            ))
        }
        let mut system = fontdb::Database::new();
        system.load_system_fonts();
        Ok(Self {
            fonts,
            system,
            font_cache: Default::default(),
            measurements: Default::default(),
        })
    }
    pub fn font_names(&self) -> Vec<String> {
        let mut names = self
            .system
            .faces()
            .flat_map(|f| f.families.iter().map(|(n, _)| n.clone()))
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        names
    }
    fn font(&self, text: &str, custom: &Option<String>) -> (std::sync::Arc<Vec<u8>>, u32) {
        if let Some(name) = custom {
            let key = if let Ok(meta) = std::fs::metadata(name) {
                format!("{name}:{}:{:?}", meta.len(), meta.modified().ok())
            } else {
                name.clone()
            };
            let mut cache = self.font_cache.lock();
            if let Some(value) = cache.get(&key) {
                return value.clone();
            }
            // Bound retained custom/system data; bundled fallback fonts live separately.
            if std::fs::metadata(name).is_ok_and(|m| m.is_file() && m.len() <= 64 * 1024 * 1024)
                && let Ok(data) = std::fs::read(name)
                && ttf_parser::Face::parse(&data, 0).is_ok()
            {
                let value = (std::sync::Arc::new(data), 0);
                return cache_font(&mut cache, key, value);
            }
            if let Some(id) = self.system.query(&fontdb::Query {
                families: &[fontdb::Family::Name(name)],
                ..Default::default()
            }) && let Some(f) = self
                .system
                .with_face_data(id, |b, i| {
                    (b.len() <= 64 * 1024 * 1024).then(|| (std::sync::Arc::new(b.to_vec()), i))
                })
                .flatten()
            {
                return cache_font(&mut cache, key, f);
            }
        }
        let index = if text.chars().any(|c| ('\u{0900}'..='\u{097f}').contains(&c)) {
            2
        } else if text.chars().any(|c| ('\u{0600}'..='\u{08ff}').contains(&c)) {
            1
        } else {
            0
        };
        self.fonts[index].clone()
    }
    fn measure(
        &self,
        text: &str,
        vertical: bool,
        size: f32,
        custom: &Option<String>,
    ) -> Result<(f32, f32)> {
        let stamp = custom
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| (m.len(), m.modified().ok()));
        let key = format!("{custom:?}:{stamp:?}:{vertical}:{}:{text}", size.to_bits());
        if let Some(value) = self.measurements.lock().get(&key).copied() {
            return Ok(value);
        }
        let (_, x, y) = self.shapes(text, vertical, size, custom)?;
        let mut cache = self.measurements.lock();
        // Only dimensions are retained; glyph outlines and large user strings are not cached.
        if cache.len() >= 2048 {
            cache.clear();
        }
        if text.len() <= 1024 {
            cache.insert(key, (x, y));
        }
        Ok((x, y))
    }
    fn shapes(
        &self,
        text: &str,
        vertical: bool,
        size: f32,
        custom: &Option<String>,
    ) -> Result<(Vec<tiny_skia::Path>, f32, f32)> {
        if vertical {
            let units = lettering_units(text);
            let mut paths = Vec::new();
            let mut cursor = 0.;
            let mut at = 0;
            while at < units.len() {
                let g = units[at];
                let numeric = g.chars().all(|c| c.is_ascii_digit());
                let ellipsis = punctuation_kind(g) == 1
                    && (g.chars().count() > 1 || g.chars().any(|c| "…⋯︙⋮".contains(c)));
                let dash = punctuation_kind(g) == 2;
                if ellipsis || dash {
                    let (shapes, height) = self.vertical_punctuation(g, size, custom, ellipsis);
                    paths.extend(
                        shapes
                            .into_iter()
                            .filter_map(|p| p.transform(Transform::from_translate(0., cursor))),
                    );
                    cursor += height;
                    at += 1;
                    continue;
                }
                let mut part = g.to_owned();
                at += 1;
                if numeric {
                    while at < units.len() && units[at].chars().all(|c| c.is_ascii_digit()) {
                        part.push_str(units[at]);
                        at += 1;
                    }
                } else if !latin_grapheme(g) {
                    while at < units.len()
                        && !latin_grapheme(units[at])
                        && punctuation_kind(units[at]) == 0
                        && !units[at].chars().all(|c| c.is_ascii_digit())
                    {
                        part.push_str(units[at]);
                        at += 1;
                    }
                }
                let upright = latin_grapheme(g) || (numeric && part.len() <= 2);
                let (shapes, advance, height) =
                    self.raw_shapes(&part, !upright && !numeric, size, custom)?;
                if shapes.is_empty() {
                    cursor += if upright { size } else { height };
                    continue;
                }
                let b = path_bounds(&shapes);
                let transform = if upright {
                    let scale = (size / advance.max(b.2 - b.0).max(1.)).min(1.);
                    Transform::from_row(
                        scale,
                        0.,
                        0.,
                        scale,
                        -(b.0 + b.2) * scale / 2.,
                        cursor + size / 2. - (b.1 + b.3) * scale / 2.,
                    )
                } else if numeric {
                    Transform::from_row(0., 1., -1., 0., (b.1 + b.3) / 2., cursor - b.0)
                } else {
                    Transform::from_translate(0., cursor)
                };
                paths.extend(shapes.into_iter().filter_map(|p| p.transform(transform)));
                cursor += if upright {
                    size
                } else if numeric {
                    advance
                } else {
                    height
                };
            }
            return Ok((paths, 0., cursor));
        }
        self.raw_shapes(text, false, size, custom)
    }

    fn vertical_punctuation(
        &self,
        text: &str,
        size: f32,
        custom: &Option<String>,
        ellipsis: bool,
    ) -> (Vec<tiny_skia::Path>, f32) {
        let count: usize = text
            .chars()
            .map(|c| match c {
                '…' | '⋯' | '︙' | '⋮' => 3,
                '⸺' => 2,
                '⸻' => 3,
                _ => 1,
            })
            .sum();
        let height = size * count as f32 / if ellipsis { 3. } else { 1. };
        let form = if ellipsis { "︙" } else { "︱" };
        if (!ellipsis || count.is_multiple_of(3))
            && let Ok((glyphs, _, advance)) = self.raw_shapes(form, true, size, custom)
            && !glyphs.is_empty()
        {
            let b = path_bounds(&glyphs);
            if b.3 - b.1 > (b.2 - b.0) * 2. && advance > 0. {
                let repeats = if ellipsis { count / 3 } else { 1 };
                let stretch = if ellipsis { 1. } else { count as f32 };
                let mut output = Vec::new();
                for i in 0..repeats {
                    for p in &glyphs {
                        if let Some(p) = p.clone().transform(Transform::from_row(
                            1.,
                            0.,
                            0.,
                            stretch,
                            -(b.0 + b.2) / 2.,
                            i as f32 * size,
                        )) {
                            output.push(p);
                        }
                    }
                }
                return (output, height);
            }
        }
        // A font without usable vertical punctuation must not leave horizontal
        // dots/dashes in the column. Draw only this punctuation, at em spacing.
        let mut builder = PathBuilder::new();
        if ellipsis {
            for i in 0..count {
                builder.push_circle(0., (i as f32 + 0.5) * size / 3., size * 0.065);
            }
        } else {
            builder.push_rect(
                tiny_skia::Rect::from_xywh(-size * 0.035, 0., size * 0.07, height).unwrap(),
            );
        }
        (builder.finish().into_iter().collect(), height)
    }

    fn raw_shapes(
        &self,
        text: &str,
        vertical: bool,
        size: f32,
        custom: &Option<String>,
    ) -> Result<(Vec<tiny_skia::Path>, f32, f32)> {
        let (data, index) = self.font(text, custom);
        let face = rustybuzz::Face::from_slice(&data, index).context("Invalid font")?;
        let bidi = unicode_bidi::BidiInfo::new(text, None);
        let mut paths = Vec::new();
        let mut x = 0f32;
        let mut y = 0f32;
        let runs: Vec<(String, bool)> = if vertical {
            vec![(text.to_owned(), false)]
        } else {
            let mut r = Vec::new();
            for para in &bidi.paragraphs {
                let (levels, ranges) = bidi.visual_runs(para, para.range.clone());
                for range in ranges {
                    r.push((text[range.clone()].to_owned(), levels[range.start].is_rtl()));
                }
            }
            r
        };
        let mut fallback_runs = Vec::new();
        for (run, rtl) in runs {
            let mut segments: Vec<(String, usize)> = Vec::new();
            for g in run.graphemes(true) {
                let supports = |f: &rustybuzz::Face<'_>| {
                    g.chars()
                        .all(|c| c.is_control() || c == '\u{200d}' || f.glyph_index(c).is_some())
                };
                let selected = if supports(&face) {
                    usize::MAX
                } else {
                    self.fonts
                        .iter()
                        .position(|(data, index)| {
                            rustybuzz::Face::from_slice(data, *index).is_some_and(|f| supports(&f))
                        })
                        .unwrap_or(usize::MAX)
                };
                if let Some((s, k)) = segments.last_mut()
                    && *k == selected
                {
                    s.push_str(g);
                    continue;
                }
                segments.push((g.into(), selected));
            }
            if rtl {
                segments.reverse();
            }
            for (segment, font) in segments {
                fallback_runs.push((segment, rtl, font));
            }
        }
        for (run, rtl, font) in fallback_runs {
            let face = if font == usize::MAX {
                rustybuzz::Face::from_slice(&data, index).unwrap()
            } else {
                rustybuzz::Face::from_slice(&self.fonts[font].0, self.fonts[font].1).unwrap()
            };
            let scale = size / face.units_per_em() as f32;
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(&run);
            buffer.guess_segment_properties();
            buffer.set_direction(if vertical {
                rustybuzz::Direction::TopToBottom
            } else if rtl {
                rustybuzz::Direction::RightToLeft
            } else {
                rustybuzz::Direction::LeftToRight
            });
            let glyphs = rustybuzz::shape(&face, &[], buffer);
            for (info, pos) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
                if info.glyph_id == 0 {
                    return Err(anyhow::anyhow!(
                        "Font lacks a required glyph; select another font"
                    ));
                }
                let mut builder = Outline(PathBuilder::new());
                if face
                    .outline_glyph(ttf_parser::GlyphId(info.glyph_id as u16), &mut builder)
                    .is_some()
                    && let Some(path) = builder.0.finish()
                    && let Some(path) = path.transform(Transform::from_row(
                        scale,
                        0.,
                        0.,
                        scale,
                        x + pos.x_offset as f32 * scale,
                        y - pos.y_offset as f32 * scale,
                    ))
                {
                    paths.push(path)
                }
                x += pos.x_advance as f32 * scale;
                y -= pos.y_advance as f32 * scale;
            }
        }
        Ok((paths, x.abs(), y.abs()))
    }
    pub fn lettering(&self, region: &Region, target_lang: &str) -> Result<Option<Pixmap>> {
        crate::safety::region(region)?;
        let b = region.bbox;
        let w = (b[2] - b[0]).max(1.) as u32;
        let h = (b[3] - b[1]).max(1.) as u32;
        let vertical = vertical_lettering(region, target_lang);
        let text = region_lettering_text(region);
        if text.is_empty() {
            return Ok(None);
        }
        let max = region.style.size.unwrap_or(48.).clamp(6., 120.);
        let candidates: Vec<f32> = if region.style.size.is_some() {
            vec![max]
        } else {
            (6..=max as u32).rev().map(|s| s as f32).collect()
        };
        for size in candidates {
            let outline = if region.style.outline_enabled {
                size * region.style.outline_width_percent / 100.
            } else {
                0.
            };
            // One extra pixel contains antialiasing. Disabled layout is unchanged.
            let halo = if region.style.outline_enabled {
                outline + 1.
            } else {
                0.
            };
            let available = if vertical { h } else { w } as f32 - 6. - 2. * halo;
            let mut lines = Vec::<String>::new();
            let mut line = String::new();
            for g in wrapping_units(&text, dialogue_region(region)) {
                if g == "\n" {
                    lines.push(std::mem::take(&mut line));
                    continue;
                }
                let candidate = format!("{line}{g}");
                let (x, y) = self.measure(&candidate, vertical, size, &region.style.font)?;
                if !line.is_empty() && if vertical { y } else { x } > available {
                    let word_break = if vertical {
                        None
                    } else {
                        unicode_linebreak::linebreaks(&line)
                            .map(|(i, _)| i)
                            .filter(|i| *i > 0 && *i < line.len())
                            .filter(|i| {
                                wrapping_units(&line, dialogue_region(region))
                                    .iter()
                                    .scan(0, |at, unit| {
                                        *at += unit.len();
                                        Some(*at)
                                    })
                                    .any(|at| at == *i)
                            })
                            .last()
                    };
                    if let Some(at) = word_break {
                        let next = line.split_off(at);
                        lines.push(std::mem::take(&mut line));
                        line = next;
                    } else if "，。！？、）》」』】.,!?;:".contains(g)
                        && wrapping_units(&line, dialogue_region(region)).len() > 1
                    {
                        let idx = line.len()
                            - wrapping_units(&line, dialogue_region(region))
                                .last()
                                .unwrap()
                                .len();
                        let last = line.split_off(idx);
                        lines.push(std::mem::take(&mut line));
                        line = last;
                    } else {
                        lines.push(std::mem::take(&mut line));
                    }
                }
                line.push_str(g);
            }
            if !line.is_empty() {
                lines.push(line)
            }
            let pitch = size * (1. + region.style.line_gap.max(0.)) + 2. * halo;
            let across = lines.len() as f32 * pitch;
            let breadth = if vertical { w } else { h } as f32 - 6.;
            if across > breadth {
                continue;
            }
            let mut placements = Vec::new();
            let mut fits = true;
            for (i, line) in lines.iter().enumerate() {
                let (paths, x, y) = self.shapes(line, vertical, size, &region.style.font)?;
                if if vertical { y } else { x } > available {
                    fits = false;
                    break;
                }
                if paths.is_empty() {
                    continue;
                }
                let bounds = paths
                    .iter()
                    .fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |b, p| {
                        let r = p.bounds();
                        (
                            b.0.min(r.left()),
                            b.1.min(r.top()),
                            b.2.max(r.right()),
                            b.3.max(r.bottom()),
                        )
                    });
                let bw = bounds.2 - bounds.0;
                let bh = bounds.3 - bounds.1;
                let (tx, ty) = if vertical {
                    (
                        (w as f32 + across) / 2. - (i as f32 + 0.5) * pitch - bw / 2. - bounds.0,
                        3. + halo - bounds.1,
                    )
                } else {
                    (
                        (w as f32 - bw) / 2. - bounds.0,
                        (h as f32 - across) / 2. + i as f32 * pitch + (pitch - bh) / 2. - bounds.1,
                    )
                };
                for p in paths {
                    placements.push((p, tx, ty));
                }
            }
            if !fits {
                continue;
            }
            if !placements.is_empty() {
                // Center one visible block, not each column independently. The
                // common translation preserves column tops, spacing and order.
                let bounds = placements.iter().fold(
                    (
                        f32::INFINITY,
                        f32::INFINITY,
                        f32::NEG_INFINITY,
                        f32::NEG_INFINITY,
                    ),
                    |b, (p, x, y)| {
                        let r = p.bounds();
                        (
                            b.0.min(r.left() + x - halo),
                            b.1.min(r.top() + y - halo),
                            b.2.max(r.right() + x + halo),
                            b.3.max(r.bottom() + y + halo),
                        )
                    },
                );
                let dx = w as f32 / 2. - (bounds.0 + bounds.2) / 2.;
                let dy = h as f32 / 2. - (bounds.1 + bounds.3) / 2.;
                // Retain the three-pixel inset after including stroke and AA.
                // Tolerate only floating-point roundoff at an exact edge.
                if bounds.0 + dx < 3. - 0.001
                    || bounds.1 + dy < 3. - 0.001
                    || bounds.2 + dx > w as f32 - 3. + 0.001
                    || bounds.3 + dy > h as f32 - 3. + 0.001
                {
                    continue;
                }
                for (_, x, y) in &mut placements {
                    *x += dx;
                    *y += dy;
                }
            }
            let mut pixmap = Pixmap::new(w, h).context("Invalid region size")?;
            if region.style.outline_enabled {
                let c = color(&region.style.outline_color);
                let mut paint = Paint::default();
                paint.set_color_rgba8(c[0], c[1], c[2], 255);
                let stroke = Stroke {
                    width: 2. * outline,
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..Default::default()
                };
                // Finish every outline before any fill, including overlapping glyphs.
                for (path, x, y) in &placements {
                    pixmap.stroke_path(
                        path,
                        &paint,
                        &stroke,
                        Transform::from_translate(*x, *y),
                        None,
                    );
                }
            }
            let c = color(&region.style.color);
            let mut paint = Paint::default();
            paint.set_color_rgba8(c[0], c[1], c[2], 255);
            for (path, x, y) in placements {
                pixmap.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::from_translate(x, y),
                    None,
                )
            }
            return Ok(Some(pixmap));
        }
        Err(anyhow::anyhow!(
            "Text overflow: enlarge region or reduce text/font size"
        ))
    }
    /// Historical solid-fill benchmark adapter. Production uses Engine::render_page.
    pub fn render_benchmark(
        &self,
        original: &DynamicImage,
        regions: &mut [Region],
        lang: &str,
        cleaned: Option<&DynamicImage>,
    ) -> Result<DynamicImage> {
        let original_rgb = original.to_rgb8();
        let mut out = cleaned
            .map(|im| im.to_rgb8())
            .unwrap_or_else(|| original_rgb.clone());
        let bubbles: Vec<_> = regions.iter().filter_map(|r| r.bubble).collect();
        for r in regions {
            if r.target.trim().is_empty() {
                continue;
            }
            let cleanup = if cleaned.is_none() && !r.overlay_only {
                Some(cleanup::analyze_with_bubbles(&original_rgb, r, &bubbles))
            } else {
                None
            };
            let mut lettering_region = r.clone();
            if let Some(ref result) = cleanup {
                let foreground = color(&r.style.color);
                if !r.allow_fill && (0..3).all(|c| foreground[c].abs_diff(result.fill[c]) < 55) {
                    // Resolve poor automatic contrast without changing the saved text style.
                    let luminance = result.fill[0] as u32 * 299
                        + result.fill[1] as u32 * 587
                        + result.fill[2] as u32 * 114;
                    lettering_region.style.color = if luminance < 128_000 {
                        "#ffffff"
                    } else {
                        "#000000"
                    }
                    .into();
                }
            }
            let letters = match self.lettering(&lettering_region, lang) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    r.review =
                        Some("No lettering remains after formatting; original preserved".into());
                    continue;
                }
                Err(e) => {
                    r.review = Some(e.to_string());
                    continue;
                }
            };
            if let Some(result) = cleanup {
                for (x, y, p) in result.mask.enumerate_pixels() {
                    if p[0] > 0 {
                        out.put_pixel(x + result.origin[0], y + result.origin[1], Rgb(result.fill));
                    }
                }
            }
            let x = r.bbox[0].max(0.) as u32;
            let y = r.bbox[1].max(0.) as u32;
            for (iy, row) in letters
                .data()
                .chunks(letters.width() as usize * 4)
                .enumerate()
            {
                for (ix, p) in row.chunks(4).enumerate() {
                    if x + ix as u32 >= out.width() || y + iy as u32 >= out.height() {
                        continue;
                    }
                    let dst = out.get_pixel_mut(x + ix as u32, y + iy as u32);
                    let a = p[3] as u32;
                    for c in 0..3 {
                        dst[c] = (p[c] as u32 + dst[c] as u32 * (255 - a) / 255).min(255) as u8;
                    }
                }
            }
            r.review = None;
        }
        Ok(DynamicImage::ImageRgb8(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn renderer() -> Renderer {
        Renderer::new(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts")).unwrap()
    }

    #[test]
    fn punctuation_runs_and_combining_marks_are_atomic() {
        assert_eq!(
            wrapping_units("\"Hi\"\n'好！'", true),
            ["\"", "H", "i\"", "\n", "'", "好", "！'"]
        );
        assert_eq!(
            wrapping_units("甲！？）\n乙❤️……』\n丙", true),
            ["甲", "！？）", "\n", "乙", "❤️……』", "\n", "丙"]
        );
        assert_eq!(
            lettering_units("甲……乙...丙——丁e\u{301}、"),
            ["甲", "……", "乙", "...", "丙", "——", "丁", "e\u{301}", "、"]
        );
        assert_eq!(
            lettering_text("按照 A、B、C、D、E 的五级分级，将怪物运送到与其等级相符的地点。"),
            "按照 A、B、C、D、E 的五级分级\n将怪物运送到与其等级相符的地点"
        );
    }

    #[test]
    fn vertical_latin_is_upright_and_stacked_with_short_numbers_preserved() {
        let r = renderer();
        let (horizontal, _, _) = r.shapes("F", false, 32., &None).unwrap();
        let (vertical, _, advance) = r.shapes("FF", true, 32., &None).unwrap();
        assert_eq!(advance, 64.);
        assert_eq!(vertical.len(), 2);
        let b = horizontal[0].bounds();
        for p in &vertical {
            assert!(
                (p.bounds().width() / p.bounds().height() - b.width() / b.height()).abs() < 0.001
            );
        }
        assert!((vertical[1].bounds().top() - vertical[0].bounds().top() - 32.).abs() < 0.01);
        assert_eq!(r.shapes("12", true, 32., &None).unwrap().2, 32.);
        assert_eq!(r.shapes("Ée\u{301}", true, 32., &None).unwrap().2, 64.);
    }

    #[test]
    fn vertical_ellipses_and_dashes_do_not_split_or_rotate_twice() {
        let r = renderer();
        for text in ["……", "...", "．．．", "——", "―"] {
            let (paths, _, advance) = r.shapes(text, true, 32., &None).unwrap();
            let b = path_bounds(&paths);
            assert!(b.3 - b.1 > (b.2 - b.0) * 2., "{text}");
            assert!(advance >= 32.);
        }
        let mut region = Region {
            bbox: [0., 0., 150., 50.],
            direction: "vertical".into(),
            target: "……".into(),
            ..Default::default()
        };
        region.style.size = Some(32.);
        assert!(
            r.lettering(&region, "zh-Hans")
                .unwrap_err()
                .to_string()
                .contains("overflow")
        );
        region.style.size = None;
        assert!(r.lettering(&region, "zh-Hans").unwrap().is_some());
        assert_eq!(region.target, "……");
        region.direction = "horizontal".into();
        region.style.size = Some(32.);
        region.bbox = [0., 0., 50., 150.];
        assert!(r.lettering(&region, "zh-Hans").is_err());
    }
    #[test]
    fn sentence_punctuation_becomes_breaks_without_changing_numbers_or_ellipses() {
        for (input, expected) in [
            ("你好，世界。再见。", "你好\n世界\n再见"),
            ("Hello, world. Goodbye.", "Hello\nworld\nGoodbye"),
            ("甲、乙｡丙､丁．戊", "甲、乙\n丙､丁\n戊"),
            (
                "3.14, 1,234.50。１２．３４，５６",
                "3.14\n1,234.50\n１２．３４，５６",
            ),
            ("等等...再说……好，嗯．．．", "等等...再说……好\n嗯．．．"),
            ("「你好。」她说，『再见。』", "「你好」\n她说\n『再见』"),
            ("\"Hello.\" Next.", "\"Hello\"\nNext"),
            ("He said, \"Hello.\"", "He said\n\"Hello\""),
            ("He said,'Hello.' Next.", "He said\n'Hello'\nNext"),
            ("Don't go, I'm here.", "Don't go\nI'm here"),
            ("甲，。 乙。\r\n丙\n\n丁", "甲\n乙\n丙\n\n丁"),
            ("你好！再见？", "你好！再见？"),
            ("a،b۔c", "a\nb\nc"),
            (" ，。 ", ""),
        ] {
            assert_eq!(lettering_text(input), expected, "{input:?}");
        }
    }

    #[test]
    fn blank_open_background_does_not_block_cleanup() {
        let im = DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 100, Rgb([255, 255, 255])));
        let r = Region {
            bbox: [20., 20., 80., 80.],
            ..Default::default()
        };
        let result = cleanup::analyze(&im.to_rgb8(), &r);
        assert_eq!(result.fill, [255, 255, 255]);
        assert!(result.mask.as_raw().iter().all(|&v| v == 0));
    }
    #[test]
    fn font_data_and_fitting_measurements_are_reused() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
        let renderer = Renderer::new(&root).unwrap();
        let custom = Some(
            root.join("NotoSansCJK-Regular.ttc")
                .to_string_lossy()
                .into_owned(),
        );
        // Use the actual bundled CJK filename, independent of the caller's working directory.
        let custom = if Path::new(custom.as_ref().unwrap()).is_file() {
            custom
        } else {
            std::fs::read_dir(&root)
                .unwrap()
                .filter_map(|e| e.ok())
                .find(|e| e.file_name().to_string_lossy().contains("CJK"))
                .map(|e| e.path().to_string_lossy().into_owned())
        };
        assert!(custom.is_some());
        let first = renderer.font("中文", &custom);
        for _ in 0..50 {
            assert!(std::sync::Arc::ptr_eq(
                &first.0,
                &renderer.font("中文", &custom).0
            ));
        }
        let measured = renderer.measure("A、B……中文", true, 30., &custom).unwrap();
        assert_eq!(
            renderer.measure("A、B……中文", true, 30., &custom).unwrap(),
            measured
        );
        assert_eq!(renderer.measurements.lock().len(), 1);
        assert_eq!(renderer.font_cache.lock().len(), 1);
    }
}
