//! Acceptance: `VPath` loses nothing, whatever the platform's native names
//! look like.

use std::ffi::OsStr;

use types::VPath;

/// Deterministic pseudo-random bytes and code units.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }
}

#[test]
fn arbitrary_bytes_round_trip() {
    let mut rng = Lcg(1);
    for len in 0..512 {
        let bytes: Vec<u8> = (0..len).map(|_| (rng.next() & 0xFF) as u8).collect();
        let path = VPath::from_bytes(bytes.clone());
        assert_eq!(path.as_bytes(), &bytes[..]);
        assert_eq!(path.into_bytes(), bytes);
    }
}

#[test]
fn arbitrary_utf16_round_trips() {
    let mut rng = Lcg(2);
    for len in 0..512 {
        // Bias towards the surrogate range so pairs and unpaired units both
        // appear often.
        let wide: Vec<u16> = (0..len)
            .map(|_| {
                let r = rng.next();
                if r.is_multiple_of(3) {
                    0xD800 + ((r >> 8) & 0x7FF) as u16
                } else {
                    (r & 0xFFFF) as u16
                }
            })
            .collect();
        let path = VPath::from_wide(&wide);
        assert_eq!(path.to_wide().as_deref(), Some(&wide[..]), "{path:?}");
    }
}

#[test]
fn native_os_strings_round_trip_without_loss() {
    // Names std can represent on every platform.
    let names = ["plain.txt", "spaced name", "ünïcödé", "日本語", "😀", ""];
    for name in names {
        let os = OsStr::new(name);
        let path = VPath::from_os_str(os);
        assert_eq!(path.as_bytes(), os.as_encoded_bytes());
        assert_eq!(path.to_string_lossy(), name);
        assert_eq!(
            path.to_wide(),
            Some(name.encode_utf16().collect::<Vec<u16>>())
        );
    }
}
