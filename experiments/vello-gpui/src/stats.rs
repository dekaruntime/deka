use std::time::Duration;

#[derive(Default, Clone)]
pub struct Series(pub Vec<f64>);

impl Series {
    pub fn push(&mut self, d: Duration) {
        self.0.push(d.as_secs_f64() * 1000.0);
    }
    pub fn summary(&self) -> String {
        if self.0.is_empty() {
            return "n/a".into();
        }
        let mut v = self.0.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let pct = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        format!(
            "mean {:6.2}  p50 {:6.2}  p95 {:6.2}  max {:6.2} ms",
            mean,
            pct(0.5),
            pct(0.95),
            v[v.len() - 1]
        )
    }
    pub fn mean(&self) -> f64 {
        self.0.iter().sum::<f64>() / self.0.len().max(1) as f64
    }
}
