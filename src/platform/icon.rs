use tao::window::Icon;

pub fn load_aurora_icon() -> Option<Icon> {
    // Generate Aurora icon programmatically — teal phoenix on dark background
    let size = 128u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);

    for y in 0..size {
        for x in 0..size {
            let fx = x as f64 / size as f64;
            let fy = y as f64 / size as f64;
            let cx = fx - 0.5;
            let cy = fy - 0.5;
            let dist = (cx * cx + cy * cy).sqrt();

            if dist > 0.48 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let alpha = if dist > 0.44 {
                ((0.48 - dist) / 0.04 * 255.0) as u8
            } else {
                255
            };

            let mut r = 6.0f64;
            let mut g = 12.0f64;
            let mut b = 24.0f64;

            let body_dist = ((fx - 0.5).powi(2) * 2.0 + (fy - 0.45).powi(2)).sqrt();
            if body_dist < 0.28 {
                let i = (1.0 - body_dist / 0.28).powi(2);
                let t = fy;
                r += 20.0 * i * (1.0 - t);
                g += 255.0 * i * (0.3 + 0.7 * (1.0 - t));
                b += 200.0 * i * t;
            }

            let wl_dist = ((fx - 0.28).powi(2) + (fy - 0.38).powi(2) * 3.0).sqrt();
            if wl_dist < 0.2 && fx < 0.5 {
                let i = (1.0 - wl_dist / 0.2).powi(2) * 0.8;
                g += 200.0 * i;
                b += 160.0 * i;
            }
            let wr_dist = ((fx - 0.72).powi(2) + (fy - 0.38).powi(2) * 3.0).sqrt();
            if wr_dist < 0.2 && fx > 0.5 {
                let i = (1.0 - wr_dist / 0.2).powi(2) * 0.8;
                g += 200.0 * i;
                b += 160.0 * i;
            }

            let hd = ((fx - 0.5).powi(2) + (fy - 0.22).powi(2)).sqrt();
            if hd < 0.1 {
                let i = (1.0 - hd / 0.1).powi(2);
                r += 100.0 * i;
                g += 255.0 * i;
                b += 200.0 * i;
            }

            let td = ((fx - 0.5).powi(2) * 4.0 + (fy - 0.7).powi(2)).sqrt();
            if td < 0.2 && fy > 0.55 {
                let i = (1.0 - td / 0.2).powi(2) * 0.6;
                g += 120.0 * i;
                b += 220.0 * i;
                r += 40.0 * i;
            }

            let ed = ((fx - 0.5).powi(2) + (fy - 0.25).powi(2)).sqrt();
            if ed < 0.025 {
                r = 255.0;
                g = 255.0;
                b = 255.0;
            }

            rgba.extend_from_slice(&[
                r.min(255.0) as u8,
                g.min(255.0) as u8,
                b.min(255.0) as u8,
                alpha,
            ]);
        }
    }
    Icon::from_rgba(rgba, size, size).ok()
}
