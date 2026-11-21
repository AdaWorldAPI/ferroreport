//! A minimal ZIP writer (deflate), enough for Office Open XML packages.

/// Builds a ZIP archive in memory.
#[derive(Default)]
pub(crate) struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

// Entries are dated 1980-01-01 00:00 so the same report always gives the same bytes.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

impl ZipWriter {
    pub fn add(&mut self, name: &str, data: &[u8]) {
        let crc = crc32(data);
        // Level 3: within 10% of the best size, several times faster on large sheets.
        let packed = miniz_oxide::deflate::compress_to_vec(data, 3);
        let offset = self.out.len() as u32;
        let name = name.as_bytes();
        let header = |sig: u32, central: bool| {
            let mut h = Vec::with_capacity(46 + name.len());
            h.extend_from_slice(&sig.to_le_bytes());
            if central {
                h.extend_from_slice(&20u16.to_le_bytes()); // made by
            }
            h.extend_from_slice(&20u16.to_le_bytes()); // needed to extract
            h.extend_from_slice(&0u16.to_le_bytes()); // flags
            h.extend_from_slice(&8u16.to_le_bytes()); // deflate
            h.extend_from_slice(&DOS_TIME.to_le_bytes());
            h.extend_from_slice(&DOS_DATE.to_le_bytes());
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // extra
            if central {
                h.extend_from_slice(&0u16.to_le_bytes()); // comment
                h.extend_from_slice(&0u16.to_le_bytes()); // disk
                h.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
                h.extend_from_slice(&0u32.to_le_bytes()); // external attributes
                h.extend_from_slice(&offset.to_le_bytes());
            }
            h.extend_from_slice(name);
            h
        };
        self.out.extend(header(0x0403_4b50, false));
        self.out.extend_from_slice(&packed);
        self.central.extend(header(0x0201_4b50, true));
        self.count += 1;
    }

    pub fn finish(mut self) -> Vec<u8> {
        let offset = self.out.len() as u32;
        let size = self.central.len() as u32;
        self.out.extend_from_slice(&self.central);
        self.out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]); // disk numbers
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&size.to_le_bytes());
        self.out.extend_from_slice(&offset.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes()); // comment
        self.out
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    let mut crc = !0u32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc_of_known_string() {
        assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926);
    }
}
