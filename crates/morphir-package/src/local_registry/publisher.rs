use super::diagnostics::resource;
use super::json::child;
use super::shape::Shape;
use super::*;
use crate::resolution::ReleaseId;
use base64::{
    Engine, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use curve25519_dalek::edwards::CompressedEdwardsY;
use ed25519_zebra::{Signature, VerificationKey};

/// The exact DSSE payload type signed by the draft.3 publisher profile.
pub const PUBLISHER_PAYLOAD_TYPE: &str =
    "application/vnd.morphir.library-release.v0.1.0-draft.3+json";
/// Owned, decoded envelope. It carries no authenticated authority.
#[derive(Debug)]
pub struct PreparedPublisherEnvelope {
    subject: ObjectSubject,
    envelope: Vec<u8>,
    payload: Vec<u8>,
    signatures: Vec<Vec<u8>>,
}
/// Signature evidence bound to a request and a snapshot of its current local policy.
///
/// Payload identity and release authorization are deliberately not established here.
/// Fields have no public constructor or deserializer, so serialized receipts cannot
/// recreate authority or rebind the evidence to another request.
///
/// ```compile_fail
/// use morphir_package::local_registry::PublisherSignatureEvidence;
/// let evidence: PublisherSignatureEvidence = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct PublisherSignatureEvidence {
    requested_release: ReleaseId,
    subject: ObjectSubject,
    publisher_rule: PublisherRule,
    verified_keys: Vec<PublisherKey>,
    payload: Vec<u8>,
    envelope: Vec<u8>,
}
impl PublisherSignatureEvidence {
    /// The requested release, independent of the unparsed payload identity.
    pub fn requested_release(&self) -> &ReleaseId {
        &self.requested_release
    }
    /// Owned object location supplied for the prepared envelope.
    pub fn subject(&self) -> &ObjectSubject {
        &self.subject
    }
    /// Owned snapshot of the longest matching rule.
    pub fn publisher_rule(&self) -> &PublisherRule {
        &self.publisher_rule
    }
    /// All distinct verified authorized keys, sorted by raw key spelling.
    pub fn verified_keys(&self) -> &[PublisherKey] {
        &self.verified_keys
    }
    /// Immutable exact payload bytes. Copy with `to_vec` if mutable storage is needed.
    pub fn payload_bytes(&self) -> &[u8] {
        &self.payload
    }
    /// Immutable exact envelope bytes.
    pub fn envelope_bytes(&self) -> &[u8] {
        &self.envelope
    }
}
/// Own exact envelope bytes before interpreting its open DSSE structure.
pub fn prepare_publisher_envelope(
    bytes: &[u8],
    subject: &ObjectSubject,
) -> Result<PreparedPublisherEnvelope, Diagnostic> {
    let owned_subject = subject.clone();
    let subject = Subject::from(&owned_subject);
    let phase = Phase::Repository;
    if bytes.len() > 1_048_576 {
        return Err(resource(
            &subject,
            phase,
            Resource::EnvelopeBytes,
            1_048_576,
        ));
    }
    let envelope = bytes.to_vec();
    let parsed = decode_json_domain(&envelope, JsonDomain::Dsse, &subject, phase)?;
    let doc = &parsed.document;
    let signatures = doc.get("signatures");
    if signatures
        .and_then(JsonNode::as_array)
        .is_some_and(|a| a.len() > 64)
    {
        return Err(resource(&subject, phase, Resource::Signatures, 64));
    }
    let mut s = Shape::new(&subject);
    let mut payload = None;
    let mut decoded_signatures = vec![];
    if open_object(
        Some(doc),
        "",
        &["payloadType", "payload", "signatures"],
        &mut s,
    ) {
        s.literal(
            doc.get("payloadType"),
            "/payloadType",
            &[PUBLISHER_PAYLOAD_TYPE],
            Code::UnsupportedPayloadType,
        );
        payload = base64_field(doc.get("payload"), "/payload", &mut s);
        if payload.as_ref().is_some_and(|p| p.len() > 1_048_576) {
            return Err(resource(
                &subject,
                phase,
                Resource::StatementBytes,
                1_048_576,
            ));
        }
        for (i, v) in s.array(signatures, "/signatures", false).iter().enumerate() {
            let p = format!("/signatures/{i}");
            if !open_object(Some(v), &p, &["sig"], &mut s) {
                continue;
            }
            s.string(v.get("keyid"), &format!("{p}/keyid"));
            if let Some(sig) = base64_field(v.get("sig"), &format!("{p}/sig"), &mut s) {
                decoded_signatures.push(sig)
            }
        }
    }
    s.supported(Some(phase))?;
    s.finish(phase)?;
    Ok(PreparedPublisherEnvelope {
        subject: owned_subject,
        envelope,
        payload: payload.expect("validated DSSE payload"),
        signatures: decoded_signatures,
    })
}
fn open_object(v: Option<&JsonNode>, p: &str, required: &[&str], s: &mut Shape) -> bool {
    let Some(JsonNode::Object(m)) = v else {
        s.add(p, Rule::InvalidType);
        return false;
    };
    for name in required {
        if !m.contains_key(*name) {
            s.add(child(p, name), Rule::MissingField)
        }
    }
    true
}
fn base64_field(v: Option<&JsonNode>, p: &str, s: &mut Shape) -> Option<Vec<u8>> {
    let text = s.string(v, p)?;
    let decoded = decode_base64(text);
    if decoded.is_none() {
        s.add(p, Rule::InvalidValue)
    }
    decoded
}
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    if !text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"+/-_=".contains(&b))
        || (text.contains(['+', '/']) && text.contains(['-', '_']))
    {
        return None;
    }
    let unpadded = text.trim_end_matches('=');
    let padding = text.len() - unpadded.len();
    if unpadded.contains('=') || padding > 2 || unpadded.len() % 4 == 1 {
        return None;
    }
    if padding > 0 && (!text.len().is_multiple_of(4) || padding != (4 - unpadded.len() % 4) % 4) {
        return None;
    }
    let normalized = unpadded.replace('-', "+").replace('_', "/");
    GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::RequireNone),
    )
    .decode(normalized)
    .ok()
}
/// Verify against the current requested release's local publisher policy.
/// Payload parsing and authenticated release reconciliation are later operations.
///
/// Every verification selects a rule from the supplied current policy. A prepared
/// envelope has no cached authorization, and key ID hints do not select keys.
/// Canonical-point checks plus Zebra's cofactored equation preserve Noble 2.4's
/// `zip215: false` behavior, including its acceptance of canonical small-order R.
/// See the [upstream verification contract](https://docs.rs/ed25519-zebra/4.2.0/ed25519_zebra/struct.VerificationKey.html).
pub fn verify_publisher_signatures(
    envelope: &PreparedPublisherEnvelope,
    requested: &ReleaseId,
    policy: &TrustPolicy,
) -> Result<PublisherSignatureEvidence, Diagnostic> {
    let Some(rule) = matching_publisher_rule(policy, requested.package_path()) else {
        return Err(Diagnostic {
            category: Category::DomainRejection,
            code: Code::UnauthorizedPublisher,
            phase: Phase::Authorization,
            witnesses: vec![Witness::Authority {
                registry: envelope.subject.registry.as_str().into(),
                release: requested.clone(),
                rule: AuthorityRule::PublisherRuleMissing,
            }],
        });
    };
    let mut pae = format!(
        "DSSEv1 {} {} {} ",
        PUBLISHER_PAYLOAD_TYPE.len(),
        PUBLISHER_PAYLOAD_TYPE,
        envelope.payload.len()
    )
    .into_bytes();
    pae.extend_from_slice(&envelope.payload);
    let mut keys = rule.public_keys().to_vec();
    keys.sort();
    keys.dedup();
    let verified_keys: Vec<_> = keys
        .into_iter()
        .filter(|key| {
            let raw = decode_hex_key(key);
            envelope
                .signatures
                .iter()
                .any(|sig| strict_verify(&raw, &pae, sig))
        })
        .collect();
    if !publisher_threshold_met(Some(rule), &verified_keys) {
        return Err(Diagnostic {
            category: Category::DomainRejection,
            code: Code::SignatureInvalid,
            phase: Phase::Authorization,
            witnesses: vec![Witness::Authentication {
                subject: (&Subject::from(&envelope.subject)).into(),
                role: PublisherRole::Publisher,
                required: rule.threshold().to_string(),
                verified: verified_keys.len().to_string(),
            }],
        });
    }
    Ok(PublisherSignatureEvidence {
        requested_release: requested.clone(),
        subject: envelope.subject.clone(),
        publisher_rule: rule.clone(),
        verified_keys,
        payload: envelope.payload.clone(),
        envelope: envelope.envelope.clone(),
    })
}
fn decode_hex_key(key: &PublisherKey) -> [u8; 32] {
    let mut raw = [0; 32];
    for (i, pair) in key
        .as_str()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
    {
        let digit = |b: u8| {
            if b.is_ascii_digit() {
                b - b'0'
            } else {
                b - b'a' + 10
            }
        };
        raw[i] = digit(pair[0]) * 16 + digit(pair[1]);
    }
    raw
}
// Noble 2.4 zip215:false requires canonical A and R and rejects small-order A,
// but checks the cofactored equation and permits canonical small-order R.
// Dalek verify_strict would silently reject valid baseline inputs. Zebra owns
// signature/scalar/equation verification; Dalek owns point decoding checks.
fn strict_verify(key: &[u8; 32], message: &[u8], signature: &[u8]) -> bool {
    let Ok(raw_sig) = <&[u8; 64]>::try_from(signature) else {
        return false;
    };
    let Some(a) = CompressedEdwardsY(*key).decompress() else {
        return false;
    };
    if a.compress().to_bytes() != *key || a.is_small_order() {
        return false;
    }
    let r_bytes: [u8; 32] = raw_sig[..32].try_into().unwrap();
    let Some(r) = CompressedEdwardsY(r_bytes).decompress() else {
        return false;
    };
    if r.compress().to_bytes() != r_bytes {
        return false;
    }
    let Ok(key) = VerificationKey::try_from(*key) else {
        return false;
    };
    key.verify(&Signature::from(*raw_sig), message).is_ok()
}

#[cfg(test)]
mod tests {
    use super::strict_verify;
    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }
    #[test]
    fn strict_profile_matches_rfc8032_known_answers_and_rejects_overflow() {
        // RFC 8032 section 7.1, tests 1 and 2; independent fixed published vectors.
        for (key, message, signature) in [
            (
                "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
                "",
                "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
            ),
            (
                "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
                "72",
                "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
            ),
        ] {
            let key: [u8; 32] = bytes(key).try_into().unwrap();
            let message = bytes(message);
            let mut signature = bytes(signature);
            assert!(strict_verify(&key, &message, &signature));
            let mut modified = message.clone();
            modified.push(0);
            assert!(!strict_verify(&key, &modified, &signature));
            signature[32..].fill(255);
            assert!(!strict_verify(&key, &message, &signature));
        }
    }
    #[test]
    fn noncanonical_r_and_x_zero_sign_bit_are_rejected() {
        let key: [u8; 32] =
            bytes("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
                .try_into()
                .unwrap();
        for r in [
            format!("ee{}7f", "ff".repeat(30)),
            format!("01{}80", "00".repeat(30)),
        ] {
            let mut signature = bytes(&r);
            signature.extend([0; 32]);
            assert!(!strict_verify(&key, b"message", &signature));
        }
    }
}
