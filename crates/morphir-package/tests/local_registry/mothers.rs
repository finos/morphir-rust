use serde_json::{Value, json};
pub fn digest() -> String {
    format!("sha256:{}", "0".repeat(64))
}
pub fn release() -> Value {
    json!({"packagePath":"example.com/finance/root","version":"1.0.0"})
}
pub fn lock() -> Value {
    let d = digest();
    let r = release();
    json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryLock","resolution":{"policy":"flat-library:0.1.0-draft.2","profile":"local-library","requiredCapabilities":["dsse-ed25519","local-directory","tuf-1.0.36"]},"graph":{"root":r,"nodes":[{"release":r,"irPackageName":"root","manifestDigest":d,"contentDigest":d,"bindings":[]}]},"registries":[{"id":"finance","snapshot":"snapshot"}],"acquisitions":[{"release":r,"registry":"finance","record":{"path":"records/root.json","digest":d},"source":{"kind":"registry-directory","path":"bundles/root"},"statement":"statement"}],"evidence":[{"id":"root","registry":"finance","kind":"tuf-root","path":"metadata/1.root.json","digest":d},{"id":"timestamp","registry":"finance","kind":"tuf-timestamp","path":"metadata/1.timestamp.json","digest":d},{"id":"snapshot","registry":"finance","kind":"tuf-snapshot","path":"metadata/1.snapshot.json","digest":d},{"id":"targets","registry":"finance","kind":"tuf-targets","path":"metadata/1.targets.json","digest":d},{"id":"statement","registry":"finance","kind":"release-statement","path":"statements/root.json","digest":d}]})
}
pub fn statement() -> Value {
    json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryReleaseStatement","release":release(),"irPackageName":"root","dependencies":[],"manifestDigest":digest(),"contentDigest":digest()})
}
pub fn record() -> Value {
    let mut s = statement();
    s["kind"] = json!("LibraryRegistryRecord");
    s["source"] = json!({"kind":"registry-directory","path":"bundles/root"});
    s["statement"] = json!({"path":"statements/root.json","digest":digest()});
    s
}
pub fn policy() -> Value {
    json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryTrustPolicy","repositories":[{"identity":digest(),"bootstrapRoot":{"version":1,"digest":digest()},"namespaces":["example.com/finance"]}],"publisherRules":[{"namespace":"example.com","publicKeys":["0".repeat(64)],"threshold":1},{"namespace":"example.com/finance","publicKeys":["1".repeat(64)],"threshold":1}],"continuedUse":"previous-authorization"})
}
