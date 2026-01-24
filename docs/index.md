---
layout: default
title: Home
nav_order: 1
permalink: /
---

<div class="hero-banner">
  <img src="{{ '/assets/images/logo_white.png' | relative_url }}" alt="Morphir" style="max-width: 300px; margin-bottom: 1rem;">
  <h1>Morphir Rust</h1>
  <p>Rust-based tooling for the Morphir ecosystem</p>
</div>

[Get Started](getting-started){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[View on GitHub](https://github.com/finos/morphir-rust){: .btn .fs-5 .mb-4 .mb-md-0 }

---

## What is Morphir?

[Morphir](https://github.com/finos/morphir) is a library of tools that work together to solve different use cases. The central idea behind Morphir is that you write your business logic once as a set of Morphir expressions and then consume them in various ways, including:

- Visualizations
- Code generation
- Type checking
- Optimization
- Execution

## Quick Start

```bash
# Install morphir
curl -fsSL https://raw.githubusercontent.com/finos/morphir-rust/main/scripts/install.sh | bash

# Migrate a Morphir IR file to V4 format
morphir ir migrate ./morphir-ir.json --output ./v4.json

# Generate JSON Schema
morphir schema --output ./morphir-ir-schema.json
```

## Morphir Ecosystem

Morphir Rust is part of the larger Morphir ecosystem:

| Project | Description |
|---------|-------------|
| [morphir](https://github.com/finos/morphir) | Core Morphir specification |
| [morphir-elm](https://github.com/finos/morphir-elm) | Reference implementation (Elm) |
| [morphir-jvm](https://github.com/finos/morphir-jvm) | JVM implementation |
| [morphir-scala](https://github.com/finos/morphir-scala) | Scala implementation |
| [morphir-dotnet](https://github.com/finos/morphir-dotnet) | .NET implementation |
| **morphir-rust** | Rust implementation (this project) |

## Quick Links

- [Release Notes](releases) - What's new in Morphir Rust
- [FINOS Morphir Project](https://morphir.finos.org)
- [LCR Interactive Demo](https://lcr-interactive.finos.org/) - See Morphir in action

## Contributing

Morphir Rust is part of the [FINOS](https://www.finos.org/) foundation. Contributions are welcome!

- [Report Issues](https://github.com/finos/morphir-rust/issues)
- [Contributing Guide](https://github.com/finos/morphir-rust/blob/main/CONTRIBUTING.md)
