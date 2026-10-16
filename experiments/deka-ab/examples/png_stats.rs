//! Print a PNG's size and its most common pixel values (sanity checks).
fn main() {
    let path = std::env::args().nth(1).unwrap_or_default();
    let Some((w, h, px)) = deka_ab::common::read_png(std::path::Path::new(&path)) else {
        eprintln!("cannot read {path}");
        return;
    };
    let mut counts = std::collections::HashMap::<[u8; 4], usize>::new();
    for p in px.chunks_exact(4) {
        *counts.entry([p[0], p[1], p[2], p[3]]).or_default() += 1;
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1));
    println!("{w}x{h}, {} distinct; top: {:?}", v.len(), &v[..v.len().min(5)]);
}
