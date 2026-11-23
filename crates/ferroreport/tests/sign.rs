//! PDF signatures (PAdES B-B): the byte range, the digest the signature covers, the
//! certificate. The cryptography itself is checked in CI with pdfsig and pyHanko on documents
//! signed with real keys (tools/check_signatures.py).

use ferroreport::sign::{Signature, SignatureAlgorithm, Signer};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

/// Signs nothing: remembers what it was asked to sign, and answers 256 bytes of 0xAB.
struct Fake {
    certs: Vec<Vec<u8>>,
    signed: Mutex<Vec<u8>>,
}

impl Signer for Fake {
    fn certificates(&self) -> &[Vec<u8>] {
        &self.certs
    }
    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::RsaSha256
    }
    fn sign(&self, message: &[u8]) -> ferroreport::Result<Vec<u8>> {
        *self.signed.lock().unwrap() = message.to_vec();
        Ok(vec![0xAB; 256])
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn a_signed_pdf_covers_everything_but_its_signature() {
    let cert = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/test-signer.der")).unwrap();
    let fake = Arc::new(Fake { certs: vec![cert.clone()], signed: Mutex::new(Vec::new()) });
    let template = r#"<jasperReport name="s" pageWidth="200" pageHeight="100" columnWidth="200" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0">
        <title><band height="20"><staticText><reportElement x="0" y="0" width="200" height="20"/><text><![CDATA[Signed]]></text></staticText></band></title>
    </jasperReport>"#;
    let signature = Signature {
        signer: fake.clone(),
        name: Some("Cristian".into()),
        reason: Some("Aprobado (revisión)".into()),
        location: None,
        contact: None,
    };
    let mut renderer = ferroreport::Renderer::new().sign(Some(signature));
    let pdf =
        renderer.render_json_as(template, &serde_json::json!([]), &serde_json::json!({}), ferroreport::Format::Pdf).unwrap();

    // The byte range: from the start to the signature, and from after it to the end.
    let at = find(&pdf, b"/ByteRange [").unwrap();
    let close = at + find(&pdf[at..], b"]").unwrap();
    let range: Vec<usize> =
        std::str::from_utf8(&pdf[at + 12..close]).unwrap().split_whitespace().map(|n| n.parse().unwrap()).collect();
    assert_eq!(range[0], 0);
    assert_eq!(range[2] + range[3], pdf.len());
    assert_eq!(pdf[range[1]], b'<');
    assert_eq!(pdf[range[2] - 1], b'>');

    // The signature: a CMS holding the certificate, whose signed attributes hold the digest of
    // those two ranges.
    let hex = std::str::from_utf8(&pdf[range[1] + 1..range[2] - 1]).unwrap().trim_end_matches('0');
    let cms: Vec<u8> = (0..hex.len() / 2 * 2).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    assert_eq!(cms[0], 0x30);
    assert!(find(&cms, &cert).is_some(), "the certificate is in the signature");
    assert!(find(&cms, &[0xAB; 256]).is_some(), "the signature value is in it");
    let mut digest = Sha256::new();
    digest.update(&pdf[..range[1]]);
    digest.update(&pdf[range[2]..]);
    let signed = fake.signed.lock().unwrap().clone();
    assert!(find(&signed, &digest.finalize()).is_some(), "the signed attributes hold the document's digest");
    assert!(find(&signed, &Sha256::digest(&cert)).is_some(), "and the certificate's (signing-certificate-v2)");
    assert!(find(&pdf, b"/SubFilter /ETSI.CAdES.detached").is_some());
    assert!(find(&pdf, b"/SigFlags 3").is_some());
}

#[test]
fn an_encrypted_pdf_is_not_signed() {
    let cert = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/test-signer.der")).unwrap();
    let fake = Arc::new(Fake { certs: vec![cert], signed: Mutex::new(Vec::new()) });
    let signature = Signature { signer: fake, name: None, reason: None, location: None, contact: None };
    let encryption = ferroreport::crypt::Encryption {
        user_password: String::new(),
        owner_password: Some("x".into()),
        permissions: Default::default(),
    };
    let mut renderer = ferroreport::Renderer::new().sign(Some(signature)).encrypt(Some(encryption));
    let template = r#"<jasperReport name="s" pageWidth="200" pageHeight="100" columnWidth="200" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0"/>"#;
    assert!(renderer.render_json_as(template, &serde_json::json!([]), &serde_json::json!({}), ferroreport::Format::Pdf).is_err());
}
