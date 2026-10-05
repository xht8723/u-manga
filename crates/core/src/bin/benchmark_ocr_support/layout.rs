//! Fixed, benchmark-only crop/layout rules. These functions see pixels and
//! detector geometry, never a page ID, transcript, reference category or model
//! prediction. Freeze this source hash before evaluating their OCR outputs.
//!
//! Limits are intentional: boundary repair follows connected foreground only;
//! it does not reconstruct an absent line. Ruby classification requires a
//! narrow, elongated side strip beside a substantially wider body line. A
//! short mark or an ambiguous fragment is retained. Grouping does not remove
//! ruby pixels already inside a body crop and cannot identify a glyph's meaning.

use super::segmentation::{Segment, bbox_quad, quad_bbox};
use image::{DynamicImage, GenericImageView};
use serde::Serialize;

pub const RULES_VERSION: &str = "layout-v1";

// Crop repair: at most 12% of the shorter crop side, constrained to 8..32 px.
// RGB foreground contrast is symmetric, so light/colored lettering is allowed.
const REPAIR_FRACTION: f32 = 0.12;
const REPAIR_MIN: u32 = 8;
const REPAIR_MAX: u32 = 32;
const FOREGROUND_CONTRAST: u8 = 52;
const BACKGROUND_TOLERANCE: u8 = 28;
const BACKGROUND_AGREEMENT: f32 = 0.60;
const REPAIR_PADDING: u32 = 2;
const MAX_REPAIR_PIXELS: usize = 12_000_000;

// The 70th area-weighted percentile avoids allowing numerous tiny ruby boxes
// to define the body font size. Ruby must cover >=70% of its own length beside
// a body line, be <=58% of its width, and span at least one body glyph pitch.
const BODY_WIDTH_QUANTILE: f32 = 0.70;
const RUBY_WIDTH_RATIO: f32 = 0.58;
const RUBY_MIN_ASPECT: f32 = 1.8;
const RUBY_MIN_OVERLAP: f32 = 0.70;
const RUBY_MAX_GAP: f32 = 0.75;
const FRAGMENT_MAX_GAP: f32 = 1.5;
const MERGE_MIN_CROSS_OVERLAP: f32 = 0.60;
const MAX_AXIS_SKEW: f32 = 0.12;

// Estimated characters are line length/body width, not recognized tokens.
// Many short dialogue columns alone do not activate the line-recognition route.
const DENSE_MANY_LINES: usize = 5;
const DENSE_MANY_CHARACTERS: f32 = 96.;
const DENSE_FEW_LINES: usize = 3;
const DENSE_FEW_CHARACTERS: f32 = 144.;
const DENSE_SINGLE_CHARACTERS: f32 = 180.;

#[derive(Debug, Clone, Serialize)]
pub struct CropDecision {
    pub bounds: [u32; 4],
    pub reasons: Vec<String>,
}

pub struct LayoutDecision {
    pub lines: Vec<Segment>,
    pub suppressed: Vec<Segment>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteDecision {
    pub use_lines: bool,
    pub estimated_characters: f32,
    pub body_lines: usize,
    pub reason: String,
}

fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.)
}

fn finite_box(b: [f32; 4]) -> bool {
    b.iter().all(|v| v.is_finite()) && b[2] > b[0] && b[3] > b[1]
}

fn color_distance(a: [u8; 3], b: [u8; 3]) -> u8 {
    (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0)
}

fn rgb(im: &DynamicImage, x: u32, y: u32) -> [u8; 3] {
    let p = im.get_pixel(x, y);
    [p[0], p[1], p[2]]
}

fn border_background(im: &DynamicImage, b: [u32; 4]) -> Option<[u8; 3]> {
    let mut samples = Vec::new();
    let sx = ((b[2] - b[0]) / 64).max(1) as usize;
    let sy = ((b[3] - b[1]) / 64).max(1) as usize;
    for offset in [0, 3] {
        let top = b[1].saturating_sub(offset);
        let bottom = (b[3] - 1 + offset).min(im.height() - 1);
        for x in (b[0]..b[2]).step_by(sx) {
            samples.push(rgb(im, x, top));
            samples.push(rgb(im, x, bottom));
        }
        let left = b[0].saturating_sub(offset);
        let right = (b[2] - 1 + offset).min(im.width() - 1);
        for y in (b[1]..b[3]).step_by(sy) {
            samples.push(rgb(im, left, y));
            samples.push(rgb(im, right, y));
        }
    }
    let median = std::array::from_fn(|c| {
        let mut values = samples.iter().map(|p| p[c]).collect::<Vec<_>>();
        values.sort_unstable();
        values[values.len() / 2]
    });
    let agreement = samples
        .iter()
        .filter(|p| color_distance(**p, median) <= BACKGROUND_TOLERANCE)
        .count() as f32
        / samples.len() as f32;
    (agreement >= BACKGROUND_AGREEMENT).then_some(median)
}

/// Bounds are exclusive at right/bottom. `neighbors` must omit this region.
/// Existing overlap is not removed, but expansion cannot add a neighbor's area.
pub fn repair_crop(im: &DynamicImage, bounds: [u32; 4], neighbors: &[[f32; 4]]) -> CropDecision {
    let original = [
        bounds[0].min(im.width()),
        bounds[1].min(im.height()),
        bounds[2].min(im.width()),
        bounds[3].min(im.height()),
    ];
    let mut result = CropDecision {
        bounds: original,
        reasons: Vec::new(),
    };
    if original[0] >= original[2] || original[1] >= original[3] {
        result
            .reasons
            .push("invalid or empty crop; no repair".into());
        return result;
    }
    let limit = (((original[2] - original[0]).min(original[3] - original[1]) as f32
        * REPAIR_FRACTION)
        .ceil() as u32)
        .clamp(REPAIR_MIN, REPAIR_MAX);
    let mut search = [
        original[0].saturating_sub(limit),
        original[1].saturating_sub(limit),
        original[2].saturating_add(limit).min(im.width()),
        original[3].saturating_add(limit).min(im.height()),
    ];
    let unconstrained = search;
    for n in neighbors.iter().copied().filter(|b| finite_box(*b)) {
        if overlap(search[1] as f32, search[3] as f32, n[1], n[3]) > 0. {
            if n[0] < original[0] as f32 && n[2] > search[0] as f32 {
                search[0] = search[0].max((n[2].ceil() as u32).min(original[0]));
            }
            if n[2] > original[2] as f32 && n[0] < search[2] as f32 {
                search[2] = search[2].min((n[0].floor().max(0.) as u32).max(original[2]));
            }
        }
        if overlap(search[0] as f32, search[2] as f32, n[0], n[2]) > 0. {
            if n[1] < original[1] as f32 && n[3] > search[1] as f32 {
                search[1] = search[1].max((n[3].ceil() as u32).min(original[1]));
            }
            if n[3] > original[3] as f32 && n[1] < search[3] as f32 {
                search[3] = search[3].min((n[1].floor().max(0.) as u32).max(original[3]));
            }
        }
    }
    if search != unconstrained {
        result
            .reasons
            .push("repair search limited by neighboring regions".into());
    }
    let Some(background) = border_background(im, original) else {
        result
            .reasons
            .push("variable boundary background; keep frozen crop".into());
        return result;
    };
    let width = (search[2] - search[0]) as usize;
    let height = (search[3] - search[1]) as usize;
    let Some(count) = width
        .checked_mul(height)
        .filter(|n| *n <= MAX_REPAIR_PIXELS)
    else {
        result
            .reasons
            .push("crop exceeds bounded repair workspace".into());
        return result;
    };
    let mut ink = vec![0u8; count];
    for y in 0..height {
        for x in 0..width {
            ink[y * width + x] = u8::from(
                color_distance(
                    rgb(im, search[0] + x as u32, search[1] + y as u32),
                    background,
                ) >= FOREGROUND_CONTRAST,
            );
        }
    }
    let mut seeds = Vec::new();
    for x in original[0]..original[2] {
        for y in [
            original[1],
            (original[1] + 1).min(original[3] - 1),
            original[3] - 1,
        ] {
            seeds.push((y - search[1]) as usize * width + (x - search[0]) as usize);
        }
    }
    for y in original[1]..original[3] {
        for x in [
            original[0],
            (original[0] + 1).min(original[2] - 1),
            original[2] - 1,
        ] {
            seeds.push((y - search[1]) as usize * width + (x - search[0]) as usize);
        }
    }
    let mut queue = Vec::new();
    let mut limited = false;
    let mut border_like = false;
    for seed in seeds {
        if ink[seed] != 1 {
            continue;
        }
        queue.clear();
        queue.push(seed);
        ink[seed] = 2;
        let mut head = 0;
        let mut component = [u32::MAX, u32::MAX, 0, 0];
        while head < queue.len() {
            let index = queue[head];
            head += 1;
            let x = index % width;
            let y = index / width;
            let ax = search[0] + x as u32;
            let ay = search[1] + y as u32;
            component[0] = component[0].min(ax);
            component[1] = component[1].min(ay);
            component[2] = component[2].max(ax + 1);
            component[3] = component[3].max(ay + 1);
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let next = ny * width + nx;
                    if ink[next] == 1 {
                        ink[next] = 2;
                        queue.push(next);
                    }
                }
            }
        }
        let cw = (component[2] - component[0]) as f32;
        let ch = (component[3] - component[1]) as f32;
        let crop_extent = (original[2] - original[0]).max(original[3] - original[1]) as f32;
        if cw.max(ch) > crop_extent * 0.65 && cw.min(ch) < (cw.max(ch) * 0.08).max(3.) {
            border_like = true;
            continue;
        }
        for side in 0..4 {
            let crosses = if side < 2 {
                component[side] < original[side]
            } else {
                component[side] > original[side]
            };
            if !crosses {
                continue;
            }
            let vertical_edge = side % 2 == 0;
            let (parallel, perpendicular, edge_length) = if vertical_edge {
                (ch, cw, (original[3] - original[1]) as f32)
            } else {
                (cw, ch, (original[2] - original[0]) as f32)
            };
            if parallel > edge_length * 0.65 && perpendicular < (parallel * 0.08).max(3.) {
                border_like = true;
                continue;
            }
            if component[side] == search[side]
                && search[side]
                    != if side < 2 {
                        0
                    } else if side == 2 {
                        im.width()
                    } else {
                        im.height()
                    }
            {
                limited = true;
                continue;
            }
            if side < 2 {
                result.bounds[side] = result.bounds[side].min(
                    component[side]
                        .saturating_sub(REPAIR_PADDING)
                        .max(search[side]),
                );
            } else {
                result.bounds[side] = result.bounds[side].max(
                    component[side]
                        .saturating_add(REPAIR_PADDING)
                        .min(search[side]),
                );
            }
        }
    }
    for (side, name) in ["left", "top", "right", "bottom"].iter().enumerate() {
        if result.bounds[side] != original[side] {
            result.reasons.push(format!(
                "{name}: recovered boundary-connected stroke, expanded {} px",
                result.bounds[side].abs_diff(original[side])
            ));
        }
    }
    if limited {
        result.reasons.push(
            "some boundary strokes exceed the bounded search; no guessed continuation".into(),
        );
    }
    if border_like {
        result
            .reasons
            .push("ignored long thin boundary components consistent with borders".into());
    }
    if result.reasons.is_empty() {
        result
            .reasons
            .push("no recoverable boundary-connected stroke".into());
    }
    result
}

#[derive(Clone, Copy)]
struct Axial {
    cross0: f32,
    cross1: f32,
    start: f32,
    end: f32,
}
impl Axial {
    fn width(self) -> f32 {
        self.cross1 - self.cross0
    }
    fn length(self) -> f32 {
        self.end - self.start
    }
    fn center(self) -> f32 {
        (self.cross0 + self.cross1) * 0.5
    }
}

fn axial(b: [f32; 4], vertical: bool) -> Axial {
    if vertical {
        Axial {
            cross0: b[0],
            cross1: b[2],
            start: b[1],
            end: b[3],
        }
    } else {
        Axial {
            cross0: b[1],
            cross1: b[3],
            start: b[0],
            end: b[2],
        }
    }
}

fn vertical_layout(lines: &[Segment], orientation: &str) -> bool {
    if orientation == "vertical" {
        return true;
    }
    if orientation == "horizontal" {
        return false;
    }
    let mut votes = [0f32; 2];
    for line in lines {
        let b = quad_bbox(&line.quad);
        if !finite_box(b) {
            continue;
        }
        let w = b[2] - b[0];
        let h = b[3] - b[1];
        if w.max(h) >= w.min(h) * 1.5 {
            votes[usize::from(h > w)] += w * h;
        }
    }
    votes[1] > votes[0]
}

fn aligned(line: &Segment) -> bool {
    let q = line.quad;
    let b = quad_bbox(&q);
    finite_box(b)
        && (q[0][1] - q[1][1]).abs().max((q[3][1] - q[2][1]).abs()) <= (b[2] - b[0]) * MAX_AXIS_SKEW
        && (q[0][0] - q[3][0]).abs().max((q[1][0] - q[2][0]).abs()) <= (b[3] - b[1]) * MAX_AXIS_SKEW
}

fn body_width(lines: &[Segment], vertical: bool) -> f32 {
    let mut candidates = lines
        .iter()
        .filter(|line| aligned(line))
        .filter_map(|line| {
            let a = axial(quad_bbox(&line.quad), vertical);
            (a.width() >= 3. && a.length() >= a.width() * 1.35)
                .then_some((a.width(), a.width() * a.length()))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return 0.;
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let target = candidates.iter().map(|x| x.1).sum::<f32>() * BODY_WIDTH_QUANTILE;
    let mut weight = 0.;
    for (width, area) in &candidates {
        weight += area;
        if weight >= target {
            return *width;
        }
    }
    candidates.last().unwrap().0
}

fn ruby_parent(index: usize, lines: &[Segment], vertical: bool, pitch: f32) -> Option<usize> {
    if pitch <= 0. || !aligned(&lines[index]) {
        return None;
    }
    let small = axial(quad_bbox(&lines[index].quad), vertical);
    if small.width() > pitch * RUBY_WIDTH_RATIO
        || small.length() < small.width() * RUBY_MIN_ASPECT
        || small.length() < pitch
    {
        return None;
    }
    let mut candidates = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i == index || !aligned(line) {
            continue;
        }
        let body = axial(quad_bbox(&line.quad), vertical);
        if body.width() < pitch * 0.75
            || small.width() > body.width() * RUBY_WIDTH_RATIO
            || overlap(body.start, body.end, small.start, small.end)
                < small.length() * RUBY_MIN_OVERLAP
        {
            continue;
        }
        let gap = if vertical {
            small.cross0 - body.cross1
        } else {
            body.cross0 - small.cross1
        };
        if gap >= -body.width() * 0.25 && gap <= pitch * RUBY_MAX_GAP {
            candidates.push((i, gap.abs()));
        }
    }
    candidates.sort_by(|a, b| a.1.total_cmp(&b.1));
    let &(index, gap) = candidates.first()?;
    // Equally plausible parents mean uncertain geometry: keep the fragment.
    if candidates
        .get(1)
        .is_some_and(|next| next.1 - gap < pitch * 0.10)
    {
        None
    } else {
        Some(index)
    }
}

fn clipped_segment(line: &Segment, bounds: [u32; 4]) -> Option<Segment> {
    if !aligned(line) {
        return None;
    }
    let b = quad_bbox(&line.quad);
    let b = [
        b[0].max(bounds[0] as f32),
        b[1].max(bounds[1] as f32),
        b[2].min(bounds[2] as f32),
        b[3].min(bounds[3] as f32),
    ];
    finite_box(b).then(|| Segment {
        quad: bbox_quad(b),
        vertical: line.vertical,
        score: line.score,
    })
}

fn mergeable(a: &Segment, b: &Segment, vertical: bool, pitch: f32) -> bool {
    if pitch <= 0. || !aligned(a) || !aligned(b) {
        return false;
    }
    let a = axial(quad_bbox(&a.quad), vertical);
    let b = axial(quad_bbox(&b.quad), vertical);
    let cross = overlap(a.cross0, a.cross1, b.cross0, b.cross1);
    let central = (a.center() - b.center()).abs() <= pitch * 0.35;
    if cross < a.width().min(b.width()) * MERGE_MIN_CROSS_OVERLAP || !central {
        return false;
    }
    let gap = (a.start.max(b.start) - a.end.min(b.end)).max(0.);
    gap <= pitch * FRAGMENT_MAX_GAP
}

fn merge(a: &Segment, b: &Segment, vertical: bool) -> Segment {
    let a_box = quad_bbox(&a.quad);
    let b_box = quad_bbox(&b.quad);
    Segment {
        quad: bbox_quad([
            a_box[0].min(b_box[0]),
            a_box[1].min(b_box[1]),
            a_box[2].max(b_box[2]),
            a_box[3].max(b_box[3]),
        ]),
        vertical,
        score: a.score.min(b.score),
    }
}

fn order_lines(lines: &mut [Segment], vertical: bool, pitch: f32) {
    // A full-height whitespace gutter separates independent horizontal text
    // blocks. Read each block completely left-to-right. Spanning headings or
    // uncertain gutters fall back to row order, without deleting any text.
    if !vertical && lines.len() >= 4 && pitch > 0. {
        let mut intervals = lines.iter().map(|l| quad_bbox(&l.quad)).collect::<Vec<_>>();
        intervals.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut end = intervals[0][2];
        let mut cut = None;
        for b in intervals.iter().skip(1) {
            if b[0] - end > pitch * 1.5 {
                let mid = (end + b[0]) * 0.5;
                let left = intervals.iter().filter(|r| r[2] < mid).count();
                let right = intervals.iter().filter(|r| r[0] > mid).count();
                if left >= 2 && right >= 2 {
                    cut = Some(mid);
                    break;
                }
            }
            end = end.max(b[2]);
        }
        if let Some(mid) = cut {
            lines.sort_by(|a, b| {
                let a = quad_bbox(&a.quad);
                let b = quad_bbox(&b.quad);
                (a[0] > mid)
                    .cmp(&(b[0] > mid))
                    .then(a[1].total_cmp(&b[1]))
                    .then(a[0].total_cmp(&b[0]))
            });
            return;
        }
    }
    // Assign whole fragments to stable column/row bands before sorting within a
    // band. Small cross-axis jitter must not reverse distant same-column text.
    let mut indices = (0..lines.len()).collect::<Vec<_>>();
    indices.sort_by(|a, b| {
        axial(quad_bbox(&lines[*a].quad), vertical)
            .center()
            .total_cmp(&axial(quad_bbox(&lines[*b].quad), vertical).center())
    });
    let mut bands: Vec<(f32, Vec<usize>)> = Vec::new();
    for i in indices {
        let a = axial(quad_bbox(&lines[i].quad), vertical);
        if let Some((center, members)) = bands
            .last_mut()
            .filter(|(center, _)| (a.center() - *center).abs() <= pitch * 0.4)
        {
            *center = (*center * members.len() as f32 + a.center()) / (members.len() + 1) as f32;
            members.push(i);
        } else {
            bands.push((a.center(), vec![i]));
        }
    }
    if vertical {
        bands.reverse();
    }
    let mut sorted = Vec::with_capacity(lines.len());
    for (_, mut members) in bands {
        members.sort_by(|a, b| {
            axial(quad_bbox(&lines[*a].quad), vertical)
                .start
                .total_cmp(&axial(quad_bbox(&lines[*b].quad), vertical).start)
        });
        sorted.extend(members.into_iter().map(|i| lines[i].clone()));
    }
    lines.clone_from_slice(&sorted);
}

pub fn group_lines(lines: Vec<Segment>, bounds: [u32; 4], orientation: &str) -> LayoutDecision {
    let vertical = vertical_layout(&lines, orientation);
    let pitch = body_width(&lines, vertical);
    let ruby = (0..lines.len())
        .map(|i| ruby_parent(i, &lines, vertical, pitch))
        .collect::<Vec<_>>();
    let mut result = LayoutDecision {
        lines: Vec::new(),
        suppressed: Vec::new(),
        reasons: Vec::new(),
    };
    let mut angled = 0;
    for (i, line) in lines.into_iter().enumerate() {
        if ruby[i].is_some() {
            result.suppressed.push(line);
        } else {
            if !aligned(&line) {
                angled += 1;
            }
            // Retain unrecognized/invalid geometry for caller diagnostics; it is
            // never quietly converted into a successful empty transcript.
            result
                .lines
                .push(clipped_segment(&line, bounds).unwrap_or(line));
        }
    }
    let before = result.lines.len();
    // Input contour enumeration must not decide which compatible pair joins
    // first; establish a geometric ordering before the deterministic merge loop.
    order_lines(&mut result.lines, vertical, pitch);
    loop {
        let mut pair = None;
        'outer: for i in 0..result.lines.len() {
            for j in i + 1..result.lines.len() {
                if mergeable(&result.lines[i], &result.lines[j], vertical, pitch) {
                    pair = Some((i, j));
                    break 'outer;
                }
            }
        }
        let Some((i, j)) = pair else {
            break;
        };
        result.lines[i] = merge(&result.lines[i], &result.lines[j], vertical);
        result.lines.remove(j);
    }
    order_lines(&mut result.lines, vertical, pitch);
    result.reasons.push(format!(
        "{} layout; estimated body width {:.1} px",
        if vertical { "vertical" } else { "horizontal" },
        pitch
    ));
    if !result.suppressed.is_empty() {
        result.reasons.push(format!(
            "{} narrow side strips classified as ruby; retained in diagnostic geometry",
            result.suppressed.len()
        ));
    }
    if before != result.lines.len() {
        result.reasons.push(format!(
            "{} aligned fragments/punctuation joined without deleting their pixels",
            before - result.lines.len()
        ));
    }
    if angled > 0 {
        result.reasons.push(format!(
            "{angled} angled/invalid segments preserved without merging"
        ));
    }
    result
}

pub fn dense_route(lines: &[Segment], bounds: [u32; 4]) -> RouteDecision {
    // Use the same geometric grouping as a feature extractor even when the
    // recognition pipeline does not apply grouping. No extra detector/OCR call.
    let grouped = group_lines(lines.to_vec(), bounds, "auto");
    let vertical = vertical_layout(&grouped.lines, "auto");
    let pitch = body_width(&grouped.lines, vertical);
    let bodies = grouped
        .lines
        .iter()
        .filter(|l| aligned(l))
        .filter_map(|line| {
            let a = axial(quad_bbox(&line.quad), vertical);
            (pitch > 0. && a.width() >= pitch * 0.65 && a.length() >= a.width() * 1.8).then_some(a)
        })
        .collect::<Vec<_>>();
    let body_lines = bodies.len();
    let estimated_characters = bodies
        .iter()
        .map(|a| a.length() / a.width().max(1.))
        .sum::<f32>();
    let use_lines = (body_lines >= DENSE_MANY_LINES
        && estimated_characters >= DENSE_MANY_CHARACTERS)
        || (body_lines >= DENSE_FEW_LINES && estimated_characters >= DENSE_FEW_CHARACTERS)
        || (body_lines >= 1 && estimated_characters >= DENSE_SINGLE_CHARACTERS);
    RouteDecision {
        use_lines,
        estimated_characters,
        body_lines,
        reason: format!(
            "{}: {body_lines} body lines, approximately {estimated_characters:.1} glyphs; fixed thresholds 5/96, 3/144 or 1/180",
            if use_lines {
                "line recognition"
            } else {
                "whole-region recognition"
            }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn line(b: [f32; 4], vertical: bool) -> Segment {
        Segment {
            quad: bbox_quad(b),
            vertical,
            score: 1.,
        }
    }

    #[test]
    fn repairs_only_a_connected_clipped_stroke() {
        let im = DynamicImage::ImageRgb8(RgbImage::from_fn(80, 60, |x, y| {
            if (37..45).contains(&x) && (22..29).contains(&y) {
                Rgb([0, 0, 0])
            } else {
                Rgb([255, 255, 255])
            }
        }));
        let d = repair_crop(&im, [10, 10, 40, 40], &[]);
        assert_eq!(d.bounds, [10, 10, 47, 40]);
    }

    #[test]
    fn repair_does_not_cross_a_neighbor_or_page_edge() {
        let im = DynamicImage::ImageRgb8(RgbImage::from_fn(50, 50, |x, y| {
            if (37..46).contains(&x) && (22..29).contains(&y) {
                Rgb([0, 0, 0])
            } else {
                Rgb([255, 255, 255])
            }
        }));
        let d = repair_crop(&im, [0, 0, 40, 40], &[[44., 0., 50., 50.]]);
        assert!(d.bounds[2] <= 44);
        assert_eq!(d.bounds[0], 0);
        assert_eq!(d.bounds[1], 0);
        assert!(d.bounds[3] <= 50);
    }

    #[test]
    fn light_strokes_are_repaired_but_distant_ink_does_not_drive_expansion() {
        let im = DynamicImage::ImageRgb8(RgbImage::from_fn(80, 60, |x, y| {
            if ((37..45).contains(&x) && (22..29).contains(&y))
                || ((46..48).contains(&x) && (34..36).contains(&y))
            {
                Rgb([255, 255, 255])
            } else {
                Rgb([0, 0, 0])
            }
        }));
        assert_eq!(
            repair_crop(&im, [10, 10, 40, 40], &[]).bounds,
            [10, 10, 47, 40]
        );
    }

    #[test]
    fn long_thin_panel_border_does_not_expand_crop() {
        let im = DynamicImage::ImageRgb8(RgbImage::from_fn(80, 80, |x, y| {
            if (39..42).contains(&x) && (5..75).contains(&y) {
                Rgb([0, 0, 0])
            } else {
                Rgb([255, 255, 255])
            }
        }));
        assert_eq!(
            repair_crop(&im, [10, 10, 40, 70], &[]).bounds,
            [10, 10, 40, 70]
        );
    }

    #[test]
    fn ruby_is_suppressed_but_small_kana_and_punctuation_remain() {
        let d = group_lines(
            vec![
                line([20., 0., 40., 80.], true),
                line([41., 10., 48., 60.], true),
                line([26., 83., 35., 94.], true),
                line([27., 99., 32., 107.], true),
            ],
            [0, 0, 80, 120],
            "vertical",
        );
        assert_eq!(d.suppressed.len(), 1);
        assert_eq!(d.lines.len(), 1);
        assert_eq!(quad_bbox(&d.lines[0].quad), [20., 0., 40., 107.]);
    }

    #[test]
    fn uncertain_short_side_mark_is_never_suppressed_as_ruby() {
        let d = group_lines(
            vec![
                line([20., 0., 40., 100.], true),
                line([43., 30., 50., 38.], false),
            ],
            [0, 0, 80, 120],
            "vertical",
        );
        assert!(d.suppressed.is_empty());
        assert_eq!(d.lines.len(), 2);
    }

    #[test]
    fn fragments_join_within_a_column_but_adjacent_columns_remain_separate() {
        let d = group_lines(
            vec![
                line([25., 0., 45., 75.], true),
                line([65., 0., 85., 30.], true),
                line([66., 40., 85., 75.], true),
            ],
            [0, 0, 100, 100],
            "vertical",
        );
        assert_eq!(d.lines.len(), 2);
        assert_eq!(quad_bbox(&d.lines[0].quad), [65., 0., 85., 75.]);
        assert_eq!(quad_bbox(&d.lines[1].quad), [25., 0., 45., 75.]);
    }

    #[test]
    fn independent_horizontal_columns_do_not_interleave() {
        let d = group_lines(
            vec![
                line([100., 0., 180., 12.], false),
                line([0., 20., 80., 32.], false),
                line([0., 0., 80., 12.], false),
                line([100., 20., 180., 32.], false),
            ],
            [0, 0, 200, 50],
            "horizontal",
        );
        let boxes = d
            .lines
            .iter()
            .map(|l| quad_bbox(&l.quad))
            .collect::<Vec<_>>();
        assert_eq!(
            boxes,
            vec![
                [0., 0., 80., 12.],
                [0., 20., 80., 32.],
                [100., 0., 180., 12.],
                [100., 20., 180., 32.]
            ]
        );
    }

    #[test]
    fn many_short_dialogue_columns_do_not_trigger_dense_routing() {
        let lines = (0..6)
            .map(|i| line([i as f32 * 30., 0., i as f32 * 30. + 18., 80.], true))
            .collect::<Vec<_>>();
        let d = dense_route(&lines, [0, 0, 200, 100]);
        assert!(!d.use_lines);
        assert_eq!(d.body_lines, 6);
    }

    #[test]
    fn dense_routing_uses_geometry_and_ignores_narrow_ruby_strips() {
        let mut lines = (0..8)
            .map(|i| line([i as f32 * 30., 0., i as f32 * 30. + 14., 240.], true))
            .collect::<Vec<_>>();
        lines.push(line([15., 20., 20., 90.], true));
        let d = dense_route(&lines, [0, 0, 250, 250]);
        assert!(d.use_lines);
        assert_eq!(d.body_lines, 8);
        assert!((d.estimated_characters - 8. * 240. / 14.).abs() < 0.01);
    }
}
