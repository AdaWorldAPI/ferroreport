//! A minimal ZIP writer (deflate), enough for Office Open XML packages.
//!
//! Small files are added whole. A large one (the sheet of a workbook) can be written a piece
//! at a time: it is compressed and sent on as it comes, and its size and checksum, which are
//! only known at its end, follow it in a data descriptor instead of preceding it.

use std::io::{self, Write};

/// Writes a ZIP archive to `out` as its files are added.
pub(crate) struct ZipStream<W: Write> {
    out: W,
    offset: u64,
    central: Vec<u8>,
    count: u16,
    open: Option<Open>,
}

/// The file being written a piece at a time.
struct Open {
    name: Vec<u8>,
    offset: u64,
    crc: Crc,
    size: u64,
    packed: u64,
    packer: Packer,
}

// Entries are dated 1980-01-01 00:00 so the same report always gives the same bytes.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;
/// Flag: sizes and checksum follow the data.
const DESCRIPTOR: u16 = 1 << 3;
// Level 3: within 10% of the best size, several times faster on large sheets.
const LEVEL: u8 = 3;

fn too_large() -> io::Error {
    io::Error::other("the workbook is larger than 4 GB, more than its format holds")
}

fn header(sig: u32, flags: u16, crc: u32, packed: u32, size: u32, name: &[u8], central_offset: Option<u32>) -> Vec<u8> {
    let mut h = Vec::with_capacity(46 + name.len());
    h.extend_from_slice(&sig.to_le_bytes());
    if central_offset.is_some() {
        h.extend_from_slice(&20u16.to_le_bytes()); // made by
    }
    h.extend_from_slice(&20u16.to_le_bytes()); // needed to extract
    h.extend_from_slice(&flags.to_le_bytes());
    h.extend_from_slice(&8u16.to_le_bytes()); // deflate
    h.extend_from_slice(&DOS_TIME.to_le_bytes());
    h.extend_from_slice(&DOS_DATE.to_le_bytes());
    h.extend_from_slice(&crc.to_le_bytes());
    h.extend_from_slice(&packed.to_le_bytes());
    h.extend_from_slice(&size.to_le_bytes());
    h.extend_from_slice(&(name.len() as u16).to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes()); // extra
    if let Some(offset) = central_offset {
        h.extend_from_slice(&0u16.to_le_bytes()); // comment
        h.extend_from_slice(&0u16.to_le_bytes()); // disk
        h.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        h.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        h.extend_from_slice(&offset.to_le_bytes());
    }
    h.extend_from_slice(name);
    h
}

impl<W: Write> ZipStream<W> {
    pub fn new(out: W) -> ZipStream<W> {
        ZipStream { out, offset: 0, central: Vec::new(), count: 0, open: None }
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.offset += bytes.len() as u64;
        Ok(())
    }

    fn offset32(&self) -> io::Result<u32> {
        u32::try_from(self.offset).map_err(|_| too_large())
    }

    /// Adds a whole file.
    pub fn add(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        debug_assert!(self.open.is_none());
        let mut crc = Crc::default();
        crc.update(data);
        let packed = miniz_oxide::deflate::compress_to_vec(data, LEVEL);
        let offset = self.offset32()?;
        let (packed_len, len) = (packed.len() as u32, data.len() as u32);
        self.put(&header(0x0403_4b50, 0, crc.sum(), packed_len, len, name.as_bytes(), None))?;
        self.put(&packed)?;
        self.central.extend(header(0x0201_4b50, 0, crc.sum(), packed_len, len, name.as_bytes(), Some(offset)));
        self.count += 1;
        Ok(())
    }

    /// Starts a file whose content comes in pieces ([`ZipStream::write`]) until [`ZipStream::end`].
    pub fn begin(&mut self, name: &str) -> io::Result<()> {
        debug_assert!(self.open.is_none());
        let offset = self.offset;
        self.offset32()?;
        self.put(&header(0x0403_4b50, DESCRIPTOR, 0, 0, 0, name.as_bytes(), None))?;
        self.open =
            Some(Open { name: name.as_bytes().to_vec(), offset, crc: Crc::default(), size: 0, packed: 0, packer: Packer::new() });
        Ok(())
    }

    /// The next piece of the file that was begun.
    pub fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let Some(open) = &mut self.open else { return Ok(()) };
        open.crc.update(data);
        open.size += data.len() as u64;
        let before = self.offset;
        let (out, offset) = (&mut self.out, &mut self.offset);
        open.packer.pack(data, false, &mut |bytes: &[u8]| {
            *offset += bytes.len() as u64;
            out.write_all(bytes)
        })?;
        open.packed += self.offset - before;
        Ok(())
    }

    /// Ends the file that was begun.
    pub fn end(&mut self) -> io::Result<()> {
        let Some(mut open) = self.open.take() else { return Ok(()) };
        let before = self.offset;
        let (out, offset) = (&mut self.out, &mut self.offset);
        open.packer.pack(&[], true, &mut |bytes: &[u8]| {
            *offset += bytes.len() as u64;
            out.write_all(bytes)
        })?;
        open.packed += self.offset - before;
        let size = u32::try_from(open.size).map_err(|_| too_large())?;
        let packed = u32::try_from(open.packed).map_err(|_| too_large())?;
        let start = u32::try_from(open.offset).map_err(|_| too_large())?;
        let mut descriptor = Vec::with_capacity(16);
        descriptor.extend_from_slice(&0x0807_4b50u32.to_le_bytes());
        descriptor.extend_from_slice(&open.crc.sum().to_le_bytes());
        descriptor.extend_from_slice(&packed.to_le_bytes());
        descriptor.extend_from_slice(&size.to_le_bytes());
        self.put(&descriptor)?;
        self.central.extend(header(0x0201_4b50, DESCRIPTOR, open.crc.sum(), packed, size, &open.name, Some(start)));
        self.count += 1;
        Ok(())
    }

    /// Writes the directory of the archive and gives the output back.
    pub fn finish(mut self) -> io::Result<W> {
        self.end()?;
        let offset = self.offset32()?;
        let size = self.central.len() as u32;
        let central = std::mem::take(&mut self.central);
        self.put(&central)?;
        let mut end = Vec::with_capacity(22);
        end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        end.extend_from_slice(&[0; 4]); // disk numbers
        end.extend_from_slice(&self.count.to_le_bytes());
        end.extend_from_slice(&self.count.to_le_bytes());
        end.extend_from_slice(&size.to_le_bytes());
        end.extend_from_slice(&offset.to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes()); // comment
        self.put(&end)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// Compresses a file piece by piece. Where the fast compressor is not built in
/// (WebAssembly), the pieces are kept and compressed together at the end.
#[cfg(not(target_arch = "wasm32"))]
struct Packer(flate2::Compress, Vec<u8>);

#[cfg(not(target_arch = "wasm32"))]
impl Packer {
    fn new() -> Packer {
        Packer(flate2::Compress::new(flate2::Compression::new(LEVEL as u32), false), vec![0; 1 << 16])
    }

    fn pack(&mut self, mut data: &[u8], last: bool, out: &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        let flush = if last { flate2::FlushCompress::Finish } else { flate2::FlushCompress::None };
        loop {
            let (read, written) = (self.0.total_in(), self.0.total_out());
            let status = self.0.compress(data, &mut self.1, flush).map_err(io::Error::other)?;
            let taken = (self.0.total_in() - read) as usize;
            let made = (self.0.total_out() - written) as usize;
            data = &data[taken..];
            out(&self.1[..made])?;
            let done = if last { status == flate2::Status::StreamEnd } else { data.is_empty() && made < self.1.len() };
            if done {
                return Ok(());
            }
            if taken == 0 && made == 0 {
                return Err(io::Error::other("the compressor made no progress"));
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
struct Packer(Vec<u8>);

#[cfg(target_arch = "wasm32")]
impl Packer {
    fn new() -> Packer {
        Packer(Vec::new())
    }

    fn pack(&mut self, data: &[u8], last: bool, out: &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        self.0.extend_from_slice(data);
        if last {
            out(&miniz_oxide::deflate::compress_to_vec(&std::mem::take(&mut self.0), LEVEL))?;
        }
        Ok(())
    }
}

/// CRC-32 of what it has been given so far.
struct Crc(u32);

impl Default for Crc {
    fn default() -> Crc {
        Crc(!0)
    }
}

const fn table() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut n = 1;
    while n < 8 {
        let mut i = 0;
        while i < 256 {
            t[n][i] = (t[n - 1][i] >> 8) ^ t[0][(t[n - 1][i] & 0xFF) as usize];
            i += 1;
        }
        n += 1;
    }
    t
}

static TABLE: [[u32; 256]; 8] = table();

impl Crc {
    /// Eight bytes at a time: a sheet can be hundreds of megabytes.
    fn update(&mut self, data: &[u8]) {
        let mut crc = self.0;
        let mut chunks = data.chunks_exact(8);
        for c in &mut chunks {
            let low = u32::from_le_bytes([c[0], c[1], c[2], c[3]]) ^ crc;
            let high = u32::from_le_bytes([c[4], c[5], c[6], c[7]]);
            crc = TABLE[7][(low & 0xFF) as usize]
                ^ TABLE[6][((low >> 8) & 0xFF) as usize]
                ^ TABLE[5][((low >> 16) & 0xFF) as usize]
                ^ TABLE[4][(low >> 24) as usize]
                ^ TABLE[3][(high & 0xFF) as usize]
                ^ TABLE[2][((high >> 8) & 0xFF) as usize]
                ^ TABLE[1][((high >> 16) & 0xFF) as usize]
                ^ TABLE[0][(high >> 24) as usize];
        }
        for &b in chunks.remainder() {
            crc = TABLE[0][((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        self.0 = crc;
    }

    fn sum(&self) -> u32 {
        !self.0
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn crc_of_known_strings() {
        let mut crc = Crc::default();
        crc.update(b"123456789");
        assert_eq!(crc.sum(), 0xCBF4_3926);
        // In pieces, across the eight-byte step.
        let text: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut whole = Crc::default();
        whole.update(&text);
        let mut pieces = Crc::default();
        for part in text.chunks(13) {
            pieces.update(part);
        }
        assert_eq!(whole.sum(), pieces.sum());
        let mut slow = !0u32;
        for &b in &text {
            slow = TABLE[0][((slow ^ b as u32) & 0xFF) as usize] ^ (slow >> 8);
        }
        assert_eq!(whole.sum(), !slow);
    }

    /// Reads an archive back: every file with its content, checked against its checksum.
    pub(crate) fn read(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
        let u16_at = |i: usize| u16::from_le_bytes([zip[i], zip[i + 1]]) as usize;
        let u32_at = |i: usize| u32::from_le_bytes([zip[i], zip[i + 1], zip[i + 2], zip[i + 3]]) as usize;
        let end = zip.len() - 22;
        assert_eq!(u32_at(end), 0x0605_4b50);
        let (count, mut at) = (u16_at(end + 10), u32_at(end + 16));
        let mut files = Vec::new();
        for _ in 0..count {
            assert_eq!(u32_at(at), 0x0201_4b50);
            let (crc, packed, size, name_len, local) =
                (u32_at(at + 16), u32_at(at + 20), u32_at(at + 24), u16_at(at + 28), u32_at(at + 42));
            let name = String::from_utf8(zip[at + 46..at + 46 + name_len].to_vec()).unwrap();
            assert_eq!(u32_at(local), 0x0403_4b50);
            let data = local + 30 + u16_at(local + 26) + u16_at(local + 28);
            let content = miniz_oxide::inflate::decompress_to_vec(&zip[data..data + packed]).unwrap();
            assert_eq!(content.len(), size);
            let mut sum = Crc::default();
            sum.update(&content);
            assert_eq!(sum.sum() as usize, crc, "{name}");
            files.push((name, content));
            at += 46 + name_len;
        }
        files
    }

    #[test]
    fn whole_files_and_files_in_pieces_read_back() {
        let big: Vec<u8> = (0..300_000u32).flat_map(|i| format!("<c r=\"A{i}\">{}</c>", i % 97).into_bytes()).collect();
        let mut zip = ZipStream::new(Vec::new());
        zip.add("first.xml", b"<a/>").unwrap();
        zip.begin("sheet.xml").unwrap();
        for piece in big.chunks(70_001) {
            zip.write(piece).unwrap();
        }
        zip.end().unwrap();
        zip.add("last.xml", b"").unwrap();
        let files = read(&zip.finish().unwrap());
        assert_eq!(files.len(), 3);
        assert_eq!(files[0], ("first.xml".to_string(), b"<a/>".to_vec()));
        assert_eq!(files[1].0, "sheet.xml");
        assert!(files[1].1 == big);
        assert_eq!(files[2], ("last.xml".to_string(), Vec::new()));
    }
}
