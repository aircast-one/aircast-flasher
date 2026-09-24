//! A block-alignment shim that lets the engine do byte-level I/O over a raw
//! device opened with direct/unbuffered I/O.
//!
//! Raw devices opened with `F_NOCACHE` (macOS), `O_DIRECT` (Linux), or
//! `FILE_FLAG_NO_BUFFERING` (Windows) require every read/write to start on a
//! sector boundary and span a whole number of sectors. The engine, however,
//! treats the device as a plain seekable byte stream. [`AlignedDevice`] bridges
//! the two: it presents byte-level `Read + Write + Seek` to the engine and
//! performs sector-aligned read-modify-write against the underlying handle.
//!
//! Alignment is fixed at [`ALIGN`] = 4096 bytes, a multiple of both 512 (classic
//! sector) and 4096 (Advanced Format / page size), so the aligned I/O it issues
//! satisfies any of those device constraints.

use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::engine::BlockDevice;

/// I/O alignment for direct/unbuffered device access. A multiple of 512 and 4096.
pub const ALIGN: u64 = 4096;

/// Wraps a raw, direct-I/O device handle and exposes byte-level
/// `Read + Write + Seek` to the engine, doing sector-aligned read-modify-write
/// underneath.
///
/// The wrapped handle `T` must itself be `Read + Write + Seek`. All actual reads
/// and writes issued to `T` start at a multiple of [`ALIGN`] and cover a whole
/// number of [`ALIGN`]-byte blocks.
pub struct AlignedDevice<T: Read + Write + Seek> {
    inner: T,
    /// The logical byte position the engine believes it is at.
    pos: u64,
}

impl<T: Read + Write + Seek> AlignedDevice<T> {
    /// Wrap an opened device handle positioned at offset 0.
    pub fn new(inner: T) -> Self {
        Self { inner, pos: 0 }
    }

    /// Round `n` down to the previous [`ALIGN`] boundary.
    fn align_down(n: u64) -> u64 {
        n - (n % ALIGN)
    }

    /// Read `block.len()` bytes starting at `aligned_off` (must be a multiple of
    /// [`ALIGN`], and `block.len()` a whole number of blocks). A short read past
    /// the end of the device is zero-filled, so reads of the tail of an
    /// odd-sized device succeed.
    fn read_block(&mut self, aligned_off: u64, block: &mut [u8]) -> io::Result<()> {
        self.inner.seek(SeekFrom::Start(aligned_off))?;
        let mut filled = 0usize;
        while filled < block.len() {
            match self.inner.read(&mut block[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        for b in &mut block[filled..] {
            *b = 0;
        }
        Ok(())
    }

    /// Write exactly one aligned block at `aligned_off`.
    fn write_block(&mut self, aligned_off: u64, block: &[u8]) -> io::Result<()> {
        self.inner.seek(SeekFrom::Start(aligned_off))?;
        self.inner.write_all(block)
    }
}

impl<T: Read + Write + Seek> Read for AlignedDevice<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // Satisfy the whole request with ONE aligned transfer: read the aligned
        // span covering [pos, pos+len) in a single device read, then copy out the
        // requested slice. (A per-block loop here would turn a 1 MiB engine read
        // into 256 tiny unbuffered device reads — far slower.)
        let start = self.pos;
        let end = start + buf.len() as u64;
        let aligned_start = Self::align_down(start);
        let aligned_end = end.div_ceil(ALIGN) * ALIGN;

        let mut span = vec![0u8; (aligned_end - aligned_start) as usize];
        self.read_block(aligned_start, &mut span)?; // one seek + bulk read, zero-filled past EOF
        let lo = (start - aligned_start) as usize;
        buf.copy_from_slice(&span[lo..lo + buf.len()]);
        self.pos = end;
        Ok(buf.len())
    }
}

impl<T: Read + Write + Seek> Write for AlignedDevice<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let start = self.pos;
        let end = start + buf.len() as u64;
        let aligned_start = Self::align_down(start);
        let aligned_end = end.div_ceil(ALIGN) * ALIGN;

        // Fast path: the request is already block-aligned (the engine's bulk
        // writes are) — one big device write, no read-modify-write.
        if aligned_start == start && aligned_end == end {
            self.write_block(aligned_start, buf)?;
            self.pos = end;
            return Ok(buf.len());
        }

        // General path: assemble the aligned span in one buffer and write it in a
        // single call. Only the partial head/tail blocks need read-modify-write
        // to preserve untouched bytes; the aligned interior is overwritten whole.
        let span_len = (aligned_end - aligned_start) as usize;
        let mut span = vec![0u8; span_len];
        let align = ALIGN as usize;
        let head_partial = start != aligned_start;
        let tail_partial = end != aligned_end;
        if head_partial {
            self.read_block(aligned_start, &mut span[..align])?;
        }
        if tail_partial && !(head_partial && span_len == align) {
            let last = span_len - align;
            self.read_block(aligned_end - ALIGN, &mut span[last..])?;
        }
        let lo = (start - aligned_start) as usize;
        span[lo..lo + buf.len()].copy_from_slice(buf);
        self.write_block(aligned_start, &span)?; // one seek + bulk write
        self.pos = end;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<T: Read + Write + Seek> Seek for AlignedDevice<T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        // Track the logical position ourselves; the underlying handle is
        // re-seeked per aligned block, so its own cursor is incidental. Only
        // SeekFrom::End needs to consult the inner handle.
        let new = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => {
                let end = self.inner.seek(SeekFrom::End(0))? as i64;
                end + d
            }
        };
        if new < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek to a negative position",
            ));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

impl<T: Read + Write + Seek> BlockDevice for AlignedDevice<T>
where
    T: BlockDevice,
{
    fn sync(&mut self) -> io::Result<()> {
        self.inner.sync()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A `Cursor<Vec<u8>>` that also satisfies `BlockDevice` so we can wrap it.
    struct MemCursor(Cursor<Vec<u8>>);

    impl Read for MemCursor {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }
    impl Write for MemCursor {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl Seek for MemCursor {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.0.seek(pos)
        }
    }
    impl BlockDevice for MemCursor {
        fn sync(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Apply a sequence of operations to both an `AlignedDevice` and a plain
    /// `Cursor`, then assert the underlying bytes are identical.
    #[test]
    fn unaligned_writes_round_trip_like_a_plain_cursor() {
        let size = 4 * ALIGN as usize + 777;
        let mut aligned = AlignedDevice::new(MemCursor(Cursor::new(vec![0u8; size])));
        let mut plain = Cursor::new(vec![0u8; size]);

        // A spread of deliberately unaligned offsets and lengths.
        let ops: &[(u64, usize, u8)] = &[
            (0, 10, 0x11),
            (10, 4096, 0x22),           // crosses a block boundary, unaligned start
            (4090, 20, 0x33),           // straddles the first boundary
            (8192, 4096, 0x44),         // fully aligned block
            (8000, 500, 0x55),          // unaligned within/around a block
            (12345, 1234, 0x66),        // odd offset + odd length
            (size as u64 - 5, 5, 0x77), // last 5 bytes
        ];

        for &(off, len, val) in ops {
            let data = vec![val; len];
            aligned.seek(SeekFrom::Start(off)).unwrap();
            aligned.write_all(&data).unwrap();
            plain.seek(SeekFrom::Start(off)).unwrap();
            plain.write_all(&data).unwrap();
        }

        let plain_bytes = plain.into_inner();

        // Read it all back through the aligned device and compare.
        let mut got = vec![0u8; size];
        aligned.seek(SeekFrom::Start(0)).unwrap();
        aligned.read_exact(&mut got).unwrap();
        assert_eq!(
            got, plain_bytes,
            "aligned read-back must match plain cursor"
        );
    }

    #[test]
    fn unaligned_reads_match_plain_cursor() {
        let size = 3 * ALIGN as usize + 123;
        let mut src = vec![0u8; size];
        for (i, b) in src.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let mut aligned = AlignedDevice::new(MemCursor(Cursor::new(src.clone())));

        let reads: &[(u64, usize)] = &[
            (0, 1),
            (1, 5),
            (4095, 2),
            (4096, 4096),
            (5000, 3000),
            (size as u64 - 10, 10),
        ];
        for &(off, len) in reads {
            aligned.seek(SeekFrom::Start(off)).unwrap();
            let mut got = vec![0u8; len];
            aligned.read_exact(&mut got).unwrap();
            assert_eq!(
                got,
                &src[off as usize..off as usize + len],
                "unaligned read at off={off} len={len} must match source"
            );
        }
    }

    #[test]
    fn seek_from_end_and_current() {
        let size = 2 * ALIGN as usize;
        let mut aligned = AlignedDevice::new(MemCursor(Cursor::new(vec![0u8; size])));

        aligned.seek(SeekFrom::End(-100)).unwrap();
        aligned.write_all(&[0xAB; 100]).unwrap();

        aligned.seek(SeekFrom::Start(size as u64 - 100)).unwrap();
        let mut got = vec![0u8; 100];
        aligned.read_exact(&mut got).unwrap();
        assert_eq!(got, vec![0xAB; 100]);

        // SeekFrom::Current
        aligned.seek(SeekFrom::Start(50)).unwrap();
        let pos = aligned.seek(SeekFrom::Current(25)).unwrap();
        assert_eq!(pos, 75);
    }
}
