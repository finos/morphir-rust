# Local-registry test data

`mothers.rs` builds small structural inputs for focused rejection tests.
It does not derive compatibility expectations or authorize any package.

`crypto_vectors.json` freezes three verification-criteria vectors checked with
`@noble/curves` 2.4.0 using `ed25519.verify(..., { zip215: false })`. The audited
TypeScript reference is finos/morphir-typescript commit
`a556ee035f88d9593eef0c7f6956ea4c5d645784`.

All values are public test material. Let B be the Ed25519 base point and T the
order-two point with compressed bytes `ec` followed by thirty `ff` bytes and
`7f`. Each payload is the UTF-8 spelling of its vector name. The message is the
exact draft.3 DSSE PAE of that payload.

| Vector | Public point A | Signature point R | Public test scalar r |
| --- | --- | --- | --- |
| identity-r | B | identity | 0 |
| mixed-order-key | B + T | B | 1 |
| mixed-order-r | B | B + T | 1 |

For each vector, compute `k = SHA512(R_bytes || A_bytes || message) mod l`, with
little-endian reduction and Ed25519 subgroup order l. The signature is
`R_bytes || little_endian_32((r + k) mod l)`. The key scalar is the public value
1. These deliberately unusual signatures establish compatibility with the
baseline's cofactored equation. They are not production signing examples.

The runtime requires canonical encodings of A and R and rejects small-order A.
It uses maintained `ed25519-zebra` verification for the cofactored equation;
`curve25519-dalek` supplies point decoding and the explicit encoding/weak-key
checks. Using `ed25519-dalek::verify_strict` instead would change acceptance of
these baseline inputs. Independent RFC 8032 known-answer tests and malformed
point/scalar/signature tests accompany the vectors.
