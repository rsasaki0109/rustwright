//! Bounded click candidates from painted quads clipped to the root viewport.

const MAX_QUADS: usize = 16;
const MAX_POINTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl Point {
    fn towards(self, other: Self, fraction: f64) -> Self {
        Self {
            x: self.x + (other.x - self.x) * fraction,
            y: self.y + (other.y - self.y) * fraction,
        }
    }
}

/// A projective mapping from a renderer viewport to its iframe content quad.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Projection {
    matrix: [f64; 9],
    inverse: [f64; 9],
}

impl Projection {
    pub(crate) fn new(quad: [f64; 8], width: f64, height: f64) -> Option<Self> {
        if !quad.iter().all(|n| n.is_finite())
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
        {
            return None;
        }
        let [x0, y0, x1, y1, x2, y2, x3, y3] = quad;
        let dx = x0 - x1 + x2 - x3;
        let dy = y0 - y1 + y2 - y3;
        let (g, h) = if dx.abs() + dy.abs() < 1e-10 {
            (0.0, 0.0)
        } else {
            let det = (x1 - x2) * (y3 - y2) - (x3 - x2) * (y1 - y2);
            if det.abs() < 1e-10 {
                return None;
            }
            (
                (dx * (y3 - y2) - dy * (x3 - x2)) / det,
                ((x1 - x2) * dy - (y1 - y2) * dx) / det,
            )
        };
        let m = [
            (x1 - x0 + g * x1) / width,
            (x3 - x0 + h * x3) / height,
            x0,
            (y1 - y0 + g * y1) / width,
            (y3 - y0 + h * y3) / height,
            y0,
            g / width,
            h / height,
            1.0,
        ];
        let inv = [
            m[4] * m[8] - m[5] * m[7],
            m[2] * m[7] - m[1] * m[8],
            m[1] * m[5] - m[2] * m[4],
            m[5] * m[6] - m[3] * m[8],
            m[0] * m[8] - m[2] * m[6],
            m[2] * m[3] - m[0] * m[5],
            m[3] * m[7] - m[4] * m[6],
            m[1] * m[6] - m[0] * m[7],
            m[0] * m[4] - m[1] * m[3],
        ];
        let det = m[0] * inv[0] + m[1] * inv[3] + m[2] * inv[6];
        if !det.is_finite() || det.abs() < 1e-10 {
            return None;
        }
        Some(Self {
            matrix: m,
            inverse: inv.map(|n| n / det),
        })
    }

    pub(crate) fn forward(&self, point: Point) -> Option<Point> {
        Self::apply(&self.matrix, point)
    }
    pub(crate) fn backward(&self, point: Point) -> Option<Point> {
        Self::apply(&self.inverse, point)
    }

    fn apply(m: &[f64; 9], p: Point) -> Option<Point> {
        let w = m[6] * p.x + m[7] * p.y + m[8];
        let point = Point {
            x: (m[0] * p.x + m[1] * p.y + m[2]) / w,
            y: (m[3] * p.x + m[4] * p.y + m[5]) / w,
        };
        (w.abs() > 1e-10 && point.x.is_finite() && point.y.is_finite()).then_some(point)
    }
}

pub(crate) fn click_candidates(quads: &[[f64; 8]], width: f64, height: f64) -> Vec<Point> {
    click_candidates_in_rect(quads, [0.0, 0.0, width, height])
}

/// Clip painted polygons to a conservative visible ancestor/viewport rectangle.
/// Native hit testing still decides whether every proposed point receives input.
pub(crate) fn click_candidates_in_rect(quads: &[[f64; 8]], rect: [f64; 4]) -> Vec<Point> {
    let [left, top, right, bottom] = rect;
    if !rect.iter().all(|value| value.is_finite()) || right <= left || bottom <= top {
        return Vec::new();
    }
    let polygons: Vec<_> = quads
        .iter()
        .take(MAX_QUADS)
        .filter_map(|quad| {
            if !quad.iter().all(|value| value.is_finite()) {
                return None;
            }
            let mut polygon: Vec<_> = quad
                .chunks_exact(2)
                .map(|p| Point { x: p[0], y: p[1] })
                .collect();
            for (axis, limit, minimum) in [
                (0, left, true),
                (0, right, false),
                (1, top, true),
                (1, bottom, false),
            ] {
                polygon = clip(&polygon, axis, limit, minimum);
            }
            centroid(&polygon).map(|center| (polygon, center))
        })
        .collect();
    let mut points = Vec::new();
    // Try each inline fragment's interior before spending probes on its edges.
    for (_, center) in &polygons {
        push_unique(&mut points, *center);
    }
    for (polygon, center) in &polygons {
        for fraction in [0.5, 0.8, 0.95] {
            for index in 0..polygon.len() {
                let vertex = polygon[index];
                let midpoint = vertex.towards(polygon[(index + 1) % polygon.len()], 0.5);
                push_unique(&mut points, center.towards(midpoint, fraction));
                push_unique(&mut points, center.towards(vertex, fraction));
            }
        }
    }
    points
}

fn push_unique(points: &mut Vec<Point>, point: Point) {
    if points.len() < MAX_POINTS
        && !points
            .iter()
            .any(|p| (p.x - point.x).abs() < 0.01 && (p.y - point.y).abs() < 0.01)
    {
        points.push(point);
    }
}

fn coordinate(point: Point, axis: usize) -> f64 {
    if axis == 0 {
        point.x
    } else {
        point.y
    }
}

fn clip(polygon: &[Point], axis: usize, limit: f64, minimum: bool) -> Vec<Point> {
    let Some(&last) = polygon.last() else {
        return Vec::new();
    };
    let inside = |point| {
        if minimum {
            coordinate(point, axis) >= limit
        } else {
            coordinate(point, axis) <= limit
        }
    };
    let mut result = Vec::new();
    let mut previous = last;
    for &current in polygon {
        if inside(previous) != inside(current) {
            let fraction = (limit - coordinate(previous, axis))
                / (coordinate(current, axis) - coordinate(previous, axis));
            result.push(previous.towards(current, fraction));
        }
        if inside(current) {
            result.push(current);
        }
        previous = current;
    }
    result
}

fn centroid(polygon: &[Point]) -> Option<Point> {
    if polygon.len() < 3 {
        return None;
    }
    let mut area = 0.0;
    let mut x = 0.0;
    let mut y = 0.0;
    for index in 0..polygon.len() {
        let a = polygon[index];
        let b = polygon[(index + 1) % polygon.len()];
        let cross = a.x * b.y - b.x * a.y;
        area += cross;
        x += (a.x + b.x) * cross;
        y += (a.y + b.y) * cross;
    }
    if area.abs() < 1e-8 {
        None
    } else {
        Some(Point {
            x: x / (3.0 * area),
            y: y / (3.0 * area),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_round_trips_affine_and_perspective_quads() {
        for quad in [
            [80.0, 100.0, 480.0, 100.0, 480.0, 300.0, 80.0, 300.0],
            [80.0, 100.0, 480.0, 120.0, 400.0, 300.0, 120.0, 300.0],
            [80.0, 100.0, 440.0, 220.0, 360.0, 400.0, 0.0, 280.0],
        ] {
            let projection = Projection::new(quad, 400.0, 200.0).unwrap();
            for (point, expected) in [
                (Point { x: 0.0, y: 0.0 }, (quad[0], quad[1])),
                (Point { x: 400.0, y: 0.0 }, (quad[2], quad[3])),
                (Point { x: 400.0, y: 200.0 }, (quad[4], quad[5])),
                (Point { x: 0.0, y: 200.0 }, (quad[6], quad[7])),
            ] {
                let mapped = projection.forward(point).unwrap();
                assert!(
                    (mapped.x - expected.0).abs() < 1e-8 && (mapped.y - expected.1).abs() < 1e-8
                );
            }
            let point = Point { x: 123.5, y: 67.25 };
            let restored = projection
                .backward(projection.forward(point).unwrap())
                .unwrap();
            assert!((restored.x - point.x).abs() < 1e-8 && (restored.y - point.y).abs() < 1e-8);
        }
        assert!(Projection::new([0.0; 8], 400.0, 200.0).is_none());
        assert!(Projection::new([f64::INFINITY; 8], 400.0, 200.0).is_none());
    }

    #[test]
    fn searches_the_actual_overflow_strip_after_scrolling_an_oversized_control() {
        let quad = [-70.0, 100.0, 330.0, 100.0, 330.0, 150.0, -70.0, 150.0];
        for (right, expected) in [(160.0, 130.0), (103.0, 101.5)] {
            let points = click_candidates_in_rect(&[quad], [100.0, 100.0, right, 160.0]);
            assert!((points[0].x - expected).abs() < 1e-9);
            assert!((points[0].y - 125.0).abs() < 1e-9);
            assert!(points
                .iter()
                .all(|p| p.x >= 100.0 && p.x <= right && p.y >= 100.0 && p.y <= 150.0));
        }
        assert!(click_candidates_in_rect(&[quad], [100.0, 0.0, 100.0, 160.0]).is_empty());
        assert!(click_candidates_in_rect(&[quad], [f64::NAN, 0.0, 160.0, 160.0]).is_empty());
    }

    #[test]
    fn clips_both_axes_before_selecting_the_center() {
        let points = click_candidates(
            &[[-10.0, -20.0, 30.0, -20.0, 30.0, 40.0, -10.0, 40.0]],
            20.0,
            30.0,
        );
        assert_eq!(points[0], Point { x: 10.0, y: 15.0 });
        assert!(points
            .iter()
            .all(|p| p.x >= 0.0 && p.x <= 20.0 && p.y >= 0.0 && p.y <= 30.0));
    }

    #[test]
    fn rotated_clipping_keeps_candidates_in_the_visible_triangle() {
        let points = click_candidates(
            &[[-100.0, 10.0, 20.0, 130.0, 0.0, 150.0, -120.0, 30.0]],
            200.0,
            200.0,
        );
        assert!((points[0].x - 20.0 / 3.0).abs() < 1e-8);
        assert!((points[0].y - 130.0).abs() < 1e-8);
        assert!(points
            .iter()
            .all(|p| p.x >= 0.0 && p.y >= p.x + 110.0 - 1e-8 && p.y <= 150.0 - p.x + 1e-8));
    }

    #[test]
    fn searches_edges_when_a_central_band_is_covered() {
        let points = click_candidates(
            &[[0.0, 0.0, 100.0, 0.0, 100.0, 50.0, 0.0, 50.0]],
            200.0,
            200.0,
        );
        assert_eq!(points[0], Point { x: 50.0, y: 25.0 });
        assert!(points.iter().any(|p| p.x < 30.0));
        assert!(points.iter().any(|p| p.x > 70.0));
    }

    #[test]
    fn prioritizes_distinct_fragments_and_accepts_reversed_winding() {
        let points = click_candidates(
            &[
                [0.0, 0.0, 0.0, 10.0, 10.0, 10.0, 10.0, 0.0],
                [50.0, 0.0, 60.0, 0.0, 60.0, 10.0, 50.0, 10.0],
            ],
            100.0,
            100.0,
        );
        assert_eq!(
            &points[..2],
            &[Point { x: 5.0, y: 5.0 }, Point { x: 55.0, y: 5.0 }]
        );
    }

    #[test]
    fn rejects_nonfinite_degenerate_and_offscreen_quads() {
        for quad in [
            [f64::NAN; 8],
            [1.0; 8],
            [-10.0, -10.0, -5.0, -10.0, -5.0, -5.0, -10.0, -5.0],
        ] {
            assert!(click_candidates(&[quad], 100.0, 100.0).is_empty());
        }
        assert!(!click_candidates(
            &[[80.0, 80.0, 81.0, 80.0, 81.0, 81.0, 80.0, 81.0]],
            100.0,
            100.0
        )
        .is_empty());
    }

    #[test]
    fn limits_search_for_many_inline_fragments() {
        let quads: Vec<_> = (0..100)
            .map(|i| {
                let x = f64::from(i) * 10.0;
                [x, 0.0, x + 8.0, 0.0, x + 8.0, 8.0, x, 8.0]
            })
            .collect();
        assert!(click_candidates(&quads, 2000.0, 100.0).len() <= MAX_POINTS);
    }
}
