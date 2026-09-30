//! Small original synthesized score/effects, shared as PCM by both audio adapters.
pub const SAMPLE_RATE: u32 = 22050;
pub fn samples(id: u8) -> Vec<f32> {
    let duration = match id {
        0 => 0.065,
        1 => 0.4,
        _ => 8.,
    };
    let count = (duration * SAMPLE_RATE as f32) as usize;
    (0..count)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            match id {
                0 => {
                    let noise = ((i.wrapping_mul(1103515245).wrapping_add(12345) >> 8) & 65535)
                        as f32
                        / 32768.
                        - 1.;
                    noise * (1. - t / duration).powi(3) * 0.09
                }
                1 => {
                    let f = if t < 0.18 { 329.63 } else { 493.88 };
                    tone(t, f) * (1. - t / duration) * (t * 100.).min(1.) * 0.15
                }
                _ => {
                    let notes = [
                        261.63, 329.63, 392., 329.63, 293.66, 349.23, 440., 349.23, 261.63, 329.63,
                        523.25, 392., 293.66, 349.23, 392., 293.66,
                    ];
                    let beat = (t / 0.5) as usize;
                    let local = t % 0.5;
                    let envelope = (local * 35.).min(1.) * (1. - local / 0.5).powi(2);
                    let melody = tone(local, notes[beat % notes.len()]) * envelope * 0.09;
                    let bass = tone(t, if beat % 8 < 4 { 130.815 } else { 146.83 }) * 0.025;
                    let edge = (t * 20.).min(1.) * ((duration - t) * 20.).min(1.);
                    (melody + bass) * edge
                }
            }
        })
        .collect()
}
fn tone(t: f32, f: f32) -> f32 {
    let p = t * f * std::f32::consts::TAU;
    p.sin() * 0.8 + (p * 2.).sin() * 0.2
}
