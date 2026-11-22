//! Direct DEX string-table extraction.
//!
//! This ports `apk_lab.dex_strings.extract_strings_from_dex_bytes`. It parses
//! only the header and `string_ids` table; no decompilation or class parsing is
//! performed. Truncated or inconsistent entries are skipped exactly as the
//! Python generator does, and decoding is plain UTF-8 with replacement, matching
//! Python's `bytes.decode("utf-8", errors="replace")` rather than modified UTF-8.

use crate::error::Error;

const DEX_HEADER_BYTES: usize = 0x70;
const STRING_IDS_SIZE_OFFSET: usize = 0x38;
const STRING_IDS_OFFSET_OFFSET: usize = 0x3C;

/// Decodes an unsigned LEB128 integer, returning `(value, bytes_read)`.
///
/// The Python oracle raises `IndexError` on truncation. Values that do not fit
/// a `u64` are rejected rather than growing without bound.
pub fn read_uleb128(data: &[u8], offset: usize) -> Result<(u64, usize), Error> {
    let mut result: u128 = 0;
    let mut shift: u32 = 0;
    let mut size = 0_usize;

    while let Some(&byte) = data.get(offset.saturating_add(size)) {
        size += 1;
        let payload = u128::from(byte & 0x7F);
        let shifted = if shift >= 128 {
            return Err(Error::invalid_artifact("ULEB128 sequence too long"));
        } else {
            let factor = 1_u128 << shift;
            payload
                .checked_mul(factor)
                .ok_or_else(|| Error::invalid_artifact("ULEB128 value out of range"))?
        };
        result |= shifted;
        if byte & 0x80 == 0 {
            return u64::try_from(result)
                .map(|value| (value, size))
                .map_err(|_| Error::invalid_artifact("ULEB128 value out of range"));
        }
        if result > u128::from(u64::MAX) {
            return Err(Error::invalid_artifact("ULEB128 value out of range"));
        }
        shift += 7;
    }

    Err(Error::invalid_artifact(
        "Truncated ULEB128 sequence in binary data",
    ))
}

/// Extracts every well-formed string from a DEX file's `string_ids` table.
///
/// A file shorter than the fixed header, or one whose magic is not `dex\n`,
/// yields no strings. Out-of-range IDs are skipped; a table that ends before its
/// declared count stops the scan.
pub fn extract_strings_from_dex_bytes(dex_bytes: &[u8]) -> Vec<String> {
    if dex_bytes.len() < DEX_HEADER_BYTES || !dex_bytes.starts_with(b"dex\n") {
        return Vec::new();
    }

    let string_ids_size = read_u32_le(dex_bytes, STRING_IDS_SIZE_OFFSET) as usize;
    let string_ids_off = read_u32_le(dex_bytes, STRING_IDS_OFFSET_OFFSET) as usize;

    let mut strings = Vec::new();
    for index in 0..string_ids_size {
        let Some(id_offset) = string_ids_off.checked_add(index.saturating_mul(4)) else {
            break;
        };
        let Some(id_end) = id_offset.checked_add(4) else {
            break;
        };
        if id_end > dex_bytes.len() {
            break;
        }
        let data_off = read_u32_le(dex_bytes, id_offset) as usize;
        if data_off >= dex_bytes.len() {
            continue;
        }
        let Ok((_utf16_size, uleb_len)) = read_uleb128(dex_bytes, data_off) else {
            continue;
        };
        let Some(str_start) = data_off.checked_add(uleb_len) else {
            continue;
        };
        if str_start > dex_bytes.len() {
            continue;
        }
        let Some(relative_null) = dex_bytes[str_start..].iter().position(|byte| *byte == 0) else {
            continue;
        };
        let raw_bytes = &dex_bytes[str_start..str_start + relative_null];
        strings.push(String::from_utf8_lossy(raw_bytes).into_owned());
    }
    strings
}

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    let mut bytes = [0_u8; 4];
    bytes.copy_from_slice(&data[offset..offset + 4]);
    u32::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_uleb128(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                out.push(byte | 0x80);
            } else {
                out.push(byte);
                break;
            }
        }
        out
    }

    /// Builds the same minimal synthetic DEX layout as the Python tests.
    fn build_mock_dex(strings: &[&str]) -> Vec<u8> {
        let header_size = 0x70_usize;
        let string_ids_off = header_size;
        let string_ids_size = strings.len();
        let data_off = string_ids_off + string_ids_size * 4;

        let mut string_data = Vec::new();
        let mut offsets = Vec::new();
        for value in strings {
            offsets.push(data_off + string_data.len());
            string_data.extend(encode_uleb128(value.chars().count() as u64));
            let encoded = value.as_bytes();
            string_data.extend_from_slice(encoded);
            string_data.push(0);
        }

        let mut dex = vec![0_u8; header_size];
        dex[0..4].copy_from_slice(b"dex\n");
        dex[4..8].copy_from_slice(b"035\x00");
        dex[0x38..0x3C].copy_from_slice(&(string_ids_size as u32).to_le_bytes());
        dex[0x3C..0x40].copy_from_slice(&(string_ids_off as u32).to_le_bytes());
        for offset in offsets {
            dex.extend_from_slice(&(offset as u32).to_le_bytes());
        }
        dex.extend_from_slice(&string_data);
        dex
    }

    #[test]
    fn uleb128_single_and_multi_byte() {
        assert_eq!(read_uleb128(&[0x00, 0x01, 0x7F], 0).unwrap(), (0, 1));
        assert_eq!(read_uleb128(&[0x00, 0x01, 0x7F], 1).unwrap(), (1, 1));
        assert_eq!(read_uleb128(&[0x00, 0x01, 0x7F], 2).unwrap(), (127, 1));
        assert_eq!(
            read_uleb128(&[0xFF, 0x80, 0x01, 0x00], 1).unwrap(),
            (128, 2)
        );
        assert_eq!(read_uleb128(&[0x80, 0x7F], 0).unwrap(), (0x7F << 7, 2));
    }

    #[test]
    fn uleb128_truncation_and_overflow_are_controlled() {
        assert!(read_uleb128(b"\x80", 0)
            .unwrap_err()
            .to_string()
            .contains("Truncated ULEB128 sequence"));
        assert!(read_uleb128(b"", 0).is_err());
        assert!(read_uleb128(&[0xFF; 24], 0).is_err());
    }

    #[test]
    fn extracts_mock_dex_strings() {
        let dex = build_mock_dex(&["Hello", "World", "Android_Test_123", ""]);
        assert_eq!(
            extract_strings_from_dex_bytes(&dex),
            vec!["Hello", "World", "Android_Test_123", ""]
        );
    }

    #[test]
    fn invalid_magic_and_short_files_yield_nothing() {
        assert!(extract_strings_from_dex_bytes(b"dex\nshort").is_empty());
        let mut bad = b"bad\n".to_vec();
        bad.resize(0x80, 0);
        assert!(extract_strings_from_dex_bytes(&bad).is_empty());
        assert!(extract_strings_from_dex_bytes(&[]).is_empty());
    }

    #[test]
    fn truncated_tables_and_bad_offsets_are_skipped() {
        let valid = build_mock_dex(&["Alpha", "Beta"]);
        assert!(extract_strings_from_dex_bytes(&valid[..0x74]).is_empty());

        let mut bad_offset = valid[..0x70].to_vec();
        bad_offset[0x38..0x3C].copy_from_slice(&1_u32.to_le_bytes());
        bad_offset.extend_from_slice(&0xFFFF_u32.to_le_bytes());
        assert!(extract_strings_from_dex_bytes(&bad_offset).is_empty());

        let mut truncated_uleb = valid[..0x70].to_vec();
        truncated_uleb[0x38..0x3C].copy_from_slice(&1_u32.to_le_bytes());
        truncated_uleb.extend_from_slice(&0x74_u32.to_le_bytes());
        truncated_uleb.push(0x80);
        assert!(extract_strings_from_dex_bytes(&truncated_uleb).is_empty());
    }

    #[test]
    fn declared_count_larger_than_table_stops_at_table_end() {
        let mut dex = build_mock_dex(&["One"]);
        dex[0x38..0x3C].copy_from_slice(&0xFFFF_u32.to_le_bytes());
        assert_eq!(extract_strings_from_dex_bytes(&dex), vec!["One"]);
    }

    #[test]
    fn decodes_unicode_and_supplementary_code_points() {
        let dex = build_mock_dex(&["héllo", "日本語", "😀", "a\u{0}b"]);
        let strings = extract_strings_from_dex_bytes(&dex);
        assert_eq!(strings, vec!["héllo", "日本語", "😀", "a"]);
    }

    #[test]
    fn null_terminator_is_not_decoded_as_part_of_the_string() {
        // Hand-build a string whose bytes contain an embedded NUL: the scan
        // stops at the first zero byte, exactly like `bytes.find(b"\x00")`.
        let mut dex = vec![0_u8; 0x70];
        dex[0..4].copy_from_slice(b"dex\n");
        dex[0x38..0x3C].copy_from_slice(&1_u32.to_le_bytes());
        dex[0x3C..0x40].copy_from_slice(&0x70_u32.to_le_bytes());
        dex.extend_from_slice(&0x74_u32.to_le_bytes());
        dex.extend_from_slice(&encode_uleb128(3));
        dex.extend_from_slice(b"a\x00b");
        dex.push(0);
        assert_eq!(extract_strings_from_dex_bytes(&dex), vec!["a"]);
    }

    #[test]
    fn modified_utf8_and_invalid_sequences_use_python_replacement() {
        // C0 80 is modified UTF-8 for NUL; Python's plain UTF-8 decoder emits
        // two replacement characters and Rust's lossy decoder agrees.
        let cases: &[(&[u8], &[u32])] = &[
            (b"\xC0\x80", &[0xFFFD, 0xFFFD]),
            (b"\xED\xA0\x80", &[0xFFFD, 0xFFFD, 0xFFFD]),
            (b"\xE1\x80A", &[0xFFFD, 0x41]),
            (b"\xE2\x82", &[0xFFFD]),
            (b"\xF0\x9F\x98", &[0xFFFD]),
            (b"\x80\x80", &[0xFFFD, 0xFFFD]),
        ];
        for (raw, expected) in cases {
            let decoded = String::from_utf8_lossy(raw);
            let code_points: Vec<u32> = decoded.chars().map(u32::from).collect();
            assert_eq!(&code_points, expected, "decoding {raw:?}");
        }
    }

    #[test]
    fn dex_level_modified_utf8_decodes_as_plain_utf8_replace() {
        let mut dex = vec![0_u8; 0x70];
        dex[0..4].copy_from_slice(b"dex\n");
        dex[0x38..0x3C].copy_from_slice(&1_u32.to_le_bytes());
        dex[0x3C..0x40].copy_from_slice(&0x70_u32.to_le_bytes());
        dex.extend_from_slice(&0x74_u32.to_le_bytes());
        dex.extend_from_slice(&encode_uleb128(1));
        dex.extend_from_slice(b"\xC0\x80");
        dex.push(0);
        assert_eq!(
            extract_strings_from_dex_bytes(&dex),
            vec!["\u{FFFD}\u{FFFD}"]
        );
    }

    #[test]
    fn deterministic_malformed_dex_inputs_never_panic() {
        let mut state = 0xD000_0000_u64;
        for length in 0..512_usize {
            for _ in 0..8 {
                let mut bytes = Vec::with_capacity(length);
                for _ in 0..length {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1);
                    bytes.push((state >> 33) as u8);
                }
                if bytes.len() >= 4 {
                    bytes[0..4].copy_from_slice(b"dex\n");
                }
                let _ = extract_strings_from_dex_bytes(&bytes);
            }
        }
    }
}
