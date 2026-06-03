use synapse_core::{PathPoint, PathSpec};

const EPSILON: f64 = 1.0e-9;
pub const DEFAULT_ARCLEN_LUT_SEGMENTS: usize = 2048;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PathError {
    #[error("path parameter t must be finite and within [0,1], got {t}")]
    InvalidT { t: f64 },
    #[error("{kind} requires at least {min} points, got {actual}")]
    NotEnoughPoints {
        kind: &'static str,
        min: usize,
        actual: usize,
    },
    #[error("{kind} point {index} must have finite coordinates")]
    NonFinitePoint { kind: &'static str, index: usize },
    #[error("{kind} parameter {field} must be finite, got {value}")]
    NonFiniteParameter {
        kind: &'static str,
        field: &'static str,
        value: f64,
    },
    #[error("{kind} parameter {field} must be greater than zero, got {value}")]
    NonPositiveParameter {
        kind: &'static str,
        field: &'static str,
        value: f64,
    },
    #[error("{kind} segment {index} is degenerate")]
    DegenerateSegment { kind: &'static str, index: usize },
    #[error("{kind} curve is degenerate")]
    DegenerateCurve { kind: &'static str },
    #[error("catmull_rom alpha must be finite and within [0,1], got {alpha}")]
    InvalidCatmullRomAlpha { alpha: f64 },
    #[error("catmull_rom tension must be finite and within [0,1], got {tension}")]
    InvalidCatmullRomTension { tension: f64 },
    #[error("sample count must be at least 2, got {samples}")]
    InvalidSampleCount { samples: usize },
    #[error("arc-length LUT segment count must be at least 1, got {segments}")]
    InvalidArcLengthSegments { segments: usize },
    #[error("path length is zero")]
    ZeroLengthPath,
    #[error("arc length s must be finite and within [0,{length}], got {s}")]
    InvalidArcLength { s: f64, length: f64 },
}

pub type PathResult<T> = Result<T, PathError>;

#[derive(Debug)]
pub struct SpatialPath<'a> {
    spec: &'a PathSpec,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ArcLengthEntry {
    t: f64,
    length: f64,
    point: PathPoint,
}

#[derive(Debug)]
pub struct ArcLengthPath<'a> {
    path: SpatialPath<'a>,
    lut: Vec<ArcLengthEntry>,
    length: f64,
}

impl<'a> SpatialPath<'a> {
    pub fn new(spec: &'a PathSpec) -> PathResult<Self> {
        validate_spec(spec)?;
        Ok(Self { spec })
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        is_closed_spec(self.spec)
    }

    pub fn point_at(&self, t: f64) -> PathResult<PathPoint> {
        validate_t(t)?;
        match self.spec {
            PathSpec::Line { from, to } => Ok(from.lerp(*to, t)),
            PathSpec::Arc {
                center,
                radius,
                start_angle_rad,
                sweep_angle_rad,
            } => arc_point(*center, *radius, *start_angle_rad, *sweep_angle_rad, t),
            PathSpec::Circle { center, radius } => {
                arc_point(*center, *radius, 0.0, std::f64::consts::TAU, t)
            }
            PathSpec::CubicBezier { p0, p1, p2, p3 } => Ok(cubic_bezier(*p0, *p1, *p2, *p3, t)),
            PathSpec::Polyline { points, closed } => polyline_point(points, *closed, t),
            PathSpec::CatmullRom {
                waypoints,
                alpha,
                tension,
                closed,
            } => catmull_rom_point(waypoints, *alpha, *tension, *closed, t),
        }
    }

    pub fn sample(&self, samples: usize) -> PathResult<Vec<PathPoint>> {
        if samples < 2 {
            return Err(PathError::InvalidSampleCount { samples });
        }

        let last = samples - 1;
        let mut points = Vec::with_capacity(samples);
        for index in 0..samples {
            points.push(self.point_at(index as f64 / last as f64)?);
        }
        Ok(points)
    }
}

impl<'a> ArcLengthPath<'a> {
    pub fn new(spec: &'a PathSpec) -> PathResult<Self> {
        Self::with_lut_segments(spec, DEFAULT_ARCLEN_LUT_SEGMENTS)
    }

    pub fn with_lut_segments(spec: &'a PathSpec, segments: usize) -> PathResult<Self> {
        if segments == 0 {
            return Err(PathError::InvalidArcLengthSegments { segments });
        }

        let path = SpatialPath::new(spec)?;
        let capacity = segments
            .checked_add(1)
            .ok_or(PathError::InvalidArcLengthSegments { segments })?;
        let mut lut = Vec::with_capacity(capacity);
        let mut previous = path.point_at(0.0)?;
        let mut length = 0.0;
        lut.push(ArcLengthEntry {
            t: 0.0,
            length,
            point: previous,
        });

        for index in 1..=segments {
            let t = index as f64 / segments as f64;
            let point = path.point_at(t)?;
            length += previous.distance_to(point);
            lut.push(ArcLengthEntry { t, length, point });
            previous = point;
        }

        if length <= EPSILON {
            return Err(PathError::ZeroLengthPath);
        }

        Ok(Self { path, lut, length })
    }

    #[must_use]
    pub const fn length(&self) -> f64 {
        self.length
    }

    pub fn point_at_arclen(&self, s: f64) -> PathResult<PathPoint> {
        validate_arc_length(s, self.length)?;
        if close_to(s, 0.0) {
            return Ok(self.lut[0].point);
        }
        if close_to(s, self.length) {
            return Ok(self.lut[self.lut.len() - 1].point);
        }

        let right_index = self
            .lut
            .partition_point(|entry| entry.length < s)
            .min(self.lut.len() - 1);
        let left_index = right_index.saturating_sub(1);
        let left = self.lut[left_index];
        let right = self.lut[right_index];
        let span = right.length - left.length;
        let t = if span <= EPSILON {
            right.t
        } else {
            (right.t - left.t).mul_add((s - left.length) / span, left.t)
        };
        self.path.point_at(t)
    }

    pub fn sample_arclen(&self, samples: usize) -> PathResult<Vec<PathPoint>> {
        if samples < 2 {
            return Err(PathError::InvalidSampleCount { samples });
        }

        let last = samples - 1;
        let mut points = Vec::with_capacity(samples);
        for index in 0..samples {
            points.push(self.point_at_arclen(self.length * index as f64 / last as f64)?);
        }
        Ok(points)
    }
}

pub fn path_point_at(spec: &PathSpec, t: f64) -> PathResult<PathPoint> {
    SpatialPath::new(spec)?.point_at(t)
}

pub fn sample_path(spec: &PathSpec, samples: usize) -> PathResult<Vec<PathPoint>> {
    SpatialPath::new(spec)?.sample(samples)
}

pub fn path_length(spec: &PathSpec) -> PathResult<f64> {
    Ok(ArcLengthPath::new(spec)?.length())
}

pub fn path_point_at_arclen(spec: &PathSpec, s: f64) -> PathResult<PathPoint> {
    ArcLengthPath::new(spec)?.point_at_arclen(s)
}

pub fn sample_path_arclen(spec: &PathSpec, samples: usize) -> PathResult<Vec<PathPoint>> {
    ArcLengthPath::new(spec)?.sample_arclen(samples)
}

fn validate_spec(spec: &PathSpec) -> PathResult<()> {
    match spec {
        PathSpec::Line { from, to } => {
            validate_points("line", &[*from, *to])?;
            ensure_segment("line", 0, *from, *to)
        }
        PathSpec::Arc {
            center,
            radius,
            start_angle_rad,
            sweep_angle_rad,
        } => {
            validate_points("arc", &[*center])?;
            validate_positive("arc", "radius", *radius)?;
            validate_finite_parameter("arc", "start_angle_rad", *start_angle_rad)?;
            validate_finite_parameter("arc", "sweep_angle_rad", *sweep_angle_rad)?;
            if sweep_angle_rad.abs() <= EPSILON {
                return Err(PathError::DegenerateCurve { kind: "arc" });
            }
            Ok(())
        }
        PathSpec::Circle { center, radius } => {
            validate_points("circle", &[*center])?;
            validate_positive("circle", "radius", *radius)
        }
        PathSpec::CubicBezier { p0, p1, p2, p3 } => {
            validate_points("cubic_bezier", &[*p0, *p1, *p2, *p3])?;
            if same_point(*p0, *p1) && same_point(*p0, *p2) && same_point(*p0, *p3) {
                return Err(PathError::DegenerateCurve {
                    kind: "cubic_bezier",
                });
            }
            Ok(())
        }
        PathSpec::Polyline { points, closed } => validate_polyline(points, *closed),
        PathSpec::CatmullRom {
            waypoints,
            alpha,
            tension,
            closed,
        } => validate_catmull_rom(waypoints, *alpha, *tension, *closed),
    }
}

fn validate_polyline(points: &[PathPoint], closed: bool) -> PathResult<()> {
    validate_points("polyline", points)?;
    let points = effective_points(points, closed);
    let min = 2;
    if points.len() < min {
        return Err(PathError::NotEnoughPoints {
            kind: "polyline",
            min,
            actual: points.len(),
        });
    }
    validate_segments("polyline", points, closed)
}

fn validate_catmull_rom(
    waypoints: &[PathPoint],
    alpha: f64,
    tension: f64,
    closed: bool,
) -> PathResult<()> {
    validate_points("catmull_rom", waypoints)?;
    if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
        return Err(PathError::InvalidCatmullRomAlpha { alpha });
    }
    if !tension.is_finite() || !(0.0..=1.0).contains(&tension) {
        return Err(PathError::InvalidCatmullRomTension { tension });
    }

    let points = effective_points(waypoints, closed);
    let min = if closed { 3 } else { 4 };
    if points.len() < min {
        return Err(PathError::NotEnoughPoints {
            kind: "catmull_rom",
            min,
            actual: points.len(),
        });
    }
    validate_segments("catmull_rom", points, closed)
}

fn validate_points(kind: &'static str, points: &[PathPoint]) -> PathResult<()> {
    for (index, point) in points.iter().enumerate() {
        if !point.is_finite() {
            return Err(PathError::NonFinitePoint { kind, index });
        }
    }
    Ok(())
}

fn validate_segments(kind: &'static str, points: &[PathPoint], closed: bool) -> PathResult<()> {
    let segment_count = segment_count(points, closed);
    for index in 0..segment_count {
        ensure_segment(
            kind,
            index,
            points[index],
            points[next_point_index(points.len(), index)],
        )?;
    }
    Ok(())
}

fn validate_positive(kind: &'static str, field: &'static str, value: f64) -> PathResult<()> {
    validate_finite_parameter(kind, field, value)?;
    if value <= 0.0 {
        return Err(PathError::NonPositiveParameter { kind, field, value });
    }
    Ok(())
}

fn validate_finite_parameter(
    kind: &'static str,
    field: &'static str,
    value: f64,
) -> PathResult<()> {
    if !value.is_finite() {
        return Err(PathError::NonFiniteParameter { kind, field, value });
    }
    Ok(())
}

fn ensure_segment(
    kind: &'static str,
    index: usize,
    from: PathPoint,
    to: PathPoint,
) -> PathResult<()> {
    if same_point(from, to) {
        return Err(PathError::DegenerateSegment { kind, index });
    }
    Ok(())
}

fn validate_t(t: f64) -> PathResult<()> {
    if !t.is_finite() || !(0.0..=1.0).contains(&t) {
        return Err(PathError::InvalidT { t });
    }
    Ok(())
}

fn validate_arc_length(s: f64, length: f64) -> PathResult<()> {
    if !s.is_finite() || s < 0.0 || s > length {
        return Err(PathError::InvalidArcLength { s, length });
    }
    Ok(())
}

fn is_closed_spec(spec: &PathSpec) -> bool {
    match spec {
        PathSpec::Line { .. } | PathSpec::CubicBezier { .. } => false,
        PathSpec::Arc {
            sweep_angle_rad, ..
        } => full_sweep(*sweep_angle_rad),
        PathSpec::Circle { .. } => true,
        PathSpec::Polyline { closed, .. } | PathSpec::CatmullRom { closed, .. } => *closed,
    }
}

fn arc_point(
    center: PathPoint,
    radius: f64,
    start_angle_rad: f64,
    sweep_angle_rad: f64,
    t: f64,
) -> PathResult<PathPoint> {
    let angle = if full_sweep(sweep_angle_rad) && close_to(t, 1.0) {
        start_angle_rad
    } else {
        sweep_angle_rad.mul_add(t, start_angle_rad)
    };

    Ok(PathPoint {
        x: radius.mul_add(angle.cos(), center.x),
        y: radius.mul_add(angle.sin(), center.y),
    })
}

fn cubic_bezier(p0: PathPoint, p1: PathPoint, p2: PathPoint, p3: PathPoint, t: f64) -> PathPoint {
    let inverse = 1.0 - t;
    let inverse2 = inverse * inverse;
    let t2 = t * t;
    let b0 = inverse2 * inverse;
    let b1 = 3.0 * inverse2 * t;
    let b2 = 3.0 * inverse * t2;
    let b3 = t2 * t;

    add(
        add(scale(p0, b0), scale(p1, b1)),
        add(scale(p2, b2), scale(p3, b3)),
    )
}

fn polyline_point(points: &[PathPoint], closed: bool, t: f64) -> PathResult<PathPoint> {
    let points = effective_points(points, closed);
    if closed && close_to(t, 1.0) {
        return Ok(points[0]);
    }

    let (segment, local_t) = segment_position(t, segment_count(points, closed));
    let from = points[segment];
    let to = points[next_point_index(points.len(), segment)];
    Ok(from.lerp(to, local_t))
}

fn catmull_rom_point(
    waypoints: &[PathPoint],
    alpha: f64,
    tension: f64,
    closed: bool,
    t: f64,
) -> PathResult<PathPoint> {
    let points = effective_points(waypoints, closed);
    if closed && close_to(t, 1.0) {
        return Ok(points[0]);
    }
    if !closed && close_to(t, 1.0) {
        return Ok(points[points.len() - 1]);
    }

    let (segment, local_t) = segment_position(t, segment_count(points, closed));
    if close_to(local_t, 0.0) {
        return Ok(points[segment]);
    }

    let p1 = points[segment];
    let p2 = points[next_point_index(points.len(), segment)];
    let p0 = catmull_neighbor(points, segment, -1, closed);
    let p3 = catmull_neighbor(points, segment, 2, closed);

    catmull_rom_segment(p0, p1, p2, p3, alpha, tension, local_t)
}

fn catmull_rom_segment(
    p0: PathPoint,
    p1: PathPoint,
    p2: PathPoint,
    p3: PathPoint,
    alpha: f64,
    tension: f64,
    local_t: f64,
) -> PathResult<PathPoint> {
    if close_to(local_t, 1.0) {
        return Ok(p2);
    }

    let t0 = 0.0;
    let t1 = knot(t0, p0, p1, alpha);
    let t2 = knot(t1, p1, p2, alpha);
    let t3 = knot(t2, p2, p3, alpha);
    let segment_span = t2 - t1;

    if t1 <= t0 || t2 <= t1 || t3 <= t2 {
        return Err(PathError::DegenerateCurve {
            kind: "catmull_rom",
        });
    }

    let m1 = catmull_start_tangent(p0, p1, p2, t0, t1, t2, segment_span, tension);
    let m2 = catmull_end_tangent(p1, p2, p3, t1, t2, t3, segment_span, tension);
    Ok(cubic_hermite(p1, m1, p2, m2, local_t))
}

#[allow(
    clippy::too_many_arguments,
    reason = "Catmull-Rom tangent is defined by three points and their four knot times plus span/tension; grouping them adds indirection without clarity"
)]
fn catmull_start_tangent(
    p0: PathPoint,
    p1: PathPoint,
    p2: PathPoint,
    t0: f64,
    t1: f64,
    t2: f64,
    segment_span: f64,
    tension: f64,
) -> PathPoint {
    let term1 = div(sub(p1, p0), t1 - t0);
    let term2 = div(sub(p2, p0), t2 - t0);
    let term3 = div(sub(p2, p1), t2 - t1);
    scale(
        add(sub(term1, term2), term3),
        segment_span * (1.0 - tension),
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "Catmull-Rom tangent is defined by three points and their four knot times plus span/tension; grouping them adds indirection without clarity"
)]
fn catmull_end_tangent(
    p1: PathPoint,
    p2: PathPoint,
    p3: PathPoint,
    t1: f64,
    t2: f64,
    t3: f64,
    segment_span: f64,
    tension: f64,
) -> PathPoint {
    let term1 = div(sub(p2, p1), t2 - t1);
    let term2 = div(sub(p3, p1), t3 - t1);
    let term3 = div(sub(p3, p2), t3 - t2);
    scale(
        add(sub(term1, term2), term3),
        segment_span * (1.0 - tension),
    )
}

fn cubic_hermite(p0: PathPoint, m0: PathPoint, p1: PathPoint, m1: PathPoint, t: f64) -> PathPoint {
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0_f64.mul_add(t3, -3.0 * t2) + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = (-2.0_f64).mul_add(t3, 3.0 * t2);
    let h11 = t3 - t2;

    add(
        add(scale(p0, h00), scale(m0, h10)),
        add(scale(p1, h01), scale(m1, h11)),
    )
}

fn catmull_neighbor(
    points: &[PathPoint],
    segment: usize,
    offset: isize,
    closed: bool,
) -> PathPoint {
    let len = points.len();
    if closed {
        let index = (segment as isize + offset).rem_euclid(len as isize) as usize;
        return points[index];
    }

    let index = segment as isize + offset;
    if index < 0 {
        return extrapolate(points[0], points[1]);
    }
    if index as usize >= len {
        return extrapolate(points[len - 1], points[len - 2]);
    }
    points[index as usize]
}

fn segment_count(points: &[PathPoint], closed: bool) -> usize {
    if closed {
        points.len()
    } else {
        points.len() - 1
    }
}

fn segment_position(t: f64, segment_count: usize) -> (usize, f64) {
    if close_to(t, 1.0) {
        return (segment_count - 1, 1.0);
    }
    let scaled = t * segment_count as f64;
    let index = (scaled.floor() as usize).min(segment_count - 1);
    (index, scaled - index as f64)
}

fn next_point_index(point_count: usize, index: usize) -> usize {
    if index + 1 == point_count {
        0
    } else {
        index + 1
    }
}

fn effective_points(points: &[PathPoint], closed: bool) -> &[PathPoint] {
    if closed && points.len() > 1 && same_point(points[0], points[points.len() - 1]) {
        &points[..points.len() - 1]
    } else {
        points
    }
}

fn knot(previous: f64, from: PathPoint, to: PathPoint, alpha: f64) -> f64 {
    previous + from.distance_to(to).powf(alpha)
}

fn full_sweep(sweep_angle_rad: f64) -> bool {
    close_to(sweep_angle_rad.abs(), std::f64::consts::TAU)
}

fn same_point(left: PathPoint, right: PathPoint) -> bool {
    left.distance_to(right) <= EPSILON
}

fn close_to(left: f64, right: f64) -> bool {
    (left - right).abs() <= EPSILON
}

fn extrapolate(anchor: PathPoint, neighbor: PathPoint) -> PathPoint {
    sub(scale(anchor, 2.0), neighbor)
}

fn add(left: PathPoint, right: PathPoint) -> PathPoint {
    PathPoint {
        x: left.x + right.x,
        y: left.y + right.y,
    }
}

fn sub(left: PathPoint, right: PathPoint) -> PathPoint {
    PathPoint {
        x: left.x - right.x,
        y: left.y - right.y,
    }
}

fn scale(point: PathPoint, scale: f64) -> PathPoint {
    PathPoint {
        x: point.x * scale,
        y: point.y * scale,
    }
}

fn div(point: PathPoint, divisor: f64) -> PathPoint {
    PathPoint {
        x: point.x / divisor,
        y: point.y / divisor,
    }
}
