# Index File Format Specification

Version: 1

This document describes the on-disk format of qndx index files. All multi-byte integers are little-endian.

## Overview

The index consists of three files stored in `.qndx/index/v1/`:

| File | Magic | Description |
|------|-------|-------------|
| `ngrams.tbl` | `QXNG` | Sorted n-gram hash lookup table |
| `postings.dat` | `QXPO` | Concatenated posting list blocks |
| `manifest.bin` | `QXMF` | Metadata and file path list |

## Common file header

Every index file starts with a 20-byte header:

```
Offset  Size  Type    Field
------  ----  ----    -----
0       4     [u8;4]  Magic bytes (identifies file type)
4       4     u32     Format version (currently 1)
8       8     u64     Payload length in bytes
16      4     u32     CRC32 checksum of payload
```

The checksum covers only the payload bytes (everything after the header). It uses the CRC32 algorithm from the `crc32fast` crate (IEEE polynomial).

Readers must:
1. Verify magic bytes match the expected file type
2. Reject versions greater than the supported maximum
3. Validate the CRC32 checksum against the payload

## ngrams.tbl

The payload is a sequence of fixed-size n-gram entries, sorted by hash value. Binary search is used for lookups.

### N-gram entry (20 bytes)

```
Offset  Size  Type  Field
------  ----  ----  -----
0       4     u32   N-gram hash (CRC32 of the n-gram bytes)
4       8     u64   Byte offset into postings.dat payload
12      4     u32   Length of the posting block in bytes
16      4     u32   Flags
```

**Flags:**

| Bit | Name | Meaning |
|-----|------|---------|
| 0 | `FLAG_SPARSE` | This entry is a sparse n-gram (length > 3 bytes). If unset, it is a standard trigram. |
| 1-31 | Reserved | Must be zero. |

The table is sorted by the `hash` field. Entries with the same hash from different sources (trigram vs sparse) are deduplicated during build -- if a hash appears in both the trigram and sparse sets, it is stored once with `FLAG_SPARSE` set.

### Hash function

N-gram hashes are computed using CRC32 (IEEE polynomial) over the raw n-gram bytes:

```rust
pub fn hash_ngram(gram: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(gram);
    hasher.finalize()
}
```

### N-gram extraction

**Trigrams**: Every overlapping 3-byte window in the file content produces a trigram hash. For input shorter than 3 bytes, no trigrams are produced.

**Sparse n-grams**: Variable-length n-grams selected by a hash-based weight function. The weight of a byte pair determines whether the sparse extraction algorithm starts a new n-gram boundary at that position. This produces fewer, longer n-grams that can cover the same literal with fewer index lookups.

Both trigram and sparse n-gram hashes are stored in the same `ngrams.tbl` file, distinguished by `FLAG_SPARSE`.

## postings.dat

The payload is a sequence of concatenated posting blocks. Each block is referenced by an n-gram entry's `(offset, len)` pair.

### Posting block format

Each block starts with a 1-byte tag that identifies the encoding:

| Tag | Name | Description |
|-----|------|-------------|
| `0x01` | Fixed-width delta | Delta-encoded u32 values, fixed 4 bytes per delta |
| `0x02` | Varint delta | Delta-encoded u32 values, LEB128-compressed deltas |
| `0x03` | Roaring | Native Roaring bitmap serialization |

The index writer selects the encoding automatically:
- Posting lists with <= 64 entries use varint delta (`0x02`)
- Posting lists with > 64 entries use Roaring (`0x03`)

The threshold (64) is configurable at build time via `DEFAULT_HYBRID_THRESHOLD`.

### Tag 0x01: Fixed-width delta encoding

```
Byte 0:     0x01 (tag)
Bytes 1-4:  u32 count (number of file IDs)
Bytes 5+:   count x u32 deltas
```

Each delta is the difference from the previous file ID (or 0 for the first). File IDs are reconstructed by prefix sum.

Example: File IDs `[3, 7, 10]` are stored as deltas `[3, 4, 3]`.

### Tag 0x02: Varint delta encoding

```
Byte 0:      0x02 (tag)
Bytes 1+:    varint count, then count x varint deltas
```

Varints use LEB128 encoding: each byte uses 7 bits for data and 1 bit as continuation flag (high bit set = more bytes follow).

| Value range | Bytes used |
|-------------|-----------|
| 0 - 127 | 1 |
| 128 - 16383 | 2 |
| 16384 - 2097151 | 3 |
| 2097152 - 268435455 | 4 |
| 268435456 - 4294967295 | 5 |

This encoding is more compact than fixed-width when deltas are small (consecutive or nearby file IDs), which is the common case for posting lists.

### Tag 0x03: Roaring bitmap

```
Byte 0:     0x03 (tag)
Bytes 1+:   Roaring bitmap serialized via RoaringBitmap::serialize_into()
```

Uses the standard Roaring bitmap wire format from the `roaring` crate. Efficient for large, dense posting lists where delta encoding would waste space.

## manifest.bin

The payload is a [postcard](https://crates.io/crates/postcard)-serialized `Manifest` struct:

```rust
struct Manifest {
    version: u32,           // Format version (1)
    file_count: u32,        // Number of indexed files
    ngram_count: u32,       // Number of unique n-grams
    postings_bytes: u64,    // Total size of postings data
    base_commit: Option<String>,  // Git commit hash (hex) or None
    files: Vec<String>,     // File paths in index order (FileId = index)
}
```

The `files` vector maps `FileId` (u32 index) to relative file paths. File IDs are assigned sequentially during the build in sorted path order.

`base_commit` records the Git HEAD commit at index time (if the indexed directory is a Git repository). This is used by the freshness model to detect which files have changed since the index was built.

## File ID assignment

File IDs are sequential u32 values assigned during the build. The ordering matches the sorted order of relative file paths:

```
FileId 0  -> "crates/core/src/lib.rs"
FileId 1  -> "crates/core/src/types.rs"
FileId 2  -> "src/main.rs"
...
```

This ordering is deterministic: the same set of files always produces the same ID assignment.

## Versioning and compatibility

The `version` field in the file header enables forward compatibility:

- **Version 1** (current): The format described in this document.
- Readers must reject files with `version > FORMAT_VERSION`.
- Writers always use the current `FORMAT_VERSION`.

When the format changes:
- Increment `FORMAT_VERSION`
- Document the new version in this file
- Old indexes must be rebuilt (`qndx index` rewrites all files)
- No incremental migration is supported in the MVP

## Size characteristics

Typical index sizes relative to source corpus:

| Corpus | Files | Source | Index | Ratio |
|--------|-------|--------|-------|-------|
| Small Rust project | 722 | 8.6 MB | 23.6 MB | 2.7x |
| Linux kernel | 92,000 | 1.0 GB | 2.0 GB | 2.0x |

The majority of index size comes from sparse n-grams. Trigram-only indexes are significantly smaller (roughly 50% of the hybrid index) but may require more posting lookups per query.

## Integrity guarantees

- **Magic bytes**: Prevent opening wrong file types
- **Version check**: Prevent reading incompatible formats
- **CRC32 checksum**: Detect corruption (computed over entire payload)
- **Payload length**: Cross-checked against actual file size at open time

The reader validates all four properties when opening an index. Any mismatch results in an error -- the reader never returns silently incorrect results from a corrupted index.
