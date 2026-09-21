pub fn html() -> (String, String) {
    let raw = include_str!("overlay.html");
    let stamp = version(raw);
    (raw.replace("__OVERLAY_VERSION__", &stamp), stamp)
}

pub fn version(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in input.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:x}")
}
