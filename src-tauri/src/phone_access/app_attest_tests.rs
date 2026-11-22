//! App Attest verification (`app_attest.rs`) against attestation objects the
//! tests build themselves: a private root, an intermediate and a credential
//! certificate, signed with `ring`, then a CBOR object around them. Apple's
//! real root can't sign test objects, so the verifier takes the root as an
//! argument; the pinned one is checked for what it is.
use crate::phone_access::app_attest::{self, Cbor, Expected};
use ring::rand::SystemRandom;
use ring::signature::{
    EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING, ECDSA_P384_SHA384_ASN1_SIGNING,
};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

// ── DER and CBOR writers ─────────────────────────────────────────────────

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match content.len() {
        n if n < 0x80 => out.push(n as u8),
        n if n < 0x100 => out.extend([0x81, n as u8]),
        n => out.extend([0x82, (n >> 8) as u8, n as u8]),
    }
    out.extend(content);
    out
}

fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

fn oid(bytes: &[u8]) -> Vec<u8> {
    tlv(0x06, bytes)
}

const OID_ECDSA_SHA384: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x03];
const OID_EC: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const OID_P384: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x22];
const OID_NONCE: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x63, 0x64, 0x08, 0x02];
const OID_BASIC: &[u8] = &[0x55, 0x1d, 0x13];

fn name(cn: &str) -> Vec<u8> {
    let attr = seq(&[oid(&[0x55, 0x04, 0x03]), tlv(0x0c, cn.as_bytes())]);
    seq(&[tlv(0x31, &attr)])
}

fn utc(secs: i64) -> Vec<u8> {
    let at = chrono::DateTime::from_timestamp(secs, 0).unwrap();
    tlv(0x17, at.format("%y%m%d%H%M%SZ").to_string().as_bytes())
}

fn extension(id: &[u8], value: &[u8]) -> Vec<u8> {
    seq(&[oid(id), tlv(0x04, value)])
}

struct Signer {
    pair: EcdsaKeyPair,
    cn: &'static str,
}

fn p384(cn: &'static str) -> Signer {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P384_SHA384_ASN1_SIGNING, &rng).unwrap();
    let pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P384_SHA384_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    Signer { pair, cn }
}

/// A certificate for `subject_point` (on `curve`), signed by `signer`.
fn cert(
    issuer: &Signer,
    subject_cn: &str,
    curve: &[u8],
    subject_point: &[u8],
    extensions: Vec<Vec<u8>>,
    window: (i64, i64),
) -> Vec<u8> {
    cert_with_times(
        issuer,
        subject_cn,
        curve,
        subject_point,
        extensions,
        (utc(window.0), utc(window.1)),
    )
}

/// [`cert`] with the two validity elements given as written (a test can make
/// them hostile).
fn cert_with_times(
    issuer: &Signer,
    subject_cn: &str,
    curve: &[u8],
    subject_point: &[u8],
    extensions: Vec<Vec<u8>>,
    times: (Vec<u8>, Vec<u8>),
) -> Vec<u8> {
    let alg = seq(&[oid(OID_ECDSA_SHA384)]);
    let key_bits = [&[0u8][..], subject_point].concat();
    let spki = seq(&[seq(&[oid(OID_EC), oid(curve)]), tlv(0x03, &key_bits)]);
    let tbs = seq(&[
        tlv(0xa0, &tlv(0x02, &[2])),
        tlv(0x02, &[1]),
        alg.clone(),
        name(issuer.cn),
        seq(&[times.0, times.1]),
        name(subject_cn),
        spki,
        tlv(0xa3, &seq(&extensions)),
    ]);
    let rng = SystemRandom::new();
    let sig = issuer.pair.sign(&rng, &tbs).unwrap();
    let sig_bits = [&[0u8][..], sig.as_ref()].concat();
    seq(&[tbs, alg, tlv(0x03, &sig_bits)])
}

fn ca_extension(is_ca: bool) -> Vec<u8> {
    let flag = if is_ca { 0xff } else { 0x00 };
    extension(OID_BASIC, &seq(&[tlv(0x01, &[flag])]))
}

fn head(major: u8, n: u64) -> Vec<u8> {
    let m = major << 5;
    match n {
        0..=23 => vec![m | n as u8],
        24..=255 => vec![m | 24, n as u8],
        256..=65535 => vec![m | 25, (n >> 8) as u8, n as u8],
        _ => {
            let mut out = vec![m | 26];
            out.extend((n as u32).to_be_bytes());
            out
        }
    }
}

fn c_text(s: &str) -> Vec<u8> {
    [head(3, s.len() as u64), s.as_bytes().to_vec()].concat()
}
fn c_bytes(b: &[u8]) -> Vec<u8> {
    [head(2, b.len() as u64), b.to_vec()].concat()
}
fn c_uint(n: u64) -> Vec<u8> {
    head(0, n)
}
fn c_nint(n: u64) -> Vec<u8> {
    head(1, n - 1)
}
fn c_map(pairs: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = head(5, pairs.len() as u64);
    for (k, v) in pairs {
        out.extend(k);
        out.extend(v);
    }
    out
}
fn c_array(items: &[Vec<u8>]) -> Vec<u8> {
    [head(4, items.len() as u64), items.concat()].concat()
}

// ── The test PKI and the object builder ──────────────────────────────────

pub(super) struct Pki {
    _root: Signer,
    inter: Signer,
    /// The root and intermediate certificates (DER).
    pub root_der: Vec<u8>,
    inter_der: Vec<u8>,
}

fn build_pki(inter_is_ca: bool) -> Pki {
    build_pki_with(inter_is_ca, (-3600, 86_400 * 30))
}

/// A PKI whose intermediate is valid from/to these offsets (seconds from now).
fn build_pki_with(inter_is_ca: bool, inter_window: (i64, i64)) -> Pki {
    let now = chrono::Utc::now().timestamp();
    let window = (now - 3600, now + 86_400 * 30);
    let inter_window = (now + inter_window.0, now + inter_window.1);
    let root = p384("Test Attestation Root");
    let inter = p384("Test Attestation Intermediate");
    let root_der = cert(
        &root,
        root.cn,
        OID_P384,
        root.pair.public_key().as_ref(),
        vec![ca_extension(true)],
        window,
    );
    let inter_der = cert(
        &root,
        inter.cn,
        OID_P384,
        inter.pair.public_key().as_ref(),
        vec![ca_extension(inter_is_ca)],
        inter_window,
    );
    Pki {
        _root: root,
        inter,
        root_der,
        inter_der,
    }
}

/// One shared PKI for the whole test process.
pub(super) fn pki() -> &'static Pki {
    static PKI: OnceLock<Pki> = OnceLock::new();
    PKI.get_or_init(|| build_pki(true))
}

/// The shared root, leaked once, for a `Ctx`'s `attest_root`.
pub(super) fn fixture_root() -> &'static [u8] {
    static ROOT: OnceLock<&'static [u8]> = OnceLock::new();
    ROOT.get_or_init(|| Box::leak(pki().root_der.clone().into_boxed_slice()))
}

pub(super) const APP_ID: &str = "TEAM000000.com.hollow.app.companion";

#[derive(Clone)]
pub(super) struct Opts {
    pub app_id: String,
    pub counter: u32,
    pub aaguid: [u8; 16],
    /// `None`: no extensions at all (iOS before 27).
    pub category: Option<u64>,
    /// The message the nonce is computed for, when it isn't the enrolled one.
    pub nonce_for: Option<String>,
    /// The nonce wrapped in `[1]` as Apple does; false puts the octet string bare.
    pub wrapped_nonce: bool,
    /// A chain longer than a credential certificate and an intermediate.
    pub extra_cert: bool,
    /// Leaf validity as (from, to) offsets from now, seconds.
    pub leaf_window: (i64, i64),
    /// Flip a bit of the leaf's signature.
    pub corrupt_leaf: bool,
    /// An extension byte the flags don't declare.
    pub undeclared_extensions: bool,
    /// The leaf's `notBefore` written as these raw `UTCTime` bytes.
    pub raw_not_before: Option<Vec<u8>>,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            app_id: APP_ID.into(),
            counter: 0,
            aaguid: *b"appattest\0\0\0\0\0\0\0",
            category: Some(2),
            nonce_for: None,
            wrapped_nonce: true,
            extra_cert: false,
            leaf_window: (-3600, 86_400),
            corrupt_leaf: false,
            undeclared_extensions: false,
            raw_not_before: None,
        }
    }
}

/// An attestation object for `message` and its standard-base64 key id.
pub(super) fn attest(pki: &Pki, message: &str, o: &Opts) -> (Vec<u8>, String) {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let leaf_key =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let point = leaf_key.public_key().as_ref().to_vec();
    let key_id = Sha256::digest(&point).to_vec();

    let cose = c_map(&[
        (c_uint(1), c_uint(2)),
        (c_uint(3), c_nint(7)),
        (c_nint(1), c_uint(1)),
        (c_nint(2), c_bytes(&point[1..33])),
        (c_nint(3), c_bytes(&point[33..])),
    ]);
    let mut auth = Sha256::digest(o.app_id.as_bytes()).to_vec();
    let extensions = o
        .category
        .map(|c| c_map(&[(c_text("apple_validation_category_01"), c_uint(c))]));
    auth.push(0x40 | if extensions.is_some() { 0x80 } else { 0 });
    auth.extend(o.counter.to_be_bytes());
    auth.extend(o.aaguid);
    auth.extend((key_id.len() as u16).to_be_bytes());
    auth.extend(&key_id);
    auth.extend(&cose);
    if let Some(ext) = &extensions {
        auth.extend(ext);
    }
    if o.undeclared_extensions {
        // Same bytes, flags without the extension bit.
        auth[32] &= !0x80;
        if extensions.is_none() {
            auth.extend(c_map(&[]));
        }
    }

    let hash_of = |m: &str| -> Vec<u8> { Sha256::digest(m.as_bytes()).to_vec() };
    let for_message = o.nonce_for.as_deref().unwrap_or(message);
    let nonce = Sha256::digest([auth.clone(), hash_of(for_message)].concat()).to_vec();
    let octets = tlv(0x04, &nonce);
    let nonce_value = seq(&[if o.wrapped_nonce {
        tlv(0xa1, &octets)
    } else {
        octets
    }]);
    let now = chrono::Utc::now().timestamp();
    let from = o
        .raw_not_before
        .as_ref()
        .map_or_else(|| utc(now + o.leaf_window.0), |raw| tlv(0x17, raw));
    let mut leaf = cert_with_times(
        &pki.inter,
        "Test Credential",
        OID_P256,
        &point,
        vec![extension(OID_NONCE, &nonce_value)],
        (from, utc(now + o.leaf_window.1)),
    );
    if o.corrupt_leaf {
        let last = leaf.len() - 1;
        leaf[last] ^= 0x01;
    }
    let mut chain = vec![c_bytes(&leaf), c_bytes(&pki.inter_der)];
    if o.extra_cert {
        chain.push(c_bytes(&pki.root_der));
    }
    let object = c_map(&[
        (c_text("fmt"), c_text("apple-appattest")),
        (
            c_text("attStmt"),
            c_map(&[
                (c_text("x5c"), c_array(&chain)),
                (c_text("receipt"), c_bytes(b"receipt")),
            ]),
        ),
        (c_text("authData"), c_bytes(&auth)),
    ]);
    (object, base64_std(&key_id))
}

fn base64_std(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn check(
    object: &[u8],
    key_id_b64: &str,
    message: &str,
    root: &[u8],
    app_id: &str,
) -> Result<(), &'static str> {
    use base64::Engine as _;
    let key_id = base64::engine::general_purpose::STANDARD
        .decode(key_id_b64)
        .unwrap();
    app_attest::verify_attestation(
        object,
        &Expected {
            app_id,
            key_id: &key_id,
            client_data_hash: Sha256::digest(message.as_bytes()).into(),
            now: chrono::Utc::now().timestamp(),
            root,
        },
    )
    .map_err(|r| r.0)
}

const MESSAGE: &str = "hollow-approval-key-v1\nph-0a1b2c3d\nAPPROVE\nDENY\nCHALLENGE";

fn ok_with(o: &Opts) -> Result<(), &'static str> {
    let pki = pki();
    let (object, key_id) = attest(pki, MESSAGE, o);
    check(&object, &key_id, MESSAGE, &pki.root_der, APP_ID)
}

// ── The attestation ──────────────────────────────────────────────────────

#[test]
fn a_genuine_attestation_passes() {
    assert_eq!(ok_with(&Opts::default()), Ok(()));
}

#[test]
fn one_without_the_extensions_passes_as_ios_before_27_makes_it() {
    let o = Opts {
        category: None,
        ..Opts::default()
    };
    assert_eq!(ok_with(&o), Ok(()));
}

#[test]
fn the_nonce_may_be_wrapped_or_bare() {
    for wrapped_nonce in [true, false] {
        let o = Opts {
            wrapped_nonce,
            ..Opts::default()
        };
        assert_eq!(ok_with(&o), Ok(()), "wrapped: {wrapped_nonce}");
    }
}

#[test]
fn only_categories_2_4_and_5_pass() {
    for (category, pass) in [
        (2, true),
        (4, true),
        (5, true),
        (0, false),
        (1, false),
        (3, false),
        (6, false),
        (99, false),
    ] {
        let o = Opts {
            category: Some(category),
            ..Opts::default()
        };
        assert_eq!(ok_with(&o).is_ok(), pass, "category {category}");
    }
}

#[test]
fn an_attestation_for_another_app_is_refused() {
    let o = Opts {
        app_id: "ABCDE12345.com.example.other".into(),
        ..Opts::default()
    };
    assert!(ok_with(&o).unwrap_err().contains("another app"));
}

#[test]
fn a_nonce_for_another_enrollment_is_refused() {
    let o = Opts {
        nonce_for: Some("a different message".into()),
        ..Opts::default()
    };
    assert!(ok_with(&o).unwrap_err().contains("another enrollment"));
    // And the right object checked against a different message.
    let pki = pki();
    let (object, key_id) = attest(pki, MESSAGE, &Opts::default());
    let err = check(&object, &key_id, "other message", &pki.root_der, APP_ID).unwrap_err();
    assert!(err.contains("another enrollment"), "{err}");
}

#[test]
fn a_counter_that_is_not_zero_or_the_development_aaguid_is_refused() {
    let counted = Opts {
        counter: 1,
        ..Opts::default()
    };
    assert!(ok_with(&counted).unwrap_err().contains("counter"));
    let develop = Opts {
        aaguid: *b"appattestdevelop",
        ..Opts::default()
    };
    assert!(ok_with(&develop).unwrap_err().contains("production"));
}

#[test]
fn another_key_id_is_refused() {
    let pki = pki();
    let (object, _) = attest(pki, MESSAGE, &Opts::default());
    let other = base64_std(&Sha256::digest(b"another key"));
    let err = check(&object, &other, MESSAGE, &pki.root_der, APP_ID).unwrap_err();
    assert!(err.contains("another key"), "{err}");
}

#[test]
fn a_chain_that_does_not_end_at_the_pinned_root_is_refused() {
    let pki = pki();
    let (object, key_id) = attest(pki, MESSAGE, &Opts::default());
    let stranger = build_pki(true);
    let err = check(&object, &key_id, MESSAGE, &stranger.root_der, APP_ID).unwrap_err();
    assert!(err.contains("signature") || err.contains("issuer"), "{err}");
}

#[test]
fn an_intermediate_that_is_not_a_ca_is_refused() {
    let weak = build_pki(false);
    let (object, key_id) = attest(&weak, MESSAGE, &Opts::default());
    let err = check(&object, &key_id, MESSAGE, &weak.root_der, APP_ID).unwrap_err();
    assert!(err.contains("isn't a CA"), "{err}");
}

#[test]
fn a_corrupted_leaf_signature_a_third_certificate_and_a_stale_leaf_are_refused() {
    let corrupt = Opts {
        corrupt_leaf: true,
        ..Opts::default()
    };
    assert!(ok_with(&corrupt).is_err());
    let long = Opts {
        extra_cert: true,
        ..Opts::default()
    };
    assert!(ok_with(&long).unwrap_err().contains("chain"));
    let stale = Opts {
        leaf_window: (-86_400 * 10, -86_400 * 5),
        ..Opts::default()
    };
    assert!(ok_with(&stale).unwrap_err().contains("valid now"));
}

#[test]
fn authenticator_bytes_the_flags_do_not_explain_are_refused() {
    let o = Opts {
        undeclared_extensions: true,
        ..Opts::default()
    };
    assert!(ok_with(&o).unwrap_err().contains("explain"));
}

#[test]
fn damaged_objects_are_refused_and_never_panic() {
    let pki = pki();
    let (object, key_id) = attest(pki, MESSAGE, &Opts::default());
    // Every truncation.
    for cut in (0..object.len()).step_by(7) {
        assert!(
            check(&object[..cut], &key_id, MESSAGE, &pki.root_der, APP_ID).is_err(),
            "cut {cut}"
        );
    }
    // Trailing bytes, garbage, an empty object.
    let mut trailing = object.clone();
    trailing.push(0);
    assert!(check(&trailing, &key_id, MESSAGE, &pki.root_der, APP_ID).is_err());
    assert!(check(b"not cbor at all", &key_id, MESSAGE, &pki.root_der, APP_ID).is_err());
    assert!(check(&[], &key_id, MESSAGE, &pki.root_der, APP_ID).is_err());
    // A single flipped byte anywhere is refused or at least never a panic.
    for at in (0..object.len()).step_by(5) {
        let mut copy = object.clone();
        copy[at] ^= 0xff;
        let _ = check(&copy, &key_id, MESSAGE, &pki.root_der, APP_ID);
    }
    // The wrong format string.
    let mut other = object.clone();
    let at = other
        .windows(15)
        .position(|w| w == b"apple-appattest")
        .unwrap();
    other[at] = b'X';
    assert!(check(&other, &key_id, MESSAGE, &pki.root_der, APP_ID)
        .unwrap_err()
        .contains("isn't an App Attest"));
}

// ── The pinned root, CBOR and points ─────────────────────────────────────

#[test]
fn the_pinned_root_is_apples_and_self_signed() {
    let digest = Sha256::digest(app_attest::APPLE_ROOT_DER);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, app_attest::APPLE_ROOT_SHA256);
    assert!(app_attest::root_self_check(
        app_attest::APPLE_ROOT_DER,
        chrono::Utc::now().timestamp()
    ));
    // The test PKI's root is no substitute for it.
    assert_ne!(pki().root_der, app_attest::APPLE_ROOT_DER);
}

#[test]
fn the_cbor_reader_refuses_what_it_does_not_read() {
    // Indefinite length, a nesting bomb, a huge array, a cut string.
    assert!(app_attest::cbor_decode(&[0x9f, 0x01, 0xff]).is_err());
    assert!(app_attest::cbor_decode(&[0x81; 40]).is_err());
    assert!(app_attest::cbor_decode(&[0x9a, 0xff, 0xff, 0xff, 0xff]).is_err());
    assert!(app_attest::cbor_decode(&[0x45, 1, 2]).is_err());
    assert!(app_attest::cbor_decode(&[]).is_err());
    let (value, used) = app_attest::cbor_decode(&[0xa1, 0x61, b'a', 0x20, 0xf6]).unwrap();
    assert_eq!(used, 4, "stops after the map");
    assert_eq!(
        value,
        Cbor::Map(vec![(Cbor::Text("a".into()), Cbor::Int(-1))])
    );
}

#[test]
fn p256_points_are_checked_to_be_on_the_curve() {
    let rng = SystemRandom::new();
    for _ in 0..8 {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let point = pair.public_key().as_ref().to_vec();
        assert!(app_attest::p256_point_valid(&point));
        let mut moved = point.clone();
        moved[64] ^= 1;
        assert!(
            !app_attest::p256_point_valid(&moved),
            "a nudged y is off the curve"
        );
        assert!(!app_attest::p256_point_valid(&point[..64]));
        let mut compressed = point.clone();
        compressed[0] = 3;
        assert!(!app_attest::p256_point_valid(&compressed));
    }
    // x at or past p, and a point at infinity written as zeros.
    let mut big = vec![4u8];
    big.extend([0xff; 64]);
    assert!(!app_attest::p256_point_valid(&big));
    let mut zero = vec![4u8];
    zero.extend([0u8; 64]);
    assert!(!app_attest::p256_point_valid(&zero));
    // The generator is on the curve.
    let g = hex_bytes("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5");
    assert!(app_attest::p256_point_valid(&g));
}

fn hex_bytes(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn hostile_certificate_dates_are_refused_and_never_panic() {
    // Twelve bytes (the right length) holding a two-byte character, a sign, a
    // space, a letter: each used to be a way to slice inside a character or to
    // parse a non-digit as a number.
    for raw in [
        "1\u{e9}3456789012Z",
        "+10101000000Z",
        "261119 90000Z",
        "26111900000AZ",
        "\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}Z",
    ] {
        let o = Opts {
            raw_not_before: Some(raw.as_bytes().to_vec()),
            ..Opts::default()
        };
        assert!(ok_with(&o).unwrap_err().contains("date"), "{raw:?}");
    }
}

#[test]
fn an_intermediate_outside_its_validity_is_refused() {
    let stale = build_pki_with(true, (-86_400 * 20, -86_400 * 10));
    let (object, key_id) = attest(&stale, MESSAGE, &Opts::default());
    let err = check(&object, &key_id, MESSAGE, &stale.root_der, APP_ID).unwrap_err();
    assert!(err.contains("valid now"), "{err}");
    let future = build_pki_with(true, (86_400 * 10, 86_400 * 20));
    let (object, key_id) = attest(&future, MESSAGE, &Opts::default());
    assert!(check(&object, &key_id, MESSAGE, &future.root_der, APP_ID).is_err());
    // Half an hour past its end is inside the hour of slack.
    let edge = build_pki_with(true, (-3600 * 5, -1800));
    let (object, key_id) = attest(&edge, MESSAGE, &Opts::default());
    assert_eq!(
        check(&object, &key_id, MESSAGE, &edge.root_der, APP_ID),
        Ok(())
    );
}

/// An attestation built outside this file (openssl and python, in the shape
/// Apple's has: a P-384 root, a P-384 intermediate signed with SHA-384, a
/// P-256 credential certificate signed with ecdsa-with-SHA256, the nonce
/// extension as `SEQUENCE { [1] { OCTET STRING } }`, a standard X.509 encoding
/// by a second implementation): the verifier accepts it, and each mutation of
/// it is refused. A real iPhone's attestation joins it on the first device run.
#[test]
fn an_attestation_built_by_openssl_verifies_and_every_mutation_fails() {
    let doc = super::fixture("approvals/attestation-vector.json");
    let bytes = |name: &str| hex_bytes(doc[name].as_str().unwrap());
    let (root, object) = (bytes("rootDer"), bytes("attestation"));
    let message = doc["message"].as_str().unwrap();
    let key_id = doc["keyId"].as_str().unwrap();
    let app_id = doc["appId"].as_str().unwrap();
    let key = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(key_id)
            .unwrap()
    };
    let at = |object: &[u8], message: &str, root: &[u8], now: i64, app: &str| {
        app_attest::verify_attestation(
            object,
            &Expected {
                app_id: app,
                key_id: &key,
                client_data_hash: Sha256::digest(message.as_bytes()).into(),
                now,
                root,
            },
        )
        .map_err(|r| r.0)
    };
    let now = doc["now"].as_i64().unwrap();
    assert_eq!(at(&object, message, &root, now, app_id), Ok(()));
    // Another message, app, root or time.
    assert!(at(&object, "other", &root, now, app_id).is_err());
    assert!(at(&object, message, &root, now, "ABCDE12345.com.example").is_err());
    assert!(at(&object, message, &pki().root_der, now, app_id).is_err());
    assert!(at(&object, message, &root, now + 86_400 * 365 * 101, app_id).is_err());
    assert!(at(&object, message, &root, now - 86_400 * 3, app_id).is_err());
    // Every flipped byte is refused (the chain is signed and authData is bound
    // by the nonce), except the bytes nothing reads: the unused `receipt`
    // entry's key text and its one value byte.
    let receipt = object.windows(7).position(|w| w == b"receipt").unwrap();
    for i in 0..object.len() {
        let mut copy = object.clone();
        copy[i] ^= 0x01;
        let unread = (receipt..receipt + 9).contains(&i);
        if at(&copy, message, &root, now, app_id).is_ok() {
            assert!(
                unread,
                "flipping byte {i} was accepted (receipt entry at {receipt})"
            );
        }
    }
}
