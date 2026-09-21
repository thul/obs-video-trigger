use std::{env, fs, path::PathBuf};

const SIZE: usize = 32;

fn icon_pixels() -> Vec<u8> {
    let mut pixels = vec![0u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let source_y = SIZE - 1 - y;
            let offset = (source_y * SIZE + x) * 4;
            let edge = !(2..=29).contains(&x) || !(2..=29).contains(&y);
            let corner = (!(6..=25).contains(&x) && !(6..=25).contains(&y))
                && ((x as isize - if x < 16 { 6 } else { 25 }).pow(2)
                    + (y as isize - if y < 16 { 6 } else { 25 }).pow(2)
                    > 16);
            let in_play = (9..=23).contains(&x)
                && (7..=24).contains(&y)
                && (y as isize - 16).unsigned_abs() <= (23 - x) / 2;
            let (red, green, blue, alpha) = if edge || corner {
                (0, 0, 0, 0)
            } else if in_play {
                (255, 255, 255, 255)
            } else {
                (0, 120, 212, 255)
            };
            pixels[offset..offset + 4].copy_from_slice(&[blue, green, red, alpha]);
        }
    }
    pixels
}

fn ico() -> Vec<u8> {
    let pixels = icon_pixels();
    let mask_size = SIZE * SIZE / 8;
    let image_size = 40 + pixels.len() + mask_size;
    let mut data = Vec::with_capacity(22 + image_size);
    data.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    data.extend_from_slice(&[SIZE as u8, SIZE as u8, 0, 0]);
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&32u16.to_le_bytes());
    data.extend_from_slice(&(image_size as u32).to_le_bytes());
    data.extend_from_slice(&22u32.to_le_bytes());
    data.extend_from_slice(&40u32.to_le_bytes());
    data.extend_from_slice(&(SIZE as i32).to_le_bytes());
    data.extend_from_slice(&((SIZE * 2) as i32).to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&32u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&((pixels.len() + mask_size) as u32).to_le_bytes());
    data.extend_from_slice(&[0; 16]);
    data.extend_from_slice(&pixels);
    data.extend(std::iter::repeat_n(0, mask_size));
    data
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }
    let icon_path = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("app.ico");
    fs::write(&icon_path, ico()).unwrap();
    winresource::WindowsResource::new()
        .set_icon(icon_path.to_str().unwrap())
        .compile()
        .unwrap();
}
