"""Checks every signed PDF in a folder: poppler's pdfsig must find the signature valid and
covering the whole document, and pyHanko must judge it valid with the given certificate as
the trust anchor, with exactly the signed attributes of PAdES baseline B-B.

Usage: check_signatures.py <folder> <signer.crt>"""
import subprocess
import sys
from pathlib import Path

from asn1crypto import pem, x509
from pyhanko.pdf_utils.reader import PdfFileReader
from pyhanko.sign.validation import validate_pdf_signature
from pyhanko_certvalidator import ValidationContext

folder, cert_file = Path(sys.argv[1]), sys.argv[2]
cert = x509.Certificate.load(pem.unarmor(open(cert_file, "rb").read())[2])
bad = []
files = sorted(folder.glob("*.pdf"))
for pdf in files:
    out = subprocess.run(["pdfsig", str(pdf)], capture_output=True, text=True).stdout
    if "Signature is Valid" not in out or "Total document signed" not in out:
        bad.append(f"{pdf.name}: pdfsig: {out.strip()[-300:]}")
        continue
    with open(pdf, "rb") as f:
        sig = PdfFileReader(f).embedded_signatures[0]
        attrs = sorted(a["type"].native for a in sig.signer_info["signed_attrs"])
        status = validate_pdf_signature(sig, ValidationContext(trust_roots=[cert]))
    if not status.bottom_line or attrs != ["content_type", "message_digest", "signing_certificate_v2"]:
        bad.append(f"{pdf.name}: pyHanko: valid={status.bottom_line}, attributes {attrs}")
for b in bad:
    print(b)
print(f"{len(files) - len(bad)}/{len(files)} signed PDFs verify")
sys.exit(1 if bad or not files else 0)
