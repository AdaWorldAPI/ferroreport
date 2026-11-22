//! PDF encryption: the standard security handler of PDF 2.0 (revision 6, AES-256), which every
//! current reader opens. A user password is asked for to open the document; an owner password
//! lifts the permissions (printing, copying…) that readers otherwise apply.

use aes::cipher::{BlockEncrypt, KeyInit};
use aes::{Aes128, Aes256};
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::{Error, Result};

/// What readers let people do without the owner password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Permissions(pub u32);

impl Permissions {
    pub const PRINTING: u32 = (1 << 2) | (1 << 11);
    pub const MODIFY_CONTENTS: u32 = 1 << 3;
    pub const COPY: u32 = 1 << 4;
    pub const MODIFY_ANNOTATIONS: u32 = 1 << 5;
    pub const FILL_IN: u32 = 1 << 8;
    pub const SCREENREADERS: u32 = 1 << 9;
    pub const ASSEMBLY: u32 = 1 << 10;
    pub const DEGRADED_PRINTING: u32 = 1 << 2;
    pub const ALL: u32 = Self::PRINTING
        | Self::MODIFY_CONTENTS
        | Self::COPY
        | Self::MODIFY_ANNOTATIONS
        | Self::FILL_IN
        | Self::SCREENREADERS
        | Self::ASSEMBLY;

    /// One permission by its JasperReports name (`PRINTING`, `COPY`…) or a short one
    /// (`print`, `copy`, `modify`, `annotate`, `fill`, `accessibility`, `assemble`).
    pub fn bits(name: &str) -> Option<u32> {
        Some(match name.trim().to_ascii_uppercase().replace('-', "_").as_str() {
            "ALL" => Self::ALL,
            "PRINTING" | "PRINT" => Self::PRINTING,
            "DEGRADED_PRINTING" | "PRINT_LOW" => Self::DEGRADED_PRINTING,
            "MODIFY_CONTENTS" | "MODIFY" => Self::MODIFY_CONTENTS,
            "COPY" => Self::COPY,
            "MODIFY_ANNOTATIONS" | "ANNOTATE" => Self::MODIFY_ANNOTATIONS,
            "FILL_IN" | "FILL" => Self::FILL_IN,
            "SCREENREADERS" | "ACCESSIBILITY" => Self::SCREENREADERS,
            "ASSEMBLY" | "ASSEMBLE" => Self::ASSEMBLY,
            "" | "NONE" => 0,
            _ => return None,
        })
    }

    /// A list of permissions separated by `|`, `,` or spaces (as JasperReports writes them).
    pub fn parse(list: &str) -> Result<Permissions> {
        let mut bits = 0;
        for name in list.split(['|', ',', ' ']).filter(|n| !n.is_empty()) {
            bits |= Self::bits(name).ok_or_else(|| {
                Error::Template(format!(
                    "unknown PDF permission `{name}` (PRINTING, DEGRADED_PRINTING, COPY, MODIFY_CONTENTS, MODIFY_ANNOTATIONS, FILL_IN, SCREENREADERS, ASSEMBLY or ALL)"
                ))
            })?;
        }
        Ok(Permissions(bits))
    }

    /// The value of `/P`: the permission bits with the bits the format reserves set.
    fn p(self) -> i32 {
        (0xFFFF_F0C0u32 | (self.0 & 0xF3C)) as i32
    }
}

/// Passwords and permissions of an encrypted PDF.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Encryption {
    /// Asked for to open the document; empty opens it without asking (permissions still apply).
    pub user_password: String,
    /// Lifts the permissions. Without one, a random password nobody knows is used.
    pub owner_password: Option<String>,
    pub permissions: Permissions,
}

/// Without the passwords, which do not belong in logs.
impl std::fmt::Debug for Encryption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encryption")
            .field("user_password", &if self.user_password.is_empty() { "" } else { "…" })
            .field("owner_password", &self.owner_password.as_ref().map(|_| "…"))
            .field("permissions", &self.permissions)
            .finish()
    }
}

/// Keys and values that encrypt one document.
pub(crate) struct Crypt {
    key: Aes256,
    iv: Aes256,
    counter: u64,
    o: [u8; 48],
    u: [u8; 48],
    oe: [u8; 32],
    ue: [u8; 32],
    perms: [u8; 16],
    p: i32,
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|e| Error::Io(format!("no random numbers for the encryption: {e}")))?;
    Ok(b)
}

/// The password as the format uses it: UTF-8, at most 127 bytes.
fn password(p: &str) -> &[u8] {
    let mut end = p.len().min(127);
    while !p.is_char_boundary(end) {
        end -= 1;
    }
    &p.as_bytes()[..end]
}

/// Algorithm 2.B of ISO 32000-2: the hash of a password.
fn hash(password: &[u8], salt: &[u8], udata: &[u8]) -> [u8; 32] {
    let mut k: Vec<u8> = Sha256::new().chain_update(password).chain_update(salt).chain_update(udata).finalize().to_vec();
    let mut round = 0u32;
    let mut e: Vec<u8> = Vec::new();
    while round < 64 || u32::from(*e.last().unwrap_or(&0)) > round - 32 {
        let mut k1 = Vec::with_capacity((password.len() + k.len() + udata.len()) * 64);
        for _ in 0..64 {
            k1.extend_from_slice(password);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(udata);
        }
        let aes = Aes128::new_from_slice(&k[..16]).expect("16 bytes");
        e = cbc(&aes, k[16..32].try_into().expect("16 bytes"), &k1);
        let n: u32 = e[..16].iter().map(|&b| u32::from(b)).sum::<u32>() % 3;
        k = match n {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
    }
    k[..32].try_into().expect("32 bytes")
}

/// CBC encryption of whole blocks.
fn cbc<C: BlockEncrypt + aes::cipher::BlockSizeUser<BlockSize = aes::cipher::consts::U16>>(
    cipher: &C,
    iv: [u8; 16],
    data: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut prev = iv;
    for block in data.chunks_exact(16) {
        let mut b = aes::Block::default();
        for i in 0..16 {
            b[i] = block[i] ^ prev[i];
        }
        cipher.encrypt_block(&mut b);
        prev.copy_from_slice(&b);
        out.extend_from_slice(&b);
    }
    out
}

fn hex(out: &mut Vec<u8>, bytes: &[u8]) {
    const H: &[u8; 16] = b"0123456789ABCDEF";
    out.push(b'<');
    for b in bytes {
        out.push(H[(b >> 4) as usize]);
        out.push(H[(b & 15) as usize]);
    }
    out.push(b'>');
}

impl Crypt {
    pub fn new(e: &Encryption) -> Result<Crypt> {
        let file_key: [u8; 32] = random()?;
        let user = password(&e.user_password);
        let random_owner;
        let owner = match &e.owner_password {
            Some(p) if !p.is_empty() => password(p),
            _ => {
                random_owner = random::<32>()?;
                &random_owner[..]
            }
        };
        let salts: [u8; 32] = random()?;
        let (uvs, uks, ovs, oks) = (&salts[..8], &salts[8..16], &salts[16..24], &salts[24..32]);
        let mut u = [0u8; 48];
        u[..32].copy_from_slice(&hash(user, uvs, &[]));
        u[32..40].copy_from_slice(uvs);
        u[40..].copy_from_slice(uks);
        let ue: [u8; 32] =
            cbc(&Aes256::new_from_slice(&hash(user, uks, &[])).expect("32"), [0; 16], &file_key).try_into().expect("32");
        let mut o = [0u8; 48];
        o[..32].copy_from_slice(&hash(owner, ovs, &u));
        o[32..40].copy_from_slice(ovs);
        o[40..].copy_from_slice(oks);
        let oe: [u8; 32] =
            cbc(&Aes256::new_from_slice(&hash(owner, oks, &u)).expect("32"), [0; 16], &file_key).try_into().expect("32");
        let p = e.permissions.p();
        let key = Aes256::new_from_slice(&file_key).expect("32");
        let mut perms = aes::Block::default();
        perms[..4].copy_from_slice(&p.to_le_bytes());
        perms[4..8].copy_from_slice(&[0xFF; 4]);
        perms[8..12].copy_from_slice(b"Tadb");
        perms[12..].copy_from_slice(&random::<4>()?);
        key.encrypt_block(&mut perms);
        Ok(Crypt {
            key,
            // Initialization vectors: a counter encrypted with a second random key, so they
            // cannot be foretold, without asking the system for random numbers each time.
            iv: Aes256::new_from_slice(&random::<32>()?).expect("32"),
            counter: 0,
            o,
            u,
            oe,
            ue,
            perms: perms.into(),
            p,
        })
    }

    /// The encryption dictionary, as the body of an object.
    pub fn dictionary(&self) -> Vec<u8> {
        let mut d = b"<</Filter/Standard/V 5/R 6/Length 256/CF<</StdCF<</AuthEvent/DocOpen/CFM/AESV3/Length 32>>>>/StmF/StdCF/StrF/StdCF/O".to_vec();
        hex(&mut d, &self.o);
        d.extend_from_slice(b"/U");
        hex(&mut d, &self.u);
        d.extend_from_slice(b"/OE");
        hex(&mut d, &self.oe);
        d.extend_from_slice(b"/UE");
        hex(&mut d, &self.ue);
        d.extend_from_slice(b"/Perms");
        hex(&mut d, &self.perms);
        d.extend_from_slice(format!("/P {}>>", self.p).as_bytes());
        d
    }

    /// AES-256-CBC with the initialization vector first and PKCS#7 padding.
    fn encrypt(&mut self, data: &[u8]) -> Vec<u8> {
        let mut iv = aes::Block::default();
        iv[..8].copy_from_slice(&self.counter.to_le_bytes());
        self.counter += 1;
        self.iv.encrypt_block(&mut iv);
        let pad = 16 - data.len() % 16;
        let mut padded = Vec::with_capacity(data.len() + pad);
        padded.extend_from_slice(data);
        padded.resize(data.len() + pad, pad as u8);
        let mut out = iv.to_vec();
        out.extend(cbc(&self.key, iv.into(), &padded));
        out
    }

    /// One indirect object (`n 0 obj … endobj`) with its strings and stream encrypted.
    pub fn object(&mut self, obj: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(obj.len() + 64);
        // The header, up to and with `obj`.
        let Some(start) = find(obj, b"obj").map(|i| i + 3) else { return obj.to_vec() };
        out.extend_from_slice(&obj[..start]);
        let mut i = start;
        // Where the value of the stream's /Length is in `out`.
        let mut length: Option<(usize, usize)> = None;
        let mut depth = 0usize;
        while i < obj.len() {
            match obj[i] {
                b'(' => {
                    let (text, end) = literal(obj, i);
                    let enc = self.encrypt(&text);
                    hex(&mut out, &enc);
                    i = end;
                }
                b'<' if obj.get(i + 1) == Some(&b'<') => {
                    depth += 1;
                    out.extend_from_slice(b"<<");
                    i += 2;
                }
                b'<' => {
                    let end = obj[i..].iter().position(|&b| b == b'>').map_or(obj.len(), |p| i + p + 1);
                    let text = unhex(&obj[i + 1..end.saturating_sub(1)]);
                    let enc = self.encrypt(&text);
                    hex(&mut out, &enc);
                    i = end;
                }
                b'>' if obj.get(i + 1) == Some(&b'>') => {
                    depth = depth.saturating_sub(1);
                    out.extend_from_slice(b">>");
                    i += 2;
                }
                b'/' if depth == 1
                    && obj[i..].starts_with(b"/Length")
                    && obj.get(i + 7).is_some_and(|b| !b.is_ascii_alphanumeric()) =>
                {
                    out.extend_from_slice(b"/Length");
                    i += 7;
                    while i < obj.len() && obj[i].is_ascii_whitespace() {
                        out.push(obj[i]);
                        i += 1;
                    }
                    let n = obj[i..].iter().take_while(|b| b.is_ascii_digit()).count();
                    length = Some((out.len(), n));
                    out.extend_from_slice(&obj[i..i + n]);
                    i += n;
                }
                b's' if depth == 0 && obj[i..].starts_with(b"stream") => {
                    let mut data_start = i + 6;
                    if obj.get(data_start) == Some(&b'\r') {
                        data_start += 1;
                    }
                    if obj.get(data_start) == Some(&b'\n') {
                        data_start += 1;
                    }
                    let end = rfind(obj, b"endstream").unwrap_or(obj.len());
                    // The data is as long as /Length says (its last byte may be an end of line).
                    let stated = length.and_then(|(at, n)| std::str::from_utf8(&out[at..at + n]).ok()?.parse::<usize>().ok());
                    let data_end = match stated {
                        Some(n) if data_start + n <= end => data_start + n,
                        _ => {
                            let mut e = end;
                            if e > data_start && obj[e - 1] == b'\n' {
                                e -= 1;
                            }
                            e
                        }
                    };
                    let enc = self.encrypt(&obj[data_start..data_end]);
                    if let Some((at, n)) = length {
                        out.splice(at..at + n, enc.len().to_string().into_bytes());
                    }
                    out.extend_from_slice(b"stream\n");
                    out.extend_from_slice(&enc);
                    // Whatever came between the data and `endstream` (an end of line).
                    out.extend_from_slice(&obj[data_end..]);
                    return out;
                }
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        out
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

fn unhex(text: &[u8]) -> Vec<u8> {
    let digits: Vec<u8> = text
        .iter()
        .filter_map(|&c| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        })
        .collect();
    digits.chunks(2).map(|p| (p[0] << 4) | p.get(1).copied().unwrap_or(0)).collect()
}

/// The bytes of the literal string that starts at `start`, and where it ends.
fn literal(s: &[u8], start: usize) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut i = start + 1;
    while i < s.len() {
        let c = s[i];
        i += 1;
        match c {
            b'\\' => {
                let Some(&n) = s.get(i) else { break };
                i += 1;
                match n {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'0'..=b'7' => {
                        let mut v = u32::from(n - b'0');
                        for _ in 0..2 {
                            match s.get(i) {
                                Some(&d @ b'0'..=b'7') => {
                                    v = v * 8 + u32::from(d - b'0');
                                    i += 1;
                                }
                                _ => break,
                            }
                        }
                        out.push(v as u8);
                    }
                    b'\r' => {
                        if s.get(i) == Some(&b'\n') {
                            i += 1;
                        }
                    }
                    b'\n' => {}
                    other => out.push(other),
                }
            }
            b'(' => {
                depth += 1;
                out.push(c);
            }
            b')' if depth == 0 => return (out, i),
            b')' => {
                depth -= 1;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    (out, i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_literal_strings() {
        let (t, end) = literal(
            br"(a\(b\) (c) \101\n\
d) rest",
            0,
        );
        assert_eq!(t, b"a(b) (c) A\nd");
        assert_eq!(
            &br"(a\(b\) (c) \101\n\
d) rest"[end..],
            b" rest"
        );
    }

    /// Opens what `Crypt` wrote the way a reader does: the file key from the password and
    /// `/UE`, then each string and stream.
    #[test]
    fn a_reader_with_the_password_gets_the_text_back() {
        use aes::cipher::BlockDecrypt;
        let decrypt = |key: &Aes256, data: &[u8]| {
            let mut prev: [u8; 16] = data[..16].try_into().unwrap();
            let mut out = Vec::new();
            for block in data[16..].chunks_exact(16) {
                let mut b = aes::Block::clone_from_slice(block);
                key.decrypt_block(&mut b);
                out.extend(b.iter().zip(prev).map(|(x, p)| x ^ p));
                prev.copy_from_slice(block);
            }
            let pad = *out.last().unwrap() as usize;
            out.truncate(out.len() - pad);
            out
        };
        let e = Encryption {
            user_password: "año".into(),
            owner_password: Some("dueño".into()),
            permissions: Permissions(Permissions::PRINTING),
        };
        let mut crypt = Crypt::new(&e).unwrap();
        let object = crypt.object(
            b"7 0 obj\n<</Title (Caf\\351 \\(ok\\)) /Alt <414243> /Length 5 /Length1 99>>\nstream\nhello\nendstream\nendobj\n",
        );
        // The password is right, and gives the file key.
        let u = &crypt.u;
        assert_eq!(hash("año".as_bytes(), &u[32..40], &[]), u[..32]);
        assert_ne!(hash(b"ano", &u[32..40], &[]), u[..32]);
        let intermediate = Aes256::new_from_slice(&hash("año".as_bytes(), &u[40..48], &[])).unwrap();
        let mut file_key = Vec::new();
        for block in crypt.ue.chunks_exact(16) {
            let mut b = aes::Block::clone_from_slice(block);
            intermediate.decrypt_block(&mut b);
            file_key.extend_from_slice(&b);
        }
        // No initialization vector for /UE: CBC with zeros, so blocks are chained.
        for i in 0..16 {
            file_key[16 + i] ^= crypt.ue[i];
        }
        let key = Aes256::new_from_slice(&file_key).unwrap();
        let text = String::from_utf8_lossy(&object).into_owned();
        let hexes: Vec<&str> = text.split('<').filter_map(|p| p.split('>').next()).filter(|h| h.len() > 40).collect();
        assert_eq!(decrypt(&key, &unhex(hexes[0].as_bytes())), b"Caf\xe9 (ok)");
        assert_eq!(decrypt(&key, &unhex(hexes[1].as_bytes())), b"ABC");
        // The stream: its new length, /Length1 untouched.
        assert!(text.contains("/Length 32 /Length1 99"), "{text}");
        let start = object.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        assert_eq!(decrypt(&key, &object[start..start + 32]), b"hello");
        assert!(text.ends_with("\nendstream\nendobj\n"));
        // The permissions, as the owner left them.
        let mut perms = aes::Block::from(crypt.perms);
        key.decrypt_block(&mut perms);
        assert_eq!(&perms[9..12], b"adb");
        assert_eq!(i32::from_le_bytes(perms[..4].try_into().unwrap()), Permissions(Permissions::PRINTING).p());
    }

    #[test]
    fn permissions_by_name() {
        assert_eq!(Permissions::parse("PRINTING|COPY").unwrap().0, Permissions::PRINTING | Permissions::COPY);
        assert!(Permissions::parse("FLY").is_err());
        assert_eq!(Permissions(0).p(), -3904);
    }
}
