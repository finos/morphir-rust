---
layout: default
title: gRPC Extension Host
nav_order: 6
parent: Morphir Extensions
---

# gRPC Extension Host

**Status:** Draft  
**Version:** 0.1.0

## Overview

The gRPC Extension Host manages extensions that communicate via gRPC. This protocol provides high performance, strong typing via Protocol Buffers, and excellent cross-language support.

## Protocol

### Transport

- **Transport**: gRPC over HTTP/2
- **Serialization**: Protocol Buffers
- **Library**: `tonic` (Rust)

### Service Definition (Protocol Buffers)

```protobuf
syntax = "proto3";

package morphir.extension;

service ExtensionService {
  rpc Initialize(InitializeRequest) returns (InitializeResponse);
  rpc GetCapabilities(Empty) returns (CapabilitiesResponse);
  rpc Call(CallRequest) returns (CallResponse);
  rpc Stream(stream StreamRequest) returns (stream StreamResponse);
}

message InitializeRequest {
  string config_json = 1;
}

message InitializeResponse {
  string status = 1;
}

message CapabilitiesResponse {
  repeated Capability capabilities = 1;
}

message Capability {
  string name = 1;
  string description = 2;
  optional string params_schema = 3;
  optional string return_schema = 4;
}

message CallRequest {
  string method = 1;
  bytes params_json = 2;
}

message CallResponse {
  oneof result {
    bytes success_json = 1;
    string error = 2;
  }
}
```

## Implementation

### State

```rust
use tonic::transport::Channel;
use morphir_extension::extension_service_client::ExtensionServiceClient;

pub struct GrpcExtensionHost {
    extensions: HashMap<ExtensionId, GrpcExtension>,
    next_id: u64,
}

struct GrpcExtension {
    name: String,
    client: ExtensionServiceClient<Channel>,
    capabilities: Vec<Capability>,
}
```

### Loading Process

```rust
async fn load_extension(&mut self, config: ExtensionConfig) -> Result<ExtensionId, ExtensionError> {
    let ExtensionSource::Grpc { endpoint } = config.source else {
        return Err(ExtensionError::InvalidSource);
    };

    // Connect to gRPC service
    let channel = Channel::from_shared(endpoint)?
        .connect()
        .await?;

    let mut client = ExtensionServiceClient::new(channel);

    // Initialize
    client.initialize(InitializeRequest {
        config_json: serde_json::to_string(&config.config)?,
    }).await?;

    // Get capabilities
    let response = client
        .get_capabilities(Empty {})
        .await?
        .into_inner();

    let id = ExtensionId(self.next_id);
    self.next_id += 1;

    self.extensions.insert(id, GrpcExtension {
        name: config.name,
        client,
        capabilities: response.capabilities,
    });

    Ok(id)
}
```

## Extension Implementation (Go Example)

```go
package main

import (
    "context"
    "encoding/json"
    "log"
    "net"

    pb "morphir/extension"
    "google.golang.org/grpc"
)

type server struct {
    pb.UnimplementedExtensionServiceServer
}

func (s *server) Initialize(ctx context.Context, req *pb.InitializeRequest) (*pb.InitializeResponse, error) {
    log.Printf("Initialized with config: %s", req.ConfigJson)
    return &pb.InitializeResponse{Status: "ready"}, nil
}

func (s *server) GetCapabilities(ctx context.Context, _ *pb.Empty) (*pb.CapabilitiesResponse, error) {
    return &pb.CapabilitiesResponse{
        Capabilities: []*pb.Capability{
            {
                Name:        "transform",
                Description: "Transform Morphir IR",
            },
        },
    }, nil
}

func (s *server) Call(ctx context.Context, req *pb.CallRequest) (*pb.CallResponse, error) {
    var params map[string]interface{}
    json.Unmarshal(req.ParamsJson, &params)

    // Process based on method
    result := map[string]interface{}{
        "transformed": true,
    }

    resultJson, _ := json.Marshal(result)
    return &pb.CallResponse{
        Result: &pb.CallResponse_SuccessJson{
            SuccessJson: resultJson,
        },
    }, nil
}

func main() {
    lis, err := net.Listen("tcp", ":50051")
    if err != nil {
        log.Fatalf("failed to listen: %v", err)
    }

    s := grpc.NewServer()
    pb.RegisterExtensionServiceServer(s, &server{})

    log.Printf("Server listening at %v", lis.Addr())
    if err := s.Serve(lis); err != nil {
        log.Fatalf("failed to serve: %v", err)
    }
}
```

## Configuration Example

```toml
[[extensions]]
name = "go-backend"
enabled = true
protocol = "grpc"

[extensions.source]
type = "grpc"
endpoint = "http://localhost:50051"

[extensions.permissions]
network = true
filesystem = []
```

## Performance Characteristics

- **Latency**: 1-5ms per call
- **Throughput**: 1,000-10,000 calls/sec per extension
- **Startup**: Instant (service already running)

## Best For

✅ High-performance scenarios  
✅ Strongly-typed contracts  
✅ Streaming data  
✅ Production services  
✅ Cross-language type safety

❌ Simple scripts  
❌ Rapid prototyping (protobuf overhead)

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
