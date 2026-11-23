//! Digital signatures on PDF documents, PAdES baseline B-B (ETSI EN 319 142-1): a CMS
//! SignedData over the whole file but the signature itself (`/SubFilter /ETSI.CAdES.detached`),
//! with the signer's certificate and its digest in the signed attributes, as Adobe Reader,
//! EU validators and `pdfsig` expect.
//!
//! The private key stays with the caller: a [`Signer`] signs the bytes it is given, so this
//! crate needs no cryptography library (the command line uses `ring`; an application can use
//! a hardware token or a signing service).

use crate::error::{Error, Result};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Space kept in the PDF for the signature (the CMS with the certificates), in bytes.
pub const RESERVED: usize = 24 * 1024;

/// The placeholder the PDF writer leaves for the byte range; [`apply`] fills it in.
pub(crate) const BYTE_RANGE_PLACEHOLDER: &str = "/ByteRange [0 0000000000 0000000000 0000000000]";

/// How a [`Signer`] signs: always over SHA-256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureAlgorithm {
    /// RSA, PKCS#1 v1.5 padding.
    RsaSha256,
    /// ECDSA on P-256, the signature DER-encoded.
    EcdsaP256Sha256,
}

/// What signs a document: a key and its certificates.
pub trait Signer: Send + Sync {
    /// The signer's certificate first, then the certificates that issued it (DER).
    fn certificates(&self) -> &[Vec<u8>];
    fn algorithm(&self) -> SignatureAlgorithm;
    /// Signs `message` (hashing it with SHA-256 first, as the algorithm says).
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>>;
}

/// A signature to put on a PDF.
#[derive(Clone)]
pub struct Signature {
    pub signer: Arc<dyn Signer>,
    /// Who signs (`/Name`), why (`/Reason`), where (`/Location`), how to reach them.
    pub name: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub contact: Option<String>,
}

impl std::fmt::Debug for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signature")
            .field("algorithm", &self.signer.algorithm())
            .field("certificates", &self.signer.certificates().len())
            .field("name", &self.name)
            .field("reason", &self.reason)
            .field("location", &self.location)
            .finish()
    }
}

/// Fills in the byte range of a PDF written with a signature placeholder, and signs it.
pub fn apply(pdf: &mut [u8], signature: &Signature) -> Result<()> {
    let marker = BYTE_RANGE_PLACEHOLDER.as_bytes();
    let at = find(pdf, marker, 0).ok_or_else(|| Error::Template("the PDF has no signature placeholder".into()))?;
    if find(pdf, marker, at + 1).is_some() {
        return Err(Error::Template("the PDF has more than one signature placeholder".into()));
    }
    let open = find(pdf, b"/Contents <", at).ok_or_else(|| Error::Template("the signature has no /Contents".into()))? + 10;
    let close = open + 1 + RESERVED * 2;
    if pdf.get(close) != Some(&b'>') {
        return Err(Error::Template("the signature's /Contents is not where it should be".into()));
    }
    let (a, b) = (open, close + 1);
    let range = format!("/ByteRange [0 {:<10} {:<10} {:<10}]", a, b, pdf.len() - b);
    if range.len() != marker.len() {
        return Err(Error::Limit("a signed PDF can be at most 9,999,999,999 bytes".into()));
    }
    pdf[at..at + marker.len()].copy_from_slice(range.as_bytes());
    let mut hash = Sha256::new();
    hash.update(&pdf[..a]);
    hash.update(&pdf[b..]);
    let cms = signed_data(&hash.finalize(), signature.signer.as_ref())?;
    if cms.len() > RESERVED {
        return Err(Error::Limit(format!(
            "the signature takes {} bytes, more than the {RESERVED} kept for it (too many certificates?)",
            cms.len()
        )));
    }
    let hex: String = cms.iter().map(|b| format!("{b:02X}")).collect();
    pdf[a + 1..a + 1 + hex.len()].copy_from_slice(hex.as_bytes());
    Ok(())
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

// --- DER ---------------------------------------------------------------------------------

fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = content.len();
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes: Vec<u8> = n.to_be_bytes().iter().copied().skip_while(|b| *b == 0).collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    der(0x30, &parts.concat())
}

/// A SET OF, its members sorted by their encoding as DER requires.
fn set(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut sorted = parts.to_vec();
    sorted.sort();
    der(0x31, &sorted.concat())
}

fn oid(dotted: &str) -> Vec<u8> {
    let arcs: Vec<u64> = dotted.split('.').filter_map(|a| a.parse().ok()).collect();
    let mut body = vec![(arcs[0] * 40 + arcs[1]) as u8];
    for &arc in &arcs[2..] {
        let mut chunk = vec![(arc & 0x7f) as u8];
        let mut rest = arc >> 7;
        while rest > 0 {
            chunk.push(0x80 | (rest & 0x7f) as u8);
            rest >>= 7;
        }
        chunk.reverse();
        body.extend(chunk);
    }
    der(0x06, &body)
}

fn int(n: u8) -> Vec<u8> {
    der(0x02, &[n])
}

fn octets(b: &[u8]) -> Vec<u8> {
    der(0x04, b)
}

/// One DER element at the start of `data`: (tag, the whole element, its content).
fn element(data: &[u8]) -> Result<(u8, &[u8], &[u8])> {
    let bad = || Error::Data("the certificate is not valid DER".into());
    let tag = *data.first().ok_or_else(bad)?;
    let first = *data.get(1).ok_or_else(bad)?;
    let (len, head) = if first < 0x80 {
        (first as usize, 2)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 {
            return Err(bad());
        }
        let bytes = data.get(2..2 + n).ok_or_else(bad)?;
        (bytes.iter().fold(0usize, |acc, b| (acc << 8) | *b as usize), 2 + n)
    };
    let whole = data.get(..head + len).ok_or_else(bad)?;
    Ok((tag, whole, &whole[head..]))
}

/// The issuer and the serial number of a certificate, as DER elements.
fn issuer_and_serial(cert: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let (_, _, certificate) = element(cert)?;
    let (_, _, tbs) = element(certificate)?;
    let mut rest = tbs;
    let mut next = || -> Result<(u8, Vec<u8>)> {
        let (tag, whole, _) = element(rest)?;
        rest = &rest[whole.len()..];
        Ok((tag, whole.to_vec()))
    };
    let (mut tag, mut item) = next()?;
    if tag == 0xA0 {
        (tag, item) = next()?; // the version
    }
    if tag != 0x02 {
        return Err(Error::Data("the certificate has no serial number".into()));
    }
    let serial = item;
    let _algorithm = next()?;
    let (_, issuer) = next()?;
    Ok((issuer, serial))
}

const SHA256: &str = "2.16.840.1.101.3.4.2.1";
const DATA: &str = "1.2.840.113549.1.7.1";

/// The CMS SignedData of a PAdES B-B signature over `digest`.
pub(crate) fn signed_data(digest: &[u8], signer: &dyn Signer) -> Result<Vec<u8>> {
    let certs = signer.certificates();
    let cert = certs.first().ok_or_else(|| Error::Data("a signature needs the signer's certificate".into()))?;
    let (issuer, serial) = issuer_and_serial(cert)?;
    let sha256 = seq(&[oid(SHA256)]);
    let attribute = |id: &str, value: Vec<u8>| seq(&[oid(id), set(&[value])]);
    let attributes = vec![
        attribute("1.2.840.113549.1.9.3", oid(DATA)),
        attribute("1.2.840.113549.1.9.4", octets(digest)),
        // signing-certificate-v2: the certificate's SHA-256, which binds it to the signature.
        attribute("1.2.840.113549.1.9.16.2.47", seq(&[seq(&[seq(&[octets(&Sha256::digest(cert))])])])),
    ];
    // What is signed is the attributes as a SET; in the SignerInfo they are [0] IMPLICIT.
    let signed = set(&attributes);
    let signature = signer.sign(&signed)?;
    let mut tagged = signed.clone();
    tagged[0] = 0xA0;
    let algorithm = match signer.algorithm() {
        SignatureAlgorithm::RsaSha256 => seq(&[oid("1.2.840.113549.1.1.11"), der(0x05, &[])]),
        SignatureAlgorithm::EcdsaP256Sha256 => seq(&[oid("1.2.840.10045.4.3.2")]),
    };
    let signer_info = seq(&[int(1), seq(&[issuer, serial]), sha256.clone(), tagged, algorithm, octets(&signature)]);
    let signed_data = seq(&[int(1), set(&[sha256]), seq(&[oid(DATA)]), der(0xA0, &certs.concat()), set(&[signer_info])]);
    Ok(seq(&[oid("1.2.840.113549.1.7.2"), der(0xA0, &signed_data)]))
}

/// The text of a PDF string, escaped (`(`, `)` and `\`), Latin-1 or UTF-16.
pub(crate) fn pdf_text(s: &str) -> String {
    if s.chars().all(|c| (c as u32) < 128) {
        let escaped: String =
            s.chars().flat_map(|c| if matches!(c, '(' | ')' | '\\') { vec!['\\', c] } else { vec![c] }).collect();
        format!("({escaped})")
    } else {
        let mut hex = String::from("<FEFF");
        for u in s.encode_utf16() {
            hex.push_str(&format!("{u:04X}"));
        }
        hex.push('>');
        hex
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_object_identifiers() {
        assert_eq!(oid("1.2.840.113549.1.7.2"), vec![0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x02]);
        assert_eq!(oid(SHA256), vec![0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01]);
    }

    #[test]
    fn long_lengths() {
        let big = der(0x04, &[0u8; 300]);
        assert_eq!(&big[..4], &[0x04, 0x82, 0x01, 0x2C]);
        let (tag, whole, content) = element(&big).unwrap();
        assert_eq!((tag, whole.len(), content.len()), (0x04, 304, 300));
    }
}
