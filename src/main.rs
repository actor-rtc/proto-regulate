//! Proto-regulate CLI tool for debugging and testing

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use log::{debug, error, info, warn};
use proto_regulate::{descriptor_to_proto, merge_by_package, parse_proto_to_file_descriptor};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "proto-regulate")]
#[command(about = "Protobuf file normalization and debugging tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Normalize proto file(s)
    /// - File mode: normalize a single proto file
    /// - Directory mode: merge all proto files by package and split output
    Normalize {
        /// Input path (file or directory)
        #[arg(value_name = "PATH")]
        input: PathBuf,

        /// Output directory (required for directory mode)
        #[arg(short, long, value_name = "DIR")]
        output: Option<PathBuf>,
    },

    /// Inspect proto file descriptor (output JSON format)
    Inspect {
        /// Proto file path
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    // 初始化日志
    env_logger::Builder::from_default_env()
        .filter_level(if cli.verbose {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .init();

    // 执行命令
    if let Err(e) = run(cli) {
        error!("执行失败: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Normalize { input, output } => {
            if input.is_file() {
                debug!("文件模式: 规范化单个文件");
                normalize_file(&input, output.as_deref())
            } else if input.is_dir() {
                debug!("目录模式: 合并并分拆 proto 文件");
                normalize_directory(&input, output.as_deref())
            } else {
                bail!("输入路径不存在或无效: {}", input.display());
            }
        }
        Commands::Inspect { file } => inspect_file(&file),
    }
}

/// 规范化单个文件
fn normalize_file(input: &Path, output: Option<&Path>) -> Result<()> {
    info!("读取文件: {}", input.display());
    let content = fs::read_to_string(input).context("读取输入文件失败")?;

    debug!("解析 proto 文件");
    let descriptor = parse_proto_to_file_descriptor(&content).context("解析 proto 文件失败")?;

    debug!("生成规范化内容");
    let normalized = descriptor_to_proto(&descriptor).context("生成规范化内容失败")?;

    if let Some(output_path) = output {
        info!("写入输出文件: {}", output_path.display());
        fs::write(output_path, normalized).context("写入输出文件失败")?;
        info!("规范化完成");
    } else {
        println!("{normalized}");
    }

    Ok(())
}

/// 规范化目录（合并后分拆）
fn normalize_directory(input: &Path, output: Option<&Path>) -> Result<()> {
    let output_dir = output.context("目录模式需要指定 --output 参数")?;

    info!("扫描目录: {}", input.display());
    let proto_files = collect_proto_files(input)?;

    if proto_files.is_empty() {
        warn!("目录中没有找到 .proto 文件");
        return Ok(());
    }

    info!("找到 {} 个 proto 文件", proto_files.len());

    // 读取所有文件内容
    let mut contents = Vec::new();
    for file in &proto_files {
        debug!("读取文件: {}", file.display());
        let content = fs::read_to_string(file)
            .with_context(|| format!("读取文件失败: {}", file.display()))?;
        contents.push(content);
    }

    // 按 package 合并
    info!("按 package 合并文件");
    let results =
        merge_by_package(contents.iter().map(|s| s.as_str()).collect()).context("合并文件失败")?;

    info!("生成 {} 个合并后的 package", results.len());

    // 创建输出目录
    fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;

    // 写入分拆后的文件
    for result in results {
        let package_name = if result.package_name.is_empty() {
            "default".to_string()
        } else {
            result.package_name.clone()
        };

        let output_file = output_dir.join(format!("{}.proto", package_name.replace('.', "_")));
        info!(
            "写入 package '{}' 到文件: {}",
            result.package_name,
            output_file.display()
        );

        fs::write(&output_file, &result.content)
            .with_context(|| format!("写入文件失败: {}", output_file.display()))?;

        debug!("指纹: {}", result.fingerprint);
    }

    info!("目录规范化完成");
    Ok(())
}

/// 收集目录中的所有 .proto 文件
fn collect_proto_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut proto_files = Vec::new();

    for entry in fs::read_dir(dir).context("读取目录失败")? {
        let entry = entry.context("读取目录项失败")?;
        let path = entry.path();

        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("proto") {
            proto_files.push(path);
        }
    }

    proto_files.sort();
    Ok(proto_files)
}

/// 查看文件 descriptor
fn inspect_file(file: &Path) -> Result<()> {
    info!("读取文件: {}", file.display());
    let content = fs::read_to_string(file).context("读取文件失败")?;

    debug!("解析 proto 文件");
    let descriptor = parse_proto_to_file_descriptor(&content).context("解析 proto 文件失败")?;

    debug!("输出 descriptor 详细信息");
    println!("{descriptor:#?}");
    Ok(())
}
