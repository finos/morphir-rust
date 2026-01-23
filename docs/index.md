---
layout: default
title: Home
nav_order: 1
permalink: /
---

# Morphir Rust

Rust-based tooling for the [Morphir](https://github.com/finos/morphir) ecosystem.
{: .fs-6 .fw-300 }

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
morphir ir migrate --input ./morphir-ir.json --output ./v4.json

# Generate JSON Schema
morphir schema --output ./morphir-ir-schema.json
```

## Quick Links

- [FINOS Morphir Project](https://github.com/finos/morphir)
- [LCR Interactive Demo](https://lcr-interactive.finos.org/) - See Morphir in action

## Contributing

Morphir Rust is part of the [FINOS](https://www.finos.org/) foundation. Contributions are welcome!

- [Report Issues](https://github.com/finos/morphir-rust/issues)
- [Contributing Guide](https://github.com/finos/morphir-rust/blob/main/CONTRIBUTING.md)
