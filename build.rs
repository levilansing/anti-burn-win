use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "src/icon.rs"]
mod icon;

fn build_ico_bytes(sizes: &[usize]) -> Vec<u8> {
    let mut images: Vec<Vec<u8>> = Vec::with_capacity(sizes.len());

    for &sz in sizes {
        let pixels = icon::render_icon_bgra(sz);
        // 40-byte BITMAPINFOHEADER + bottom-up 32-bit BGRA XOR mask + 32-bit padded 1-bit AND mask
        let xor_bytes = sz * sz * 4;
        let and_row_stride = ((sz + 31) / 32) * 4;
        let and_bytes = and_row_stride * sz;
        let total_size = 40 + xor_bytes + and_bytes;

        let mut dib = Vec::with_capacity(total_size);
        // biSize = 40
        dib.extend_from_slice(&40u32.to_le_bytes());
        // biWidth = sz
        dib.extend_from_slice(&(sz as i32).to_le_bytes());
        // biHeight = sz * 2 (XOR + AND combined height in ICO format)
        dib.extend_from_slice(&((sz as i32) * 2).to_le_bytes());
        // biPlanes = 1
        dib.extend_from_slice(&1u16.to_le_bytes());
        // biBitCount = 32
        dib.extend_from_slice(&32u16.to_le_bytes());
        // biCompression = BI_RGB (0)
        dib.extend_from_slice(&0u32.to_le_bytes());
        // biSizeImage
        dib.extend_from_slice(&((xor_bytes + and_bytes) as u32).to_le_bytes());
        // biXPelsPerMeter, biYPelsPerMeter, biClrUsed, biClrImportant
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());

        // Write bottom-up BGRA pixels
        for y in (0..sz).rev() {
            for x in 0..sz {
                let px = pixels[y * sz + x];
                dib.extend_from_slice(&px.to_le_bytes());
            }
        }

        // Write bottom-up 1-bit AND mask (1 = transparent where alpha == 0, 0 = opaque)
        for y in (0..sz).rev() {
            let mut row = vec![0u8; and_row_stride];
            for x in 0..sz {
                let alpha = (pixels[y * sz + x] >> 24) & 0xFF;
                if alpha == 0 {
                    row[x / 8] |= 1 << (7 - (x % 8));
                }
            }
            dib.extend_from_slice(&row);
        }

        images.push(dib);
    }

    let count = sizes.len() as u16;
    let mut ico = Vec::new();
    // ICONDIR header (6 bytes)
    ico.extend_from_slice(&0u16.to_le_bytes()); // Reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // Type = 1 (Icon)
    ico.extend_from_slice(&count.to_le_bytes());

    let mut offset = 6u32 + (count as u32) * 16;
    for (i, &sz) in sizes.iter().enumerate() {
        let img_len = images[i].len() as u32;
        let w_byte = if sz >= 256 { 0u8 } else { sz as u8 };
        let h_byte = if sz >= 256 { 0u8 } else { sz as u8 };
        ico.push(w_byte);
        ico.push(h_byte);
        ico.push(0); // bColorCount
        ico.push(0); // bReserved
        ico.extend_from_slice(&1u16.to_le_bytes()); // wPlanes = 1
        ico.extend_from_slice(&32u16.to_le_bytes()); // wBitCount = 32
        ico.extend_from_slice(&img_len.to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += img_len;
    }

    for img in images {
        ico.extend_from_slice(&img);
    }

    ico
}

fn find_rc_exe() -> Option<PathBuf> {
    let candidates = [
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\rc.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\rc.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\rc.exe",
    ];
    for c in &candidates {
        let p = Path::new(c);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }

    // Fallback: scan Windows Kits\10\bin for any x64\rc.exe
    let kits_bin = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(entries) = fs::read_dir(kits_bin) {
        for entry in entries.flatten() {
            let p = entry.path().join("x64").join("rc.exe");
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=build.rs");

    let sizes = [256usize, 64, 48, 32, 24, 16];
    let ico_bytes = build_ico_bytes(&sizes);

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let assets_dir = manifest_dir.join("assets");
    let _ = fs::create_dir_all(&assets_dir);
    let ico_path = assets_dir.join("app_icon.ico");
    fs::write(&ico_path, &ico_bytes).expect("Failed to write assets/app_icon.ico");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out_ico_path = out_dir.join("app_icon.ico");
    fs::write(&out_ico_path, &ico_bytes).expect("Failed to write OUT_DIR/app_icon.ico");

    let rc_path = out_dir.join("app_icon.rc");
    let res_path = out_dir.join("app_icon.res");

    let escaped_ico = out_ico_path.to_string_lossy().replace('\\', "\\\\");
    let rc_content = format!("1 ICON \"{}\"\r\n", escaped_ico);
    fs::write(&rc_path, rc_content).expect("Failed to write app_icon.rc");

    if let Some(rc_exe) = find_rc_exe() {
        let status = Command::new(rc_exe)
            .arg("/nologo")
            .arg("/fo")
            .arg(&res_path)
            .arg(&rc_path)
            .status();

        if let Ok(st) = status {
            if st.success() && res_path.exists() {
                println!("cargo:rustc-link-arg-bins={}", res_path.display());
            }
        }
    }
}
