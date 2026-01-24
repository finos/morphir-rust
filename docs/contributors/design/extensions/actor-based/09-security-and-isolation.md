---
layout: default
title: Security and Isolation
nav_order: 11
parent: Morphir Extensions
---

# Security and Isolation

**Status:** Draft  
**Version:** 0.1.0

## Overview

The Morphir Extension System implements defense-in-depth security through multiple layers of isolation, capability-based permissions, and resource limits.

## Security Principles

1. **Principle of Least Privilege**: Extensions only get permissions they explicitly request
2. **Defense in Depth**: Multiple security layers (process, sandbox, permissions)
3. **Fail Secure**: Security failures result in denial, not escalation
4. **Explicit Over Implicit**: All capabilities must be declared
5. **Auditability**: All extension actions are logged and traceable

## Isolation Mechanisms

### Process Isolation (Stdio, JSON-RPC, gRPC)

Each extension runs in a separate OS process:

**Benefits:**

- Complete memory isolation
- OS-level resource limits
- Crash isolation (extension crash doesn't affect Morphir)
- Can be run with restricted user permissions

**Implementation:**

```rust
let child = Command::new(&command)
    .args(&args)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true)  // Clean up on host exit
    .spawn()?;
```

### WASM Sandboxing (Extism, Component)

WASM extensions run in a strict sandbox:

**Restrictions:**

- No direct filesystem access
- No network access
- No system calls
- Memory limited and isolated
- Execution time limited

**Capabilities:**

- Extensions can only call explicitly provided host functions
- All I/O goes through host APIs

```rust
let mut plugin = Plugin::new(&manifest, [], true)?;  // true = with wasi

// Memory limit
plugin.set_memory_limits(10_000_000, 100_000_000)?;  // min, max bytes

// Timeout
plugin.set_timeout(Duration::from_secs(5))?;
```

## Permission System

### Permission Model

Extensions declare required permissions in configuration:

```toml
[extensions.permissions]
network = true
filesystem = ["/data/output", "/tmp"]
max_memory = "100MB"
max_execution_time = "30s"
```

### Permission Types

#### Network Access

```rust
pub struct NetworkPermissions {
    /// Allow any network access
    pub enabled: bool,

    /// Allow only specific hosts
    pub allowed_hosts: Option<Vec<String>>,

    /// Allow only specific ports
    pub allowed_ports: Option<Vec<u16>>,
}
```

**Enforcement:**

- Process extensions: Use OS-level firewall rules
- WASM extensions: No network access (sandbox)

#### Filesystem Access

```rust
pub struct FilesystemPermissions {
    /// Allowed paths (read and write)
    pub paths: Vec<PathBuf>,

    /// Read-only paths
    pub readonly_paths: Vec<PathBuf>,
}
```

**Enforcement:**

- Process extensions: Use OS-level permissions (chroot, AppArmor, SELinux)
- WASM extensions: Mount only specified paths via WASI

#### Resource Limits

```rust
pub struct ResourceLimits {
    /// Maximum memory usage
    pub max_memory: Option<ByteSize>,

    /// Maximum CPU time per call
    pub max_execution_time: Option<Duration>,

    /// Maximum number of concurrent calls
    pub max_concurrent_calls: Option<usize>,
}
```

## Threat Model

### Threats Considered

1. **Malicious Extensions**
   - Trying to access unauthorized resources
   - Consuming excessive resources (DoS)
   - Data exfiltration
   - Privilege escalation

2. **Compromised Extensions**
   - Vulnerable dependencies
   - Injected malicious code
   - Supply chain attacks

3. **Extension Bugs**
   - Memory corruption
   - Resource leaks
   - Crashes affecting availability

### Threats NOT Considered

1. **Host System Compromise**: If the host OS is compromised, extensions cannot be secured
2. **Side-Channel Attacks**: Timing attacks, speculative execution (Spectre/Meltdown)
3. **Physical Access**: Local attacker with physical access
4. **Supply Chain** (Partially): We trust the extension source as declared, but not the extension behavior

## Security by Protocol

### Stdio Extensions

**Security Posture:** Medium  
**Isolation:** Process  
**Attack Surface:** OS process APIs

**Mitigations:**

- Run with restricted user (non-root)
- Use seccomp/AppArmor/SELinux
- Resource limits via cgroups
- Network isolation via firewall rules

**Example (Linux):**

```rust
// Run with restricted user
Command::new(&command)
    .uid(1001)  // unprivileged user
    .gid(1001)
```

### JSON-RPC / gRPC Extensions

**Security Posture:** Low (networked)  
**Isolation:** Network + Process  
**Attack Surface:** Network protocols, HTTP/gRPC libraries

**Mitigations:**

- mTLS for authentication
- Rate limiting
- Input validation
- Network segmentation

**Risks:**

- Extension can make arbitrary network calls
- Shared with other services
- Exposed to network attacks

### Extism WASM Extensions

**Security Posture:** High  
**Isolation:** WASM sandbox  
**Attack Surface:** WASM runtime, host functions

**Mitigations:**

- Strict memory isolation
- No direct system access
- Explicit capability granting
- Runtime limits

**Example:**

```rust
// Strict sandbox
let mut plugin = Plugin::new(&manifest, [], true)?;

// Memory limits
plugin.set_memory_limits(1_000_000, 10_000_000)?;

// Timeout
plugin.set_timeout(Duration::from_secs(5))?;

// No host functions = no I/O
```

### Component Model Extensions

**Security Posture:** High  
**Isolation:** WASM sandbox + WASI  
**Attack Surface:** WASM runtime, WASI interfaces

**Mitigations:**

- All Extism mitigations
- Capability-based WASI
- Fine-grained resource control

**Example:**

```rust
// Only allow specific directories
let mut ctx = WasiCtxBuilder::new()
    .preopened_dir(
        Dir::open_ambient_dir("/data/output", ambient_authority())?,
        "/output",
    )?
    .build();
```

## Security Best Practices

### For Extension Developers

1. **Minimize Permissions**: Only request what you need
2. **Validate Inputs**: Never trust input from Morphir core
3. **Handle Errors**: Don't leak sensitive info in error messages
4. **Avoid Dependencies**: Fewer dependencies = smaller attack surface
5. **Use WASM When Possible**: Strongest isolation

### For Morphir Core Developers

1. **Validate Extension Outputs**: Don't trust extension results
2. **Rate Limit Calls**: Prevent DoS via excessive calls
3. **Monitor Resource Usage**: Track memory, CPU, errors
4. **Log Everything**: Audit trail for security incidents
5. **Fail Closed**: On error, deny access

### For Operators

1. **Review Extensions**: Audit code before enabling
2. **Use Minimal Permissions**: Grant least privilege
3. **Monitor Logs**: Watch for suspicious behavior
4. **Update Regularly**: Keep runtime dependencies updated
5. **Network Isolation**: Use firewalls, VLANs

## Audit Logging

All extension operations are logged:

```rust
#[instrument(skip(self))]
async fn call_extension(
    &mut self,
    name: String,
    method: String,
    params: Value,
) -> Result<Value, ExtensionError> {
    info!(
        extension = %name,
        method = %method,
        "Extension call started"
    );

    let result = self.call_internal(name, method, params).await;

    match &result {
        Ok(_) => info!("Extension call succeeded"),
        Err(e) => warn!(error = %e, "Extension call failed"),
    }

    result
}
```

## Security Checklist

Before enabling an extension:

- [ ] Extension source is trusted
- [ ] Permissions are minimal
- [ ] Network access is justified
- [ ] Filesystem access is scoped
- [ ] Resource limits are set
- [ ] Extension has been reviewed
- [ ] Monitoring is enabled
- [ ] Incident response plan exists

## Future Enhancements

1. **Certificate Pinning**: For JSON-RPC/gRPC extensions
2. **Code Signing**: Verify extension integrity
3. **Runtime Policy Engine**: Dynamic permission adjustment
4. **Security Profiles**: Predefined security levels (low/medium/high)
5. **Attestation**: Remote attestation for WASM modules

## Related

### Morphir Rust Design Documents

- **[Morphir Extensions](../README.md)** - Extension system overview
- **[WASM Components](../wasm-component.md)** - Component model integration
- **[Tasks](../tasks.md)** - Task system definition

### Main Morphir Documentation

- [Morphir Documentation](https://morphir.finos.org) - Main Morphir documentation site
- [Morphir LLMs.txt](https://morphir.finos.org/llms.txt) - Machine-readable documentation index
- [Morphir IR v4 Design](https://morphir.finos.org/docs/design/draft/ir/) - IR v4 design documents
- [Morphir IR Specification](https://morphir.finos.org/docs/morphir-ir-specification/) - Complete IR specification
