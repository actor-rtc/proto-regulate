//! Package-based protobuf file merging.
//!
//! Merges multiple proto file contents by package name, producing
//! normalized, deduplicated output with semantic fingerprints.

use crate::text_gen::{TextGenerator, TextGeneratorOptions, TEXT_GENERATOR_VERSION};
use anyhow::{anyhow, bail, Context, Result};
use protobuf::descriptor::FileDescriptorProto;
use protobuf_parse::Parser;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tempfile::TempDir;

/// Version of the merge algorithm.
/// Format: "{merge_version}+{text_gen_version}"
pub const MERGE_ALGORITHM_VERSION: &str =
    const_format::formatcp!("1.0.0+{}", TEXT_GENERATOR_VERSION);

/// Result of merging proto files by package.
#[derive(Debug, Clone)]
pub struct MergeResult {
    /// Package name (empty string for files without package declaration)
    pub package_name: String,
    /// Normalized proto content
    pub content: String,
    /// Semantic fingerprint of the content
    pub fingerprint: String,
    /// Non-fatal warnings encountered during merge
    pub warnings: Vec<String>,
}

/// Merges multiple proto file contents by package name.
///
/// # Arguments
///
/// * `files` - Vector of proto file contents (as strings)
///
/// # Returns
///
/// A vector of `MergeResult`, one per unique package, sorted by package name.
///
/// # Errors
///
/// Returns error if:
/// - Any file fails to parse
/// - Duplicate definitions found within the same package
/// - Syntax version conflicts within the same package
/// - Invalid proto content
///
/// # Example
///
/// ```no_run
/// use proto_regulate::merge::merge_by_package;
///
/// let file1 = r#"
///     syntax = "proto3";
///     package foo.bar;
///     message User { string name = 1; }
/// "#;
///
/// let file2 = r#"
///     syntax = "proto3";
///     package foo.bar;
///     message Profile { int32 age = 1; }
/// "#;
///
/// let results = merge_by_package(vec![file1, file2]).unwrap();
/// assert_eq!(results.len(), 1);
/// assert_eq!(results[0].package_name, "foo.bar");
/// ```
pub fn merge_by_package(files: Vec<&str>) -> Result<Vec<MergeResult>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }

    // Step 1: Parse all files
    let parsed_files = parse_all_files(&files)?;

    // Step 2: Group by package
    let grouped = group_by_package(parsed_files)?;

    // Step 3: Merge each package group
    let mut results = Vec::new();
    for (package_name, file_group) in grouped {
        let merge_result = merge_package_group(&package_name, file_group)?;
        results.push(merge_result);
    }

    // Step 4: Sort by package name for deterministic output
    results.sort_by(|a, b| a.package_name.cmp(&b.package_name));

    Ok(results)
}

// ========== Internal Implementation ==========

struct ParsedFile {
    descriptor: FileDescriptorProto,
    #[allow(dead_code)]
    original_content: String,
}

fn parse_all_files(files: &[&str]) -> Result<Vec<ParsedFile>> {
    let mut parsed = Vec::new();

    for (idx, content) in files.iter().enumerate() {
        let descriptor =
            parse_proto_content(content).with_context(|| format!("Failed to parse file #{idx}"))?;

        parsed.push(ParsedFile {
            descriptor,
            original_content: content.to_string(),
        });
    }

    Ok(parsed)
}

fn parse_proto_content(content: &str) -> Result<FileDescriptorProto> {
    // Create temporary directory for parsing
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
    let file_name = "input.proto";
    let temp_path = temp_dir.path().join(file_name);
    std::fs::write(&temp_path, content).context("Failed to write temp file")?;

    // Handle imports by creating dummy files
    create_dummy_imports(content, &temp_dir)?;

    // Parse using protobuf-parse
    let parsed = Parser::new()
        .pure()
        .include(temp_dir.path())
        .input(&temp_path)
        .file_descriptor_set()
        .context("Protobuf parsing failed")?;

    // Find our input file in the parsed results
    let file_descriptor = parsed
        .file
        .into_iter()
        .find(|d| d.name() == file_name)
        .ok_or_else(|| anyhow!("Could not find parsed file descriptor"))?;

    Ok(file_descriptor)
}

fn create_dummy_imports(content: &str, temp_dir: &TempDir) -> Result<()> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("import ") {
            let path_str = trimmed
                .trim_start_matches("import ")
                .trim_start_matches("public ")
                .trim_start_matches("weak ")
                .trim_matches(|c| c == '"' || c == ';')
                .trim();

            // Skip google standard imports
            if !path_str.starts_with("google/protobuf/") {
                let import_path = temp_dir.path().join(path_str);
                if let Some(parent) = import_path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("Failed to create import dir: {path_str}"))?;
                }
                std::fs::write(&import_path, "syntax = \"proto3\";")
                    .with_context(|| format!("Failed to create dummy import: {path_str}"))?;
            }
        }
    }
    Ok(())
}

fn group_by_package(files: Vec<ParsedFile>) -> Result<BTreeMap<String, Vec<ParsedFile>>> {
    let mut groups: BTreeMap<String, Vec<ParsedFile>> = BTreeMap::new();

    for file in files {
        let package = file.descriptor.package.clone().unwrap_or_default();
        groups.entry(package).or_default().push(file);
    }

    Ok(groups)
}

fn merge_package_group(package_name: &str, files: Vec<ParsedFile>) -> Result<MergeResult> {
    let mut warnings = Vec::new();

    // Validate syntax consistency
    let syntax = validate_syntax_consistency(&files, &mut warnings)?;

    // Create merged descriptor
    let mut merged = FileDescriptorProto::new();
    merged.set_syntax(syntax.to_string());
    if !package_name.is_empty() {
        merged.set_package(package_name.to_string());
    }

    // Merge imports (deduplicated and sorted)
    merge_imports(&files, &mut merged);

    // Merge file options (first wins, warn on conflicts)
    merge_file_options(&files, &mut merged, &mut warnings)?;

    // Merge messages (check for duplicates)
    merge_messages(&files, &mut merged)?;

    // Merge enums (check for duplicates)
    merge_enums(&files, &mut merged)?;

    // Merge services (check for duplicates)
    merge_services(&files, &mut merged)?;

    // Merge extensions
    merge_extensions(&files, &mut merged);

    // Generate canonical text using TextGenerator
    let mut generator = TextGenerator::new(TextGeneratorOptions::default());
    let content = generator
        .format_file(&merged)
        .context("Failed to generate canonical text")?;

    // Generate fingerprint
    let fingerprint =
        crate::generate_fingerprint(&content).context("Failed to generate fingerprint")?;

    Ok(MergeResult {
        package_name: package_name.to_string(),
        content,
        fingerprint,
        warnings,
    })
}

fn validate_syntax_consistency<'a>(
    files: &'a [ParsedFile],
    _warnings: &mut [String],
) -> Result<&'a str> {
    let mut syntaxes = BTreeSet::new();

    for file in files {
        let syntax = file.descriptor.syntax.as_deref().unwrap_or("proto2");
        syntaxes.insert(syntax);
    }

    if syntaxes.len() > 1 {
        bail!(
            "Syntax version conflict: found {syntaxes:?}. All files in the same package must use the same syntax version."
        );
    }

    Ok(syntaxes.into_iter().next().unwrap_or("proto2"))
}

fn merge_imports(files: &[ParsedFile], merged: &mut FileDescriptorProto) {
    let mut all_imports = BTreeSet::new();
    let mut public_imports = BTreeSet::new();
    let mut weak_imports = BTreeSet::new();

    for file in files {
        // Collect all imports
        for dep in file.descriptor.dependency.iter() {
            all_imports.insert(dep.clone());
        }

        // Track public imports
        for &idx in file.descriptor.public_dependency.iter() {
            if let Some(dep) = file.descriptor.dependency.get(idx as usize) {
                public_imports.insert(dep.clone());
            }
        }

        // Track weak imports
        for &idx in file.descriptor.weak_dependency.iter() {
            if let Some(dep) = file.descriptor.dependency.get(idx as usize) {
                weak_imports.insert(dep.clone());
            }
        }
    }

    // Build merged import lists
    let imports: Vec<_> = all_imports.into_iter().collect();
    merged.dependency = imports.clone();

    // Build index maps for public and weak
    for (idx, dep) in imports.iter().enumerate() {
        if public_imports.contains(dep) {
            merged.public_dependency.push(idx as i32);
        }
        if weak_imports.contains(dep) {
            merged.weak_dependency.push(idx as i32);
        }
    }
}

fn merge_file_options(
    files: &[ParsedFile],
    merged: &mut FileDescriptorProto,
    warnings: &mut Vec<String>,
) -> Result<()> {
    // Use first file's options as base
    if let Some(first) = files.first() {
        if let Some(opts) = first.descriptor.options.as_ref() {
            merged.options = protobuf::MessageField::some(opts.clone());
        }
    }

    // Check for conflicts in subsequent files
    for (idx, file) in files.iter().enumerate().skip(1) {
        if let Some(opts) = file.descriptor.options.as_ref() {
            if let Some(merged_opts) = merged.options.as_ref() {
                // Compare key options
                if opts.java_package != merged_opts.java_package && opts.java_package.is_some() {
                    warnings.push(format!(
                        "File #{idx}: java_package option conflict (using first occurrence)"
                    ));
                }
                if opts.go_package != merged_opts.go_package && opts.go_package.is_some() {
                    warnings.push(format!(
                        "File #{idx}: go_package option conflict (using first occurrence)"
                    ));
                }
            }
        }
    }

    Ok(())
}

fn merge_messages(files: &[ParsedFile], merged: &mut FileDescriptorProto) -> Result<()> {
    let mut seen_names = HashMap::new();
    let mut all_messages = Vec::new();

    for (file_idx, file) in files.iter().enumerate() {
        for message in file.descriptor.message_type.iter() {
            let name = message.name();

            // Check for duplicates
            if let Some(&prev_idx) = seen_names.get(name) {
                bail!("Duplicate message '{name}' found in files #{prev_idx} and #{file_idx}");
            }

            seen_names.insert(name.to_string(), file_idx);
            all_messages.push(message.clone());
        }
    }

    // Sort by name for determinism
    all_messages.sort_by(|a, b| a.name().cmp(b.name()));
    merged.message_type = all_messages;

    Ok(())
}

fn merge_enums(files: &[ParsedFile], merged: &mut FileDescriptorProto) -> Result<()> {
    let mut seen_names = HashMap::new();
    let mut all_enums = Vec::new();

    for (file_idx, file) in files.iter().enumerate() {
        for enum_type in file.descriptor.enum_type.iter() {
            let name = enum_type.name();

            // Check for duplicates
            if let Some(&prev_idx) = seen_names.get(name) {
                bail!("Duplicate enum '{name}' found in files #{prev_idx} and #{file_idx}");
            }

            seen_names.insert(name.to_string(), file_idx);
            all_enums.push(enum_type.clone());
        }
    }

    // Sort by name for determinism
    all_enums.sort_by(|a, b| a.name().cmp(b.name()));
    merged.enum_type = all_enums;

    Ok(())
}

fn merge_services(files: &[ParsedFile], merged: &mut FileDescriptorProto) -> Result<()> {
    let mut seen_names = HashMap::new();
    let mut all_services = Vec::new();

    for (file_idx, file) in files.iter().enumerate() {
        for service in file.descriptor.service.iter() {
            let name = service.name();

            // Check for duplicates
            if let Some(&prev_idx) = seen_names.get(name) {
                bail!("Duplicate service '{name}' found in files #{prev_idx} and #{file_idx}");
            }

            seen_names.insert(name.to_string(), file_idx);
            all_services.push(service.clone());
        }
    }

    // Sort by name for determinism
    all_services.sort_by(|a, b| a.name().cmp(b.name()));
    merged.service = all_services;

    Ok(())
}

fn merge_extensions(files: &[ParsedFile], merged: &mut FileDescriptorProto) {
    let mut all_extensions = Vec::new();

    for file in files {
        for extension in file.descriptor.extension.iter() {
            all_extensions.push(extension.clone());
        }
    }

    // Sort by extendee and then by field number
    all_extensions.sort_by(|a, b| {
        a.extendee
            .cmp(&b.extendee)
            .then_with(|| a.number().cmp(&b.number()))
    });

    merged.extension = all_extensions;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_algorithm_version() {
        assert!(MERGE_ALGORITHM_VERSION.starts_with("1.0.0+"));
        assert!(MERGE_ALGORITHM_VERSION.contains(TEXT_GENERATOR_VERSION));
    }

    #[test]
    fn test_empty_input() {
        let result = merge_by_package(vec![]).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_single_file() {
        let proto = r#"
syntax = "proto3";
package test;

message User {
  string name = 1;
}
"#;

        let results = merge_by_package(vec![proto]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].package_name, "test");
        assert!(results[0].content.contains("message User"));
        assert!(!results[0].fingerprint.is_empty());
    }

    #[test]
    fn test_merge_same_package() {
        let file1 = r#"
syntax = "proto3";
package foo;

message User {
  string name = 1;
}
"#;

        let file2 = r#"
syntax = "proto3";
package foo;

message Profile {
  int32 age = 1;
}
"#;

        let results = merge_by_package(vec![file1, file2]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].package_name, "foo");
        assert!(results[0].content.contains("message User"));
        assert!(results[0].content.contains("message Profile"));
    }

    #[test]
    fn test_multiple_packages() {
        let file1 = r#"
syntax = "proto3";
package foo;

message Foo {}
"#;

        let file2 = r#"
syntax = "proto3";
package bar;

message Bar {}
"#;

        let results = merge_by_package(vec![file1, file2]).unwrap();
        assert_eq!(results.len(), 2);

        // Should be sorted by package name
        assert_eq!(results[0].package_name, "bar");
        assert_eq!(results[1].package_name, "foo");
    }

    #[test]
    fn test_duplicate_message_error() {
        let file1 = r#"
syntax = "proto3";
package test;

message User {
  string name = 1;
}
"#;

        let file2 = r#"
syntax = "proto3";
package test;

message User {
  string email = 1;
}
"#;

        let result = merge_by_package(vec![file1, file2]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Duplicate message 'User'"));
    }

    #[test]
    fn test_syntax_conflict_error() {
        let file1 = r#"
syntax = "proto2";
package test;

message Foo {}
"#;

        let file2 = r#"
syntax = "proto3";
package test;

message Bar {}
"#;

        let result = merge_by_package(vec![file1, file2]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Syntax version conflict"));
    }

    #[test]
    fn test_empty_package() {
        let proto = r#"
syntax = "proto3";

message Orphan {
  string data = 1;
}
"#;

        let results = merge_by_package(vec![proto]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].package_name, "");
    }

    #[test]
    fn test_deterministic_output() {
        let file1 = r#"
syntax = "proto3";
package test;

message B {}
message A {}
"#;

        let file2 = r#"
syntax = "proto3";
package test;

message A {}
message B {}
"#;

        // Run twice with different input order
        let results1 = merge_by_package(vec![file1]).unwrap();
        let results2 = merge_by_package(vec![file2]).unwrap();

        // Content should be identical (sorted)
        assert_eq!(results1[0].content, results2[0].content);
        assert_eq!(results1[0].fingerprint, results2[0].fingerprint);
    }
}
