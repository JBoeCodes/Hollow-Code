//! Apple App Attest verification for approval-key enrollment (build run
//! 11-17, row B4, ENG-172; design `docs/phone-api/approvals-design.md`, "1. Two
//! keys per phone, enrolled once").
//!
//! The phone proves that an unmodified build of the companion app, on a real
//! iPhone, made the enrollment: `DCAppAttestService.attestKey` returns a CBOR
//! attestation object, which this module checks against Apple's "Validating
//! apps that connect to your server" steps. It holds a small CBOR decoder and a
//! small DER (X.509) reader, since nothing in the lock graph parses either the
//! way this needs; `ring` verifies the chain's P-384 signatures. Apple's App
//! Attestation root CA is pinned in the binary ([`APPLE_ROOT_DER`]).
//!
//! Everything here is a pure function of its inputs (bytes in, `Result` out)
//! and never panics on hostile input: every length is checked, nesting and
//! element counts are capped, and the CBOR decoder refuses indefinite lengths.

use chrono::{NaiveDate, TimeZone, Utc};
use ring::signature::{UnparsedPublicKey, ECDSA_P384_SHA256_ASN1, ECDSA_P384_SHA384_ASN1};
use sha2::{Digest, Sha256};

/// Apple's App Attestation root CA (DER), as published at
/// `https://www.apple.com/certificateauthority/Apple_App_Attestation_Root_CA.pem`.
/// Self-signed, P-384, valid to 2045-03-15.
pub(crate) const APPLE_ROOT_DER: &[u8] = include_bytes!("apple_app_attestation_root.der");

/// SHA-256 of [`APPLE_ROOT_DER`], lower-case hex: the fingerprint to compare
/// with Apple's published certificate (a test pins it).
#[cfg(test)]
pub(crate) const APPLE_ROOT_SHA256: &str =
    "1cb9823ba28ba6ad2d33a006941de2ae4f513ef1d4e831b9f7e0fa7b6242c932";

/// The production App Attest AAGUID: `appattest` and seven zero bytes
/// (TestFlight and the App Store use production; development is
/// `appattestdevelop`).
const AAGUID_PRODUCTION: [u8; 16] = *b"appattest\0\0\0\0\0\0\0";

/// The `apple_validation_category_01` values accepted: 2 TestFlight, 4 App
/// Store, 5 enterprise or ad hoc (design decision 8; dropping 5 is the
/// stricter choice).
const CATEGORIES_ACCEPTED: [u64; 3] = [2, 4, 5];

/// How far outside a certificate's validity the clock may be (Apple's
/// credential certificates are short-lived, so this stays tight).
const CLOCK_SLACK_SECS: i64 = 3600;

const CBOR_DEPTH_MAX: usize = 12;
const CBOR_ITEMS_MAX: u64 = 256;

/// Why an attestation was refused: a short fixed reason, safe to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Reject(pub &'static str);

type Res<T> = Result<T, Reject>;

fn err<T>(why: &'static str) -> Res<T> {
    Err(Reject(why))
}

// ── CBOR (RFC 8949), the subset an attestation object uses ──────────────

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Cbor {
    Int(i128),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Cbor>),
    Map(Vec<(Cbor, Cbor)>),
    /// A float, bool, null or undefined: nothing here reads one.
    Other,
}

impl Cbor {
    fn get(&self, key: &str) -> Option<&Cbor> {
        match self {
            Cbor::Map(pairs) => pairs
                .iter()
                .find(|(k, _)| matches!(k, Cbor::Text(t) if t == key))
                .map(|(_, v)| v),
            _ => None,
        }
    }
    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Cbor::Bytes(b) => Some(b),
            _ => None,
        }
    }
}

/// Decode one CBOR item from the start of `input`: the item and the bytes it
/// took.
pub(crate) fn cbor_decode(input: &[u8]) -> Res<(Cbor, usize)> {
    let mut at = 0;
    let value = cbor_item(input, &mut at, 0)?;
    Ok((value, at))
}

fn take<'a>(input: &'a [u8], at: &mut usize, n: usize) -> Res<&'a [u8]> {
    let end = at.checked_add(n).filter(|end| *end <= input.len());
    let Some(end) = end else {
        return err("the attestation is cut short");
    };
    let slice = &input[*at..end];
    *at = end;
    Ok(slice)
}

fn cbor_item(input: &[u8], at: &mut usize, depth: usize) -> Res<Cbor> {
    if depth > CBOR_DEPTH_MAX {
        return err("the attestation nests too deeply");
    }
    let head = take(input, at, 1)?[0];
    let (major, info) = (head >> 5, head & 0x1f);
    let arg = match info {
        0..=23 => u64::from(info),
        24 => u64::from(take(input, at, 1)?[0]),
        25 => u64::from(u16::from_be_bytes(take(input, at, 2)?.try_into().unwrap())),
        26 => u64::from(u32::from_be_bytes(take(input, at, 4)?.try_into().unwrap())),
        27 => u64::from_be_bytes(take(input, at, 8)?.try_into().unwrap()),
        _ => return err("the attestation uses a CBOR form Hollow doesn't read"),
    };
    match major {
        0 => Ok(Cbor::Int(i128::from(arg))),
        1 => Ok(Cbor::Int(-1 - i128::from(arg))),
        2 | 3 => {
            let len = usize::try_from(arg).map_err(|_| Reject("a CBOR string is too long"))?;
            let bytes = take(input, at, len)?;
            if major == 2 {
                Ok(Cbor::Bytes(bytes.to_vec()))
            } else {
                let text =
                    std::str::from_utf8(bytes).map_err(|_| Reject("a CBOR text isn't UTF-8"))?;
                Ok(Cbor::Text(text.to_string()))
            }
        }
        4 => {
            if arg > CBOR_ITEMS_MAX {
                return err("a CBOR array is too long");
            }
            let mut items = Vec::new();
            for _ in 0..arg {
                items.push(cbor_item(input, at, depth + 1)?);
            }
            Ok(Cbor::Array(items))
        }
        5 => {
            if arg > CBOR_ITEMS_MAX {
                return err("a CBOR map is too long");
            }
            let mut pairs = Vec::new();
            for _ in 0..arg {
                let key = cbor_item(input, at, depth + 1)?;
                let value = cbor_item(input, at, depth + 1)?;
                pairs.push((key, value));
            }
            Ok(Cbor::Map(pairs))
        }
        // A tag wraps one item.
        6 => cbor_item(input, at, depth + 1),
        // 7: simple values and floats, which carry no payload past `arg`.
        _ => Ok(Cbor::Other),
    }
}

// ── DER (X.509), the subset a certificate chain uses ────────────────────

struct Tlv<'a> {
    tag: u8,
    content: &'a [u8],
    /// The whole element, tag and length included.
    full: &'a [u8],
}

fn der_read(input: &[u8]) -> Res<(Tlv<'_>, &[u8])> {
    if input.len() < 2 {
        return err("a certificate is cut short");
    }
    let tag = input[0];
    if tag & 0x1f == 0x1f {
        return err("a certificate uses a long tag");
    }
    let first = input[1];
    let (len, header) = if first < 0x80 {
        (usize::from(first), 2)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || input.len() < 2 + n {
            return err("a certificate length is invalid");
        }
        let mut len = 0usize;
        for b in &input[2..2 + n] {
            len = (len << 8) | usize::from(*b);
        }
        (len, 2 + n)
    };
    let end = header
        .checked_add(len)
        .filter(|end| *end <= input.len())
        .ok_or(Reject("a certificate is cut short"))?;
    Ok((
        Tlv {
            tag,
            content: &input[header..end],
            full: &input[..end],
        },
        &input[end..],
    ))
}

/// The elements of a constructed value's content.
fn der_children(content: &[u8]) -> Res<Vec<Tlv<'_>>> {
    let mut out = Vec::new();
    let mut rest = content;
    while !rest.is_empty() {
        let (tlv, next) = der_read(rest)?;
        out.push(tlv);
        rest = next;
        if out.len() > 64 {
            return err("a certificate has too many fields");
        }
    }
    Ok(out)
}

/// One element that must be the whole of `input`.
fn der_only(input: &[u8], tag: u8) -> Res<Tlv<'_>> {
    let (tlv, rest) = der_read(input)?;
    if !rest.is_empty() || tlv.tag != tag {
        return err("a certificate field is malformed");
    }
    Ok(tlv)
}

const OID_EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const OID_P384: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x22];
const OID_ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const OID_ECDSA_SHA384: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x03];
/// 1.2.840.113635.100.8.2: the App Attest nonce extension.
const OID_NONCE: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x63, 0x64, 0x08, 0x02];
const OID_BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1d, 0x13];

/// The digest a certificate's ECDSA signature uses. Apple's chain mixes them
/// (the root signs the intermediate with SHA-384; the credential certificate
/// is signed with SHA-256 by the intermediate's P-384 key), so each link's own
/// algorithm picks the verifier.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SigDigest {
    Sha256,
    Sha384,
}

fn sig_digest_of(oid: &[u8]) -> Option<SigDigest> {
    match oid {
        OID_ECDSA_SHA256 => Some(SigDigest::Sha256),
        OID_ECDSA_SHA384 => Some(SigDigest::Sha384),
        _ => None,
    }
}

struct Cert<'a> {
    /// The signed bytes (the whole `tbsCertificate` element).
    tbs: &'a [u8],
    /// The digest its signature algorithm names (`ecdsa-with-SHA256` or
    /// `-SHA384`, the same in `tbsCertificate` and outside it), else `None`.
    sig_digest: Option<SigDigest>,
    /// The ECDSA signature (DER `Ecdsa-Sig-Value`).
    sig: &'a [u8],
    issuer: &'a [u8],
    subject: &'a [u8],
    not_before: i64,
    not_after: i64,
    curve: &'a [u8],
    point: &'a [u8],
    extensions: Vec<(&'a [u8], &'a [u8])>,
}

fn der_time(tlv: &Tlv<'_>) -> Res<i64> {
    let text = std::str::from_utf8(tlv.content).map_err(|_| Reject("a certificate date is bad"))?;
    let digits = text
        .strip_suffix('Z')
        .ok_or(Reject("a certificate date is bad"))?;
    // Digits only: the byte slicing below must never cut a character, and a
    // sign or a space must not parse as a number.
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return err("a certificate date is bad");
    }
    let (year, rest) = match tlv.tag {
        0x17 if digits.len() == 12 => {
            let yy: i32 = digits[..2]
                .parse()
                .map_err(|_| Reject("a certificate date is bad"))?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, &digits[2..])
        }
        0x18 if digits.len() == 14 => (
            digits[..4]
                .parse()
                .map_err(|_| Reject("a certificate date is bad"))?,
            &digits[4..],
        ),
        _ => return err("a certificate date is bad"),
    };
    let part = |from: usize| -> Res<u32> {
        rest[from..from + 2]
            .parse()
            .map_err(|_| Reject("a certificate date is bad"))
    };
    let (month, day, hour, minute, second) = (part(0)?, part(2)?, part(4)?, part(6)?, part(8)?);
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|d| d.and_hms_opt(hour, minute, second))
        .ok_or(Reject("a certificate date is bad"))?;
    Ok(Utc.from_utc_datetime(&date).timestamp())
}

fn parse_cert(der: &[u8]) -> Res<Cert<'_>> {
    let cert = der_only(der, 0x30)?;
    let parts = der_children(cert.content)?;
    let [tbs, sig_alg, sig] = parts.as_slice() else {
        return err("a certificate isn't a certificate");
    };
    if tbs.tag != 0x30 || sig_alg.tag != 0x30 || sig.tag != 0x03 {
        return err("a certificate isn't a certificate");
    }
    // BIT STRING: the first byte counts unused bits, which a signature has none of.
    let [0, sig_bytes @ ..] = sig.content else {
        return err("a certificate signature is malformed");
    };
    let outer_alg = der_children(sig_alg.content)?;
    let outer = outer_alg
        .first()
        .filter(|o| o.tag == 0x06)
        .and_then(|o| sig_digest_of(o.content));

    let fields = der_children(tbs.content)?;
    let mut at = 0;
    // [0] EXPLICIT version, optional.
    if fields.first().is_some_and(|f| f.tag == 0xa0) {
        at += 1;
    }
    // serial, signature algorithm, issuer, validity, subject, spki.
    let get = |i: usize, tag: u8| -> Res<&Tlv<'_>> {
        fields
            .get(at + i)
            .filter(|f| f.tag == tag)
            .ok_or(Reject("a certificate is missing a field"))
    };
    get(0, 0x02)?;
    let inner_alg = der_children(get(1, 0x30)?.content)?;
    let inner = inner_alg
        .first()
        .filter(|o| o.tag == 0x06)
        .and_then(|o| sig_digest_of(o.content));
    let issuer = get(2, 0x30)?.full;
    let validity = der_children(get(3, 0x30)?.content)?;
    let [nb, na] = validity.as_slice() else {
        return err("a certificate's validity is malformed");
    };
    let subject = get(4, 0x30)?.full;
    let spki = der_children(get(5, 0x30)?.content)?;
    let [key_alg, key_bits] = spki.as_slice() else {
        return err("a certificate's key is malformed");
    };
    let key_alg = der_children(key_alg.content)?;
    let (Some(alg_oid), Some(curve)) = (key_alg.first(), key_alg.get(1)) else {
        return err("a certificate's key is malformed");
    };
    if alg_oid.tag != 0x06
        || alg_oid.content != OID_EC_PUBLIC_KEY
        || curve.tag != 0x06
        || key_bits.tag != 0x03
    {
        return err("a certificate's key isn't an EC key");
    }
    let [0, point @ ..] = key_bits.content else {
        return err("a certificate's key is malformed");
    };
    let mut extensions = Vec::new();
    for field in &fields[at + 6..] {
        if field.tag != 0xa3 {
            continue;
        }
        let list = der_only(field.content, 0x30)?;
        for ext in der_children(list.content)? {
            if ext.tag != 0x30 {
                return err("a certificate extension is malformed");
            }
            let parts = der_children(ext.content)?;
            let oid = parts.first().filter(|o| o.tag == 0x06);
            // The value is the last element; a BOOLEAN `critical` may sit between.
            let value = parts.last().filter(|v| v.tag == 0x04);
            let (Some(oid), Some(value)) = (oid, value) else {
                return err("a certificate extension is malformed");
            };
            extensions.push((oid.content, value.content));
        }
    }
    Ok(Cert {
        tbs: tbs.full,
        // Both copies of the algorithm must agree (a mismatch is a forgery tell).
        sig_digest: outer.filter(|o| Some(*o) == inner),
        sig: sig_bytes,
        issuer,
        subject,
        not_before: der_time(nb)?,
        not_after: der_time(na)?,
        curve: curve.content,
        point,
        extensions,
    })
}

impl Cert<'_> {
    fn extension(&self, oid: &[u8]) -> Option<&[u8]> {
        self.extensions
            .iter()
            .find(|(o, _)| *o == oid)
            .map(|(_, v)| *v)
    }

    fn valid_at(&self, now: i64) -> bool {
        now + CLOCK_SLACK_SECS >= self.not_before && now - CLOCK_SLACK_SECS <= self.not_after
    }

    /// `basicConstraints` says CA: true.
    fn is_ca(&self) -> bool {
        let Some(value) = self.extension(OID_BASIC_CONSTRAINTS) else {
            return false;
        };
        let Ok(seq) = der_only(value, 0x30) else {
            return false;
        };
        der_children(seq.content)
            .map(|c| {
                c.first()
                    .is_some_and(|b| b.tag == 0x01 && b.content == [0xff])
            })
            .unwrap_or(false)
    }
}

/// `child` was signed by `parent`'s P-384 key.
fn signed_by(child: &Cert<'_>, parent: &Cert<'_>) -> Res<()> {
    let Some(digest) = child.sig_digest else {
        return err("a certificate isn't signed with ECDSA and SHA-256 or SHA-384");
    };
    if child.issuer != parent.subject {
        return err("a certificate's issuer isn't the next certificate");
    }
    if parent.curve != OID_P384 || parent.point.len() != 97 || parent.point[0] != 4 {
        return err("an issuer key isn't P-384");
    }
    let algorithm = match digest {
        SigDigest::Sha256 => &ECDSA_P384_SHA256_ASN1,
        SigDigest::Sha384 => &ECDSA_P384_SHA384_ASN1,
    };
    UnparsedPublicKey::new(algorithm, parent.point)
        .verify(child.tbs, child.sig)
        .map_err(|_| Reject("a certificate's signature doesn't verify"))
}

// ── P-256 points ─────────────────────────────────────────────────────────

/// A 256-bit number, four little-endian words.
type U256 = [u64; 4];

/// p = 2^256 - 2^224 + 2^192 + 2^96 - 1.
const P256_P: U256 = [
    0xffff_ffff_ffff_ffff,
    0x0000_0000_ffff_ffff,
    0x0000_0000_0000_0000,
    0xffff_ffff_0000_0001,
];
const P256_B: U256 = [
    0x3bce_3c3e_27d2_604b,
    0x651d_06b0_cc53_b0f6,
    0xb3eb_bd55_7698_86bc,
    0x5ac6_35d8_aa3a_93e7,
];

fn u256_from_be(bytes: &[u8]) -> U256 {
    let mut out = [0u64; 4];
    for (i, chunk) in bytes.chunks(8).rev().enumerate() {
        out[i] = u64::from_be_bytes(chunk.try_into().unwrap());
    }
    out
}

fn u256_ge(a: &U256, b: &U256) -> bool {
    for i in (0..4).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

/// `a - b`, wrapping; also whether it borrowed.
fn u256_sub(a: &U256, b: &U256) -> (U256, bool) {
    let mut out = [0u64; 4];
    let mut borrow = false;
    for i in 0..4 {
        let (d1, b1) = a[i].overflowing_sub(b[i]);
        let (d2, b2) = d1.overflowing_sub(u64::from(borrow));
        out[i] = d2;
        borrow = b1 || b2;
    }
    (out, borrow)
}

/// `a + b`, wrapping; also whether it carried.
fn u256_add(a: &U256, b: &U256) -> (U256, bool) {
    let mut out = [0u64; 4];
    let mut carry = false;
    for i in 0..4 {
        let (s1, c1) = a[i].overflowing_add(b[i]);
        let (s2, c2) = s1.overflowing_add(u64::from(carry));
        out[i] = s2;
        carry = c1 || c2;
    }
    (out, carry)
}

fn add_mod(a: &U256, b: &U256) -> U256 {
    let (sum, carry) = u256_add(a, b);
    if carry || u256_ge(&sum, &P256_P) {
        u256_sub(&sum, &P256_P).0
    } else {
        sum
    }
}

fn sub_mod(a: &U256, b: &U256) -> U256 {
    let (diff, borrow) = u256_sub(a, b);
    if borrow {
        u256_add(&diff, &P256_P).0
    } else {
        diff
    }
}

/// `a * b` mod p by double and add: slow (256 steps) and enough for one point.
fn mul_mod(a: &U256, b: &U256) -> U256 {
    let mut out = [0u64; 4];
    for bit in (0..256).rev() {
        out = add_mod(&out, &out);
        if b[bit / 64] >> (bit % 64) & 1 == 1 {
            out = add_mod(&out, a);
        }
    }
    out
}

/// `point` is an X9.63 uncompressed P-256 point (`04 || x || y`) on the curve.
pub(crate) fn p256_point_valid(point: &[u8]) -> bool {
    if point.len() != 65 || point[0] != 4 {
        return false;
    }
    let x = u256_from_be(&point[1..33]);
    let y = u256_from_be(&point[33..65]);
    if u256_ge(&x, &P256_P) || u256_ge(&y, &P256_P) {
        return false;
    }
    // y^2 = x^3 - 3x + b
    let x2 = mul_mod(&x, &x);
    let x3 = mul_mod(&x2, &x);
    let three_x = add_mod(&add_mod(&x, &x), &x);
    let rhs = add_mod(&sub_mod(&x3, &three_x), &P256_B);
    mul_mod(&y, &y) == rhs
}

// ── The attestation ──────────────────────────────────────────────────────

/// What the attestation has to say for this enrollment.
pub(crate) struct Expected<'a> {
    /// `<team>.<bundle id>`, e.g. `TEAM000000.com.hollow.app.companion`.
    pub app_id: &'a str,
    /// The App Attest key id, decoded (32 bytes).
    pub key_id: &'a [u8],
    /// SHA-256 of the enrollment message the phone attested.
    pub client_data_hash: [u8; 32],
    /// Unix seconds.
    pub now: i64,
    /// The root the chain must end at ([`APPLE_ROOT_DER`] outside tests).
    pub root: &'a [u8],
}

/// Check an App Attest attestation object against `expected`, following Apple's
/// validation steps. `Ok(())` means: Apple vouched for a key with this key id,
/// made by this app, on a real device, over this exact enrollment message.
pub(crate) fn verify_attestation(object: &[u8], expected: &Expected<'_>) -> Res<()> {
    let (root_value, used) = cbor_decode(object)?;
    if used != object.len() {
        return err("the attestation has bytes after its end");
    }
    if !matches!(root_value.get("fmt"), Some(Cbor::Text(t)) if t == "apple-appattest") {
        return err("the attestation isn't an App Attest attestation");
    }
    let statement = root_value
        .get("attStmt")
        .ok_or(Reject("the attestation has no statement"))?;
    let auth_data = root_value
        .get("authData")
        .and_then(Cbor::bytes)
        .ok_or(Reject("the attestation has no authenticator data"))?;
    let chain = match statement.get("x5c") {
        Some(Cbor::Array(items)) => items,
        _ => return err("the attestation has no certificate chain"),
    };
    let [Some(leaf_der), Some(inter_der)] = [
        chain.first().and_then(Cbor::bytes),
        chain.get(1).and_then(Cbor::bytes),
    ] else {
        return err("the attestation's chain isn't a certificate and an intermediate");
    };
    if chain.len() != 2 {
        return err("the attestation's chain isn't a certificate and an intermediate");
    }

    // 1. The chain: credential certificate, intermediate, the pinned root.
    let root = parse_cert(expected.root)?;
    let inter = parse_cert(inter_der)?;
    let leaf = parse_cert(leaf_der)?;
    signed_by(&leaf, &inter)?;
    signed_by(&inter, &root)?;
    if !inter.is_ca() {
        return err("the intermediate certificate isn't a CA");
    }
    if !inter.valid_at(expected.now) || !leaf.valid_at(expected.now) {
        return err("a certificate in the chain isn't valid now");
    }
    if leaf.curve != OID_P256 || leaf.point.len() != 65 || leaf.point[0] != 4 {
        return err("the credential key isn't a P-256 key");
    }

    // 2. The nonce: SHA-256(authData || clientDataHash) is in the credential
    // certificate's extension 1.2.840.113635.100.8.2.
    let mut hasher = Sha256::new();
    hasher.update(auth_data);
    hasher.update(expected.client_data_hash);
    let nonce = hasher.finalize();
    let extension = leaf
        .extension(OID_NONCE)
        .ok_or(Reject("the credential certificate has no nonce"))?;
    if nonce_in(extension)? != nonce.as_slice() {
        return err("the attestation was made for another enrollment");
    }

    // 3. The key id is SHA-256 of the credential public key.
    if Sha256::digest(leaf.point).as_slice() != expected.key_id {
        return err("the attestation is for another key");
    }

    // 4. The authenticator data.
    let auth = parse_auth_data(auth_data)?;
    if auth.rp_id_hash != Sha256::digest(expected.app_id.as_bytes()).as_slice() {
        return err("the attestation is for another app");
    }
    if auth.counter != 0 {
        return err("the attestation's counter isn't zero");
    }
    if auth.aaguid != AAGUID_PRODUCTION {
        return err("the attestation isn't from the production App Attest");
    }
    if auth.credential_id != expected.key_id {
        return err("the attestation's credential isn't the key");
    }
    if let Some(category) = auth.category {
        if !CATEGORIES_ACCEPTED.contains(&category) {
            return err("the app build isn't a TestFlight, App Store or same-team build");
        }
    }
    Ok(())
}

/// The nonce inside the extension's value: a DER sequence holding one octet
/// string, which Apple wraps in a `[1]` context tag.
fn nonce_in(value: &[u8]) -> Res<&[u8]> {
    let seq = der_only(value, 0x30)?;
    let items = der_children(seq.content)?;
    let [item] = items.as_slice() else {
        return err("the nonce extension is malformed");
    };
    let octets = if item.tag == 0xa1 {
        der_only(item.content, 0x04)?
    } else if item.tag == 0x04 {
        der_only(item.full, 0x04)?
    } else {
        return err("the nonce extension is malformed");
    };
    Ok(octets.content)
}

struct AuthData<'a> {
    rp_id_hash: &'a [u8],
    counter: u32,
    aaguid: &'a [u8],
    credential_id: &'a [u8],
    /// `apple_validation_category_01`, when the extensions carry it.
    category: Option<u64>,
}

fn parse_auth_data(data: &[u8]) -> Res<AuthData<'_>> {
    const ATTESTED: u8 = 0x40;
    const EXTENSIONS: u8 = 0x80;
    let mut at = 0;
    let rp_id_hash = take(data, &mut at, 32)?;
    let flags = take(data, &mut at, 1)?[0];
    let counter = u32::from_be_bytes(take(data, &mut at, 4)?.try_into().unwrap());
    if flags & ATTESTED == 0 {
        return err("the attestation carries no credential data");
    }
    let aaguid = take(data, &mut at, 16)?;
    let id_len = usize::from(u16::from_be_bytes(
        take(data, &mut at, 2)?.try_into().unwrap(),
    ));
    let credential_id = take(data, &mut at, id_len)?;
    // The credential public key (COSE): read only to find where it ends.
    let (_, used) = cbor_decode(&data[at..])?;
    at += used;
    let category = if at < data.len() {
        if flags & EXTENSIONS == 0 {
            return err("the authenticator data has bytes it doesn't explain");
        }
        let (extensions, used) = cbor_decode(&data[at..])?;
        if at + used != data.len() {
            return err("the authenticator data has bytes after its extensions");
        }
        match extensions.get("apple_validation_category_01") {
            None => None,
            Some(Cbor::Int(n)) => {
                Some(u64::try_from(*n).map_err(|_| Reject("the build category is invalid"))?)
            }
            Some(_) => return err("the build category is invalid"),
        }
    } else {
        // iOS before 27 adds no extensions; the nonce binds these bytes, so
        // they can't have been stripped afterwards.
        None
    };
    Ok(AuthData {
        rp_id_hash,
        counter,
        aaguid,
        credential_id,
        category,
    })
}

/// Tests: the pinned root is a self-signed P-384 certificate valid at `now`.
#[cfg(test)]
pub(crate) fn root_self_check(root: &[u8], now: i64) -> bool {
    let Ok(cert) = parse_cert(root) else {
        return false;
    };
    signed_by(&cert, &cert).is_ok() && cert.is_ca() && cert.valid_at(now)
}
