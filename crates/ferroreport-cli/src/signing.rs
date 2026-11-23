//! The key that signs PDFs (`--sign-key`, `--sign-cert`): PEM files, read with `ring`.

use std::sync::Arc;

use base64::Engine;
use ferroreport::sign::{SignatureAlgorithm, Signer};
use ferroreport::{Error, Result};
use ring::signature::{EcdsaKeyPair, KeyPair, RsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING, RSA_PKCS1_SHA256};

enum Key {
    Rsa(RsaKeyPair),
    Ecdsa(EcdsaKeyPair),
}

pub struct PemSigner {
    key: Key,
    certificates: Vec<Vec<u8>>,
    rng: ring::rand::SystemRandom,
}

impl Signer for PemSigner {
    fn certificates(&self) -> &[Vec<u8>] {
        &self.certificates
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        match self.key {
            Key::Rsa(_) => SignatureAlgorithm::RsaSha256,
            Key::Ecdsa(_) => SignatureAlgorithm::EcdsaP256Sha256,
        }
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
        match &self.key {
            Key::Rsa(key) => {
                let mut out = vec![0; key.public().modulus_len()];
                key.sign(&RSA_PKCS1_SHA256, &self.rng, message, &mut out)
                    .map_err(|_| Error::Data("the RSA key could not sign".into()))?;
                Ok(out)
            }
            Key::Ecdsa(key) => Ok(key
                .sign(&self.rng, message)
                .map_err(|_| Error::Data("the ECDSA key could not sign".into()))?
                .as_ref()
                .to_vec()),
        }
    }
}

/// The blocks of a PEM file: (label, DER).
fn pem_blocks(text: &str) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut label = None;
    let mut body = String::new();
    for line in text.lines().map(str::trim) {
        if let Some(l) = line.strip_prefix("-----BEGIN ").and_then(|l| l.strip_suffix("-----")) {
            label = Some(l.to_string());
            body.clear();
        } else if line.starts_with("-----END ") {
            if let Some(l) = label.take() {
                if let Ok(der) = base64::engine::general_purpose::STANDARD.decode(body.as_bytes()) {
                    out.push((l, der));
                }
            }
        } else if label.is_some() && !line.contains(':') {
            body.push_str(line);
        }
    }
    out
}

fn read(path: &str) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| Error::Io(format!("{path}: {e}")))
}

/// The key in `key_path` and the certificates in `cert_paths` (the signer's first; a file can
/// hold the whole chain, and the key file may hold the certificate too). A `.p12`/`.pfx` key
/// file holds both; its password comes from FERROREPORT_SIGN_PASSWORD.
pub fn load(key_path: &str, cert_paths: &[String]) -> Result<Arc<dyn Signer>> {
    let lower = key_path.to_ascii_lowercase();
    let bad = |what: &str| Error::Data(format!("{key_path}: {what}"));
    let (label, der, mut certificates) = if lower.ends_with(".p12") || lower.ends_with(".pfx") {
        let password = std::env::var("FERROREPORT_SIGN_PASSWORD").unwrap_or_default();
        let (key, certs) = pkcs12(key_path, &password)?;
        ("PRIVATE KEY".to_string(), key, certs)
    } else {
        let blocks = pem_blocks(&read(key_path)?);
        let (label, der) = blocks
            .iter()
            .find(|(l, _)| l.ends_with("PRIVATE KEY"))
            .cloned()
            .ok_or_else(|| bad("no private key (a PEM file with -----BEGIN PRIVATE KEY-----, or a .p12)"))?;
        let certs = blocks.into_iter().filter(|(l, _)| l == "CERTIFICATE").map(|(_, d)| d).collect::<Vec<_>>();
        (label, der, certs)
    };
    let der = &der;
    let rng = ring::rand::SystemRandom::new();
    let key = match label.as_str() {
        "PRIVATE KEY" => match RsaKeyPair::from_pkcs8(der) {
            Ok(k) => Key::Rsa(k),
            Err(_) => Key::Ecdsa(
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, der, &rng)
                    .map_err(|_| bad("the key is neither RSA (2048 bits or more) nor ECDSA P-256"))?,
            ),
        },
        "RSA PRIVATE KEY" => Key::Rsa(RsaKeyPair::from_der(der).map_err(|_| bad("not a usable RSA key (2048 bits or more)"))?),
        "ENCRYPTED PRIVATE KEY" => {
            return Err(bad("the key is encrypted: decrypt it (openssl pkcs8 -in key.pem -out plain.pem) or use --sign-p12"))
        }
        _ => return Err(bad("convert the key to PKCS#8: openssl pkcs8 -topk8 -nocrypt -in key.pem -out key8.pem")),
    };
    for path in cert_paths {
        certificates.extend(pem_blocks(&read(path)?).into_iter().filter(|(l, _)| l == "CERTIFICATE").map(|(_, d)| d));
    }
    if certificates.is_empty() {
        return Err(Error::Data("signing needs the signer's certificate (--sign-cert cert.pem)".into()));
    }
    // The key's public half is inside its certificate: that one goes first.
    let public = match &key {
        Key::Rsa(k) => k.public_key().as_ref().to_vec(),
        Key::Ecdsa(k) => k.public_key().as_ref().to_vec(),
    };
    let holds = |cert: &[u8]| cert.windows(public.len()).any(|w| w == public.as_slice());
    match certificates.iter().position(|c| holds(c)) {
        Some(i) => {
            let own = certificates.remove(i);
            certificates.insert(0, own);
        }
        None => return Err(Error::Data("none of the certificates is the signing key's (--sign-cert)".into())),
    }
    Ok(Arc::new(PemSigner { key, certificates, rng }))
}

/// The private key (PKCS#8) and the certificates of a PKCS#12 file: the AES and PBKDF2 that
/// OpenSSL 3 writes, and the older Triple DES and RC2 that Windows and many certificate
/// authorities still use.
fn pkcs12(path: &str, password: &str) -> Result<(Vec<u8>, Vec<Vec<u8>>)> {
    use p12::{CertBag, ContentInfo, SafeBag, SafeBagKind};
    let bytes = std::fs::read(path).map_err(|e| Error::Io(format!("{path}: {e}")))?;
    let bad = |what: &str| Error::Data(format!("{path}: {what}"));
    let pfx = p12::PFX::parse(&bytes).map_err(|_| bad("not a PKCS#12 file"))?;
    let ContentInfo::Data(auth) = &pfx.auth_safe else {
        return Err(bad("a PKCS#12 file protected with a public key is not supported"));
    };
    let contents = yasna::parse_der(auth, |r| r.collect_sequence_of(ContentInfo::parse)).map_err(|_| bad("damaged contents"))?;
    let (mut keys, mut certs) = (Vec::new(), Vec::new());
    for content in contents {
        let data = match content {
            ContentInfo::Data(d) => d,
            ContentInfo::EncryptedData(e) => {
                let info = e.encrypted_content_info;
                decrypt(path, &info.content_encryption_algorithm, &info.encrypted_content, password)?
            }
            ContentInfo::OtherContext(_) => continue,
        };
        let bags = yasna::parse_der(&data, |r| r.collect_sequence_of(SafeBag::parse)).map_err(|_| bad("damaged contents"))?;
        for bag in bags {
            match bag.bag {
                SafeBagKind::CertBag(CertBag::X509(c)) => certs.push(c),
                SafeBagKind::Pkcs8ShroudedKeyBag(k) => {
                    keys.push(decrypt(path, &k.encryption_algorithm, &k.encrypted_data, password)?)
                }
                _ => {}
            }
        }
    }
    let key = keys.into_iter().next().ok_or_else(|| bad("no private key in it"))?;
    Ok((key, certs))
}

fn decrypt(path: &str, algorithm: &p12::AlgorithmIdentifier, data: &[u8], password: &str) -> Result<Vec<u8>> {
    let wrong = || Error::Data(format!("{path}: wrong password (FERROREPORT_SIGN_PASSWORD), or a damaged file"));
    match algorithm {
        p12::AlgorithmIdentifier::OtherAlg(other) if other.algorithm_type.to_string() == "1.2.840.113549.1.5.13" => {
            pbes2(other.params.as_deref().unwrap_or_default(), data, password).ok_or_else(wrong)
        }
        p12::AlgorithmIdentifier::OtherAlg(other) => {
            Err(Error::Data(format!("{path}: encrypted with {}, which is not supported", other.algorithm_type)))
        }
        // The old PKCS#12 schemes take the password as UTF-16 with a closing zero.
        legacy => {
            let mut bmp: Vec<u8> = password.encode_utf16().flat_map(u16::to_be_bytes).collect();
            bmp.extend([0, 0]);
            legacy.decrypt_pbe(data, &bmp).ok_or_else(wrong)
        }
    }
}

/// PBES2 (RFC 8018): a key from PBKDF2, then AES or Triple DES in CBC mode.
fn pbes2(params: &[u8], data: &[u8], password: &str) -> Option<Vec<u8>> {
    use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    let (salt, iterations, prf, cipher, iv) = yasna::parse_der(params, |r| {
        r.read_sequence(|r| {
            let (salt, iterations, prf) = r.next().read_sequence(|r| {
                let _pbkdf2 = r.next().read_oid()?;
                r.next().read_sequence(|r| {
                    let salt = r.next().read_bytes()?;
                    let iterations = r.next().read_u64()?;
                    let _length = r.read_optional(|r| r.read_u64())?;
                    let prf = r.read_optional(|r| {
                        r.read_sequence(|r| {
                            let oid = r.next().read_oid()?;
                            r.read_optional(|r| r.read_null())?;
                            Ok(oid.to_string())
                        })
                    })?;
                    Ok((salt, iterations, prf))
                })
            })?;
            let (cipher, iv) = r.next().read_sequence(|r| Ok((r.next().read_oid()?.to_string(), r.next().read_bytes()?)))?;
            Ok((salt, iterations, prf, cipher, iv))
        })
    })
    .ok()?;
    if iterations > 10_000_000 {
        return None;
    }
    let key_len = match cipher.as_str() {
        "2.16.840.1.101.3.4.1.2" => 16,
        "2.16.840.1.101.3.4.1.22" | "1.2.840.113549.3.7" => 24,
        "2.16.840.1.101.3.4.1.42" => 32,
        _ => return None,
    };
    let mut key = vec![0u8; key_len];
    match prf.as_deref() {
        None | Some("1.2.840.113549.2.7") => {
            pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), &salt, iterations as u32, &mut key)
        }
        Some("1.2.840.113549.2.9") => {
            pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, iterations as u32, &mut key)
        }
        Some("1.2.840.113549.2.11") => {
            pbkdf2::pbkdf2_hmac::<sha2::Sha512>(password.as_bytes(), &salt, iterations as u32, &mut key)
        }
        _ => return None,
    }
    let mut buf = data.to_vec();
    let plain = match cipher.as_str() {
        "2.16.840.1.101.3.4.1.2" => {
            cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, &iv).ok()?.decrypt_padded_mut::<Pkcs7>(&mut buf).ok()?.to_vec()
        }
        "2.16.840.1.101.3.4.1.22" => {
            cbc::Decryptor::<aes::Aes192>::new_from_slices(&key, &iv).ok()?.decrypt_padded_mut::<Pkcs7>(&mut buf).ok()?.to_vec()
        }
        "2.16.840.1.101.3.4.1.42" => {
            cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, &iv).ok()?.decrypt_padded_mut::<Pkcs7>(&mut buf).ok()?.to_vec()
        }
        _ => {
            cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv).ok()?.decrypt_padded_mut::<Pkcs7>(&mut buf).ok()?.to_vec()
        }
    };
    Some(plain)
}
