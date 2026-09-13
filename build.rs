use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let path = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("codex-air.ico");
    // Original Air mark: three rising strokes. A DIB icon needs no runtime decoder.
    let side = 64usize;
    let bitmap_size = side * side * 4 + side * side / 8;
    let mut bytes = Vec::new();
    bytes.extend([0, 0, 1, 0, 1, 0, 64, 64, 0, 0, 1, 0, 32, 0]);
    bytes.extend(((40 + bitmap_size) as u32).to_le_bytes());
    bytes.extend(22u32.to_le_bytes());
    for value in [40u32, 64, 128] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend([1, 0, 32, 0]);
    for value in [0u32, bitmap_size as u32, 0, 0, 0, 0] {
        bytes.extend(value.to_le_bytes());
    }
    for y in (0..side).rev() {
        for x in 0..side {
            let stroke = [
                (15., 39., 26., 26.),
                (26., 39., 37., 26.),
                (37., 39., 48., 26.),
            ]
            .iter()
            .any(|&(ax, ay, bx, by)| {
                let dx = bx - ax;
                let dy = by - ay;
                let t = (((x as f32 - ax) * dx + (y as f32 - ay) * dy) / (dx * dx + dy * dy))
                    .clamp(0., 1.);
                (x as f32 - ax - t * dx).powi(2) + (y as f32 - ay - t * dy).powi(2) < 5.
            });
            bytes.extend(if stroke {
                [0xfa, 0xb4, 0x89, 255]
            } else {
                [0x2e, 0x1e, 0x1e, 255]
            });
        }
    }
    bytes.resize(bytes.len() + side * side / 8, 0);
    fs::write(&path, bytes).unwrap();
    winresource::WindowsResource::new()
        .set_icon(path.to_str().unwrap())
        .set("ProductName", "Codex Air")
        .set("FileDescription", "Codex Air")
        .set(
            "LegalCopyright",
            "Copyright 2026 Codex Air contributors. GPL-3.0-only.",
        )
        .compile()
        .expect("Failed to compile Windows application resources");
}
