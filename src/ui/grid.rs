pub struct GridInfo {
    pub cols: usize,
    pub rows: usize,
    pub cell_width: f32,
    pub cell_height: f32,
}

pub const MIN_CELL_WIDTH: f32 = 200.0;

/// Calculate a balanced grid layout where cells are as square as possible.
///
/// Instead of packing as many columns as the width allows (which produces
/// wide, short cells for e.g. 11 cameras at 1280px → 6 cols × 2 rows =
/// 213×330 cells), this iterates over every possible column count and picks
/// the layout that:
///
/// 1. Minimises empty slots (the grid should be as tight as possible)
/// 2. Keeps cells as square as possible (minimises |cell_w − cell_h|)
/// 3. Ensures every cell is at least `MIN_CELL_WIDTH` wide and tall
///
/// The result is a grid where every camera occupies an equal-sized cell and
/// at most one slot is left empty.
///
/// When the viewport is too small for any layout to keep cells at
/// `MIN_CELL_WIDTH` on both axes (many cameras in a tiled/half-snapped window,
/// common on Wayland/COSMIC), it falls back to the most balanced grid that
/// still respects the horizontal minimum — `ceil(sqrt(n))` columns — rather
/// than collapsing to a single tall column of letterbox slivers.
pub fn calc_grid(camera_count: usize, window_width: f32, available_height: f32) -> GridInfo {
    if camera_count == 0 {
        return GridInfo { cols: 0, rows: 0, cell_width: 0.0, cell_height: 0.0 };
    }

    let max_cols = ((window_width / MIN_CELL_WIDTH).floor() as usize).max(1);

    // Balanced fallback, used only if the search below finds nothing valid.
    let mut best_cols = ((camera_count as f32).sqrt().ceil() as usize)
        .clamp(1, max_cols.min(camera_count));
    let mut best_rows = camera_count.div_ceil(best_cols);
    let mut best_empty = camera_count; // worst case: every slot is empty
    let mut best_aspect_diff = f32::INFINITY;

    for cols in 1..=max_cols {
        let rows = camera_count.div_ceil(cols);
        let cell_w = window_width / cols as f32;
        let cell_h = available_height / rows as f32;

        if cell_w < MIN_CELL_WIDTH || cell_h < MIN_CELL_WIDTH {
            continue;
        }
        if rows as f32 * cell_h > available_height {
            continue;
        }

        let empty = cols * rows - camera_count;
        let aspect_diff = (cell_w - cell_h).abs();

        // Prefer fewer empty slots; break ties with more square cells.
        if empty < best_empty || (empty == best_empty && aspect_diff < best_aspect_diff) {
            best_empty = empty;
            best_aspect_diff = aspect_diff;
            best_cols = cols;
            best_rows = rows;
        }
    }

    GridInfo {
        cols: best_cols,
        rows: best_rows,
        cell_width: window_width / best_cols as f32,
        cell_height: available_height / best_rows as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: the grid must always fit within the window.
    fn invariant_fits(n: usize, w: f32, h: f32) {
        let g = calc_grid(n, w, h);
        assert!(g.cols * g.rows >= n, "n={n}: {}×{} < {}", g.cols, g.rows, n);
        assert!((g.cols as f32 * g.cell_width - w).abs() < 1.0, "n={n}: cols*cell_w={:.1} != {}", g.cols as f32 * g.cell_width, w);
        assert!((g.rows as f32 * g.cell_height - h).abs() < 1.0, "n={n}: rows*cell_h={:.1} != {}", g.rows as f32 * g.cell_height, h);
    }

    /// Helper: every cell must be at least MIN_CELL_WIDTH on the short side,
    /// and every camera in the grid gets the same cell size.
    fn invariant_equal_sized(n: usize, w: f32, h: f32) {
        let g = calc_grid(n, w, h);
        assert!(g.cell_width >= MIN_CELL_WIDTH, "n={n}: cell_w={}", g.cell_width);
    }

    #[test]
    fn single_camera() {
        let grid = calc_grid(1, 1280.0, 660.0);
        assert_eq!(grid.cols, 1);
        assert_eq!(grid.rows, 1);
    }

    #[test]
    fn four_cameras_square_grid() {
        // 4 cameras: cols=2 (empty=0, aspect=310) beats cols=4 (empty=0, aspect=340).
        let grid = calc_grid(4, 1280.0, 660.0);
        assert_eq!(grid.cols, 2);
        assert_eq!(grid.rows, 2);
        assert!((grid.cell_width - 640.0).abs() < 0.01);
        assert!((grid.cell_height - 330.0).abs() < 0.01);
    }

    #[test]
    fn five_cameras() {
        // 5 cameras: cols=5 (empty=0, aspect=404) wins over cols=2 (empty=1) and cols=3 (empty=1).
        let grid = calc_grid(5, 1280.0, 660.0);
        assert_eq!(grid.cols, 5);
        assert_eq!(grid.rows, 1);
        assert!((grid.cell_width - 256.0).abs() < 0.01);
        assert!((grid.cell_height - 660.0).abs() < 0.01);
    }

    #[test]
    fn six_cameras() {
        // 6 cameras: cols=3 (empty=0, aspect=96.67) wins over cols=2 (empty=0, aspect=420).
        let grid = calc_grid(6, 1280.0, 660.0);
        assert_eq!(grid.cols, 3);
        assert_eq!(grid.rows, 2);
        assert!((grid.cell_width - 1280.0 / 3.0).abs() < 0.01);
        assert!((grid.cell_height - 330.0).abs() < 0.01);
    }

    #[test]
    fn seven_cameras_wraps() {
        // 7 cameras: cols=4 (empty=1, aspect=10) wins over cols=3 (empty=2).
        let grid = calc_grid(7, 1280.0, 660.0);
        assert_eq!(grid.cols, 4);
        assert_eq!(grid.rows, 2);
        assert!((grid.cell_width - 320.0).abs() < 0.01);
        assert!((grid.cell_height - 330.0).abs() < 0.01);
    }

    #[test]
    fn eleven_cameras_4x3() {
        // 11 cameras: cols=4 (empty=1, aspect=100) wins over cols=6 (empty=1, aspect=116.67).
        let grid = calc_grid(11, 1280.0, 660.0);
        assert_eq!(grid.cols, 4);
        assert_eq!(grid.rows, 3);
        assert!((grid.cell_width - 320.0).abs() < 0.01);
        assert!((grid.cell_height - 220.0).abs() < 0.01);
    }

    #[test]
    fn many_cameras() {
        // 16 cameras at 1920×1000: cols=4 (empty=0, aspect=230) wins over cols=8 (empty=0, aspect=260).
        let grid = calc_grid(16, 1920.0, 1000.0);
        assert_eq!(grid.cols, 4);
        assert_eq!(grid.rows, 4);
        assert!((grid.cell_width - 480.0).abs() < 0.01);
        assert!((grid.cell_height - 250.0).abs() < 0.01);
    }

    #[test]
    fn cell_dimensions_fill_height() {
        let grid = calc_grid(4, 1280.0, 660.0);
        assert!((grid.cell_width - 640.0).abs() < 0.01);
        assert!((grid.cell_height - 330.0).abs() < 0.01);
    }

    #[test]
    fn cell_dimensions_two_rows() {
        // 8 cameras: cols=4 (empty=0, aspect=10) wins over cols=3 (empty=1).
        let grid = calc_grid(8, 1280.0, 660.0);
        assert_eq!(grid.cols, 4);
        assert_eq!(grid.rows, 2);
        assert!((grid.cell_width - 320.0).abs() < 0.01);
        assert!((grid.cell_height - 330.0).abs() < 0.01);
    }

    #[test]
    fn narrow_window() {
        // 400px wide → max 2 cols → 2×2 grid.
        let grid = calc_grid(4, 400.0, 660.0);
        assert_eq!(grid.cols, 2);
        assert_eq!(grid.rows, 2);
        assert!((grid.cell_width - 200.0).abs() < 0.01);
        assert!((grid.cell_height - 330.0).abs() < 0.01);
    }

    #[test]
    fn zero_cameras() {
        let grid = calc_grid(0, 1280.0, 660.0);
        assert_eq!(grid.cols, 0);
    }

    #[test]
    fn cells_are_never_below_minimum_width() {
        for &n in &[1usize, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 16, 20] {
            invariant_equal_sized(n, 1280.0, 660.0);
        }
    }

    #[test]
    fn grid_fits_within_window() {
        for &n in &[1usize, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 16, 20] {
            invariant_fits(n, 1280.0, 660.0);
        }
    }

    #[test]
    fn eleven_cameras_joinville_scenario() {
        // 11 cameras at 1280×720 (the default window size minus toolbar/sidebar).
        let grid = calc_grid(11, 1280.0, 660.0);
        assert!(grid.cols * grid.rows >= 11, "grid {}×{} must fit 11 cameras", grid.cols, grid.rows);
        assert!(grid.cell_width >= MIN_CELL_WIDTH, "cell_w={}", grid.cell_width);
    }
}
