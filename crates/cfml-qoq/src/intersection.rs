//! Join-aware intersection builder + flat-vec storage.
//!
//! An *intersection* is a row of 1-based table-row indices (`0` = NULL sentinel
//! for an unmatched outer-join side), one entry per table in FROM order. The
//! set of intersections produced by joining N tables is stored column-major-ish
//! in [`Intersections`]: one flat `Vec<usize>` of length `width * n_rows` with
//! a constant `width = N`. This eliminates the per-row `Vec<usize>` allocation
//! that `Vec<Vec<usize>>` would impose (a 750k-row 2-table join previously paid
//! for 750k separate 2-element heap allocations).
//!
//! The builder seeds with the first table's rows and folds each JOIN in turn,
//! filtering by its `ON` predicate (evaluated by a caller-supplied closure that
//! holds the table data + expression evaluator). This is the Rust analogue of
//! BoxLang's `QoQIntersectionGenerator`.

use crate::ast::JoinType;

/// Flat-storage set of intersections.
///
/// `flat.len() == width * len()`; row `i` is `&flat[i*width .. (i+1)*width]`.
#[derive(Clone, Debug, Default)]
pub struct Intersections {
    pub width: usize,
    pub flat: Vec<usize>,
}

impl Intersections {
    #[inline]
    pub fn new(width: usize) -> Self {
        Self { width, flat: Vec::new() }
    }

    #[inline]
    pub fn with_capacity(width: usize, cap_rows: usize) -> Self {
        Self {
            width,
            flat: Vec::with_capacity(width.saturating_mul(cap_rows)),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        if self.width == 0 {
            0
        } else {
            self.flat.len() / self.width
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.flat.is_empty()
    }

    #[inline]
    pub fn row(&self, i: usize) -> &[usize] {
        let w = self.width;
        &self.flat[i * w..(i + 1) * w]
    }

    #[inline]
    pub fn push_row(&mut self, row: &[usize]) {
        debug_assert_eq!(row.len(), self.width);
        self.flat.extend_from_slice(row);
    }

    /// Iterate rows as `&[usize]` slices of length `width`.
    #[inline]
    pub fn iter(&self) -> std::slice::ChunksExact<'_, usize> {
        // chunks_exact panics on chunk size 0 — guard that here.
        if self.width == 0 {
            (&[][..]).chunks_exact(1)
        } else {
            self.flat.chunks_exact(self.width)
        }
    }

    /// Rayon parallel chunked iteration.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn par_chunks_rows(
        &self,
        rows_per_chunk: usize,
    ) -> impl rayon::iter::ParallelIterator<Item = InterChunk<'_>> {
        use rayon::prelude::*;
        let w = self.width.max(1);
        let cs = rows_per_chunk.max(1).saturating_mul(w);
        let width = self.width;
        self.flat
            .par_chunks(cs)
            .map(move |s| InterChunk { width, flat: s })
    }
}

/// A borrowed slice of [`Intersections`] addressable by row.
#[derive(Clone, Copy)]
pub struct InterChunk<'a> {
    pub width: usize,
    pub flat: &'a [usize],
}

impl<'a> InterChunk<'a> {
    #[inline]
    pub fn len(&self) -> usize {
        if self.width == 0 {
            0
        } else {
            self.flat.len() / self.width
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.flat.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> std::slice::ChunksExact<'a, usize> {
        if self.width == 0 {
            (&[][..]).chunks_exact(1)
        } else {
            self.flat.chunks_exact(self.width)
        }
    }
}

/// Build the set of row intersections for `table_rows` (row count per table,
/// FROM order) joined per `join_types` (one entry per table after the seed).
///
/// `eval_on(join_index, candidate)` returns whether `candidate` (a 1-based
/// intersection of width `join_index + 2`) satisfies that join's `ON` predicate;
/// callers return `true` for CROSS / comma joins (no `ON`).
pub fn build_intersections<F>(
    table_rows: &[usize],
    join_types: &[JoinType],
    max_size: usize,
    mut eval_on: F,
) -> Result<Intersections, usize>
where
    F: FnMut(usize, &[usize]) -> bool,
{
    if table_rows.is_empty() {
        return Ok(Intersections::new(0));
    }

    // Seed with the first table's rows: width=1, one entry per row.
    let mut inters = Intersections::with_capacity(1, table_rows[0]);
    for r in 1..=table_rows[0] {
        inters.flat.push(r);
    }

    let mut scratch: Vec<usize> = Vec::new();

    for (k, &jt) in join_types.iter().enumerate() {
        let right_rows = table_rows[k + 1];
        let upper_bound = inters.len().saturating_mul(right_rows);
        if upper_bound > max_size {
            return Err(upper_bound);
        }
        let width = k + 1; // width of `inters` rows going in
        let new_width = width + 1;
        let mut next = Intersections::new(new_width);

        match jt {
            JoinType::Inner | JoinType::Cross => {
                for inter in inters.iter() {
                    for r in 1..=right_rows {
                        scratch.clear();
                        scratch.extend_from_slice(inter);
                        scratch.push(r);
                        if eval_on(k, &scratch) {
                            next.flat.extend_from_slice(&scratch);
                        }
                    }
                }
            }
            JoinType::Left => {
                for inter in inters.iter() {
                    let mut matched = false;
                    for r in 1..=right_rows {
                        scratch.clear();
                        scratch.extend_from_slice(inter);
                        scratch.push(r);
                        if eval_on(k, &scratch) {
                            next.flat.extend_from_slice(&scratch);
                            matched = true;
                        }
                    }
                    if !matched {
                        next.flat.extend_from_slice(inter);
                        next.flat.push(0); // NULL right row
                    }
                }
            }
            JoinType::Right => {
                for r in 1..=right_rows {
                    let mut matched = false;
                    for inter in inters.iter() {
                        scratch.clear();
                        scratch.extend_from_slice(inter);
                        scratch.push(r);
                        if eval_on(k, &scratch) {
                            next.flat.extend_from_slice(&scratch);
                            matched = true;
                        }
                    }
                    if !matched {
                        for _ in 0..width {
                            next.flat.push(0);
                        }
                        next.flat.push(r);
                    }
                }
            }
            JoinType::Full => {
                let mut right_matched = vec![false; right_rows + 1];
                for inter in inters.iter() {
                    let mut matched = false;
                    for r in 1..=right_rows {
                        scratch.clear();
                        scratch.extend_from_slice(inter);
                        scratch.push(r);
                        if eval_on(k, &scratch) {
                            next.flat.extend_from_slice(&scratch);
                            matched = true;
                            right_matched[r] = true;
                        }
                    }
                    if !matched {
                        next.flat.extend_from_slice(inter);
                        next.flat.push(0);
                    }
                }
                for r in 1..=right_rows {
                    if !right_matched[r] {
                        for _ in 0..width {
                            next.flat.push(0);
                        }
                        next.flat.push(r);
                    }
                }
            }
        }
        inters = next;
    }

    Ok(inters)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(inters: &Intersections) -> Vec<Vec<usize>> {
        inters.iter().map(|r| r.to_vec()).collect()
    }

    fn build<F: FnMut(usize, &[usize]) -> bool>(
        rcounts: &[usize],
        jts: &[JoinType],
        f: F,
    ) -> Vec<Vec<usize>> {
        rows(&build_intersections(rcounts, jts, usize::MAX, f).unwrap())
    }

    // ON that matches when the two most-recent indices are equal (a "diagonal").
    fn diag(_k: usize, cand: &[usize]) -> bool {
        let n = cand.len();
        n >= 2 && cand[n - 1] == cand[n - 2]
    }

    fn always(_k: usize, _c: &[usize]) -> bool {
        true
    }

    #[test]
    fn single_table() {
        assert_eq!(build(&[3], &[], always), vec![vec![1], vec![2], vec![3]]);
        assert!(build(&[0], &[], always).is_empty());
    }

    #[test]
    fn cross_join() {
        let got = build(&[2, 2], &[JoinType::Cross], always);
        assert_eq!(got, vec![vec![1, 1], vec![1, 2], vec![2, 1], vec![2, 2]]);
    }

    #[test]
    fn inner_join_filters() {
        let got = build(&[3, 3], &[JoinType::Inner], diag);
        assert_eq!(got, vec![vec![1, 1], vec![2, 2], vec![3, 3]]);
    }

    #[test]
    fn left_join_null_fills_unmatched() {
        let got = build(&[3, 2], &[JoinType::Left], diag);
        assert_eq!(got, vec![vec![1, 1], vec![2, 2], vec![3, 0]]);
    }

    #[test]
    fn right_join_null_fills_unmatched() {
        let got = build(&[2, 3], &[JoinType::Right], diag);
        assert_eq!(got, vec![vec![1, 1], vec![2, 2], vec![0, 3]]);
    }

    #[test]
    fn full_join_null_fills_both_sides() {
        let got = build(&[3, 3], &[JoinType::Full], |_, c| {
            let n = c.len();
            n >= 2 && c[n - 1] == c[n - 2] && c[n - 1] <= 2
        });
        assert!(got.contains(&vec![1, 1]));
        assert!(got.contains(&vec![2, 2]));
        assert!(got.contains(&vec![3, 0]));
        assert!(got.contains(&vec![0, 3]));
        assert_eq!(got.len(), 4);
    }

    #[test]
    fn three_table_cross() {
        let got = build(&[2, 2, 2], &[JoinType::Cross, JoinType::Cross], always);
        assert_eq!(got.len(), 8);
        assert_eq!(got[0], vec![1, 1, 1]);
        assert_eq!(got[7], vec![2, 2, 2]);
    }
}
