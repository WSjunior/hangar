// A compensação só compara alturas medidas na mesma geometria.
pub(super) fn geometry_changed<T: PartialEq>(previous: Option<T>, current: T) -> bool {
    previous.is_some_and(|previous| previous != current)
}

pub(super) fn held_slack(current: f32, before: f32, after: f32, max: f32) -> f32 {
    (current + before - after).clamp(0., max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_and_height_changes_invalidate_the_old_reference() {
        let large = (1200., 800.);
        assert!(!geometry_changed(None, large));
        assert!(!geometry_changed(Some(large), large));
        assert!(geometry_changed(Some(large), (820., 800.)));
        assert!(geometry_changed(Some(large), (1200., 600.)));
        assert!(geometry_changed(Some((820., 600.)), large));
    }

    #[test]
    fn content_shrink_is_held_and_growth_consumes_only_the_slack() {
        assert_eq!(held_slack(0., 400., 300., 160.), 100.);
        assert_eq!(held_slack(100., 300., 340., 160.), 60.);
        assert_eq!(held_slack(60., 340., 500., 160.), 0.);
        assert_eq!(held_slack(0., 500., 100., 160.), 160.);
        assert_eq!(held_slack(160., 100., 100., 160.), 160.);
    }
}
