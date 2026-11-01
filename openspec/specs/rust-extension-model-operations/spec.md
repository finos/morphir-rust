# rust-extension-model-operations Specification

## Purpose

Defines how the Rust extension (`morphir-rust`) takes part in MEP `0.2.0-draft.1`: which protocol versions and
model transport tuples it declares, how it compiles Rust source into a model envelope and generates Rust from
one, and which requests it refuses.

## Requirements

### Requirement: Declared protocol versions
The extension SHALL declare MEP `0.2.0-draft.1` and MEP `0.1` in its describe claims, in its release
descriptor claims and in its registry entry. It SHALL select `0.2.0-draft.1` only when the host offers that
version together with a model transport offer, and SHALL select `0.1` otherwise when the host offers `0.1`.

#### Scenario: A host that offers only MEP 0.1
- **WHEN** a host initializes the extension with protocol versions `["0.1"]`
- **THEN** the extension selects `0.1`
- **AND** the initialize result has no `modelTransport` member

#### Scenario: A host that offers MEP 0.2.0-draft.1 with a model transport offer
- **WHEN** a host initializes the extension with protocol versions `["0.2.0-draft.1", "0.1"]` and a model
  transport offer that contains the extension's tuples
- **THEN** the extension selects `0.2.0-draft.1`
- **AND** the initialize result's `modelTransport` holds the tuples that both peers support, in the host's
  order, and the smaller of each limit

#### Scenario: A host that offers MEP 0.2.0-draft.1 without a model transport offer
- **WHEN** a host initializes the extension with protocol versions `["0.2.0-draft.1", "0.1"]` and no model
  transport offer
- **THEN** the extension selects `0.1`

### Requirement: Advertised model transport tuples
The extension SHALL advertise exactly two model transport tuples, both with profile `morphir-ir`, codec
`morphir-json`, envelope version `0.1.0-draft.1` and IR version `4.0.0`: one for
`morphir.frontend.compileModel` and one for `morphir.backend.generateModel`. It SHALL NOT advertise an Ion
tuple and SHALL NOT advertise a guarantee.

#### Scenario: Describe claims list the tuples
- **WHEN** a host sends `morphir.extension.describe`
- **THEN** `capabilities.modelTransport.tuples` holds exactly the two tuples
- **AND** `capabilities.modelTransport.guarantees` is empty

#### Scenario: A host that offers only Ion
- **WHEN** a host offers MEP `0.2.0-draft.1` with only `morphir-model` / `ion-binary` tuples
- **THEN** the agreed model transport holds no tuple, so no model operation is available in the session

### Requirement: Compile Rust source into a model envelope
For an agreed `morphir.frontend.compileModel` tuple, the extension SHALL compile the request's source with the
same rules as `morphir.frontend.compile` at IR v4. A successful result SHALL carry a model envelope whose
payload is the IR v4 JSON document that `morphir.frontend.compile` returns for the same source, with the
envelope's tuple, `modelId` equal to the package name and `revision` equal to `sha256:` and the payload digest.

#### Scenario: A supported source file
- **WHEN** a host sends `morphir.frontend.compileModel` for one supported `.rs` document
- **THEN** the result has `success: true`, the compiled module names and a model envelope
- **AND** the decoded payload equals the recorded golden IR v4 document

#### Scenario: A source file outside the supported subset
- **WHEN** a host sends `morphir.frontend.compileModel` for a document that the frontend rejects
- **THEN** the result has `success: false` and the frontend's diagnostics
- **AND** the result has no `model` member

### Requirement: Generate Rust from a model envelope
For an agreed `morphir.backend.generateModel` tuple, the extension SHALL decode the model envelope's payload
and generate Rust with the same rules as `morphir.backend.generate`, for target `rust`.

#### Scenario: A model envelope from compileModel
- **WHEN** a host sends `morphir.backend.generateModel` with the envelope that `compileModel` returned
- **THEN** the result has `success: true` and the artifacts that `morphir.backend.generate` returns for the
  payload's IR
- **AND** the generated `lib.rs` equals the recorded golden output

### Requirement: Refuse requests outside the agreed tuple
The extension SHALL refuse a model operation whose tuple was not agreed in the session, whose envelope does
not match the tuple, or whose payload does not match its length or SHA-256, with a JSON-RPC error and without
running the frontend or backend. It SHALL refuse a generation that requires a guarantee.

#### Scenario: A tuple that was not agreed
- **WHEN** a host sends `morphir.backend.generateModel` with an Ion tuple, or with an IR version other than
  `4.0.0`
- **THEN** the response is a JSON-RPC error and has no result

#### Scenario: A model envelope with another envelope version
- **WHEN** a host sends `morphir.backend.generateModel` with a model whose `envelopeVersion` is not
  `0.1.0-draft.1`
- **THEN** the response is a JSON-RPC error and has no result

#### Scenario: A model operation on the MEP 0.1 line
- **WHEN** a session selected MEP `0.1` and the host sends `morphir.frontend.compileModel`
- **THEN** the response is a JSON-RPC error

#### Scenario: A generation that requires a guarantee
- **WHEN** a host sends `morphir.backend.generateModel` with a required guarantee
- **THEN** the extension does not generate artifacts
