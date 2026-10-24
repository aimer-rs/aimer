//! Keyboard traversal over a list of options that may contain disabled rows.
//!
//! Every list control (`RadioGroup`, `Select`, `Autocomplete`) moves its
//! highlight the same way, so the rule lives here once. The functions are pure
//! over a length and an "is this row available" predicate, which lets the
//! autocomplete walk its filtered view without materialising it.

/// The first available index.
#[inline]
pub(crate) fn first(len: usize, available: impl Fn(usize) -> bool) -> Option<usize> {
    (0..len).find(|&index| available(index))
}

/// The last available index.
#[inline]
pub(crate) fn last(len: usize, available: impl Fn(usize) -> bool) -> Option<usize> {
    (0..len).rev().find(|&index| available(index))
}

/// The next available index after `from`, wrapping at the ends.
///
/// With no current index, moving forward lands on the first available row and
/// moving backward on the last. If `from` is the only available row it is
/// returned again, so the caller can tell "nowhere to go" (`None`) from "stay".
pub(crate) fn step(
    len: usize,
    from: Option<usize>,
    forward: bool,
    available: impl Fn(usize) -> bool,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(from) = from.filter(|&from| from < len) else {
        return if forward {
            first(len, available)
        } else {
            last(len, available)
        };
    };
    (1..=len)
        .map(|offset| {
            if forward {
                (from + offset) % len
            } else {
                (from + len - offset % len) % len
            }
        })
        .find(|&index| available(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: usize) -> bool {
        true
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        assert_eq!(step(3, Some(2), true, all), Some(0));
        assert_eq!(step(3, Some(0), false, all), Some(2));
    }

    #[test]
    fn stepping_skips_unavailable_rows() {
        let available = |index: usize| index != 1;
        assert_eq!(step(3, Some(0), true, available), Some(2));
        assert_eq!(step(3, Some(2), false, available), Some(0));
    }

    #[test]
    fn starting_from_nothing_lands_on_an_edge() {
        let available = |index: usize| index != 0;
        assert_eq!(step(3, None, true, available), Some(1));
        assert_eq!(step(3, None, false, available), Some(2));
    }

    #[test]
    fn a_lone_available_row_is_returned_again() {
        assert_eq!(step(3, Some(1), true, |index| index == 1), Some(1));
    }

    #[test]
    fn nothing_available_or_empty_yields_nowhere() {
        assert_eq!(step(0, None, true, all), None);
        assert_eq!(step(3, Some(0), true, |_| false), None);
        assert_eq!(first(3, |_| false), None);
        assert_eq!(last(0, all), None);
    }

    #[test]
    fn an_out_of_range_start_is_treated_as_nothing() {
        assert_eq!(step(2, Some(9), true, all), Some(0));
    }
}
