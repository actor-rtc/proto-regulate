# proto-regulate

Protobuf file normalization, merging and formatting tools.

## Features

- **Package-based Merging**: Merge multiple proto files by package name
- **Normalization**: Convert proto files to canonical format
- **Fingerprinting**: Generate semantic fingerprints for proto files
- **Descriptor Rendering**: Convert FileDescriptorProto back to proto text

## Usage

### Merge proto files by package

```rust
use proto_regulate::merge_by_package;

let file1 = r#"
    syntax = "proto3";
    package foo.bar;
    message User { string name = 1; }
"#;

let file2 = r#"
    syntax = "proto3";
    package foo.bar;
    message Profile { int32 age = 1; }
"#;

let results = merge_by_package(vec![file1, file2])?;
for result in results {
    println!("Package: {}", result.package_name);
    println!("Content:\n{}", result.content);
    println!("Fingerprint: {}", result.fingerprint);
}
```

### Convert descriptor to proto text

```rust
use proto_regulate::{parse_proto_to_file_descriptor, descriptor_to_proto};

let proto_content = r#"
    syntax = "proto3";
    message Test { string field = 1; }
"#;

let descriptor = parse_proto_to_file_descriptor(proto_content)?;
let normalized = descriptor_to_proto(&descriptor)?;
println!("{}", normalized);
```

## License

Apache-2.0
