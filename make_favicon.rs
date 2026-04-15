// Generates a small base64 favicon from logoAuro.png for embedding in HTML
// Run: cargo run --bin make_favicon
use image::imageops::FilterType;
use std::fs;

fn main() {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap().parent().unwrap();
    let src = if std::path::Path::new("logoAuro.png").exists() {
        "logoAuro.png".to_string()
    } else {
        dir.join("logoAuro.png").to_string_lossy().to_string()
    };

    let img = image::open(&src).expect("Cannot open logoAuro.png");

    for &size in &[32u32, 64, 128] {
        let resized = img
            .resize_exact(size, size, FilterType::Lanczos3)
            .to_rgba8();
        let mut buf = Vec::new();
        {
            use image::ImageEncoder;
            let enc = image::codecs::png::PngEncoder::new(&mut buf);
            enc.write_image(&resized, size, size, image::ExtendedColorType::Rgba8)
                .unwrap();
        }
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
        let out = format!("data:image/png;base64,{}", b64);
        let fname = format!("assets/favicon{}.b64", size);
        fs::write(&fname, &out).unwrap();
        println!("Written {} ({} bytes)", fname, out.len());
    }
}
