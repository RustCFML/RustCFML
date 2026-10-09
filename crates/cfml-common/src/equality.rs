//! Deep CFML value equality, shared by the stdlib (`arrayFind`,
//! `arrayContains`, …) and the VM's member-function dispatch (`indexOf`,
//! `lastIndexOf`) so the two cannot drift. It lives here rather than in
//! `cfml-stdlib` because the VM does not depend on that crate.

use crate::dynamic::CfmlValue;

/// Structural equality with CFML semantics: structs and arrays compare by
/// contents (with a backing-pointer short-circuit that also makes cyclic graphs
/// safe), queries by columns and cells, component instances by reference, and
/// everything else by string form.
pub fn deep_equal(a: &CfmlValue, b: &CfmlValue, nocase: bool) -> bool {
    match (a, b) {
        // Two strings compare BORROWED. They would otherwise fall to the
        // `as_string()` arm at the bottom, which allocates a fresh `String` for
        // each side on every comparison — so a scan like
        // `arrayFindNoCase( widgets, name )` over N strings allocated 2N of
        // them before it could answer. Same semantics as that arm, which also
        // uses `eq_ignore_ascii_case` for the nocase form.
        (CfmlValue::String(x), CfmlValue::String(y)) => {
            if nocase {
                x.as_str().eq_ignore_ascii_case(y.as_str())
            } else {
                x.as_str() == y.as_str()
            }
        }
        (CfmlValue::Struct(sa), CfmlValue::Struct(sb)) => {
            // Identity short-circuit: two references to the SAME backing handle
            // are equal without walking their contents. This is both correct (a
            // value equals itself) and essential for cycle safety — a
            // self-referential struct graph (e.g. a Wheels model with a circular
            // association: `profile.author = author; author.profile = profile`)
            // would otherwise recurse forever here. Lucee compares CFC instances
            // by reference, so `arrayContains(visited, obj)` detects an
            // already-seen object by identity, which is exactly how Wheels'
            // `allErrors(includeAssociations=true)` breaks the cycle. (Wheels
            // model.errorsSpec "handles circular reference" stack-overflowed the
            // whole TestBox suite without this.)
            if sa.backing_ptr() == sb.backing_ptr() {
                return true;
            }
            if sa.len() != sb.len() {
                return false;
            }
            for (k, va) in sa.iter() {
                match sb.get_ci(&k) {
                    Some(vb) => {
                        if !deep_equal(&va, &vb, nocase) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            true
        }
        (CfmlValue::Array(aa), CfmlValue::Array(ab)) => {
            // Same identity short-circuit as Struct (above) — same backing
            // handle is equal without recursing, so a cyclic array graph is safe.
            if aa.backing_ptr() == ab.backing_ptr() {
                return true;
            }
            let sa = aa.snapshot();
            let sb = ab.snapshot();
            sa.len() == sb.len()
                && sa
                    .iter()
                    .zip(sb.iter())
                    .all(|(x, y)| deep_equal(x, y, nocase))
        }
        // Queries compare STRUCTURALLY: same columns (case-insensitively, in
        // order) and the same cell values. Falling through to the `as_string()`
        // arm below made every query equal to every other one — every query
        // stringifies the same way — so `arrayFind(arr, someQuery)` matched the
        // first query in the array whatever it held, and Preside's
        // `getFolderAncestors()` cycle guard fired on the first parent (GH #473).
        (CfmlValue::Query(qa), CfmlValue::Query(qb)) => {
            if qa.backing_ptr() == qb.backing_ptr() {
                return true;
            }
            qa.with_read(|da| {
                qb.with_read(|db| {
                    if da.columns.len() != db.columns.len()
                        || da.row_count() != db.row_count()
                    {
                        return false;
                    }
                    if !da
                        .columns
                        .iter()
                        .zip(db.columns.iter())
                        .all(|(x, y)| x.eq_ignore_ascii_case(y))
                    {
                        return false;
                    }
                    for ci in 0..da.columns.len() {
                        for ri in 0..da.row_count() {
                            if !deep_equal(&da.data[ci][ri], &db.data[ci][ri], nocase) {
                                return false;
                            }
                        }
                    }
                    true
                })
            })
        }
        // A complex value never equals a scalar (or a struct-vs-array mismatch).
        (CfmlValue::Struct(_), _) | (_, CfmlValue::Struct(_)) => false,
        (CfmlValue::Array(_), _) | (_, CfmlValue::Array(_)) => false,
        (CfmlValue::Query(_), _) | (_, CfmlValue::Query(_)) => false,
        // Flyweight component instances compare by REFERENCE (Lucee/ACF parity —
        // same as the marker backing-ptr identity above). Without this an Instance
        // fell to the `_ => as_string()` arm below, where EVERY component
        // stringifies to "<Component>" → any two components compared EQUAL, so
        // `arrayContains(seen, obj)` matched the first component of any class
        // (breaking Wheels' circular-association cycle detection).
        _ if a.as_component().is_some_and(|c| c.is_instance_backed())
            || b.as_component().is_some_and(|c| c.is_instance_backed()) =>
        {
            crate::component::same_component_instance(a, b)
        }
        _ => {
            if nocase {
                a.as_string().eq_ignore_ascii_case(&b.as_string())
            } else {
                a.as_string() == b.as_string()
            }
        }
    }
}
