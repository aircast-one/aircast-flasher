use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::engine::{BlockDevice, BLOCK};

const CHUNK: u64 = BLOCK as u64;

struct Chunk {
    data: Vec<u8>,
    dirty: bool,
}

pub(crate) struct CachedDevice<'a> {
    inner: &'a mut dyn BlockDevice,
    len: u64,
    pos: u64,
    chunks: BTreeMap<u64, Chunk>,
}

impl<'a> CachedDevice<'a> {
    pub(crate) fn new(inner: &'a mut dyn BlockDevice) -> io::Result<Self> {
        let len = inner.seek(SeekFrom::End(0))?;
        Ok(Self {
            inner,
            len,
            pos: 0,
            chunks: BTreeMap::new(),
        })
    }

    fn chunk(&mut self, index: u64) -> io::Result<&mut Chunk> {
        match self.chunks.entry(index) {
            std::collections::btree_map::Entry::Occupied(cached) => Ok(cached.into_mut()),
            std::collections::btree_map::Entry::Vacant(slot) => {
                let start = index * CHUNK;
                let mut data = vec![0u8; CHUNK.min(self.len - start) as usize];
                self.inner.seek(SeekFrom::Start(start))?;
                self.inner.read_exact(&mut data)?;
                Ok(slot.insert(Chunk { data, dirty: false }))
            }
        }
    }

    fn at(&mut self) -> io::Result<(&mut Chunk, usize)> {
        let offset = (self.pos % CHUNK) as usize;
        self.chunk(self.pos / CHUNK).map(|chunk| (chunk, offset))
    }

    pub(crate) fn dirty_mut(&mut self) -> impl Iterator<Item = &mut [u8]> {
        self.chunks
            .values_mut()
            .filter(|chunk| chunk.dirty)
            .map(|chunk| chunk.data.as_mut_slice())
    }

    pub(crate) fn commit(self) -> io::Result<()> {
        self.chunks
            .iter()
            .filter(|(_, chunk)| chunk.dirty)
            .try_for_each(|(index, chunk)| {
                self.inner.seek(SeekFrom::Start(index * CHUNK))?;
                self.inner.write_all(&chunk.data)
            })?;
        self.inner.sync()
    }
}

impl Read for CachedDevice<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.len {
            return Ok(0);
        }
        let (chunk, offset) = self.at()?;
        let n = buf.len().min(chunk.data.len() - offset);
        buf[..n].copy_from_slice(&chunk.data[offset..offset + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Write for CachedDevice<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.pos >= self.len {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "write past the end of the card",
            ));
        }
        let (chunk, offset) = self.at()?;
        let n = buf.len().min(chunk.data.len() - offset);
        chunk.data[offset..offset + n].copy_from_slice(&buf[..n]);
        chunk.dirty = true;
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for CachedDevice<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
        };
        self.pos = target.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of the card",
            )
        })?;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Counted {
        cur: Cursor<Vec<u8>>,
        writes: usize,
    }

    impl Read for Counted {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.cur.read(buf)
        }
    }
    impl Write for Counted {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.cur.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.cur.seek(pos)
        }
    }
    impl BlockDevice for Counted {
        fn sync(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn edit(cache: &mut CachedDevice<'_>, edits: &[(u64, usize, u8)]) {
        edits.iter().for_each(|&(at, n, byte)| {
            cache.seek(SeekFrom::Start(at)).unwrap();
            cache.write_all(&vec![byte; n]).unwrap();
        });
    }

    #[test]
    fn nothing_reaches_the_card_until_commit_and_then_matches_a_plain_write() {
        let len = 3 * BLOCK + 777;
        let source: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        let edits = [
            (10, 20, 0x11),
            (CHUNK - 5, 10, 0x22),
            (2 * CHUNK + 3, BLOCK, 0x33),
            (len as u64 - 4, 4, 0x44),
        ];
        let mut plain = Cursor::new(source.clone());
        edits.iter().for_each(|&(at, n, byte)| {
            plain.seek(SeekFrom::Start(at)).unwrap();
            plain.write_all(&vec![byte; n]).unwrap();
        });
        let mut card = Counted {
            cur: Cursor::new(source.clone()),
            writes: 0,
        };

        let mut dropped = CachedDevice::new(&mut card).unwrap();
        edit(&mut dropped, &edits);
        let mut back = vec![0u8; 30];
        dropped.seek(SeekFrom::Start(0)).unwrap();
        dropped.read_exact(&mut back).unwrap();
        assert_eq!(back, plain.get_ref()[..30]);
        dropped.seek(SeekFrom::End(0)).unwrap();
        assert!(dropped.write(&[1]).is_err(), "the card must not grow");
        drop(dropped);
        assert_eq!((card.writes, card.cur.get_ref()), (0, &source));

        let mut cache = CachedDevice::new(&mut card).unwrap();
        edit(&mut cache, &edits);
        cache.commit().unwrap();
        assert_eq!(card.cur.get_ref(), plain.get_ref());
        assert_eq!(card.writes, 4, "one write per touched chunk");
    }
}
