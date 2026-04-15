// Converts brand.svg → assets/aurora.ico
// Run: cargo run --bin make_ico
use std::fs::File;
use std::io::Write;

fn main() {
    let svg_path = if std::path::Path::new("brand.svg").exists() {
        "brand.svg".to_string()
    } else {
        let exe = std::env::current_exe().unwrap();
        let dir = exe.parent().unwrap().parent().unwrap().parent().unwrap();
        dir.join("brand.svg").to_string_lossy().to_string()
    };

    let svg_data = std::fs::read_to_string(&svg_path).expect("Cannot read brand.svg");
    let sizes = [256u32, 128, 64, 48, 32, 16];
    let mut images: Vec<(u32, Vec<u8>)> = Vec::new();

    for &s in &sizes {
        // Parse and render SVG at each size
        let opt = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_str(&svg_data, &opt).expect("Failed to parse SVG");
        let mut pixmap = tiny_skia::Pixmap::new(s, s).unwrap();
        let scale = s as f32 / 512.0; // brand.svg is 512x512
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );

        // Encode as PNG
        let png_data = pixmap.encode_png().expect("PNG encode failed");
        images.push((s, png_data));
    }

    // Build ICO
    let count = images.len() as u16;
    let header_size = 6 + count as usize * 16;
    let mut offsets: Vec<u32> = Vec::new();
    let mut offset = header_size as u32;
    for (_, data) in &images {
        offsets.push(offset);
        offset += data.len() as u32;
    }

    let mut ico: Vec<u8> = Vec::new();
    ico.extend_from_slice(&[0u8, 0]); // reserved
    ico.extend_from_slice(&[1u8, 0]); // type: icon
    ico.extend_from_slice(&(count as u16).to_le_bytes());

    for (i, (s, data)) in images.iter().enumerate() {
        let sz = if *s >= 256 { 0u8 } else { *s as u8 };
        ico.push(sz);
        ico.push(sz);
        ico.push(0);
        ico.push(0);
        ico.extend_from_slice(&[1u8, 0]);
        ico.extend_from_slice(&[32u8, 0]);
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offsets[i].to_le_bytes());
    }
    for (_, data) in &images {
        ico.extend_from_slice(data);
    }

    let mut f = File::create("assets/aurora.ico").expect("Cannot create assets/aurora.ico");
    f.write_all(&ico).unwrap();
    println!(
        "Created assets/aurora.ico ({} bytes, {} sizes)",
        ico.len(),
        sizes.len()
    );
}
