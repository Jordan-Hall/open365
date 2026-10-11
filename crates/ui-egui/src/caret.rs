//! Blink timing of the text caret (Windows: shown for one blink time, hidden for one, restarted by input).

/// Whether a caret is shown `elapsed` seconds after its last restart (keystroke, caret move or focus gain), and
/// when the next toggle is due. `half` is the system blink time; `None` is INFINITE: always shown, no repaint needed.
#[must_use]
pub fn blink_phase(elapsed: f64, half: Option<f64>) -> (bool, Option<f64>) {
    let Some(half) = half.filter(|h| *h > 0.0) else {
        return (true, None);
    };
    let elapsed = elapsed.max(0.0);
    let cycle = 2.0 * half;
    let in_cycle = elapsed % cycle;
    if in_cycle < half {
        (true, Some(half - in_cycle))
    } else {
        (false, Some(cycle - in_cycle))
    }
}

#[cfg(test)]
mod tests {
    use super::blink_phase;

    #[test]
    fn shown_then_hidden_each_blink_time() {
        assert_eq!(blink_phase(0.0, Some(0.5)), (true, Some(0.5)));
        assert!(blink_phase(0.49, Some(0.5)).0);
        assert_eq!(blink_phase(0.5, Some(0.5)), (false, Some(0.5)));
        assert!(blink_phase(1.0, Some(0.5)).0);
    }

    #[test]
    fn infinite_blink_time_never_hides() {
        assert_eq!(blink_phase(123.0, None), (true, None));
    }
}
