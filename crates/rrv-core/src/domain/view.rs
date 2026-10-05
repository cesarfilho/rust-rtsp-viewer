//! View-mode logic: grid density presets, pagination math, carousel timing,
//! and camera ordering. Pure — no iced, no GStreamer, no I/O.
//!
//! The UI layer (`src/ui/`) owns the mutable runtime state (`current_page`,
//! timers); everything here is a value type or a pure function so `cargo test`
//! can exercise the arithmetic that used to live inline in `grid_layout.rs`.

/// Shortest carousel dwell time, in seconds.
pub const ROTATE_MIN_SECS: u64 = 3;
/// Longest carousel dwell time, in seconds.
pub const ROTATE_MAX_SECS: u64 = 300;
/// Default carousel dwell time when nothing is configured.
pub const ROTATE_DEFAULT_SECS: u64 = 10;
/// Step applied by the toolbar `+` / `-` buttons and the config parser.
pub const ROTATE_STEP_SECS: u64 = 5;
/// After a manual page change / camera pick, the carousel stays paused this
/// long so the user can actually look at what they navigated to.
pub const INTERACTION_PAUSE_SECS: u64 = 8;
/// Whether `sync_active_streams` keeps the *next* page's cameras decoding even
/// when the carousel is off, so a manual flip forward is instant. Costs one
/// extra page of live pipelines; off by default so `pause_hidden` actually
/// saves CPU on a 2-page grid. (With the carousel running the next page is
/// always kept hot regardless, for a seamless rotation.)
pub const PREFETCH_NEXT_PAGE: bool = false;

/// Grid density. `Auto` keeps the historical behaviour (every visible camera on
/// one balanced page, cell size chosen by `grid::calc_grid`). `Fixed` pins the
/// column/row count, which is also what enables pagination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridMode {
    #[default]
    Auto,
    Fixed {
        cols: u8,
        rows: u8,
    },
}

/// The fixed presets offered in the toolbar and cycled by the `g` shortcut,
/// in cycle order. `Auto` is prepended by [`GridMode::cycle`].
pub const FIXED_PRESETS: [(u8, u8); 3] = [(2, 2), (3, 3), (4, 4)];

impl GridMode {
    /// How many cells one page shows. `Auto` puts every visible camera on a
    /// single page (no pagination); `Fixed` is `cols * rows`.
    pub fn page_size(&self, visible_count: usize) -> usize {
        match self {
            GridMode::Auto => visible_count.max(1),
            GridMode::Fixed { cols, rows } => (*cols as usize * *rows as usize).max(1),
        }
    }

    /// `Fixed` column/row count, or `None` for `Auto` (which is laid out by
    /// `grid::calc_grid`).
    pub fn dims(&self) -> Option<(usize, usize)> {
        match self {
            GridMode::Auto => None,
            GridMode::Fixed { cols, rows } => Some((*cols as usize, *rows as usize)),
        }
    }

    /// Next mode for the `g` shortcut: Auto → 2×2 → 3×3 → 4×4 → Auto.
    pub fn cycle(self) -> GridMode {
        match self {
            GridMode::Auto => {
                let (c, r) = FIXED_PRESETS[0];
                GridMode::Fixed { cols: c, rows: r }
            }
            GridMode::Fixed { cols, rows } => {
                match FIXED_PRESETS
                    .iter()
                    .position(|&(c, r)| c == cols && r == rows)
                {
                    Some(i) if i + 1 < FIXED_PRESETS.len() => {
                        let (c, r) = FIXED_PRESETS[i + 1];
                        GridMode::Fixed { cols: c, rows: r }
                    }
                    // Last preset, or an unknown custom size → back to Auto.
                    _ => GridMode::Auto,
                }
            }
        }
    }

    /// Short label used by the toolbar pick-list and persisted to `view.toml`.
    pub fn as_str(&self) -> String {
        match self {
            GridMode::Auto => "auto".to_string(),
            GridMode::Fixed { cols, rows } => format!("{cols}x{rows}"),
        }
    }

    /// Parse `"auto"`, `"3x3"`, `"2x4"`, … Case- and space-insensitive.
    /// Unknown strings return `None` so the caller can warn and fall back.
    pub fn parse(s: &str) -> Option<GridMode> {
        let s = s.trim().to_lowercase();
        if s == "auto" {
            return Some(GridMode::Auto);
        }
        let (c, r) = s.split_once('x')?;
        let cols: u8 = c.trim().parse().ok()?;
        let rows: u8 = r.trim().parse().ok()?;
        if cols == 0 || rows == 0 || cols > 6 || rows > 6 {
            return None;
        }
        Some(GridMode::Fixed { cols, rows })
    }
}

impl std::fmt::Display for GridMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GridMode::Auto => write!(f, "Auto"),
            GridMode::Fixed { cols, rows } => write!(f, "{cols}×{rows}"),
        }
    }
}

/// Every choice that describes *how* the cameras are shown, as opposed to the
/// live runtime state (which page we're on, timer deadlines). Persisted to
/// `view.toml` and seeded from `[view]` in `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewSettings {
    pub mode: GridMode,
    pub rotate_enabled: bool,
    /// Carousel dwell time. Always kept within `ROTATE_MIN_SECS..=ROTATE_MAX_SECS`.
    pub rotate_secs: u64,
    /// Display order as a permutation of `0..n_cameras`. May be stale (fewer or
    /// more entries than cameras); [`normalize_order`] repairs it and
    /// [`apply_order`] tolerates it.
    pub order: Vec<usize>,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            mode: GridMode::Auto,
            rotate_enabled: false,
            rotate_secs: ROTATE_DEFAULT_SECS,
            order: Vec::new(),
        }
    }
}

impl ViewSettings {
    /// Clamp `rotate_secs` into range and make `order` a clean permutation of
    /// `0..n`. Call after loading from config / state.
    pub fn sanitize(&mut self, n_cameras: usize) {
        self.rotate_secs = self.rotate_secs.clamp(ROTATE_MIN_SECS, ROTATE_MAX_SECS);
        normalize_order(&mut self.order, n_cameras);
    }
}

/// Number of pages needed to show `visible_count` cameras `page_size` at a time.
/// Always at least 1, even with zero cameras.
pub fn page_count(visible_count: usize, page_size: usize) -> usize {
    if page_size == 0 {
        return 1;
    }
    visible_count.div_ceil(page_size).max(1)
}

/// Force `page` into `0..page_count`.
pub fn clamp_page(page: usize, page_count: usize) -> usize {
    if page_count == 0 {
        0
    } else {
        page.min(page_count - 1)
    }
}

/// Next page with wrap-around.
pub fn next_page(page: usize, page_count: usize) -> usize {
    if page_count == 0 {
        0
    } else {
        (page + 1) % page_count
    }
}

/// Previous page with wrap-around.
pub fn prev_page(page: usize, page_count: usize) -> usize {
    if page_count == 0 {
        0
    } else {
        (page + page_count - 1) % page_count
    }
}

/// The `[start, end)` index range into an ordered camera list that `page`
/// occupies. Clamped so it never runs past `len`.
pub fn page_slice(page: usize, page_size: usize, len: usize) -> std::ops::Range<usize> {
    if page_size == 0 || len == 0 {
        return 0..0;
    }
    let start = (page * page_size).min(len);
    let end = (start + page_size).min(len);
    start..end
}

/// Reorder `visible` (camera indices under the current group filter, ascending)
/// by the user's saved `order`. Indices missing from `order` are appended in
/// their natural order, so a stale/short `order` still yields a total order.
pub fn apply_order(order: &[usize], visible: &[usize]) -> Vec<usize> {
    let rank = |idx: usize| -> usize { order.iter().position(|&o| o == idx).unwrap_or(usize::MAX) };
    let mut out = visible.to_vec();
    // Stable sort keeps natural order among the "not in `order`" tail and among
    // any duplicate ranks.
    out.sort_by_key(|&idx| (rank(idx), idx));
    out
}

/// Make `order` a permutation of exactly `0..n`: drop out-of-range and
/// duplicate entries, then append whatever is missing in ascending order.
pub fn normalize_order(order: &mut Vec<usize>, n: usize) {
    let mut seen = vec![false; n];
    let mut cleaned = Vec::with_capacity(n);
    for &idx in order.iter() {
        if idx < n && !seen[idx] {
            seen[idx] = true;
            cleaned.push(idx);
        }
    }
    for (idx, was_seen) in seen.iter().enumerate() {
        if !was_seen {
            cleaned.push(idx);
        }
    }
    *order = cleaned;
}

/// Swap `cam_idx` with its predecessor in `order`. No-op if `cam_idx` is absent
/// or already first. `order` is assumed normalized.
pub fn move_up(order: &mut [usize], cam_idx: usize) {
    if let Some(pos) = order.iter().position(|&o| o == cam_idx)
        && pos > 0
    {
        order.swap(pos, pos - 1);
    }
}

/// Swap `cam_idx` with its successor in `order`. No-op if `cam_idx` is absent
/// or already last. `order` is assumed normalized.
pub fn move_down(order: &mut [usize], cam_idx: usize) {
    if let Some(pos) = order.iter().position(|&o| o == cam_idx)
        && pos + 1 < order.len()
    {
        order.swap(pos, pos + 1);
    }
}

/// Apply one `+`/`-` toolbar step to the carousel interval, staying in range.
pub fn step_rotate_secs(current: u64, up: bool) -> u64 {
    let next = if up {
        current.saturating_add(ROTATE_STEP_SECS)
    } else {
        current.saturating_sub(ROTATE_STEP_SECS)
    };
    next.clamp(ROTATE_MIN_SECS, ROTATE_MAX_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_puts_everything_on_one_page() {
        assert_eq!(GridMode::Auto.page_size(11), 11);
        assert_eq!(page_count(11, GridMode::Auto.page_size(11)), 1);
        // Never zero, even with no cameras.
        assert_eq!(GridMode::Auto.page_size(0), 1);
    }

    #[test]
    fn fixed_page_size_and_dims() {
        let m = GridMode::Fixed { cols: 3, rows: 3 };
        assert_eq!(m.page_size(11), 9);
        assert_eq!(m.dims(), Some((3, 3)));
        assert_eq!(GridMode::Auto.dims(), None);
    }

    #[test]
    fn page_count_rounds_up_and_floors_at_one() {
        assert_eq!(page_count(11, 9), 2);
        assert_eq!(page_count(9, 9), 1);
        assert_eq!(page_count(0, 9), 1);
        assert_eq!(page_count(10, 0), 1);
    }

    #[test]
    fn page_slice_is_clamped() {
        assert_eq!(page_slice(0, 9, 11), 0..9);
        assert_eq!(page_slice(1, 9, 11), 9..11);
        // Page past the end yields an empty range, not a panic.
        assert_eq!(page_slice(5, 9, 11), 11..11);
        assert_eq!(page_slice(0, 9, 0), 0..0);
    }

    #[test]
    fn paging_wraps_both_ways() {
        assert_eq!(next_page(0, 3), 1);
        assert_eq!(next_page(2, 3), 0);
        assert_eq!(prev_page(0, 3), 2);
        assert_eq!(prev_page(1, 3), 0);
        assert_eq!(next_page(0, 1), 0);
    }

    #[test]
    fn clamp_page_pins_into_range() {
        assert_eq!(clamp_page(5, 3), 2);
        assert_eq!(clamp_page(1, 3), 1);
        assert_eq!(clamp_page(0, 0), 0);
    }

    #[test]
    fn cycle_walks_auto_then_presets_then_back() {
        let m = GridMode::Auto;
        let m = m.cycle();
        assert_eq!(m, GridMode::Fixed { cols: 2, rows: 2 });
        let m = m.cycle();
        assert_eq!(m, GridMode::Fixed { cols: 3, rows: 3 });
        let m = m.cycle();
        assert_eq!(m, GridMode::Fixed { cols: 4, rows: 4 });
        assert_eq!(m.cycle(), GridMode::Auto);
        // Unknown custom size also returns to Auto.
        assert_eq!(GridMode::Fixed { cols: 5, rows: 2 }.cycle(), GridMode::Auto);
    }

    #[test]
    fn grid_mode_round_trips_through_string() {
        for m in [
            GridMode::Auto,
            GridMode::Fixed { cols: 2, rows: 2 },
            GridMode::Fixed { cols: 3, rows: 4 },
        ] {
            assert_eq!(GridMode::parse(&m.as_str()), Some(m));
        }
        assert_eq!(
            GridMode::parse("  3 X 3 "),
            Some(GridMode::Fixed { cols: 3, rows: 3 })
        );
        assert_eq!(GridMode::parse("nonsense"), None);
        assert_eq!(GridMode::parse("0x3"), None);
        assert_eq!(GridMode::parse("9x9"), None);
    }

    #[test]
    fn apply_order_honours_saved_order_and_tolerates_stale_entries() {
        // Full, valid permutation.
        assert_eq!(apply_order(&[2, 0, 1], &[0, 1, 2]), vec![2, 0, 1]);
        // Group filter: only some cameras visible, order still respected.
        assert_eq!(apply_order(&[3, 1, 2, 0], &[0, 2, 3]), vec![3, 2, 0]);
        // Stale order missing index 2 → 2 appended after the ranked ones.
        assert_eq!(apply_order(&[1, 0], &[0, 1, 2]), vec![1, 0, 2]);
        // Order with a bogus index is simply ignored.
        assert_eq!(apply_order(&[99, 1], &[0, 1]), vec![1, 0]);
        // Empty order → natural order.
        assert_eq!(apply_order(&[], &[0, 1, 2]), vec![0, 1, 2]);
    }

    #[test]
    fn normalize_order_produces_a_clean_permutation() {
        let mut o = vec![2, 0];
        normalize_order(&mut o, 4);
        assert_eq!(o, vec![2, 0, 1, 3]);

        let mut o = vec![5, 1, 1, 0, 9];
        normalize_order(&mut o, 3);
        assert_eq!(o, vec![1, 0, 2]);

        let mut o = vec![];
        normalize_order(&mut o, 3);
        assert_eq!(o, vec![0, 1, 2]);

        let mut o = vec![0, 1, 2];
        normalize_order(&mut o, 0);
        assert_eq!(o, Vec::<usize>::new());
    }

    #[test]
    fn move_up_down_swap_neighbours_and_clamp_at_ends() {
        let mut o = vec![0, 1, 2, 3];
        move_up(&mut o, 2);
        assert_eq!(o, vec![0, 2, 1, 3]);
        move_down(&mut o, 2);
        assert_eq!(o, vec![0, 1, 2, 3]);
        // No-ops at the ends.
        move_up(&mut o, 0);
        assert_eq!(o, vec![0, 1, 2, 3]);
        move_down(&mut o, 3);
        assert_eq!(o, vec![0, 1, 2, 3]);
        // Absent index is ignored.
        move_up(&mut o, 99);
        assert_eq!(o, vec![0, 1, 2, 3]);
    }

    #[test]
    fn step_rotate_secs_clamps() {
        assert_eq!(step_rotate_secs(10, true), 15);
        assert_eq!(step_rotate_secs(10, false), 5);
        assert_eq!(step_rotate_secs(ROTATE_MIN_SECS, false), ROTATE_MIN_SECS);
        assert_eq!(step_rotate_secs(ROTATE_MAX_SECS, true), ROTATE_MAX_SECS);
        assert_eq!(step_rotate_secs(4, false), ROTATE_MIN_SECS);
    }

    #[test]
    fn sanitize_clamps_interval_and_fixes_order() {
        let mut v = ViewSettings {
            mode: GridMode::Auto,
            rotate_enabled: true,
            rotate_secs: 9999,
            order: vec![3, 1],
        };
        v.sanitize(4);
        assert_eq!(v.rotate_secs, ROTATE_MAX_SECS);
        assert_eq!(v.order, vec![3, 1, 0, 2]);
    }
}
