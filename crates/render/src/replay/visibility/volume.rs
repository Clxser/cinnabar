//! Convex eye-to-near-plane intersection uses the same lens, rotation and near distance as rendering.

use glam::DVec3;
use recording_studio::camera::CameraPose;

const INTERSECTION_EPSILON: f64 = 1e-8;

#[derive(Clone, Copy)]
pub(super) struct Bounds {
    pub min: DVec3,
    pub max: DVec3,
}

impl Bounds {
    /// Finite points alone can enter the spatial index or its separating-axis queries.
    pub fn points(points: &[DVec3]) -> Option<Self> {
        let first = *points.first()?;
        let mut bounds = Self {
            min: first,
            max: first,
        };
        for &point in points {
            if !point.is_finite() {
                return None;
            }
            bounds.min = bounds.min.min(point);
            bounds.max = bounds.max.max(point);
        }
        Some(bounds)
    }

    /// Parent nodes conservatively contain every child volume.
    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Broad-phase overlap includes exact face contact with the near plane.
    pub fn overlaps(self, other: Self) -> bool {
        let tolerance = DVec3::splat(INTERSECTION_EPSILON);
        self.min.cmple(other.max + tolerance).all() && other.min.cmple(self.max + tolerance).all()
    }

    /// Binary corner order shares each positive edge with corner zero.
    pub fn corners(self) -> [DVec3; 8] {
        std::array::from_fn(|corner| {
            DVec3::from_array(std::array::from_fn(|axis| {
                if corner & (1 << axis) == 0 {
                    self.min[axis]
                } else {
                    self.max[axis]
                }
            }))
        })
    }
}

pub(super) struct NearVolume {
    points: [DVec3; 5],
    pub bounds: Bounds,
}

impl NearVolume {
    /// The apex and four near-plane corners form the clipped volume that must not cut a body.
    pub fn new(pose: CameraPose, size: [u32; 2]) -> Option<Self> {
        pose.validate().ok()?;
        if size.contains(&0) {
            return None;
        }
        let transform = crate::replay::scene::camera_transform(pose).ok()?;
        let projection = crate::replay::scene::camera_projection(pose, size);
        let near = projection.near;
        let height = near * (projection.fov * 0.5).tan();
        let width = height * projection.aspect_ratio;
        let origin = transform.translation.as_dvec3();
        let local = [
            glam::Vec3::ZERO,
            glam::Vec3::new(-width, -height, -near),
            glam::Vec3::new(width, -height, -near),
            glam::Vec3::new(width, height, -near),
            glam::Vec3::new(-width, height, -near),
        ];
        let points = local.map(|point| origin + (transform.rotation * point).as_dvec3());
        Some(Self {
            bounds: Bounds::points(&points)?,
            points,
        })
    }

    /// SAT handles rotation, nonuniform scale, shear and planar boxes without expanding to a world AABB.
    pub fn intersects(&self, corners: &[DVec3; 8]) -> bool {
        let box_edges = [
            corners[1] - corners[0],
            corners[2] - corners[0],
            corners[4] - corners[0],
        ];
        // A collapsed point or line has no drawable triangles.
        if box_edges.iter().enumerate().all(|(i, edge)| {
            box_edges
                .iter()
                .skip(i + 1)
                .all(|other| edge.cross(*other).length_squared() == 0.0)
        }) {
            return false;
        }
        let rays = std::array::from_fn::<_, 4, _>(|i| self.points[i + 1] - self.points[0]);
        let near_edges = [
            self.points[2] - self.points[1],
            self.points[4] - self.points[1],
        ];
        let separated = |axis: DVec3| {
            let length = axis.length();
            if length == 0.0 {
                return false;
            }
            let range = |points: &[DVec3]| {
                points
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                        let value = axis.dot(*point - self.points[0]);
                        (low.min(value), high.max(value))
                    })
            };
            let (a, b) = (range(corners), range(&self.points));
            let tolerance = length * INTERSECTION_EPSILON;
            a.1 + tolerance < b.0 || b.1 + tolerance < a.0
        };
        if separated(near_edges[0].cross(near_edges[1]))
            || (0..4).any(|i| separated(rays[i].cross(rays[(i + 1) % 4])))
            || (0..3).any(|i| separated(box_edges[i].cross(box_edges[(i + 1) % 3])))
        {
            return false;
        }
        for edge in box_edges {
            for other in near_edges.into_iter().chain(rays) {
                if separated(edge.cross(other)) {
                    return false;
                }
            }
        }
        true
    }
}
