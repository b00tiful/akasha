/// One position in a three-dimensional flight path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Waypoint {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// An insertion-ordered collection of waypoints.
#[derive(Debug, Default)]
pub struct FlightPath {
    waypoints: Vec<Waypoint>,
}

impl FlightPath {
    /// Add one waypoint to the end of the path.
    pub fn push(&mut self, waypoint: Waypoint) {
        self.waypoints.push(waypoint);
    }

    /// Return the number of stored waypoints.
    #[must_use]
    pub fn len(&self) -> usize {
        self.waypoints.len()
    }

    /// Return whether the path contains no waypoints.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.waypoints.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{FlightPath, Waypoint};

    #[test]
    fn preserves_insertion_count() {
        let mut path = FlightPath::default();
        path.push(Waypoint {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        });

        assert_eq!(path.len(), 1);
        assert!(!path.is_empty());
    }
}
