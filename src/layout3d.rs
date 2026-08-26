use crate::vec3::Vec3;

pub trait Layout3d {
    const PIXEL_COUNT: usize;

    fn shapes(&self) -> impl Iterator<Item = &Grid>;

    fn points(&self) -> impl Iterator<Item = Vec3> {
        self.shapes().flat_map(|s| s.points())
    }
}

pub struct CubeFaces {
    faces: [Grid; 6],
}

impl CubeFaces {
    const SIDES: usize = 6;
    const PIXEL_PER_SIDE: usize = 25;

    pub fn new() -> Self {
        Self {
            faces: [
                Grid {
                    start: Vec3::new(-14., 14., 19.),
                    horizontal_end: Vec3::new(14., 14., 19.),
                    vertical_end: Vec3::new(-14., -14., 19.),
                    horizontal_count: 5,
                    vertical_count: 5,
                },
                Grid {
                    start: Vec3::new(19., 14., 14.),
                    horizontal_end: Vec3::new(19., 14., -14.),
                    vertical_end: Vec3::new(19., -14., 14.),
                    horizontal_count: 5,
                    vertical_count: 5,
                },
                Grid {
                    start: Vec3::new(14., -19., 14.),
                    horizontal_end: Vec3::new(14., -19., -14.),
                    vertical_end: Vec3::new(-14., -19., 14.),
                    horizontal_count: 5,
                    vertical_count: 5,
                },
                Grid {
                    start: Vec3::new(14., -14., -19.),
                    horizontal_end: Vec3::new(14., 14., -19.),
                    vertical_end: Vec3::new(-14., -14., -19.),
                    horizontal_count: 5,
                    vertical_count: 5,
                },
                Grid {
                    start: Vec3::new(-19., -14., -14.),
                    horizontal_end: Vec3::new(-19., 14., -14.),
                    vertical_end: Vec3::new(-19., -14., 14.),
                    horizontal_count: 5,
                    vertical_count: 5,
                },
                Grid {
                    start: Vec3::new(-14., 19., -14.),
                    horizontal_end: Vec3::new(14., 19., -14.),
                    vertical_end: Vec3::new(-14., 19., 14.),
                    horizontal_count: 5,
                    vertical_count: 5,
                },
            ],
        }
    }
}

impl Layout3d for CubeFaces {
    const PIXEL_COUNT: usize = CubeFaces::SIDES * CubeFaces::PIXEL_PER_SIDE;

    fn shapes(&self) -> impl Iterator<Item = &Grid> {
        self.faces.iter()
    }
}

pub struct Grid {
    start: Vec3,
    horizontal_end: Vec3,
    vertical_end: Vec3,
    horizontal_count: usize,
    vertical_count: usize,
}

impl Grid {
    pub const fn pixel_count(&self) -> usize {
        self.vertical_count * self.horizontal_count
    }

    pub fn points(&self) -> GridIterator {
        let horizontal_step =
            (self.horizontal_end - self.start) / (self.horizontal_count as f32 - 1.);
        let vertical_step = (self.vertical_end - self.start) / (self.vertical_count as f32 - 1.);
        GridIterator::new(
            self.start,
            vertical_step,
            horizontal_step,
            self.horizontal_count,
            self.vertical_count,
        )
    }
}

pub struct GridIterator {
    start: Vec3,
    vertical_step: Vec3,
    horizontal_step: Vec3,
    row_count: usize,
    col_count: usize,
    row_index: usize,
    col_index: usize,
}

impl GridIterator {
    pub fn new(
        start: Vec3,
        vertical_step: Vec3,
        horizontal_step: Vec3,
        row_count: usize,
        col_count: usize,
    ) -> Self {
        Self {
            start,
            vertical_step,
            horizontal_step,
            row_count,
            col_count,
            row_index: 0,
            col_index: 0,
        }
    }
}

impl Iterator for GridIterator {
    type Item = Vec3;

    fn next(&mut self) -> Option<Self::Item> {
        if self.row_index >= self.row_count {
            return None;
        }

        let point = self.start
            + self.vertical_step * self.row_index as f32
            + self.horizontal_step * self.col_index as f32;

        self.col_index += 1;
        if self.col_index >= self.col_count {
            self.row_index += 1;
            self.col_index = 0;
        }
        Some(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_grid() -> Grid {
        Grid {
            start: Vec3::new(0., 0., 0.),
            horizontal_end: Vec3::new(4., 0., 0.),
            vertical_end: Vec3::new(0., 4., 0.),
            horizontal_count: 3,
            vertical_count: 3,
        }
    }

    #[test]
    fn grid_pixel_count_is_rows_times_cols() {
        let grid = test_grid();
        assert_eq!(grid.pixel_count(), 9);
    }

    #[test]
    fn grid_points_are_row_major_and_evenly_spaced() {
        let grid = test_grid();

        let expected = [
            Vec3::new(0., 0., 0.),
            Vec3::new(2., 0., 0.),
            Vec3::new(4., 0., 0.),
            Vec3::new(0., 2., 0.),
            Vec3::new(2., 2., 0.),
            Vec3::new(4., 2., 0.),
            Vec3::new(0., 4., 0.),
            Vec3::new(2., 4., 0.),
            Vec3::new(4., 4., 0.),
        ];

        assert!(grid.points().eq(expected));
    }

    #[test]
    fn grid_points_terminates_after_pixel_count() {
        // Regression test: `GridIterator` used to never advance `col_index`,
        // making it loop forever instead of stopping after `pixel_count()` items.
        let grid = test_grid();
        assert_eq!(grid.points().count(), grid.pixel_count());
    }

    #[test]
    fn cube_faces_has_six_shapes() {
        let cube = CubeFaces::new();
        assert_eq!(cube.shapes().count(), CubeFaces::SIDES);
    }

    #[test]
    fn cube_faces_pixel_count_matches_sides_times_pixels_per_side() {
        assert_eq!(
            CubeFaces::PIXEL_COUNT,
            CubeFaces::SIDES * CubeFaces::PIXEL_PER_SIDE
        );
    }

    #[test]
    fn cube_faces_points_yields_pixel_count_points() {
        let cube = CubeFaces::new();
        assert_eq!(cube.points().count(), CubeFaces::PIXEL_COUNT);
    }

    #[test]
    fn cube_faces_first_point_matches_first_face_start() {
        let cube = CubeFaces::new();
        let first_face = cube.shapes().next().expect("cube has at least one face");
        let first_point = first_face
            .points()
            .next()
            .expect("grid has at least one point");
        assert_eq!(first_point, first_face.start);
    }
}
