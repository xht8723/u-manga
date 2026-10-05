use anyhow::{Context, Result, ensure};
use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use imageproc::{
    geometric_transformations::{Interpolation, Projection, warp_into},
    point::Point,
};

#[derive(Clone)]
pub struct Segment {
    pub quad: [[f32; 2]; 4],
    pub vertical: bool,
    pub score: f32,
}
pub fn quad_bbox(q: &[[f32; 2]; 4]) -> [f32; 4] {
    [
        q.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
        q.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
        q.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max),
        q.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max),
    ]
}
pub fn bbox_quad(b: [f32; 4]) -> [[f32; 2]; 4] {
    [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]
}
pub fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}
fn cross(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}
fn subtract(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
// Parallel-edge offset of the minimum-area rectangle. This is deliberately an
// explicit benchmark variant, not asserted byte-identical to Clipper's rounded
// offset of the original contour in Paddle's reference implementation.
fn expand(q: [[f32; 2]; 4], d: f32) -> [[f32; 2]; 4] {
    let signed = (0..4).map(|i| cross(q[i], q[(i + 1) % 4])).sum::<f32>();
    let mut lines = [([0.; 2], [0.; 2]); 4];
    for i in 0..4 {
        let v = subtract(q[(i + 1) % 4], q[i]);
        let len = (v[0] * v[0] + v[1] * v[1]).sqrt().max(0.001);
        let sign = if signed > 0. { 1. } else { -1. };
        let offset = [v[1] / len * d * sign, -v[0] / len * d * sign];
        lines[i] = ([q[i][0] + offset[0], q[i][1] + offset[1]], v);
    }
    let mut out = q;
    for i in 0..4 {
        let (a, u) = lines[(i + 3) % 4];
        let (b, v) = lines[i];
        let denom = cross(u, v);
        if denom.abs() > 0.001 {
            let t = cross(subtract(b, a), v) / denom;
            out[i] = [a[0] + u[0] * t, a[1] + u[1] * t];
        }
    }
    out
}
pub fn postprocess(prob: &[f32], mw: u32, mh: u32, width: u32, height: u32) -> Vec<Segment> {
    let mask = GrayImage::from_raw(
        mw,
        mh,
        prob.iter()
            .map(|p| if *p > 0.3 { 255 } else { 0 })
            .collect(),
    )
    .unwrap();
    let mut result = Vec::new();
    for c in imageproc::contours::find_contours::<i32>(&mask)
        .into_iter()
        .filter(|c| c.parent.is_none())
        .take(1000)
    {
        if c.points.len() < 4 {
            continue;
        }
        let rect = imageproc::geometry::min_area_rect(&c.points);
        let mut q = rect.map(|p| [p.x as f32, p.y as f32]);
        q.sort_by(|a, b| a[1].total_cmp(&b[1]));
        if q[0][0] > q[1][0] {
            q.swap(0, 1);
        }
        if q[2][0] > q[3][0] {
            q.swap(2, 3);
        }
        q.swap(2, 3);
        let rw = distance(q[0], q[1]);
        let rh = distance(q[0], q[3]);
        if rw.min(rh) < 3. {
            continue;
        }
        let b = quad_bbox(&q);
        let x0 = b[0].max(0.) as u32;
        let y0 = b[1].max(0.) as u32;
        let x1 = b[2].ceil().min(mw as f32) as u32;
        let y1 = b[3].ceil().min(mh as f32) as u32;
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let mut polygon = GrayImage::new(x1 - x0 + 1, y1 - y0 + 1);
        let points = q
            .iter()
            .map(|p| {
                Point::new(
                    (p[0] - x0 as f32).round() as i32,
                    (p[1] - y0 as f32).round() as i32,
                )
            })
            .collect::<Vec<_>>();
        imageproc::drawing::draw_polygon_mut(&mut polygon, &points, Luma([255]));
        let mut sum = 0.;
        let mut count = 0.;
        for y in y0..y1 {
            for x in x0..x1 {
                if polygon.get_pixel(x - x0, y - y0)[0] > 0 {
                    sum += prob[(y * mw + x) as usize];
                    count += 1.;
                }
            }
        }
        let score = if count > 0. { sum / count } else { 0. };
        if score < 0.6 {
            continue;
        }
        let mut q = expand(q, rw * rh * 1.5 / (2. * (rw + rh)));
        for p in &mut q {
            p[0] = (p[0] * width as f32 / mw as f32).clamp(0., width as f32 - 1.);
            p[1] = (p[1] * height as f32 / mh as f32).clamp(0., height as f32 - 1.);
        }
        let vertical = distance(q[0], q[3]) > distance(q[0], q[1]) * 1.5;
        result.push(Segment {
            quad: q,
            vertical,
            score,
        });
    }
    result
}
pub fn rectify(im: &RgbImage, quad: [[f32; 2]; 4], rotate: bool) -> Result<DynamicImage> {
    let width = distance(quad[0], quad[1])
        .max(distance(quad[2], quad[3]))
        .ceil()
        .max(1.) as u32;
    let height = distance(quad[0], quad[3])
        .max(distance(quad[1], quad[2]))
        .ceil()
        .max(1.) as u32;
    ensure!(
        width <= 10000 && height <= 10000,
        "Implausible line dimensions"
    );
    let from = quad.map(|p| (p[0], p[1]));
    let to = [
        (0., 0.),
        (width as f32, 0.),
        (width as f32, height as f32),
        (0., height as f32),
    ];
    let proj =
        Projection::from_control_points(from, to).context("Degenerate line quadrilateral")?;
    let mut line = RgbImage::new(width, height);
    warp_into(
        im,
        &proj,
        Interpolation::Bilinear,
        Rgb([255, 255, 255]),
        &mut line,
    );
    let im = DynamicImage::ImageRgb8(line);
    Ok(if rotate { im.rotate270() } else { im })
}
/// Preserve the detected line's rotation, but expose only the frozen parent ROI
/// to interpolation. Samples outside it become white, never neighboring text.
pub fn rectify_clipped(
    im: &RgbImage,
    mut quad: [[f32; 2]; 4],
    rotate: bool,
    bounds: [u32; 4],
) -> Result<DynamicImage> {
    ensure!(
        bounds[0] < bounds[2]
            && bounds[1] < bounds[3]
            && bounds[2] <= im.width()
            && bounds[3] <= im.height(),
        "Invalid frozen parent clip bounds"
    );
    let parent = image::imageops::crop_imm(
        im,
        bounds[0],
        bounds[1],
        bounds[2] - bounds[0],
        bounds[3] - bounds[1],
    )
    .to_image();
    for point in &mut quad {
        point[0] -= bounds[0] as f32;
        point[1] -= bounds[1] as f32;
    }
    rectify(&parent, quad, rotate)
}
pub fn sort(lines: &mut [Segment], orientation: &str) {
    let vertical = orientation == "vertical"
        || (orientation == "auto" && lines.iter().filter(|l| l.vertical).count() * 2 > lines.len());
    lines.sort_by(|a, b| {
        let a = quad_bbox(&a.quad);
        let b = quad_bbox(&b.quad);
        if vertical {
            b[0].total_cmp(&a[0]).then(a[1].total_cmp(&b[1]))
        } else {
            a[1].total_cmp(&b[1]).then(a[0].total_cmp(&b[0]))
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offset_keeps_rotated_edges_parallel() {
        let q = [[10., 0.], [20., 10.], [10., 20.], [0., 10.]];
        let e = expand(q, 2.);
        for i in 0..4 {
            let a = subtract(q[(i + 1) % 4], q[i]);
            let b = subtract(e[(i + 1) % 4], e[i]);
            assert!(cross(a, b).abs() < 0.001);
        }
        assert!(e[0][1] < 0.);
    }
    #[test]
    fn vertical_order_is_right_to_left() {
        let mut s = vec![
            Segment {
                quad: bbox_quad([0., 0., 10., 100.]),
                vertical: true,
                score: 1.,
            },
            Segment {
                quad: bbox_quad([20., 0., 30., 100.]),
                vertical: true,
                score: 1.,
            },
        ];
        sort(&mut s, "vertical");
        assert_eq!(s[0].quad[0][0], 20.);
    }
    #[test]
    fn rectification_never_samples_beyond_frozen_parent() {
        let image = RgbImage::from_fn(60, 20, |x, _| {
            if x < 20 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            }
        });
        let clipped = rectify_clipped(&image, bbox_quad([0., 0., 50., 20.]), false, [0, 0, 20, 20])
            .unwrap()
            .to_rgb8();
        assert_eq!(clipped.get_pixel(5, 5).0, [255, 0, 0]);
        assert_eq!(clipped.get_pixel(35, 5).0, [255, 255, 255]);
        assert!(!clipped.pixels().any(|pixel| pixel.0 == [0, 0, 255]));
    }
    #[test]
    fn clipping_rejects_out_of_page_bounds() {
        let image = RgbImage::new(20, 20);
        assert!(
            rectify_clipped(&image, bbox_quad([0., 0., 20., 20.]), false, [0, 0, 21, 20]).is_err()
        );
    }
}
