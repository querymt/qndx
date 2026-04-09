//! File format versioning: magic bytes, version, and checksum headers.
//!
//! Every index file (`ngrams.tbl`, `postings.dat`, `manifest.bin`) starts with
//! a fixed-size header for forward compatibility and corruption detection.
//!
//! Header layout (24 bytes):
//!   [0..4]   magic bytes (identifies file type)
//!   [4..8]   format version (u32 LE)
//!   [8..16]  payload length in bytes (u64 LE)
//!   [16..24] rapidhash-v3 checksum of payload (u64 LE)

use crate::FileId;
use std::io::{self, Read, Write};

/// Header size in bytes.
pub const HEADER_SIZE: usize = 24;

/// Current format version.
pub const FORMAT_VERSION: u32 = 2;

/// Magic bytes for `ngrams.tbl`.
pub const MAGIC_NGRAMS: [u8; 4] = *b"QXNG";

/// Magic bytes for `postings.dat`.
pub const MAGIC_POSTINGS: [u8; 4] = *b"QXPO";

/// Magic bytes for `manifest.bin`.
pub const MAGIC_MANIFEST: [u8; 4] = *b"QXMF";

/// Size of a single ngram table entry on disk (hash:4 + offset:8 + len:4 + flags:4 = 20 bytes).
pub const NGRAM_ENTRY_SIZE: usize = 20;

/// Flag: this n-gram entry is a sparse n-gram (length > 3).
pub const FLAG_SPARSE: u32 = 1 << 0;

/// A versioned file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub payload_len: u64,
    pub checksum: u64,
}

/// Errors from format validation.
#[derive(Debug)]
pub enum FormatError {
    /// I/O error reading or writing.
    Io(io::Error),
    /// Magic bytes do not match expected file type.
    BadMagic { expected: [u8; 4], found: [u8; 4] },
    /// Version is not supported.
    UnsupportedVersion { found: u32, max_supported: u32 },
    /// Payload checksum mismatch (corruption detected).
    ChecksumMismatch { expected: u64, computed: u64 },
    /// Payload length does not match actual data.
    PayloadLengthMismatch { expected: u64, actual: u64 },
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::Io(e) => write!(f, "I/O error: {}", e),
            FormatError::BadMagic { expected, found } => write!(
                f,
                "bad magic: expected {:?}, found {:?}",
                std::str::from_utf8(expected).unwrap_or("??"),
                std::str::from_utf8(found).unwrap_or("??"),
            ),
            FormatError::UnsupportedVersion {
                found,
                max_supported,
            } => write!(
                f,
                "unsupported version: {} (max supported: {})",
                found, max_supported,
            ),
            FormatError::ChecksumMismatch { expected, computed } => write!(
                f,
                "checksum mismatch: expected 0x{:016x}, computed 0x{:016x}",
                expected, computed,
            ),
            FormatError::PayloadLengthMismatch { expected, actual } => write!(
                f,
                "payload length mismatch: header says {} bytes, actual {} bytes",
                expected, actual,
            ),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<io::Error> for FormatError {
    fn from(e: io::Error) -> Self {
        FormatError::Io(e)
    }
}

/// Compute a deterministic rapidhash-v3 checksum of a byte slice.
pub fn compute_checksum(data: &[u8]) -> u64 {
    rapidhash::v3::rapidhash_v3(data)
}

/// Write a file header followed by payload.
pub fn write_with_header<W: Write>(
    writer: &mut W,
    magic: [u8; 4],
    payload: &[u8],
) -> Result<(), FormatError> {
    let checksum = compute_checksum(payload);
    let header = FileHeader {
        magic,
        version: FORMAT_VERSION,
        payload_len: payload.len() as u64,
        checksum,
    };
    write_header(writer, &header)?;
    writer.write_all(payload)?;
    Ok(())
}

/// Write just the header bytes.
fn write_header<W: Write>(writer: &mut W, header: &FileHeader) -> Result<(), FormatError> {
    writer.write_all(&header.magic)?;
    writer.write_all(&header.version.to_le_bytes())?;
    writer.write_all(&header.payload_len.to_le_bytes())?;
    writer.write_all(&header.checksum.to_le_bytes())?;
    Ok(())
}

/// Validate a file header from a byte slice (for use with mmap).
///
/// Checks magic bytes, version, and payload length against the file size.
/// Returns the validated header. Does NOT check the payload checksum (call
/// `validate_checksum_from_slice` separately if needed).
pub fn validate_header_from_slice(
    data: &[u8],
    expected_magic: [u8; 4],
) -> Result<FileHeader, FormatError> {
    if data.len() < HEADER_SIZE {
        return Err(FormatError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "file too small for header",
        )));
    }

    let mut magic = [0u8; 4];
    magic.copy_from_slice(&data[0..4]);
    if magic != expected_magic {
        return Err(FormatError::BadMagic {
            expected: expected_magic,
            found: magic,
        });
    }

    let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if version > FORMAT_VERSION {
        return Err(FormatError::UnsupportedVersion {
            found: version,
            max_supported: FORMAT_VERSION,
        });
    }

    let payload_len = u64::from_le_bytes(data[8..16].try_into().unwrap());
    let checksum = u64::from_le_bytes(data[16..24].try_into().unwrap());

    let actual_payload_len = data.len() as u64 - HEADER_SIZE as u64;
    if payload_len != actual_payload_len {
        return Err(FormatError::PayloadLengthMismatch {
            expected: payload_len,
            actual: actual_payload_len,
        });
    }

    Ok(FileHeader {
        magic,
        version,
        payload_len,
        checksum,
    })
}

/// Validate the payload checksum of the payload region in a byte slice.
///
/// `data` should be the full file contents (header + payload).
/// This is an O(n) operation over the entire payload.
pub fn validate_checksum_from_slice(data: &[u8], header: &FileHeader) -> Result<(), FormatError> {
    let payload = &data[HEADER_SIZE..];
    let computed = compute_checksum(payload);
    if computed != header.checksum {
        return Err(FormatError::ChecksumMismatch {
            expected: header.checksum,
            computed,
        });
    }
    Ok(())
}

/// Get the payload region from a validated file slice (skips header).
pub fn payload_from_slice(data: &[u8]) -> &[u8] {
    &data[HEADER_SIZE..]
}

/// Read and validate a file header + payload from a reader.
/// Returns the validated payload bytes.
pub fn read_with_header<R: Read>(
    reader: &mut R,
    expected_magic: [u8; 4],
) -> Result<Vec<u8>, FormatError> {
    let header = read_header(reader, expected_magic)?;

    let mut payload = vec![0u8; header.payload_len as usize];
    reader.read_exact(&mut payload)?;

    let computed = compute_checksum(&payload);
    if computed != header.checksum {
        return Err(FormatError::ChecksumMismatch {
            expected: header.checksum,
            computed,
        });
    }

    Ok(payload)
}

/// Read and validate just the header.
fn read_header<R: Read>(
    reader: &mut R,
    expected_magic: [u8; 4],
) -> Result<FileHeader, FormatError> {
    let mut buf = [0u8; HEADER_SIZE];
    reader.read_exact(&mut buf)?;

    let mut magic = [0u8; 4];
    magic.copy_from_slice(&buf[0..4]);

    if magic != expected_magic {
        return Err(FormatError::BadMagic {
            expected: expected_magic,
            found: magic,
        });
    }

    let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    if version > FORMAT_VERSION {
        return Err(FormatError::UnsupportedVersion {
            found: version,
            max_supported: FORMAT_VERSION,
        });
    }

    let payload_len = u64::from_le_bytes(buf[8..16].try_into().unwrap());
    let checksum = u64::from_le_bytes(buf[16..24].try_into().unwrap());

    Ok(FileHeader {
        magic,
        version,
        payload_len,
        checksum,
    })
}

// ---------------------------------------------------------------------------
// Serialization helpers for ngram entries
// ---------------------------------------------------------------------------

/// Serialize an ngram entry to bytes (20 bytes).
pub fn serialize_ngram_entry(entry: &crate::NgramEntry) -> [u8; NGRAM_ENTRY_SIZE] {
    let mut buf = [0u8; NGRAM_ENTRY_SIZE];
    buf[0..4].copy_from_slice(&entry.hash.to_le_bytes());
    buf[4..12].copy_from_slice(&entry.offset.to_le_bytes());
    buf[12..16].copy_from_slice(&entry.len.to_le_bytes());
    buf[16..20].copy_from_slice(&entry.flags.to_le_bytes());
    buf
}

/// Deserialize an ngram entry from bytes (20 bytes).
pub fn deserialize_ngram_entry(buf: &[u8; NGRAM_ENTRY_SIZE]) -> crate::NgramEntry {
    crate::NgramEntry {
        hash: u32::from_le_bytes(buf[0..4].try_into().unwrap()),
        offset: u64::from_le_bytes(buf[4..12].try_into().unwrap()),
        len: u32::from_le_bytes(buf[12..16].try_into().unwrap()),
        flags: u32::from_le_bytes(buf[16..20].try_into().unwrap()),
    }
}

// ---------------------------------------------------------------------------
// Serialization helpers for postings (delta encoding with fixed-width u32)
// ---------------------------------------------------------------------------

/// Encode a sorted list of FileIds as delta-encoded little-endian u32s.
/// Format: [count:u32] [delta_0:u32] [delta_1:u32] ...
pub fn encode_postings(ids: &[FileId]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + ids.len() * 4);
    buf.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    let mut prev: u32 = 0;
    for &id in ids {
        let delta = id - prev;
        buf.extend_from_slice(&delta.to_le_bytes());
        prev = id;
    }
    buf
}

/// Decode delta-encoded postings back to a sorted Vec<FileId>.
pub fn decode_postings(data: &[u8]) -> Vec<FileId> {
    if data.len() < 4 {
        return Vec::new();
    }
    let count = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    let mut ids = Vec::with_capacity(count);
    let mut prev: u32 = 0;
    let mut offset = 4;
    for _ in 0..count {
        if offset + 4 > data.len() {
            break;
        }
        let delta = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        prev += delta;
        ids.push(prev);
        offset += 4;
    }
    ids
}

// ---------------------------------------------------------------------------
// Serialization helpers for postings (delta encoding with varint compression)
// ---------------------------------------------------------------------------

/// Encode a u32 as a variable-length integer (LEB128-style).
/// Returns the number of bytes written.
fn encode_varint(mut value: u32, buf: &mut Vec<u8>) -> usize {
    let mut count = 0;
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        count += 1;
        if value == 0 {
            break;
        }
    }
    count
}

/// Decode a varint from a byte slice. Returns (value, bytes_consumed).
/// Returns None if the slice is too short or the varint is malformed.
fn decode_varint(data: &[u8]) -> Option<(u32, usize)> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    for (i, &byte) in data.iter().enumerate() {
        if shift >= 35 {
            return None; // Overflow protection
        }
        result |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
    }
    None // Unterminated varint
}

/// Encode a sorted list of FileIds as delta-encoded varints.
/// Format: [count:varint] [delta_0:varint] [delta_1:varint] ...
///
/// This is more compact than the fixed-width encoding when deltas are small,
/// which is common for posting lists with nearby file IDs.
pub fn encode_postings_varint(ids: &[FileId]) -> Vec<u8> {
    // Estimate: count varint + at most 5 bytes per delta
    let mut buf = Vec::with_capacity(5 + ids.len() * 3);
    encode_varint(ids.len() as u32, &mut buf);
    let mut prev: u32 = 0;
    for &id in ids {
        let delta = id - prev;
        encode_varint(delta, &mut buf);
        prev = id;
    }
    buf
}

/// Decode varint delta-encoded postings back to a sorted Vec<FileId>.
pub fn decode_postings_varint(data: &[u8]) -> Vec<FileId> {
    if data.is_empty() {
        return Vec::new();
    }
    let (count, mut offset) = match decode_varint(data) {
        Some(v) => v,
        None => return Vec::new(),
    };
    let count = count as usize;
    let mut ids = Vec::with_capacity(count);
    let mut prev: u32 = 0;
    for _ in 0..count {
        if offset >= data.len() {
            break;
        }
        let (delta, consumed) = match decode_varint(&data[offset..]) {
            Some(v) => v,
            None => break,
        };
        prev += delta;
        ids.push(prev);
        offset += consumed;
    }
    ids
}

/// Measure the encoded size of a posting list using varint encoding
/// without actually allocating a full buffer. Useful for size comparisons.
pub fn varint_encoded_size(ids: &[FileId]) -> usize {
    fn varint_size(mut value: u32) -> usize {
        let mut size = 1;
        while value >= 0x80 {
            value >>= 7;
            size += 1;
        }
        size
    }
    let mut total = varint_size(ids.len() as u32);
    let mut prev: u32 = 0;
    for &id in ids {
        total += varint_size(id - prev);
        prev = id;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn header_roundtrip() {
        let payload = b"hello world test data";
        let mut buf = Vec::new();
        write_with_header(&mut buf, MAGIC_NGRAMS, payload).unwrap();

        let mut reader = Cursor::new(&buf);
        let read_payload = read_with_header(&mut reader, MAGIC_NGRAMS).unwrap();
        assert_eq!(read_payload, payload);
    }

    #[test]
    fn bad_magic_rejected() {
        let payload = b"test";
        let mut buf = Vec::new();
        write_with_header(&mut buf, MAGIC_NGRAMS, payload).unwrap();

        let mut reader = Cursor::new(&buf);
        let result = read_with_header(&mut reader, MAGIC_POSTINGS);
        assert!(matches!(result, Err(FormatError::BadMagic { .. })));
    }

    #[test]
    fn corrupted_payload_detected() {
        let payload = b"test data here";
        let mut buf = Vec::new();
        write_with_header(&mut buf, MAGIC_MANIFEST, payload).unwrap();

        // Corrupt a byte in the payload
        let last = buf.len() - 1;
        buf[last] ^= 0xFF;

        let mut reader = Cursor::new(&buf);
        let result = read_with_header(&mut reader, MAGIC_MANIFEST);
        assert!(matches!(result, Err(FormatError::ChecksumMismatch { .. })));
    }

    #[test]
    fn ngram_entry_roundtrip() {
        let entry = crate::NgramEntry {
            hash: 0xDEADBEEF,
            offset: 12345678,
            len: 256,
            flags: 0,
        };
        let bytes = serialize_ngram_entry(&entry);
        let decoded = deserialize_ngram_entry(&bytes);
        assert_eq!(entry, decoded);
    }

    #[test]
    fn postings_roundtrip() {
        let ids = vec![1, 5, 10, 20, 100];
        let encoded = encode_postings(&ids);
        let decoded = decode_postings(&encoded);
        assert_eq!(ids, decoded);
    }

    #[test]
    fn postings_empty() {
        let ids: Vec<FileId> = vec![];
        let encoded = encode_postings(&ids);
        let decoded = decode_postings(&encoded);
        assert_eq!(ids, decoded);
    }

    #[test]
    fn postings_single() {
        let ids = vec![42];
        let encoded = encode_postings(&ids);
        let decoded = decode_postings(&encoded);
        assert_eq!(ids, decoded);
    }

    #[test]
    fn unsupported_version_rejected() {
        let payload = b"test";
        let mut buf = Vec::new();
        // Write header with a future version
        buf.extend_from_slice(&MAGIC_NGRAMS);
        buf.extend_from_slice(&99u32.to_le_bytes()); // version 99
        buf.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        buf.extend_from_slice(&compute_checksum(payload).to_le_bytes());
        buf.extend_from_slice(payload);

        let mut reader = Cursor::new(&buf);
        let result = read_with_header(&mut reader, MAGIC_NGRAMS);
        assert!(matches!(
            result,
            Err(FormatError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn empty_payload_roundtrip() {
        let payload = b"";
        let mut buf = Vec::new();
        write_with_header(&mut buf, MAGIC_MANIFEST, payload).unwrap();

        let mut reader = Cursor::new(&buf);
        let read_payload = read_with_header(&mut reader, MAGIC_MANIFEST).unwrap();
        assert!(read_payload.is_empty());
    }

    // --- varint encoding tests ---

    #[test]
    fn varint_roundtrip_small() {
        let mut buf = Vec::new();
        encode_varint(0, &mut buf);
        assert_eq!(buf.len(), 1);
        assert_eq!(decode_varint(&buf), Some((0, 1)));

        buf.clear();
        encode_varint(127, &mut buf);
        assert_eq!(buf.len(), 1);
        assert_eq!(decode_varint(&buf), Some((127, 1)));
    }

    #[test]
    fn varint_roundtrip_medium() {
        let mut buf = Vec::new();
        encode_varint(128, &mut buf);
        assert_eq!(buf.len(), 2);
        assert_eq!(decode_varint(&buf), Some((128, 2)));

        buf.clear();
        encode_varint(16383, &mut buf);
        assert_eq!(buf.len(), 2);
        assert_eq!(decode_varint(&buf), Some((16383, 2)));
    }

    #[test]
    fn varint_roundtrip_large() {
        let mut buf = Vec::new();
        encode_varint(u32::MAX, &mut buf);
        assert_eq!(buf.len(), 5);
        assert_eq!(decode_varint(&buf), Some((u32::MAX, 5)));
    }

    #[test]
    fn varint_postings_roundtrip() {
        let ids = vec![1, 5, 10, 20, 100];
        let encoded = encode_postings_varint(&ids);
        let decoded = decode_postings_varint(&encoded);
        assert_eq!(ids, decoded);
    }

    #[test]
    fn varint_postings_empty() {
        let ids: Vec<FileId> = vec![];
        let encoded = encode_postings_varint(&ids);
        let decoded = decode_postings_varint(&encoded);
        assert_eq!(ids, decoded);
    }

    #[test]
    fn varint_postings_single() {
        let ids = vec![42];
        let encoded = encode_postings_varint(&ids);
        let decoded = decode_postings_varint(&encoded);
        assert_eq!(ids, decoded);
    }

    #[test]
    fn varint_postings_large_ids() {
        let ids = vec![100_000, 200_000, 300_000, 400_000];
        let encoded = encode_postings_varint(&ids);
        let decoded = decode_postings_varint(&encoded);
        assert_eq!(ids, decoded);
    }

    #[test]
    fn varint_postings_consecutive() {
        // Consecutive IDs: deltas are all 1 -> each is 1 byte
        let ids: Vec<FileId> = (0..100).collect();
        let encoded = encode_postings_varint(&ids);
        let decoded = decode_postings_varint(&encoded);
        assert_eq!(ids, decoded);
        // varint should be much smaller than fixed-width
        let fixed = encode_postings(&ids);
        assert!(
            encoded.len() < fixed.len(),
            "varint ({}) should be smaller than fixed ({})",
            encoded.len(),
            fixed.len()
        );
    }

    #[test]
    fn varint_encoded_size_matches() {
        let ids = vec![1, 5, 10, 20, 100, 1000, 10000];
        let encoded = encode_postings_varint(&ids);
        let estimated = varint_encoded_size(&ids);
        assert_eq!(encoded.len(), estimated);
    }
}
