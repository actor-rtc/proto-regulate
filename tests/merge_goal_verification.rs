//! 验证项目初衷目标：fn(Vec<String>) -> Result<Vec<(String, String, String)>>
//! 将多个 proto 文本转化为按 package 分组的 (package, content, fingerprint)

use proto_regulate::merge::merge_by_package;

#[test]
fn test_merge_goal_basic() {
    // 输入：多个 proto 文本
    let file1 = r#"syntax = "proto3";
package foo.bar;
message User { string name = 1; }"#;

    let file2 = r#"syntax = "proto3";
package foo.bar;
message Profile { int32 age = 1; }"#;

    let file3 = r#"syntax = "proto3";
package baz;
message Product { string id = 1; }"#;

    // 调用核心函数
    let results = merge_by_package(vec![file1, file2, file3]).unwrap();

    // 验证结果
    assert_eq!(results.len(), 2, "应该有 2 个 package");

    // 验证第一个 package (baz)
    assert_eq!(results[0].package_name, "baz");
    assert!(results[0].content.contains("message Product"));
    assert!(!results[0].fingerprint.is_empty());

    // 验证第二个 package (foo.bar)
    assert_eq!(results[1].package_name, "foo.bar");
    assert!(results[1].content.contains("message User"));
    assert!(results[1].content.contains("message Profile"));
    assert!(!results[1].fingerprint.is_empty());

    println!("\n✅ 核心功能验证通过！");
    println!("   输入: Vec<&str> (可以从 Vec<String> 转换)");
    println!("   输出: Vec<MergeResult>");
}

#[test]
fn test_convert_to_tuple_format() {
    // 验证可以轻松转换为元组格式
    let file1 = r#"syntax = "proto3";
package test;
message Foo { string bar = 1; }"#;

    let results = merge_by_package(vec![file1]).unwrap();

    // 转换为用户期望的元组格式: Vec<(String, String, String)>
    let tuple_format: Vec<(String, String, String)> = results
        .into_iter()
        .map(|r| (r.package_name, r.content, r.fingerprint))
        .collect();

    assert_eq!(tuple_format.len(), 1);
    assert_eq!(tuple_format[0].0, "test"); // package_name
    assert!(!tuple_format[0].1.is_empty()); // content
    assert!(!tuple_format[0].2.is_empty()); // fingerprint

    println!("\n✅ 元组格式转换验证通过！");
    println!("   格式: Vec<(String, String, String)>");
    println!("   内容: (package_name, content, fingerprint)");
}

#[test]
fn test_merge_same_package_different_messages() {
    // 同一个 package 下的多个文件应该合并
    let file1 = r#"syntax = "proto3";
package api.v1;
message Request { string id = 1; }"#;

    let file2 = r#"syntax = "proto3";
package api.v1;
message Response { int32 code = 1; }"#;

    let file3 = r#"syntax = "proto3";
package api.v1;
enum Status { UNKNOWN = 0; OK = 1; }"#;

    let results = merge_by_package(vec![file1, file2, file3]).unwrap();

    // 应该只有一个 package
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].package_name, "api.v1");

    // 所有定义都应该在合并后的内容中
    let content = &results[0].content;
    assert!(content.contains("message Request"));
    assert!(content.contains("message Response"));
    assert!(content.contains("enum Status"));

    println!("\n✅ 同 package 合并验证通过！");
    println!("   输入: 3 个文件 (相同 package)");
    println!("   输出: 1 个合并结果");
}

#[test]
fn test_fingerprint_consistency() {
    // 验证相同内容产生相同 fingerprint
    let file1 = r#"syntax = "proto3";
package test;
message Msg { string field = 1; }"#;

    let results1 = merge_by_package(vec![file1]).unwrap();
    let results2 = merge_by_package(vec![file1]).unwrap();

    assert_eq!(results1[0].fingerprint, results2[0].fingerprint);

    println!("\n✅ Fingerprint 一致性验证通过！");
}

#[test]
fn test_wrapper_function() {
    // 演示如何包装成用户期望的签名
    fn merge_to_tuples(
        files: Vec<String>,
    ) -> anyhow::Result<Vec<(String, String, String)>> {
        let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
        let results = merge_by_package(file_refs)?;
        Ok(results
            .into_iter()
            .map(|r| (r.package_name, r.content, r.fingerprint))
            .collect())
    }

    // 测试包装函数
    let files = vec![
        r#"syntax = "proto3"; package a; message A {}"#.to_string(),
        r#"syntax = "proto3"; package b; message B {}"#.to_string(),
    ];

    let tuples = merge_to_tuples(files).unwrap();
    assert_eq!(tuples.len(), 2);

    println!("\n✅ 包装函数验证通过！");
    println!("   签名: fn(Vec<String>) -> Result<Vec<(String, String, String)>>");
}
