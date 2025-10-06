//! Proto-regulate: Protobuf file normalization, merging and formatting tools
//!
//! This library provides tools for:
//! - Merging multiple proto files by package
//! - Normalizing proto file formatting
//! - Generating semantic fingerprints
//! - Converting descriptors to proto text

pub mod merge;
pub mod text_gen;

// Re-export main types
pub use merge::{merge_by_package, MergeResult};
pub use text_gen::{descriptor_to_proto, TextGenerator, TextGeneratorOptions};

use anyhow::{Context, Result};
use protobuf::descriptor::FileDescriptorProto;
use protobuf_parse::Parser;
use sha2::{Digest, Sha256};

/// Parse proto content string into FileDescriptorProto.
pub fn parse_proto_to_file_descriptor(proto_content: &str) -> Result<FileDescriptorProto> {
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
    let file_name = "input.proto";
    let temp_path = temp_dir.path().join(file_name);
    std::fs::write(&temp_path, proto_content).context("Failed to write temp file")?;

    // Create dummy imports
    for line in proto_content.lines() {
        if line.trim().starts_with("import ") {
            let path_str = line
                .trim()
                .trim_start_matches("import ")
                .trim_start_matches("public ")
                .trim_start_matches("weak ")
                .trim_matches(|c| c == '"' || c == ';')
                .trim();

            if !path_str.starts_with("google/protobuf/") {
                let import_path = temp_dir.path().join(path_str);
                if let Some(parent) = import_path.parent() {
                    std::fs::create_dir_all(parent).context("Failed to create import dirs")?;
                }
                std::fs::write(&import_path, "syntax = \"proto3\";")
                    .context("Failed to create dummy import")?;
            }
        }
    }

    // Parse
    let parsed = Parser::new()
        .pure()
        .include(temp_dir.path())
        .input(&temp_path)
        .file_descriptor_set()
        .context("Protobuf parsing failed")?;

    let file_descriptor = parsed
        .file
        .into_iter()
        .find(|d| d.name() == file_name)
        .context("Could not find the parsed file descriptor")?;

    Ok(file_descriptor)
}

/// Generate semantic fingerprint for proto content.
pub fn generate_fingerprint(proto_content: &str) -> Result<String> {
    let descriptor = parse_proto_to_file_descriptor(proto_content)?;
    let normalized = text_gen::descriptor_to_proto(&descriptor)?;

    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let hash_result = hasher.finalize();

    Ok(format!("{hash_result:x}"))
}
