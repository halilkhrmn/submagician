//! The OpenSubtitles "moviehash": file size plus the 64-bit little-endian word sums of the first
//! and the last 64 KiB. Several providers accept it; it identifies the exact file, not the movie.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::{Error, Result};

const CHUNK: u64 = 64 * 1024;

pub fn file_hash(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    hash_reader(&mut file, size)
}

pub fn hash_reader<R: Read + Seek>(reader: &mut R, size: u64) -> Result<String> {
    if size < CHUNK * 2 {
        return Err(Error::TooSmallToHash(size));
    }
    let mut hash = size;
    let mut buf = vec![0u8; CHUNK as usize];
    for offset in [0, size - CHUNK] {
        reader.seek(SeekFrom::Start(offset))?;
        reader.read_exact(&mut buf)?;
        for word in buf.chunks_exact(8) {
            hash = hash.wrapping_add(u64::from_le_bytes(word.try_into().unwrap()));
        }
    }
    Ok(format!("{hash:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn rejects_small_files() {
        let mut c = Cursor::new(vec![0u8; 1000]);
        assert!(matches!(hash_reader(&mut c, 1000), Err(Error::TooSmallToHash(1000))));
    }

    #[test]
    fn zero_file_hash_is_its_size() {
        let size = 3 * CHUNK;
        let mut c = Cursor::new(vec![0u8; size as usize]);
        assert_eq!(hash_reader(&mut c, size).unwrap(), format!("{size:016x}"));
    }

    #[test]
    fn sums_head_and_tail_words() {
        let size = 2 * CHUNK + 8;
        let mut data = vec![0u8; size as usize];
        data[0] = 1; // first word of the head
        let tail_start = (size - CHUNK) as usize;
        data[tail_start..tail_start + 8].copy_from_slice(&u64::MAX.to_le_bytes()); // wraps
        data[CHUNK as usize + 4] = 0xff; // between head and tail: ignored
        let mut c = Cursor::new(data);
        // size + 1 + u64::MAX (wrapping) == size
        assert_eq!(hash_reader(&mut c, size).unwrap(), format!("{size:016x}"));
    }
}
