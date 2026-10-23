use browser_use::wire::{MAX_FRAME, read_frame, write_frame};
use serde_json::json;
use std::io::{self, Cursor, Read};

#[test]
fn reads_fragmented_native_endian_frames_and_clean_eof() {
    struct Fragmented(Cursor<Vec<u8>>);
    impl Read for Fragmented {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0.read(&mut b[..1])
        }
    }
    let bytes = b"{\"id\":7}";
    let mut frame = (bytes.len() as u32).to_ne_bytes().to_vec();
    frame.extend(bytes);
    let mut reader = Fragmented(Cursor::new(frame));
    assert_eq!(read_frame(&mut reader).unwrap(), Some(json!({"id":7})));
    assert_eq!(read_frame(&mut reader).unwrap(), None);
}

#[test]
fn rejects_oversized_zero_truncated_and_invalid_frames() {
    for bytes in [
        0u32.to_ne_bytes().to_vec(),
        ((MAX_FRAME + 1) as u32).to_ne_bytes().to_vec(),
        vec![1, 0],
        vec![4, 0, 0, 0, b'{'],
        vec![1, 0, 0, 0, b'!'],
    ] {
        assert!(read_frame(&mut Cursor::new(bytes)).is_err());
    }
}

#[test]
fn writes_native_frames_and_rejects_oversized_payloads() {
    let mut out = Vec::new();
    write_frame(&mut out, &json!({"id":7})).unwrap();
    assert_eq!(&out[..4], &8u32.to_ne_bytes());
    assert_eq!(&out[4..], b"{\"id\":7}");
    assert!(write_frame(&mut Vec::new(), &json!("x".repeat(MAX_FRAME))).is_err());
}
