//! CLI integration tests

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn get_binary_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push("proto-regulate");
    path
}

#[test]
fn test_cli_normalize_file_mode() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.proto");
    let output_file = temp_dir.path().join("output.proto");

    // 创建测试输入文件
    let proto_content = r#"
syntax = "proto3";
package test;

message User {
  string name = 1;
  int32 age = 2;
}
"#;
    fs::write(&input_file, proto_content).unwrap();

    // 运行 CLI
    let output = Command::new(get_binary_path())
        .arg("normalize")
        .arg(&input_file)
        .arg("-o")
        .arg(&output_file)
        .output()
        .expect("Failed to execute CLI");

    assert!(output.status.success(), "CLI failed: {output:?}");

    // 验证输出文件存在并包含预期内容
    let result = fs::read_to_string(&output_file).unwrap();
    assert!(result.contains("syntax = \"proto3\";"));
    assert!(result.contains("package test;"));
    assert!(result.contains("message User"));
}

#[test]
fn test_cli_normalize_stdout() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.proto");

    let proto_content = r#"
syntax = "proto3";
message Test { string field = 1; }
"#;
    fs::write(&input_file, proto_content).unwrap();

    // 运行 CLI，输出到 stdout
    let output = Command::new(get_binary_path())
        .arg("normalize")
        .arg(&input_file)
        .output()
        .expect("Failed to execute CLI");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("syntax = \"proto3\";"));
    assert!(stdout.contains("message Test"));
}

#[test]
fn test_cli_normalize_directory_mode() {
    let input_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();

    // 创建多个测试文件
    fs::write(
        input_dir.path().join("file1.proto"),
        r#"
syntax = "proto3";
package foo;
message User { string name = 1; }
"#,
    )
    .unwrap();

    fs::write(
        input_dir.path().join("file2.proto"),
        r#"
syntax = "proto3";
package foo;
message Profile { int32 age = 1; }
"#,
    )
    .unwrap();

    fs::write(
        input_dir.path().join("file3.proto"),
        r#"
syntax = "proto3";
package bar;
message Settings { bool enabled = 1; }
"#,
    )
    .unwrap();

    // 运行 CLI
    let output = Command::new(get_binary_path())
        .arg("normalize")
        .arg(input_dir.path())
        .arg("-o")
        .arg(output_dir.path())
        .output()
        .expect("Failed to execute CLI");

    assert!(output.status.success(), "CLI failed: {output:?}");

    // 验证输出文件
    let foo_file = output_dir.path().join("foo.proto");
    let bar_file = output_dir.path().join("bar.proto");

    assert!(foo_file.exists(), "foo.proto should exist");
    assert!(bar_file.exists(), "bar.proto should exist");

    let foo_content = fs::read_to_string(&foo_file).unwrap();
    assert!(foo_content.contains("package foo;"));
    assert!(foo_content.contains("message User"));
    assert!(foo_content.contains("message Profile"));

    let bar_content = fs::read_to_string(&bar_file).unwrap();
    assert!(bar_content.contains("package bar;"));
    assert!(bar_content.contains("message Settings"));
}

#[test]
fn test_cli_inspect_command() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.proto");

    let proto_content = r#"
syntax = "proto3";
package test;
message User { string name = 1; }
"#;
    fs::write(&input_file, proto_content).unwrap();

    // 运行 inspect 命令
    let output = Command::new(get_binary_path())
        .arg("inspect")
        .arg(&input_file)
        .output()
        .expect("Failed to execute CLI");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    // 验证 Debug 格式的 descriptor 输出
    assert!(stdout.contains("FileDescriptorProto"));
    assert!(stdout.contains("package"));
    assert!(stdout.contains("message_type"));
}

#[test]
fn test_cli_error_handling_missing_file() {
    let output = Command::new(get_binary_path())
        .arg("normalize")
        .arg("/nonexistent/file.proto")
        .output()
        .expect("Failed to execute CLI");

    assert!(!output.status.success(), "Should fail for missing file");
}

#[test]
fn test_cli_error_handling_directory_without_output() {
    let temp_dir = TempDir::new().unwrap();

    let output = Command::new(get_binary_path())
        .arg("normalize")
        .arg(temp_dir.path())
        .output()
        .expect("Failed to execute CLI");

    assert!(
        !output.status.success(),
        "Should fail when directory mode lacks -o flag"
    );
}

#[test]
fn test_cli_verbose_flag() {
    let temp_dir = TempDir::new().unwrap();
    let input_file = temp_dir.path().join("input.proto");

    let proto_content = r#"
syntax = "proto3";
message Test { string field = 1; }
"#;
    fs::write(&input_file, proto_content).unwrap();

    // 运行 CLI 带 verbose 标志
    let output = Command::new(get_binary_path())
        .arg("-v")
        .arg("normalize")
        .arg(&input_file)
        .output()
        .expect("Failed to execute CLI");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).unwrap();
    // 验证有 DEBUG 级别日志
    assert!(stderr.contains("DEBUG") || stderr.contains("读取文件"));
}
