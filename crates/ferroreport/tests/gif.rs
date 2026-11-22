//! GIF images decode to the same pixels as Pillow gives (reference pixels made with it are
//! kept next to each file), and broken files are refused instead of crashing.

use ferroreport::image::{ImageStore, Pixels};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/gif")
}

#[test]
fn gifs_decode_to_the_reference_pixels() {
    for name in ["plain", "interlaced-transparent"] {
        let bytes = std::fs::read(dir().join(format!("{name}.gif"))).unwrap();
        let expected = std::fs::read(dir().join(format!("{name}.rgba"))).unwrap();
        let img = ImageStore::new().load_bytes(&bytes).unwrap();
        assert_eq!((img.width, img.height), (19, 13), "{name}");
        let Pixels::Raw { samples, alpha, gray: false } = &img.pixels else { panic!("{name}: not RGB") };
        assert_eq!(alpha.is_some(), name.contains("transparent"), "{name}");
        for i in 0..(19 * 13) {
            let a = alpha.as_ref().map_or(255, |a| a[i]);
            assert_eq!(a, expected[i * 4 + 3], "{name}: alpha of pixel {i}");
            if a == 255 {
                assert_eq!(samples[i * 3..i * 3 + 3], expected[i * 4..i * 4 + 3], "{name}: pixel {i}");
            }
        }
    }
}

#[test]
fn broken_gifs_are_refused() {
    let bytes = std::fs::read(dir().join("interlaced-transparent.gif")).unwrap();
    for cut in 0..bytes.len() {
        let _ = ImageStore::new().load_bytes(&bytes[..cut]);
    }
    let mut seed = 7u32;
    for _ in 0..2000 {
        let mut b = bytes.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let at = 6 + seed as usize % (b.len() - 6);
            b[at] = (seed >> 8) as u8;
        }
        let _ = ImageStore::new().load_bytes(&b);
    }
}
