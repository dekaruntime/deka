//! Original, code-drawn pixel artwork. No external game assets or sprite sheets.
use crate::scene::GlyphImage;
struct Pixels {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}
impl Pixels {
    fn new(w: usize, h: usize) -> Self {
        Self {
            width: w,
            height: h,
            rgba: vec![0; w * h * 4],
        }
    }
    fn box_at(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        for py in y.max(0)..(y + h).min(self.height as i32) {
            for px in x.max(0)..(x + w).min(self.width as i32) {
                let i = (py as usize * self.width + px as usize) * 4;
                self.rgba[i..i + 4].copy_from_slice(&[
                    (c >> 16) as u8,
                    (c >> 8) as u8,
                    c as u8,
                    255,
                ]);
            }
        }
    }
    fn oval(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        for py in 0..h {
            for px in 0..w {
                let dx = (px as f32 + 0.5) / w as f32 * 2. - 1.;
                let dy = (py as f32 + 0.5) / h as f32 * 2. - 1.;
                if dx * dx + dy * dy <= 1. {
                    self.box_at(x + px, y + py, 1, 1, c);
                }
            }
        }
    }
    fn finish(self, id: &str) -> GlyphImage {
        GlyphImage {
            id: format!("world-{id}"),
            width: self.width,
            height: self.height,
            rgba: self.rgba,
        }
    }
}
pub fn sprites() -> Vec<GlyphImage> {
    let mut result = vec![];
    for flowers in [false, true] {
        let mut p = Pixels::new(32, 32);
        for (x, y) in [(3, 8), (23, 4), (15, 23), (29, 27)] {
            p.box_at(x, y, 1, 3, 0x74a664);
            p.box_at(x + 2, y + 1, 1, 2, 0x74a664);
            p.box_at(x + 1, y + 3, 1, 1, 0xa7c98b);
        }
        if flowers {
            for (x, y) in [(9, 7), (24, 22)] {
                p.box_at(x, y, 1, 4, 0x4f894f);
                p.box_at(x - 1, y, 3, 2, 0xf0d6a0);
                p.box_at(x, y, 1, 1, 0xd78a75);
            }
        }
        result.push(p.finish(if flowers { "flowers" } else { "grass" }));
    }
    let mut t = Pixels::new(32, 48);
    t.oval(3, 37, 29, 9, 0x729a60);
    t.box_at(13, 25, 7, 17, 0x694f3c);
    t.box_at(15, 25, 3, 17, 0x92704b);
    t.box_at(11, 40, 11, 2, 0x694f3c);
    t.oval(2, 12, 29, 24, 0x315e46);
    t.oval(1, 9, 28, 24, 0x40774d);
    t.oval(5, 1, 23, 25, 0x4d8953);
    t.oval(5, 2, 19, 16, 0x6b9d5f);
    t.oval(3, 15, 13, 12, 0x659956);
    t.oval(18, 17, 10, 10, 0x386e49);
    for (x, y) in [(10, 5), (6, 20), (19, 11), (13, 27)] {
        t.box_at(x, y, 3, 1, 0x8bb66d);
    }
    result.push(t.finish("tree"));
    for (i, c) in [0xbc6261, 0x587ba0, 0x668657].into_iter().enumerate() {
        let mut h = Pixels::new(64, 64);
        h.oval(3, 53, 61, 11, 0x739760);
        h.box_at(5, 28, 54, 32, 0x685743);
        h.box_at(7, 29, 50, 29, 0xebd9ad);
        h.box_at(7, 53, 50, 5, 0xbda87e);
        h.box_at(7, 57, 50, 3, 0x826e52);
        for y in (33..53).step_by(6) {
            h.box_at(7, y, 50, 1, 0xddc99d);
        }
        for y in 0..27 {
            let left = (26 - y) / 2;
            h.box_at(left, 6 + y, 64 - 2 * left, 1, 0x493e42);
            h.box_at(left + 2, 6 + y, 60 - 2 * left, 1, c);
        }
        for y in (9..32).step_by(5) {
            h.box_at((32 - y) / 2, y, 64 - (32 - y), 1, 0x8a5253);
        }
        h.box_at(0, 32, 64, 3, 0x654e46);
        h.box_at(2, 32, 60, 1, 0xd49b78);
        h.box_at(46, 3, 7, 13, 0x725950);
        h.box_at(45, 2, 9, 3, 0xb59b80);
        for x in [12, 43] {
            h.box_at(x, 39, 9, 11, 0x887b59);
            h.box_at(x + 1, 40, 7, 8, 0x6499a2);
            h.box_at(x + 2, 41, 3, 3, 0xc3dfcd);
            h.box_at(x + 4, 40, 1, 8, 0xe9d6a7);
            h.box_at(x, 49, 9, 2, 0xf2e2b8);
        }
        h.box_at(26, 40, 13, 20, 0x76644e);
        h.box_at(28, 42, 9, 18, 0x544b40);
        h.box_at(29, 43, 7, 9, 0x938268);
        h.box_at(35, 53, 1, 2, 0xe2be72);
        h.box_at(24, 60, 17, 3, 0xc8bd99);
        h.box_at(23, 63, 19, 1, 0x8a8068);
        result.push(h.finish(&format!("house-{i}")));
    }
    for dir in 0..4 {
        for step in 0..2 {
            let mut p = Pixels::new(16, 24);
            p.oval(2, 20, 13, 4, 0x5f8261);
            p.box_at(5, 17, 3, 5 - step, 0x344752);
            p.box_at(9, 17 + step, 3, 5 - step, 0x344752);
            p.box_at(4, 21 - step, 4, 2, 0x3c3739);
            p.box_at(9, 21, 4, 2, 0x3c3739);
            p.box_at(4, 10, 9, 8, 0xbb7450);
            p.box_at(5, 11, 7, 5, 0xe5a862);
            p.box_at(3, 12 + step, 2, 5, 0xe9bd8c);
            p.box_at(12, 12 - step, 2, 5, 0xe9bd8c);
            p.box_at(5, 4, 7, 7, 0xe9bd8c);
            p.box_at(4, 3, 9, 4, 0x493d3b);
            p.box_at(5, 2, 7, 2, 0x654d40);
            if dir == 1 {
                p.box_at(5, 5, 7, 5, 0x654d40);
                p.box_at(5, 12, 7, 6, 0x5b7370);
                p.box_at(6, 13, 5, 4, 0x82948a);
            } else {
                if dir != 3 {
                    p.box_at(6, 7, 1, 1, 0x3b3436);
                }
                if dir != 2 {
                    p.box_at(10, 7, 1, 1, 0x3b3436);
                }
            }
            result.push(p.finish(&format!("hero-{dir}-{step}")));
        }
    }
    let mut p = Pixels::new(12, 12);
    p.box_at(5, 6, 2, 6, 0x785e41);
    p.box_at(1, 1, 11, 7, 0x76593f);
    p.box_at(2, 2, 9, 5, 0xe5cb97);
    p.box_at(4, 3, 5, 1, 0x9b8059);
    p.box_at(4, 5, 3, 1, 0x9b8059);
    result.push(p.finish("sign"));
    let mut p = Pixels::new(12, 10);
    p.box_at(0, 3, 12, 2, 0xd5c6a0);
    p.box_at(0, 7, 12, 2, 0xae9b78);
    p.box_at(2, 0, 3, 10, 0xf0ddb3);
    p.box_at(8, 0, 3, 10, 0xf0ddb3);
    result.push(p.finish("fence"));
    let mut p = Pixels::new(86, 58);
    p.oval(0, 0, 86, 58, 0x759460);
    p.oval(3, 2, 80, 53, 0xc6c6a0);
    p.oval(6, 4, 74, 48, 0x528a8d);
    p.oval(9, 6, 68, 42, 0x6aa7a4);
    for (x, y) in [(15, 15), (40, 10), (30, 31), (61, 24), (52, 41)] {
        p.box_at(x, y, 8, 1, 0xafd0ba);
        p.box_at(x + 3, y + 2, 5, 1, 0x85bdb1);
    }
    p.oval(17, 37, 9, 5, 0x658d58);
    p.box_at(21, 37, 2, 2, 0xe7cb91);
    result.push(p.finish("pond"));
    result
}
