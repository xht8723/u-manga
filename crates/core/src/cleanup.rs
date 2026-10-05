//! Pixel-only cleanup: prefer an enclosed interior, fall back to the text rectangle.
use crate::{render::color, types::Region};
use image::{GrayImage, RgbImage};
use std::collections::VecDeque;

pub struct Cleanup {
    pub origin: [u32; 2],
    pub mask: GrayImage,
    pub fill: [u8; 3],
    /// An enclosed interior when found; None for text-rectangle fallback or manual masks.
    pub interior: Option<GrayImage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalysisIssue {
    Background,
    OpenBoundary,
    Texture,
    IncompleteInk,
    NoInk,
}

#[derive(Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    right: u32,
    bottom: u32,
}
impl Rect {
    fn bounds(b: [f32; 4], image: &RgbImage, pad: f32) -> Self {
        Self {
            x: (b[0] - pad).floor().max(0.).min(image.width() as f32) as u32,
            y: (b[1] - pad).floor().max(0.).min(image.height() as f32) as u32,
            right: (b[2] + pad).ceil().max(0.).min(image.width() as f32) as u32,
            bottom: (b[3] + pad).ceil().max(0.).min(image.height() as f32) as u32,
        }
    }
    fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && x < self.right && y >= self.y && y < self.bottom
    }
    fn width(&self) -> u32 {
        self.right.saturating_sub(self.x)
    }
    fn height(&self) -> u32 {
        self.bottom.saturating_sub(self.y)
    }
}
fn distance(a: [u8; 3], b: [u8; 3]) -> u8 {
    (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap()
}
fn median(samples: &[[u8; 3]]) -> [u8; 3] {
    std::array::from_fn(|c| {
        let mut v: Vec<_> = samples.iter().map(|p| p[c]).collect();
        let mid = v.len() / 2;
        *v.select_nth_unstable(mid).1
    })
}
fn neighbours(i: usize, w: usize, h: usize, diagonal: bool, mut visit: impl FnMut(usize)) {
    let (x, y) = (i % w, i / w);
    for dy in -1_i32..=1 {
        for dx in -1_i32..=1 {
            if dx == 0 && dy == 0 || !diagonal && dx != 0 && dy != 0 {
                continue;
            }
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            if nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32 {
                visit(ny as usize * w + nx as usize);
            }
        }
    }
}
fn dilate(input: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = input.to_vec();
    for (i, &set) in input.iter().enumerate() {
        if set {
            neighbours(i, w, h, true, |j| out[j] = true);
        }
    }
    out
}
fn otsu(values: &[u8]) -> u8 {
    let mut counts = [0u64; 256];
    for &v in values {
        counts[v as usize] += 1;
    }
    let total = values.len() as f64;
    let sum: f64 = counts
        .iter()
        .enumerate()
        .map(|(i, &n)| i as f64 * n as f64)
        .sum();
    let (mut left, mut left_sum, mut best, mut threshold) = (0., 0., 0., 0);
    for (i, &n) in counts.iter().enumerate() {
        left += n as f64;
        left_sum += i as f64 * n as f64;
        if left == 0. || left == total {
            continue;
        }
        let diff = left_sum / left - (sum - left_sum) / (total - left);
        let score = left * (total - left) * diff * diff;
        if score > best {
            best = score;
            threshold = i as u8;
        }
    }
    threshold
}

fn background(im: &RgbImage, search: Rect) -> Result<([u8; 3], u8), AnalysisIssue> {
    let mut samples = Vec::new();
    let mut bins = [0usize; 512];
    // Sample across the search area, including inside the text rectangle, not just its edge.
    let step = ((search.width() as usize * search.height() as usize / 16000) as f64)
        .sqrt()
        .max(1.) as usize;
    for y in (search.y..search.bottom).step_by(step) {
        for x in (search.x..search.right).step_by(step) {
            let p = im.get_pixel(x, y).0;
            if x + 1 < im.width() && distance(p, im.get_pixel(x + 1, y).0) > 35
                || y + 1 < im.height() && distance(p, im.get_pixel(x, y + 1).0) > 35
            {
                continue;
            }
            bins[((p[0] as usize >> 5) << 6)
                | ((p[1] as usize >> 5) << 3)
                | (p[2] as usize >> 5)] += 1;
            samples.push(p);
        }
    }
    let peak = bins
        .iter()
        .enumerate()
        .max_by_key(|&(_, n)| n)
        .map(|(i, _)| i)
        .ok_or(AnalysisIssue::Background)?;
    let seed: [u8; 3] = [
        ((peak >> 6) * 32 + 16) as u8,
        (((peak >> 3) & 7) * 32 + 16) as u8,
        ((peak & 7) * 32 + 16) as u8,
    ];
    let mut near: Vec<_> = samples
        .iter()
        .copied()
        .filter(|p| distance(*p, seed) < 40)
        .collect();
    if near.len() < 12 || near.len() * 3 < samples.len() {
        return Err(AnalysisIssue::Background);
    }
    let fill = median(&near);
    near.retain(|p| distance(*p, fill) < 40);
    let mut noise: Vec<_> = near.iter().map(|p| distance(*p, fill)).collect();
    if noise.is_empty() {
        return Err(AnalysisIssue::Background);
    }
    noise.sort_unstable();
    let tolerance = ((noise[noise.len() * 9 / 10] as f32 * 1.5 + 8.) as u8).clamp(22, 42);
    Ok((fill, tolerance))
}

fn manual(im: &RgbImage, r: &Region) -> Cleanup {
    let rect = Rect::bounds(r.bbox, im, 3.);
    let mut mask = GrayImage::new(rect.width(), rect.height());
    let fill = color(&r.style.fill);
    // Solid override still selects contrasting pixels; it does not erase the rectangle.
    for (x, y, p) in mask.enumerate_pixels_mut() {
        if distance(im.get_pixel(x + rect.x, y + rect.y).0, fill) > 25 {
            p[0] = 255;
        }
    }
    Cleanup {
        origin: [rect.x, rect.y],
        mask,
        fill,
        interior: None,
    }
}

/// No quality gate: uncertainty chooses a bounded fallback, never hides a translation.
pub fn analyze(im: &RgbImage, r: &Region) -> Cleanup {
    analyze_with_bubbles(im, r, &[])
}

pub fn analyze_with_bubbles(im: &RgbImage, r: &Region, bubbles: &[[f32; 4]]) -> Cleanup {
    if r.allow_fill {
        return manual(im, r);
    }
    enclosed(im, r, bubbles).unwrap_or_else(|_| text_mask(im, r))
}

fn text_mask(im: &RgbImage, r: &Region) -> Cleanup {
    let text = Rect::bounds(r.bbox, im, 0.);
    // Never fill the entire detector balloon rectangle. The fallback stays close to text.
    let bounds = Rect::bounds(r.bbox, im, 3.);
    let fill = background(im, text)
        .map(|(fill, _)| fill)
        .unwrap_or_else(|_| {
            // Highly textured/edge-heavy crops still need a best-effort background estimate.
            let mut bins = vec![Vec::new(); 512];
            let step = ((text.width() as usize * text.height() as usize / 16000) as f64)
                .sqrt()
                .max(1.) as usize;
            for y in (text.y..text.bottom).step_by(step) {
                for x in (text.x..text.right).step_by(step) {
                    let p = im.get_pixel(x, y).0;
                    bins[((p[0] as usize >> 5) << 6)
                        | ((p[1] as usize >> 5) << 3)
                        | (p[2] as usize >> 5)]
                        .push(p);
                }
            }
            let dominant = bins.into_iter().max_by_key(|b| b.len()).unwrap();
            if dominant.is_empty() {
                color(&r.style.fill)
            } else {
                median(&dominant)
            }
        });
    let (w, h) = (bounds.width() as usize, bounds.height() as usize);
    let mut mask = GrayImage::new(w as u32, h as u32);
    let mut ink = vec![false; w * h];
    for y in text.y..text.bottom {
        for x in text.x..text.right {
            ink[(y - bounds.y) as usize * w + (x - bounds.x) as usize] =
                distance(im.get_pixel(x, y).0, fill) > 25;
        }
    }
    // Fill counters, then grow through the antialias fringe, including punctuation.
    let mut exterior = vec![false; w * h];
    let mut q = VecDeque::new();
    for i in 0..w * h {
        if (i % w == 0 || i % w + 1 == w || i / w == 0 || i / w + 1 == h) && !ink[i] {
            exterior[i] = true;
            q.push_back(i);
        }
    }
    while let Some(i) = q.pop_front() {
        neighbours(i, w, h, false, |j| {
            if !ink[j] && !exterior[j] {
                exterior[j] = true;
                q.push_back(j);
            }
        });
    }
    ink = exterior.into_iter().map(|v| !v).collect();
    for _ in 0..3 {
        ink = dilate(&ink, w, h);
    }
    for (p, set) in mask.as_mut().iter_mut().zip(ink) {
        *p = if set { 255 } else { 0 };
    }
    Cleanup {
        origin: [bounds.x, bounds.y],
        mask,
        fill,
        interior: None,
    }
}

fn enclosed(im: &RgbImage, r: &Region, bubbles: &[[f32; 4]]) -> Result<Cleanup, AnalysisIssue> {
    let text = Rect::bounds(r.bbox, im, 0.);
    if text.width() < 2 || text.height() < 2 {
        return Err(AnalysisIssue::NoInk);
    }
    let short = text.width().min(text.height()) as f32;
    let mut bounds = r.bubble.unwrap_or(r.bbox);
    if r.bubble.is_some() {
        loop {
            let previous = bounds;
            for b in bubbles {
                if bounds[0] < b[2] && bounds[2] > b[0] && bounds[1] < b[3] && bounds[3] > b[1] {
                    bounds = [
                        bounds[0].min(b[0]),
                        bounds[1].min(b[1]),
                        bounds[2].max(b[2]),
                        bounds[3].max(b[3]),
                    ];
                }
            }
            if bounds == previous {
                break;
            }
        }
    }
    bounds = [
        bounds[0].min(r.bbox[0]),
        bounds[1].min(r.bbox[1]),
        bounds[2].max(r.bbox[2]),
        bounds[3].max(r.bbox[3]),
    ];
    let initial = if r.bubble.is_some() {
        (short * 0.08).clamp(8., 24.)
    } else {
        (short * 0.65).clamp(32., 220.)
    };
    let extent = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]);
    let mut previous = None;
    let mut last = AnalysisIssue::OpenBoundary;
    for pad in [initial, initial + extent * 0.3, initial + extent * 0.8] {
        let search = Rect::bounds(bounds, im, pad);
        let key = [search.x, search.y, search.right, search.bottom];
        if previous == Some(key) {
            continue;
        }
        previous = Some(key);
        let mut candidate = r.clone();
        if candidate.bubble.is_some() {
            candidate.bubble = Some(bounds);
        }
        match automatic(im, &candidate, text, search) {
            Ok(result) => return Ok(result),
            Err(e) => {
                last = e;
                if e != AnalysisIssue::OpenBoundary {
                    break;
                }
            }
        }
    }
    Err(last)
}

fn automatic(
    im: &RgbImage,
    r: &Region,
    text: Rect,
    search: Rect,
) -> Result<Cleanup, AnalysisIssue> {
    let short = text.width().min(text.height()) as f32;
    let (w, h) = (search.width() as usize, search.height() as usize);
    let n = w * h;
    let (fill, tolerance) = background(im, text)?;
    let mut contrast = vec![0; n];
    for y in 0..h {
        for x in 0..w {
            contrast[y * w + x] = distance(
                im.get_pixel(search.x + x as u32, search.y + y as u32).0,
                fill,
            );
        }
    }
    let threshold = otsu(&contrast).clamp(tolerance.saturating_add(8), 110);
    let ink: Vec<_> = contrast.iter().map(|&v| v > threshold).collect();
    // A one-pixel barrier joins antialiased outlines without bridging real openings.
    let walls = dilate(&ink, w, h);
    let mut seen = vec![false; n];
    let mut best = Vec::new();
    let mut best_overlap = 0;
    let mut largest_open = 0;
    for seed in 0..n {
        if walls[seed] || seen[seed] {
            continue;
        }
        let mut q = VecDeque::from([seed]);
        let mut component = Vec::new();
        let mut overlap = 0;
        let mut touches = false;
        let mut page_edges = 0u8;
        seen[seed] = true;
        while let Some(i) = q.pop_front() {
            component.push(i);
            let (x, y) = (i % w, i / w);
            if x == 0 {
                if search.x == 0 {
                    page_edges |= 1;
                } else {
                    touches = true;
                }
            }
            if y == 0 {
                if search.y == 0 {
                    page_edges |= 2;
                } else {
                    touches = true;
                }
            }
            if x + 1 == w {
                if search.right == im.width() {
                    page_edges |= 4;
                } else {
                    touches = true;
                }
            }
            if y + 1 == h {
                if search.bottom == im.height() {
                    page_edges |= 8;
                } else {
                    touches = true;
                }
            }
            overlap += usize::from(text.contains(search.x + x as u32, search.y + y as u32));
            neighbours(i, w, h, false, |j| {
                if !seen[j] && !walls[j] {
                    seen[j] = true;
                    q.push_back(j);
                }
            });
        }
        // A page-cropped balloon can use one physical page edge, never a search-box edge.
        let cropped = r.bubble.is_some_and(|b| {
            page_edges.count_ones() == 1
                && (page_edges == 1 && b[0] < 16.
                    || page_edges == 2 && b[1] < 16.
                    || page_edges == 4 && b[2] > im.width() as f32 - 16.
                    || page_edges == 8 && b[3] > im.height() as f32 - 16.)
        });
        if touches || page_edges != 0 && !cropped {
            largest_open = largest_open.max(overlap);
        } else if overlap > best_overlap {
            best_overlap = overlap;
            best = component;
        }
    }
    let text_area = text.width() as usize * text.height() as usize;
    if best_overlap * 100 < text_area * 42 || largest_open > best_overlap {
        return Err(AnalysisIssue::OpenBoundary);
    }
    let mut inner = vec![false; n];
    for &i in &best {
        inner[i] = true;
    }
    // Fill holes occupied by letters, but leave the balloon outline connected to the exterior.
    let mut exterior = vec![false; n];
    let mut q = VecDeque::new();
    for i in 0..n {
        if (i % w == 0 || i % w + 1 == w || i / w == 0 || i / w + 1 == h) && !inner[i] {
            exterior[i] = true;
            q.push_back(i);
        }
    }
    while let Some(i) = q.pop_front() {
        neighbours(i, w, h, false, |j| {
            if !inner[j] && !exterior[j] {
                exterior[j] = true;
                q.push_back(j);
            }
        });
    }
    for i in 0..n {
        inner[i] = !exterior[i];
    }
    // Large differences between background patches indicate shading/art rather than paper grain.
    let mut patch_colors = Vec::new();
    for ty in 0..4 {
        for tx in 0..4 {
            let mut samples = Vec::new();
            for y in (text.y + (text.height() * ty / 4)..text.y + (text.height() * (ty + 1) / 4))
                .step_by(2)
            {
                for x in (text.x + (text.width() * tx / 4)..text.x + (text.width() * (tx + 1) / 4))
                    .step_by(2)
                {
                    let i = (y - search.y) as usize * w + (x - search.x) as usize;
                    if inner[i] && !walls[i] {
                        samples.push(im.get_pixel(x, y).0);
                    }
                }
            }
            if samples.len() > 12 {
                patch_colors.push(median(&samples));
            }
        }
    }
    if patch_colors.iter().any(|p| distance(*p, fill) > 28) {
        return Err(AnalysisIssue::Texture);
    }
    let expanded = Rect::bounds(r.bbox, im, 4.);
    let inset = (short * 0.08).clamp(3., 14.) as u32;
    let core = Rect {
        x: text.x + inset,
        y: text.y + inset,
        right: text.right.saturating_sub(inset),
        bottom: text.bottom.saturating_sub(inset),
    };
    let mut selected = vec![false; n];
    seen.fill(false);
    let (mut accepted, mut rejected_core, mut specks, mut count) = (0usize, 0usize, 0usize, 0usize);
    let mut compact_dots = Vec::new();
    for seed in 0..n {
        if !ink[seed] || seen[seed] {
            continue;
        }
        let mut q = VecDeque::from([seed]);
        let mut component = Vec::new();
        seen[seed] = true;
        let (mut overlap, mut in_core, mut outside) = (0usize, 0usize, 0usize);
        let (mut xmin, mut ymin, mut xmax, mut ymax) = (w, h, 0, 0);
        while let Some(i) = q.pop_front() {
            component.push(i);
            let (x, y) = (i % w, i / w);
            let (px, py) = (search.x + x as u32, search.y + y as u32);
            xmin = xmin.min(x);
            xmax = xmax.max(x);
            ymin = ymin.min(y);
            ymax = ymax.max(y);
            overlap += usize::from(text.contains(px, py));
            in_core += usize::from(core.contains(px, py));
            outside += usize::from(!inner[i] || !expanded.contains(px, py));
            neighbours(i, w, h, true, |j| {
                if ink[j] && !seen[j] {
                    seen[j] = true;
                    q.push_back(j);
                }
            });
        }
        if overlap == 0 {
            continue;
        }
        let size = component.len();
        if outside > 0
            || size > text_area / 3
            || (xmax - xmin + 1) * (ymax - ymin + 1) > text_area * 3 / 4
        {
            rejected_core += in_core;
            continue;
        }
        if size < 3 {
            specks += size;
            continue;
        }
        let cw = xmax - xmin + 1;
        let ch = ymax - ymin + 1;
        let dot_limit = (short * 0.1).clamp(5., 18.) as usize;
        if cw <= dot_limit && ch <= dot_limit && size * 100 > cw * ch * 65 {
            compact_dots.push(((xmin + xmax) / 2, (ymin + ymax) / 2, cw, ch));
        }
        accepted += size;
        count += 1;
        if size < 10 {
            specks += size;
        }
        for i in component {
            selected[i] = true;
        }
    }
    // A repeated compact shape over two dimensions is a screentone, unlike varied
    // kana/radicals or the one-dimensional dots in an ellipsis.
    let mut repeated = std::collections::BTreeMap::<(usize, usize), Vec<_>>::new();
    for p in compact_dots {
        if p.2 * 3 >= p.3 * 2 && p.3 * 3 >= p.2 * 2 {
            repeated
                .entry((p.2.div_ceil(2), p.3.div_ceil(2)))
                .or_default()
                .push(p);
        }
    }
    let compact_dots = repeated
        .into_values()
        .max_by_key(|v| v.len())
        .unwrap_or_default();
    if compact_dots.len() >= 18 {
        let min_x = compact_dots.iter().map(|p| p.0).min().unwrap();
        let max_x = compact_dots.iter().map(|p| p.0).max().unwrap();
        let min_y = compact_dots.iter().map(|p| p.1).min().unwrap();
        let max_y = compact_dots.iter().map(|p| p.1).max().unwrap();
        let size = compact_dots.iter().map(|p| p.2.max(p.3)).sum::<usize>() / compact_dots.len();
        if max_x - min_x > size * 6 && max_y - min_y > size * 6 {
            return Err(AnalysisIssue::Texture);
        }
    }
    if accepted == 0 {
        return Err(AnalysisIssue::NoInk);
    }
    if rejected_core > accepted / 25 + 8 {
        return Err(AnalysisIssue::IncompleteInk);
    }
    if accepted * 100 > text_area * 40 || (specks * 5 > accepted && count > 20) {
        return Err(AnalysisIssue::Texture);
    }
    // Include character counters and the pale antialias/outline fringe. Leaving white
    // counters behind on scanned paper makes the old Japanese visible as negative ghosts.
    let mut outside_mask = vec![false; n];
    let mut q = VecDeque::new();
    for i in 0..n {
        if (i % w == 0 || i % w + 1 == w || i / w == 0 || i / w + 1 == h) && !selected[i] {
            outside_mask[i] = true;
            q.push_back(i);
        }
    }
    while let Some(i) = q.pop_front() {
        neighbours(i, w, h, false, |j| {
            if !selected[j] && !outside_mask[j] {
                outside_mask[j] = true;
                q.push_back(j);
            }
        });
    }
    let mut grown: Vec<_> = outside_mask.iter().map(|v| !v).collect();
    for _ in 0..if short >= 100. { 3 } else { 2 } {
        grown = dilate(&grown, w, h);
    }
    let mut mask = GrayImage::new(w as u32, h as u32);
    let mut interior = GrayImage::new(w as u32, h as u32);
    for i in 0..n {
        interior.as_mut()[i] = if inner[i] { 255 } else { 0 };
        mask.as_mut()[i] = if grown[i] && inner[i] { 255 } else { 0 };
    }
    // Keep automatic mask expansion inside the validated interior.
    for (i, inside) in inner.iter().enumerate() {
        if !inside {
            mask.as_mut()[i] = 0;
        }
    }
    Ok(Cleanup {
        origin: [search.x, search.y],
        mask,
        fill,
        interior: Some(interior),
    })
}
