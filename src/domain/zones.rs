use serde::Deserialize;

/// A 2D point in normalized coordinates (0.0..=1.0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn is_valid(&self) -> bool {
        self.x >= 0.0 && self.x <= 1.0 && self.y >= 0.0 && self.y <= 1.0
    }
}

/// A named zone (polygon) for motion detection filtering.
/// Motion inside the zone triggers events; motion outside is ignored.
#[derive(Debug, Clone)]
pub struct MotionZone {
    /// Zone name (e.g. "Driveway").
    pub name: String,
    /// Polygon vertices in normalized coordinates.
    pub vertices: Vec<Point>,
    /// Whether this zone is currently active.
    pub enabled: bool,
}

impl MotionZone {
    pub fn new(name: impl Into<String>, vertices: Vec<Point>) -> Self {
        Self { name: name.into(), vertices, enabled: true }
    }

    /// Check if a point (in normalized 0..=1 coords) is inside
    /// this zone using the ray-casting algorithm.
    pub fn contains(&self, point: Point) -> bool {
        if self.vertices.len() < 3 || !self.enabled {
            return false;
        }

        let n = self.vertices.len();
        let mut inside = false;
        let mut j = n - 1;

        for i in 0..n {
            let vi = self.vertices[i];
            let vj = self.vertices[j];

            if ((vi.y > point.y) != (vj.y > point.y))
                && (point.x < (vj.x - vi.x) * (point.y - vi.y) / (vj.y - vi.y) + vi.x)
            {
                inside = !inside;
            }
            j = i;
        }

        inside
    }

    /// Number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// Whether the polygon is valid (at least 3 vertices).
    pub fn is_valid(&self) -> bool {
        self.vertices.len() >= 3 && self.vertices.iter().all(|p| p.is_valid())
    }
}

/// TOML mirror for a zone entry.
#[derive(Debug, Deserialize, Clone)]
pub struct MotionZoneFile {
    pub name: String,
    pub vertices: Vec<PointFile>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PointFile {
    pub x: f64,
    pub y: f64,
}

impl MotionZoneFile {
    pub fn into_zone(self) -> MotionZone {
        MotionZone {
            name: self.name,
            vertices: self.vertices.into_iter()
                .map(|p| Point::new(p.x, p.y))
                .collect(),
            enabled: self.enabled.unwrap_or(true),
        }
    }
}

/// Per-camera zone configuration.
#[derive(Debug, Clone, Default)]
pub struct ZoneConfig {
    pub zones: Vec<MotionZone>,
}

impl ZoneConfig {
    /// Check if a point is inside any active zone.
    /// If no zones are defined, returns true (all motion counts).
    pub fn is_motion_allowed(&self, point: Point) -> bool {
        if self.zones.is_empty() {
            return true;
        }
        self.zones.iter()
            .filter(|z| z.enabled)
            .any(|z| z.contains(point))
    }

    /// Filter a list of changed pixel coordinates to only those
    /// inside active zones. Coordinates are in normalized 0..=1.
    pub fn filter_motion_points(&self, points: &[(f64, f64)]) -> Vec<(f64, f64)> {
        if self.zones.is_empty() {
            return points.to_vec();
        }
        points.iter()
            .copied()
            .filter(|&(x, y)| self.is_motion_allowed(Point::new(x, y)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_zone() -> MotionZone {
        MotionZone::new("test", vec![
            Point::new(0.25, 0.25),
            Point::new(0.75, 0.25),
            Point::new(0.75, 0.75),
            Point::new(0.25, 0.75),
        ])
    }

    #[test]
    fn point_valid() {
        assert!(Point::new(0.5, 0.5).is_valid());
        assert!(Point::new(0.0, 0.0).is_valid());
        assert!(Point::new(1.0, 1.0).is_valid());
        assert!(!Point::new(-0.1, 0.5).is_valid());
        assert!(!Point::new(0.5, 1.1).is_valid());
    }

    #[test]
    fn zone_contains_center() {
        let z = square_zone();
        assert!(z.contains(Point::new(0.5, 0.5)));
    }

    #[test]
    fn zone_outside() {
        let z = square_zone();
        assert!(!z.contains(Point::new(0.1, 0.1)));
        assert!(!z.contains(Point::new(0.9, 0.9)));
    }

    #[test]
    fn zone_disabled() {
        let mut z = square_zone();
        z.enabled = false;
        assert!(!z.contains(Point::new(0.5, 0.5)));
    }

    #[test]
    fn zone_too_few_vertices() {
        let z = MotionZone::new("t", vec![Point::new(0.0, 0.0), Point::new(1.0, 1.0)]);
        assert!(!z.contains(Point::new(0.5, 0.5)));
    }

    #[test]
    fn zone_valid() {
        assert!(square_zone().is_valid());
        let z = MotionZone::new("t", vec![Point::new(0.0, 0.0)]);
        assert!(!z.is_valid());
    }

    #[test]
    fn zone_config_no_zones_allows_all() {
        let config = ZoneConfig::default();
        assert!(config.is_motion_allowed(Point::new(0.5, 0.5)));
    }

    #[test]
    fn zone_config_filters_points() {
        let config = ZoneConfig {
            zones: vec![square_zone()],
        };
        let points = vec![
            (0.5, 0.5), // inside
            (0.1, 0.1), // outside
        ];
        let filtered = config.filter_motion_points(&points);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0], (0.5, 0.5));
    }

    #[test]
    fn zone_config_multiple_zones() {
        let config = ZoneConfig {
            zones: vec![
                MotionZone::new("a", vec![
                    Point::new(0.0, 0.0),
                    Point::new(0.3, 0.0),
                    Point::new(0.3, 0.3),
                    Point::new(0.0, 0.3),
                ]),
                MotionZone::new("b", vec![
                    Point::new(0.7, 0.7),
                    Point::new(1.0, 0.7),
                    Point::new(1.0, 1.0),
                    Point::new(0.7, 1.0),
                ]),
            ],
        };
        assert!(config.is_motion_allowed(Point::new(0.1, 0.1)));
        assert!(!config.is_motion_allowed(Point::new(0.5, 0.5)));
        assert!(config.is_motion_allowed(Point::new(0.8, 0.8)));
    }

    #[test]
    fn zone_vertex_count() {
        assert_eq!(square_zone().vertex_count(), 4);
    }

    #[test]
    fn zone_file_conversion() {
        let f = MotionZoneFile {
            name: "Driveway".into(),
            vertices: vec![
                PointFile { x: 0.0, y: 0.0 },
                PointFile { x: 1.0, y: 0.0 },
                PointFile { x: 1.0, y: 1.0 },
            ],
            enabled: Some(false),
        };
        let z = f.into_zone();
        assert_eq!(z.name, "Driveway");
        assert_eq!(z.vertices.len(), 3);
        assert!(!z.enabled);
    }
}
