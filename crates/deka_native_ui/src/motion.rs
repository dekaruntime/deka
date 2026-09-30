use crate::Style;
/// Analytic unit-mass damped spring; sampling is independent of frame cadence.
pub(crate) fn progress(style: &Style, elapsed: f64) -> (f32, bool) {
    if elapsed < 0. {
        return (0., false);
    }
    if style.duration_ms <= 0. {
        return (1., true);
    }
    if style.motion.stiffness > 0. {
        let t = elapsed as f32 / 1000.;
        let w = style.motion.stiffness.sqrt();
        let a = style.motion.damping / 2.;
        let displacement = if (a - w).abs() < 0.001 {
            (1. + w * t) * (-w * t).exp()
        } else if a < w {
            let d = (w * w - a * a).sqrt();
            (-a * t).exp() * ((d * t).cos() + a / d * (d * t).sin())
        } else {
            let d = (a * a - w * w).sqrt();
            let r1 = -a + d;
            let r2 = -a - d;
            (r2 * (r1 * t).exp() - r1 * (r2 * t).exp()) / (r2 - r1)
        };
        // Use an envelope rather than a zero crossing to decide when to stop.
        let decay = if a < w { a } else { a - (a * a - w * w).sqrt() };
        let done = elapsed >= 10000. || (1. + w * t) * (-decay * t).exp() < 0.001;
        return (if done { 1. } else { 1. - displacement }, done);
    }
    let p = (elapsed / style.duration_ms as f64).clamp(0., 1.) as f32;
    (
        match style.easing {
            1 => 1. - (1. - p).powi(3),
            2 => p * p * (3. - 2. * p),
            _ => p,
        },
        p >= 1.,
    )
}
pub(crate) fn presence(style: &Style, effect: u8) -> Style {
    let mut result = style.clone();
    if effect != 0 {
        result.opacity = 0.;
    }
    if effect == 2 {
        result.translate_y += 24.;
    }
    if effect == 3 {
        result.scale *= 0.85;
    }
    result
}
pub(crate) fn keyframes(style: &mut Style, elapsed: f64) -> bool {
    if style.motion.frames.is_empty() || style.duration_ms <= 0. {
        return false;
    }
    let duration = style.duration_ms as f64;
    let cycles = elapsed.max(0.) / duration;
    let infinite = style.motion.repeats == 0;
    let done = !infinite && cycles >= style.motion.repeats as f64;
    let cycle = if done {
        style.motion.repeats as u64 - 1
    } else {
        cycles.floor() as u64
    };
    let mut p = if done {
        1.
    } else {
        (cycles - cycle as f64) as f32
    };
    if style.motion.alternate && cycle % 2 == 1 {
        p = 1. - p;
    }
    for property in 0..5 {
        let frames: Vec<_> = style
            .motion
            .frames
            .iter()
            .filter(|f| f.property == property)
            .collect();
        if frames.is_empty() {
            continue;
        }
        let pair = frames
            .windows(2)
            .find(|w| p <= w[1].at)
            .unwrap_or(&frames[frames.len() - 2..]);
        let f = (p - pair[0].at) / (pair[1].at - pair[0].at);
        let value = pair[0].value + (pair[1].value - pair[0].value) * f;
        match property {
            0 => style.opacity *= value,
            1 => style.translate_x += value,
            2 => style.translate_y += value,
            3 => style.scale *= value,
            4 => style.rotate += value,
            _ => unreachable!(),
        }
    }
    !done
}
