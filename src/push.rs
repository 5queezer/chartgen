//! Web Push for server alerts (ADR-0045): the encryption of RFC 8291 and the VAPID
//! token of RFC 8292. Sending is the webhook path's (`webhook::deliver_request`), which
//! pins the address and retries; this only builds what is sent.
//!
//! OpenSSL is already linked through `native-tls`, so this adds no crate.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use openssl::bn::{BigNum, BigNumContext};
use openssl::derive::Deriver;
use openssl::ec::{EcGroup, EcKey, EcPoint, PointConversionForm};
use openssl::ecdsa::EcdsaSig;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::sha::sha256;
use openssl::symm::{encrypt_aead, Cipher};
use rand::Rng;
use serde_json::json;
use sha2::Sha256;

use crate::webhook::Request;

/// The record size in the header; one record is all this ever sends.
const RECORD_SIZE: u32 = 4096;
/// What fits one record: its size less the padding delimiter and the 16-byte tag.
/// RFC 8291 allows 3993 so that the whole message stays within 4096 bytes.
const MAX_PLAINTEXT: usize = 3993;
/// How long a push service keeps a message for a device that is off: as long as a
/// delivery may wait here, because a price that old is no news.
const TTL_SECS: u32 = 3600;
/// RFC 8292 allows 24 hours; a token is made for each delivery.
const TOKEN_SECS: i64 = 12 * 3600;

/// A 65-byte point and a 16-byte secret in base64url, with padding if a browser adds it.
pub const MAX_P256DH_LEN: usize = 88;
pub const MAX_AUTH_LEN: usize = 24;

const FAILED: &str = "could not encrypt the push message";
const BAD_KEYS: &str = "push keys must be a P-256 point and a 16-byte secret";

fn failed<E>(_: E) -> String {
    FAILED.to_string()
}

fn group() -> Result<EcGroup, String> {
    EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).map_err(failed)
}

fn decode(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()
}

/// One block of HKDF-SHA256 (RFC 5869), which is all this needs: at most 32 bytes.
fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let hmac = |key: &[u8], parts: &[&[u8]]| {
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes a key of any length");
        for part in parts {
            mac.update(part);
        }
        mac.finalize().into_bytes()
    };
    let prk = hmac(salt, &[ikm]);
    hmac(&prk, &[info, &[1]])[..len].to_vec()
}

/// A browser's `p256dh` and `auth`, checked: the point is on the curve.
pub struct DeviceKeys {
    public: Vec<u8>,
    auth: Vec<u8>,
}

impl DeviceKeys {
    /// The raw text is what gets stored, so its length is checked before decoding, which
    /// would otherwise ignore any amount of padding. Only the uncompressed form: a browser
    /// could not decrypt a message made from a hybrid one.
    pub fn parse(p256dh: &str, auth: &str) -> Result<Self, String> {
        if p256dh.len() > MAX_P256DH_LEN || auth.len() > MAX_AUTH_LEN {
            return Err(BAD_KEYS.into());
        }
        let public = decode(p256dh).filter(|k| k.len() == 65 && k[0] == 4);
        let auth = decode(auth).filter(|k| k.len() == 16);
        let (Some(public), Some(auth)) = (public, auth) else {
            return Err(BAD_KEYS.into());
        };
        let group = group()?;
        let mut ctx = BigNumContext::new().map_err(failed)?;
        EcPoint::from_bytes(&group, &public, &mut ctx).map_err(|_| BAD_KEYS.to_string())?;
        Ok(Self { public, auth })
    }
}

fn public_bytes(key: &EcKey<Private>, group: &EcGroup) -> Result<Vec<u8>, String> {
    let mut ctx = BigNumContext::new().map_err(failed)?;
    key.public_key()
        .to_bytes(group, PointConversionForm::UNCOMPRESSED, &mut ctx)
        .map_err(failed)
}

/// `plaintext` for `device` under RFC 8291, from a new key and salt each time.
pub fn encrypt(plaintext: &[u8], device: &DeviceKeys) -> Result<Vec<u8>, String> {
    let group = group()?;
    let key = EcKey::generate(&group).map_err(failed)?;
    encrypt_with(plaintext, device, &key, &rand::rng().random::<[u8; 16]>())
}

fn encrypt_with(
    plaintext: &[u8],
    device: &DeviceKeys,
    server_key: &EcKey<Private>,
    salt: &[u8; 16],
) -> Result<Vec<u8>, String> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err("the push message is too long".into());
    }
    let group = group()?;
    let server_public = public_bytes(server_key, &group)?;
    let mut ctx = BigNumContext::new().map_err(failed)?;
    let point = EcPoint::from_bytes(&group, &device.public, &mut ctx).map_err(failed)?;
    let peer = PKey::from_ec_key(EcKey::from_public_key(&group, &point).map_err(failed)?)
        .map_err(failed)?;
    let own = PKey::from_ec_key(server_key.clone()).map_err(failed)?;
    let mut deriver = Deriver::new(&own).map_err(failed)?;
    deriver.set_peer(&peer).map_err(failed)?;
    let secret = deriver.derive_to_vec().map_err(failed)?;

    let key_info = [
        b"WebPush: info\0".as_slice(),
        &device.public,
        &server_public,
    ]
    .concat();
    let ikm = hkdf(&device.auth, &secret, &key_info, 32);
    let cek = hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16);
    let nonce = hkdf(salt, &ikm, b"Content-Encoding: nonce\0", 12);

    // The delimiter of the last record, and no padding.
    let record = [plaintext, &[2]].concat();
    let mut tag = [0u8; 16];
    let sealed = encrypt_aead(
        Cipher::aes_128_gcm(),
        &cek,
        Some(&nonce),
        &[],
        &record,
        &mut tag,
    )
    .map_err(failed)?;

    let mut message = Vec::with_capacity(86 + sealed.len() + tag.len());
    message.extend_from_slice(salt);
    message.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    message.push(server_public.len() as u8);
    message.extend_from_slice(&server_public);
    message.extend_from_slice(&sealed);
    message.extend_from_slice(&tag);
    Ok(message)
}

/// The application server key. Not `Debug`: it holds the private key.
pub struct Vapid {
    key: EcKey<Private>,
    public: String,
    subject: String,
}

impl Vapid {
    /// A new private key, as stored and as `CHARTGEN_VAPID_PRIVATE_KEY` takes it.
    pub fn new_secret() -> Result<String, String> {
        let group = group()?;
        let key = EcKey::generate(&group).map_err(failed)?;
        let bytes = key.private_key().to_vec_padded(32).map_err(failed)?;
        Ok(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// The error never repeats the key.
    pub fn from_secret(secret: &str, subject: &str) -> Result<Self, String> {
        const BAD: &str = "the VAPID private key must be 32 bytes of base64url";
        let bytes = decode(secret).filter(|b| b.len() == 32).ok_or(BAD)?;
        if !(subject.starts_with("mailto:") || subject.starts_with("https://")) {
            return Err("the VAPID subject must be a mailto: or https: URL".into());
        }
        let group = group()?;
        let private = BigNum::from_slice(&bytes).map_err(|_| BAD)?;
        let ctx = BigNumContext::new().map_err(failed)?;
        let mut point = EcPoint::new(&group).map_err(failed)?;
        point
            .mul_generator(&group, &private, &ctx)
            .map_err(|_| BAD)?;
        let key = EcKey::from_private_components(&group, &private, &point).map_err(|_| BAD)?;
        key.check_key().map_err(|_| BAD)?;
        let public = URL_SAFE_NO_PAD.encode(public_bytes(&key, &group)?);
        Ok(Self {
            key,
            public,
            subject: subject.to_string(),
        })
    }

    /// What a browser subscribes with: `applicationServerKey`.
    pub fn public_key(&self) -> &str {
        &self.public
    }

    /// The `Authorization` header for a push service at `origin`.
    pub fn authorization(&self, origin: &str, now_secs: i64) -> Result<String, String> {
        let part = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
        let unsigned = format!(
            "{}.{}",
            part(json!({ "typ": "JWT", "alg": "ES256" })),
            part(json!({
                "aud": origin,
                "exp": now_secs + TOKEN_SECS,
                "sub": self.subject,
            })),
        );
        let digest = sha256(unsigned.as_bytes());
        let sig = EcdsaSig::sign(&digest, &self.key).map_err(failed)?;
        let raw = [
            sig.r().to_vec_padded(32).map_err(failed)?,
            sig.s().to_vec_padded(32).map_err(failed)?,
        ]
        .concat();
        Ok(format!(
            "vapid t={unsigned}.{}, k={}",
            URL_SAFE_NO_PAD.encode(raw),
            self.public
        ))
    }

    /// The request that sends `plaintext` to `device`, through the service at `origin`.
    pub fn request(
        &self,
        origin: &str,
        device: &DeviceKeys,
        plaintext: &[u8],
        now_secs: i64,
    ) -> Result<Request, String> {
        Ok(Request {
            headers: vec![
                ("authorization", self.authorization(origin, now_secs)?),
                ("content-encoding", "aes128gcm".to_string()),
                ("content-type", "application/octet-stream".to_string()),
                ("ttl", TTL_SECS.to_string()),
                ("urgency", "high".to_string()),
            ],
            body: encrypt(plaintext, device)?,
            honor_retry_after: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::symm::decrypt_aead;

    const PLAINTEXT: &str = "When I grow up, I want to be a watermelon";
    const AS_PRIVATE: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
    const AS_PUBLIC: &str =
        "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
    const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
    const MESSAGE: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

    fn private_key(secret: &str, public: &str) -> EcKey<Private> {
        let group = group().unwrap();
        let mut ctx = BigNumContext::new().unwrap();
        let point = EcPoint::from_bytes(&group, &decode(public).unwrap(), &mut ctx).unwrap();
        let d = BigNum::from_slice(&decode(secret).unwrap()).unwrap();
        EcKey::from_private_components(&group, &d, &point).unwrap()
    }

    /// The browser's side, for a message from any key.
    fn decrypt(message: &[u8], ua: &EcKey<Private>, auth: &[u8]) -> Vec<u8> {
        let group = group().unwrap();
        let (salt, rest) = message.split_at(16);
        assert_eq!(rest[..4], RECORD_SIZE.to_be_bytes());
        let id_len = rest[4] as usize;
        let (server_public, sealed) = rest[5..].split_at(id_len);
        let mut ctx = BigNumContext::new().unwrap();
        let point = EcPoint::from_bytes(&group, server_public, &mut ctx).unwrap();
        let peer = PKey::from_ec_key(EcKey::from_public_key(&group, &point).unwrap()).unwrap();
        let own = PKey::from_ec_key(ua.clone()).unwrap();
        let mut deriver = Deriver::new(&own).unwrap();
        deriver.set_peer(&peer).unwrap();
        let secret = deriver.derive_to_vec().unwrap();
        let info = [
            b"WebPush: info\0".as_slice(),
            &public_bytes(ua, &group).unwrap(),
            server_public,
        ]
        .concat();
        let ikm = hkdf(auth, &secret, &info, 32);
        let cek = hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16);
        let nonce = hkdf(salt, &ikm, b"Content-Encoding: nonce\0", 12);
        let (body, tag) = sealed.split_at(sealed.len() - 16);
        let mut record =
            decrypt_aead(Cipher::aes_128_gcm(), &cek, Some(&nonce), &[], body, tag).unwrap();
        assert_eq!(record.pop(), Some(2), "the last record's delimiter");
        record
    }

    /// RFC 8291, Appendix A: the same keys and salt must give the same bytes.
    #[test]
    fn encryption_matches_the_rfc_8291_test_vector() {
        let device = DeviceKeys::parse(UA_PUBLIC, AUTH).unwrap();
        let salt: [u8; 16] = decode(SALT).unwrap().try_into().unwrap();
        let message = encrypt_with(
            PLAINTEXT.as_bytes(),
            &device,
            &private_key(AS_PRIVATE, AS_PUBLIC),
            &salt,
        )
        .unwrap();
        assert_eq!(URL_SAFE_NO_PAD.encode(message), MESSAGE);
    }

    #[test]
    fn the_test_vector_decrypts_with_the_browsers_keys() {
        let plain = decrypt(
            &decode(MESSAGE).unwrap(),
            &private_key(UA_PRIVATE, UA_PUBLIC),
            &decode(AUTH).unwrap(),
        );
        assert_eq!(plain, PLAINTEXT.as_bytes());
    }

    #[test]
    fn each_message_has_its_own_key_and_salt_and_decrypts() {
        let device = DeviceKeys::parse(UA_PUBLIC, AUTH).unwrap();
        let (a, b) = (
            encrypt(b"same", &device).unwrap(),
            encrypt(b"same", &device).unwrap(),
        );
        assert_ne!(a[..16], b[..16], "salt");
        assert_ne!(a[21..86], b[21..86], "server key");
        let ua = private_key(UA_PRIVATE, UA_PUBLIC);
        assert_eq!(decrypt(&a, &ua, &decode(AUTH).unwrap()), b"same");
    }

    #[test]
    fn a_message_too_long_for_a_record_is_refused() {
        let device = DeviceKeys::parse(UA_PUBLIC, AUTH).unwrap();
        let max = encrypt(&vec![b'x'; MAX_PLAINTEXT], &device).unwrap();
        assert_eq!(max.len(), 4096);
        assert!(encrypt(&vec![b'x'; MAX_PLAINTEXT + 1], &device).is_err());
    }

    #[test]
    fn keys_that_are_not_a_point_and_a_secret_are_refused() {
        let off_curve = URL_SAFE_NO_PAD.encode([4u8; 65]);
        for (p256dh, auth) in [
            ("", AUTH),
            (UA_PUBLIC, ""),
            ("not base64!", AUTH),
            (off_curve.as_str(), AUTH),
            (UA_PUBLIC, UA_PUBLIC),
            (AUTH, AUTH),
        ] {
            assert!(DeviceKeys::parse(p256dh, auth).is_err(), "{p256dh} {auth}");
        }
        // As some browsers pad them.
        let padded = format!("{AUTH}==");
        assert!(DeviceKeys::parse(UA_PUBLIC, &padded).is_ok());
        assert!(DeviceKeys::parse(&format!("{UA_PUBLIC}="), AUTH).is_ok());
    }

    /// The raw text is stored, so padding must not get a megabyte past the length check.
    #[test]
    fn padding_cannot_make_a_key_longer_than_its_limit() {
        let long = |key: &str, limit: usize| format!("{key}{}", "=".repeat(limit + 1 - key.len()));
        assert!(DeviceKeys::parse(&long(UA_PUBLIC, MAX_P256DH_LEN), AUTH).is_err());
        assert!(DeviceKeys::parse(UA_PUBLIC, &long(AUTH, MAX_AUTH_LEN)).is_err());
        let huge = format!("{UA_PUBLIC}{}", "=".repeat(1_900_000));
        assert!(DeviceKeys::parse(&huge, AUTH).is_err());
    }

    /// A browser could not decrypt what a hybrid point produces.
    #[test]
    fn only_the_uncompressed_form_of_a_point_is_taken() {
        let mut point = decode(UA_PUBLIC).unwrap();
        let hybrid = if point[64] & 1 == 0 { 6 } else { 7 };
        point[0] = hybrid;
        assert!(DeviceKeys::parse(&URL_SAFE_NO_PAD.encode(&point), AUTH).is_err());
    }

    #[test]
    fn a_vapid_token_is_signed_for_the_origin_and_verifies() {
        let secret = Vapid::new_secret().unwrap();
        let vapid = Vapid::from_secret(&secret, "mailto:ops@example.com").unwrap();
        let header = vapid
            .authorization("https://fcm.googleapis.com", 1_000)
            .unwrap();
        let (token, key) = header
            .strip_prefix("vapid t=")
            .unwrap()
            .split_once(", k=")
            .unwrap();
        assert_eq!(key, vapid.public_key());
        let (unsigned, sig) = token.rsplit_once('.').unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&decode(unsigned.split_once('.').unwrap().1).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["sub"], "mailto:ops@example.com");
        assert_eq!(claims["exp"], 1_000 + TOKEN_SECS);

        let raw = decode(sig).unwrap();
        assert_eq!(raw.len(), 64, "JOSE: r and s, not DER");
        let sig = EcdsaSig::from_private_components(
            BigNum::from_slice(&raw[..32]).unwrap(),
            BigNum::from_slice(&raw[32..]).unwrap(),
        )
        .unwrap();
        let group = group().unwrap();
        let mut ctx = BigNumContext::new().unwrap();
        let point = EcPoint::from_bytes(&group, &decode(key).unwrap(), &mut ctx).unwrap();
        let public = EcKey::from_public_key(&group, &point).unwrap();
        assert!(sig.verify(&sha256(unsigned.as_bytes()), &public).unwrap());
    }

    #[test]
    fn the_stored_key_gives_the_same_public_key_each_start() {
        let secret = Vapid::new_secret().unwrap();
        let a = Vapid::from_secret(&secret, "https://x.example").unwrap();
        let b = Vapid::from_secret(&secret, "https://x.example").unwrap();
        assert_eq!(a.public_key(), b.public_key());
        assert_eq!(decode(a.public_key()).unwrap().len(), 65);
    }

    #[test]
    fn a_bad_key_or_subject_is_refused_and_the_error_keeps_the_key() {
        let good = Vapid::new_secret().unwrap();
        assert!(Vapid::from_secret(&good, "ops@example.com").is_err());
        assert!(Vapid::from_secret(&good, "http://x.example").is_err());
        for bad in ["", "short", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"] {
            let e = match Vapid::from_secret(bad, "https://x.example") {
                Err(e) => e,
                Ok(_) => panic!("{bad} passed"),
            };
            assert!(bad.is_empty() || !e.contains(bad), "{e}");
        }
    }

    #[test]
    fn a_request_says_what_a_push_service_asks_for() {
        let vapid = Vapid::from_secret(&Vapid::new_secret().unwrap(), "https://x.example").unwrap();
        let device = DeviceKeys::parse(UA_PUBLIC, AUTH).unwrap();
        let req = vapid
            .request("https://web.push.apple.com", &device, b"hi", 1_000)
            .unwrap();
        let header = |name: &str| {
            req.headers
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(header("content-encoding"), Some("aes128gcm"));
        assert_eq!(header("ttl"), Some("3600"));
        assert_eq!(header("urgency"), Some("high"));
        assert!(header("authorization").unwrap().starts_with("vapid t="));
        let ua = private_key(UA_PRIVATE, UA_PUBLIC);
        assert_eq!(decrypt(&req.body, &ua, &decode(AUTH).unwrap()), b"hi");
    }
}
