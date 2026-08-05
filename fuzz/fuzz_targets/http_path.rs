#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    if let Ok(path) = std::str::from_utf8(input) {
        // Exercise the same percent-decoding and path component handling used
        // by static requests; malformed text must never panic.
        let decoded = percent_encoding::percent_decode_str(path).decode_utf8();
        if let Ok(decoded) = decoded {
            let _ = std::path::Path::new(decoded.as_ref()).components().count();
        }
    }
});
