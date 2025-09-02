use clap::{Arg, Command};
use colored::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use terminal_size::{Width, terminal_size};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirEntry {
    name: String,
    size: u64,
    is_dir: bool,
    path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DirReport {
    total_size: u64,
    entries: Vec<DirEntry>,
    path: String,
}

fn main() {
    let matches = Command::new("yp")
        .name("YP - 目录空间查看器")
        .version("0.1.2")
        .author("Your Name")
        .about("一个功能强大的目录空间占用查看工具")
        .arg(
            Arg::new("path")
                .short('p')
                .long("path")
                .value_name("PATH")
                .help("指定要分析的目录路径")
                .default_value("."),
        )
        .arg(
            Arg::new("sort")
                .short('s')
                .long("sort")
                .help("按大小排序显示")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("json")
                .short('j')
                .long("json")
                .help("以JSON格式输出")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("chart")
                .short('c')
                .long("chart")
                .help("显示ASCII艺术风格条形图")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("recursive")
                .short('r')
                .long("recursive")
                .help("递归显示所有子目录")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("summary")
                .short('S')
                .long("summary")
                .help("只显示目录和总大小，不显示详细内容")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    let path = matches.get_one::<String>("path").unwrap();
    let sort_by_size = matches.get_flag("sort");
    let json_output = matches.get_flag("json");
    let show_chart = matches.get_flag("chart");
    let recursive = matches.get_flag("recursive");
    let summary_only = matches.get_flag("summary");

    match analyze_directory(path, recursive) {
        Ok(mut report) => {
            if sort_by_size {
                report.entries.sort_by(|a, b| b.size.cmp(&a.size));
            }

            if json_output {
                if summary_only {
                    output_json_summary(&report);
                } else {
                    output_json(&report);
                }
            } else if summary_only {
                output_summary(&report);
            } else {
                output_text(&report, show_chart);
            }
        }
        Err(e) => {
            eprintln!("{} 分析目录时出错: {}", "错误:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

fn analyze_directory(path: &str, recursive: bool) -> Result<DirReport, Box<dyn std::error::Error>> {
    let path = Path::new(path);
    if !path.exists() {
        return Err(format!("路径不存在: {}", path.display()).into());
    }

    let mut entries = Vec::new();
    let mut total_size = 0u64;

    if recursive {
        // 递归遍历所有文件和目录
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            if entry.path() == path {
                continue; // 跳过根目录本身
            }

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let size = if metadata.is_file() {
                metadata.len()
            } else {
                calculate_dir_size(entry.path())?
            };

            total_size += if metadata.is_file() { size } else { 0 };

            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                size,
                is_dir: metadata.is_dir(),
                path: entry.path().to_string_lossy().to_string(),
            });
        }
    } else {
        // 只分析当前目录的直接子项
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            let name = entry.file_name().to_string_lossy().to_string();

            let size = if metadata.is_file() {
                metadata.len()
            } else {
                calculate_dir_size(&entry.path())?
            };

            total_size += size;

            entries.push(DirEntry {
                name,
                size,
                is_dir: metadata.is_dir(),
                path: entry.path().to_string_lossy().to_string(),
            });
        }
    }

    Ok(DirReport {
        total_size,
        entries,
        path: path.to_string_lossy().to_string(),
    })
}

fn calculate_dir_size(path: &Path) -> Result<u64, Box<dyn std::error::Error>> {
    let mut size = 0u64;

    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        if let Ok(metadata) = entry.metadata()
            && metadata.is_file()
        {
            size += metadata.len();
        }
    }

    Ok(size)
}

fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = size as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

fn truncate_filename(name: &str, max_width: usize) -> String {
    if name.chars().count() <= max_width {
        name.to_string()
    } else if max_width <= 3 {
        "...".to_string()
    } else {
        let mut result = String::new();
        let chars: Vec<char> = name.chars().collect();
        let available = max_width - 3; // 为"..."保留空间

        // 尝试保留开头和结尾
        if available >= 6 {
            let start_chars = available / 2;
            let end_chars = available - start_chars;

            for &ch in &chars[..start_chars.min(chars.len())] {
                result.push(ch);
            }
            result.push_str("...");
            if chars.len() > start_chars {
                let start_pos = chars.len().saturating_sub(end_chars);
                for &ch in &chars[start_pos..] {
                    result.push(ch);
                }
            }
        } else {
            // 如果空间太小，只保留开头部分
            for &ch in &chars[..available.min(chars.len())] {
                result.push(ch);
            }
            result.push_str("...");
        }

        result
    }
}

fn get_terminal_width() -> usize {
    if let Some((Width(w), _)) = terminal_size() {
        w as usize
    } else {
        80 // 默认宽度
    }
}

fn output_json(report: &DirReport) {
    match serde_json::to_string_pretty(report) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("JSON序列化错误: {}", e),
    }
}

fn output_json_summary(report: &DirReport) {
    let summary = serde_json::json!({
        "path": report.path,
        "total_size": report.total_size,
        "file_count": report.entries.len()
    });

    match serde_json::to_string_pretty(&summary) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("JSON序列化错误: {}", e),
    }
}

fn output_summary(report: &DirReport) {
    let terminal_width = get_terminal_width();
    let display_width = terminal_width.min(80); // 限制显示宽度

    println!("{}", "═".repeat(display_width).cyan().bold());
    println!("{} {}", "目录:".green().bold(), report.path.yellow());
    println!(
        "{} {}",
        "总大小:".green().bold(),
        format_size(report.total_size).cyan().bold()
    );
    println!(
        "{} {} 个项目",
        "项目数:".green().bold(),
        report.entries.len().to_string().yellow().bold()
    );
    println!("{}", "═".repeat(display_width).cyan().bold());
}

fn output_text(report: &DirReport, show_chart: bool) {
    let terminal_width = get_terminal_width();
    let display_width = terminal_width.min(80); // 限制显示宽度

    println!("{}", "═".repeat(display_width).cyan().bold());
    println!("{} {}", "目录:".green().bold(), report.path.yellow());
    println!(
        "{} {}",
        "总大小:".green().bold(),
        format_size(report.total_size).cyan().bold()
    );
    println!("{}", "═".repeat(display_width).cyan().bold());

    if report.entries.is_empty() {
        println!("{}", "目录为空".yellow());
        return;
    }

    // 计算布局参数
    let size_width = 12; // 大小列的宽度
    let chart_width = if show_chart { 42 } else { 0 }; // 条形图宽度 [40个字符 + 2个括号]
    let icon_width = 3; // emoji + 空格
    let spacing = 2; // 列之间的间距

    // 计算文件名可用宽度（使用显示宽度而不是终端宽度）
    let used_width = icon_width + size_width + chart_width + spacing * 2;
    let filename_width = if display_width > used_width + 10 {
        display_width - used_width
    } else {
        30 // 最小宽度
    };

    // 计算最大大小用于条形图
    let max_size = report.entries.iter().map(|e| e.size).max().unwrap_or(1);

    for entry in &report.entries {
        let size_str = format_size(entry.size);

        let type_icon = if entry.is_dir { "📁" } else { "📄" };

        // 截断文件名以适应可用空间
        let truncated_name = truncate_filename(&entry.name, filename_width);
        let colored_name = if entry.is_dir {
            truncated_name.blue().bold()
        } else {
            truncated_name.white()
        };

        if show_chart {
            let bar_length = if max_size > 0 {
                ((entry.size as f64 / max_size as f64) * 40.0) as usize
            } else {
                0
            };

            let bar = "█".repeat(bar_length);
            let bar_colored = if entry.is_dir {
                bar.blue()
            } else {
                bar.green()
            };

            println!(
                "{} {:<width$} {:>12} [{}{}]",
                type_icon,
                colored_name,
                size_str.cyan(),
                bar_colored,
                " ".repeat(40 - bar_length),
                width = filename_width
            );
        } else {
            println!(
                "{} {:<width$} {:>12}",
                type_icon,
                colored_name,
                size_str.cyan(),
                width = filename_width
            );
        }
    }

    println!("{}", "═".repeat(display_width).cyan().bold());
    println!(
        "{} {} 个项目",
        "共计:".green().bold(),
        report.entries.len().to_string().yellow().bold()
    );
}
