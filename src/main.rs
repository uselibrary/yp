// src/main.rs
use clap::{Arg, Command};
use colored::*;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use terminal_size::{Width, terminal_size};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
        .version("0.2.2")
        .author("Your Name")
        .about("一个高性能的目录空间占用查看工具（并行、一次遍历聚合）")
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
                .help("按大小从大到小排序显示")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("json")
                .short('j')
                .long("json")
                .help("以 JSON 格式输出")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("chart")
                .short('c')
                .long("chart")
                .help("显示 ASCII 条形图")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("recursive")
                .short('r')
                .long("recursive")
                .help("递归显示所有子目录（列出整棵子树）")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("summary")
                .short('S')
                .long("summary")
                .help("只显示目录/总大小/项目数，不显示详细条目")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("exclude")
                .short('e')
                .long("exclude")
                .value_name("PATTERN")
                .help("排除指定的文件或文件夹（可多次使用）")
                .action(clap::ArgAction::Append),
        )
        .get_matches();

    let path = matches.get_one::<String>("path").unwrap();
    let sort_by_size = matches.get_flag("sort");
    let json_output = matches.get_flag("json");
    let show_chart = matches.get_flag("chart");
    let recursive = matches.get_flag("recursive");
    let summary_only = matches.get_flag("summary");
    let excludes: Vec<String> = matches
        .get_many::<String>("exclude")
        .map(|vals| vals.map(|s| s.to_string()).collect())
        .unwrap_or_default();

    match analyze_directory(path, recursive, &excludes) {
        Ok(mut report) => {
            if sort_by_size {
                report
                    .entries
                    .sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
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

/// 检查路径是否应该被排除
/// depth: 当前深度（0 表示根目录的直接子项）
/// name: 文件/目录名称
/// excludes: 排除模式列表
/// only_root: 如果为 true，只在根目录（depth=0）应用排除规则
fn should_exclude(depth: usize, name: &str, excludes: &[String], only_root: bool) -> bool {
    if only_root && depth > 0 {
        return false;
    }
    excludes.iter().any(|pattern| name == pattern)
}

/// 顶层分析入口：
/// - 递归模式：一次并行遍历整棵子树，返回所有条目（含文件与目录），并聚合 total_size（仅文件之和）
/// - 非递归模式：并行处理"直接子项"；
///    - 文件：直接读取大小；
///    - 目录：并行计算其子树大小，但不展开其子项到 entries（只返回该目录一条记录）。
fn analyze_directory(
    path: &str,
    recursive: bool,
    excludes: &[String],
) -> Result<DirReport, Box<dyn std::error::Error>> {
    let root = Path::new(path);
    if !root.exists() {
        return Err(format!("路径不存在: {}", root.display()).into());
    }
    if !root.is_dir() {
        // 与原语义保持一致：允许给文件路径，输出一个“父目录”的报告也不直观；
        // 这里直接把该文件作为单条记录返回。
        let meta = fs::metadata(root)?;
        let size = meta.len();
        let entry = DirEntry {
            name: root
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| root.display().to_string()),
            size,
            is_dir: false,
            path: root.to_string_lossy().to_string(),
        };
        return Ok(DirReport {
            total_size: size,
            entries: vec![entry],
            path: root.to_string_lossy().to_string(),
        });
    }

    if recursive {
        // 并行递归：一次遍历，返回所有条目（不包含根目录本身）
        // depth=0 表示根目录的直接子项
        let (_, entries) = scan_dir_recursive(root, excludes, 0)?;
        let total_size = entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
        Ok(DirReport {
            total_size,
            entries,
            path: root.to_string_lossy().to_string(),
        })
    } else {
        // 非递归：并行处理直接子项；目录大小为其整个子树大小，但不展开其子节点到 entries
        let read_dir = fs::read_dir(root)?;
        let items: Vec<_> = read_dir.collect();
        let items: Vec<_> = items
            .into_par_iter()
            .filter_map(|res| res.ok())
            .filter_map(|entry| {
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                // 检查是否应该排除（depth=0 表示根目录的直接子项，only_root=true 只排除根目录下的项）
                if should_exclude(0, &name, excludes, true) {
                    return None;
                }

                match entry.metadata() {
                    Ok(m) => {
                        if m.is_file() {
                            let size = m.len();
                            Some(DirEntry {
                                name,
                                size,
                                is_dir: false,
                                path: p.to_string_lossy().to_string(),
                            })
                        } else if m.is_dir() {
                            // 计算该目录的整个子树大小（一次遍历该子树），但不展开
                            // depth=1 因为这些是根目录的子目录
                            match dir_size_parallel(&p, excludes, 1) {
                                Ok(size) => Some(DirEntry {
                                    name,
                                    size,
                                    is_dir: true,
                                    path: p.to_string_lossy().to_string(),
                                }),
                                Err(_) => None, // 跳过不可访问目录
                            }
                        } else {
                            None
                        }
                    }
                    Err(_) => None, // 跳过无法读取元数据的条目
                }
            })
            .collect();

        let total_size = items.iter().map(|e| e.size).sum();
        Ok(DirReport {
            total_size,
            entries: items,
            path: root.to_string_lossy().to_string(),
        })
    }
}

/// 并行计算某目录子树的总文件大小（O(N) 一次遍历）。
fn dir_size_parallel(
    path: &Path,
    excludes: &[String],
    depth: usize,
) -> Result<u64, Box<dyn std::error::Error>> {
    if !path.is_dir() {
        let meta = fs::metadata(path)?;
        return Ok(if meta.is_file() { meta.len() } else { 0 });
    }

    // 并行地遍历直接子项；对文件直接计入，对目录递归调用。
    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => return Ok(0), // 无权限等情况：按 0 处理
    };
    let entries: Vec<_> = read_dir.collect();

    let sum = entries
        .into_par_iter()
        .filter_map(|e| e.ok())
        .map(|entry| {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            // 检查是否应该排除（only_root=true 只在根目录排除）
            if should_exclude(depth, &name, excludes, true) {
                return 0;
            }

            match entry.metadata() {
                Ok(m) => {
                    if m.is_file() {
                        m.len()
                    } else if m.is_dir() {
                        // 递归地并行求和
                        dir_size_parallel(&p, excludes, depth + 1).unwrap_or(0)
                    } else {
                        0
                    }
                }
                Err(_) => 0,
            }
        })
        .sum();

    Ok(sum)
}

/// 递归并行扫描子树，返回 (该子树文件总大小, 子树所有条目列表)。
/// 返回的 entries **包含** 所有文件与目录条目（不包含 root 本身，以便顶层保持与旧行为一致）。
fn scan_dir_recursive(
    path: &Path,
    excludes: &[String],
    depth: usize,
) -> Result<(u64, Vec<DirEntry>), Box<dyn std::error::Error>> {
    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => return Ok((0, Vec::new())), // 无权限：返回空
    };
    let children: Vec<_> = read_dir.collect();

    // 用 rayon 对直接子项做并行处理；每个目录子项自身再并行递归。
    let results: Vec<(u64, Vec<DirEntry>)> = children
        .into_par_iter()
        .filter_map(|res| res.ok())
        .map(|entry| {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            // 检查是否应该排除（only_root=true 只在根目录排除）
            if should_exclude(depth, &name, excludes, true) {
                return (0, Vec::new());
            }

            match entry.metadata() {
                Ok(m) => {
                    if m.is_file() {
                        let size = m.len();
                        let me = DirEntry {
                            name,
                            size,
                            is_dir: false,
                            path: p.to_string_lossy().to_string(),
                        };
                        (size, vec![me])
                    } else if m.is_dir() {
                        // 递归：拿到子树大小与其条目，然后把"目录本身"也作为一条记录加入
                        match scan_dir_recursive(&p, excludes, depth + 1) {
                            Ok((sub_size, mut sub_entries)) => {
                                let me = DirEntry {
                                    name,
                                    size: sub_size,
                                    is_dir: true,
                                    path: p.to_string_lossy().to_string(),
                                };
                                sub_entries.push(me);
                                (sub_size, sub_entries)
                            }
                            Err(_) => (0, Vec::new()),
                        }
                    } else {
                        (0, Vec::new())
                    }
                }
                Err(_) => (0, Vec::new()),
            }
        })
        .collect();

    let mut total = 0u64;
    let mut all_entries = Vec::new();
    for (sz, mut list) in results {
        total += sz;
        all_entries.append(&mut list);
    }
    Ok((total, all_entries))
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

/// 尽量保持和原逻辑一致的“中间省略”截断（仍基于 char/宽度，复杂 emoji 可能有边缘情况）
fn truncate_filename(name: &str, max_width: usize) -> String {
    let display_width = name.width();

    if display_width <= max_width {
        name.to_string()
    } else if max_width <= 3 {
        "...".to_string()
    } else {
        let mut result = String::new();
        let chars: Vec<char> = name.chars().collect();
        let available = max_width - 3;

        if available >= 6 {
            let start_chars = available / 2;
            let end_chars = available - start_chars;

            // 前半段
            let mut current_width = 0;
            let mut start_end = 0;
            for (i, ch) in chars.iter().enumerate() {
                let char_width = ch.width().unwrap_or(0);
                if current_width + char_width > start_chars {
                    break;
                }
                current_width += char_width;
                start_end = i + 1;
            }
            for &ch in &chars[..start_end] {
                result.push(ch);
            }
            result.push_str("...");

            // 后半段
            if chars.len() > start_end {
                let mut end_width = 0;
                let mut end_start = chars.len();
                for (i, ch) in chars.iter().enumerate().rev() {
                    let char_width = ch.width().unwrap_or(0);
                    if end_width + char_width > end_chars {
                        break;
                    }
                    end_width += char_width;
                    end_start = i;
                }
                for &ch in &chars[end_start..] {
                    result.push(ch);
                }
            }
        } else {
            // 空间较小，只保留开头
            let mut current_width = 0;
            for ch in chars.iter() {
                let char_width = ch.width().unwrap_or(0);
                if current_width + char_width > available {
                    break;
                }
                current_width += char_width;
                result.push(*ch);
            }
            result.push_str("...");
        }

        result
    }
}

fn get_terminal_width() -> usize {
    if let Some((Width(w), _)) = terminal_size() {
        // 更好地利用宽度，上限放宽到 160
        (w as usize).clamp(60, 160)
    } else {
        100 // 默认宽度
    }
}

fn output_json(report: &DirReport) {
    match serde_json::to_string_pretty(report) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("JSON 序列化错误: {}", e),
    }
}

fn output_json_summary(report: &DirReport) {
    let (file_cnt, dir_cnt) = report.entries.iter().fold((0usize, 0usize), |(f, d), e| {
        if e.is_dir { (f, d + 1) } else { (f + 1, d) }
    });

    let summary = serde_json::json!({
        "path": report.path,
        "total_size": report.total_size,
        "item_count": report.entries.len(),
        "file_count": file_cnt,
        "dir_count": dir_cnt
    });

    match serde_json::to_string_pretty(&summary) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("JSON 序列化错误: {}", e),
    }
}

fn output_summary(report: &DirReport) {
    let display_width = get_terminal_width();

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

// 去除 ANSI 颜色码（简易版，匹配 \x1b ... m）
fn strip_ansi_codes(text: &str) -> String {
    let mut result = String::new();
    let mut in_escape = false;

    for ch in text.chars() {
        if ch == '\x1b' {
            in_escape = true;
            continue;
        }
        if in_escape {
            if ch == 'm' {
                in_escape = false;
            }
            continue;
        }
        result.push(ch);
    }

    result
}

fn output_text(report: &DirReport, show_chart: bool) {
    let display_width = get_terminal_width();

    // 布局参数
    let size_width = 12;
    let chart_width = if show_chart { 42 } else { 0 }; // [40 个块 + 两侧括号]
    let icon_width = 3; // emoji + 空格
    let spacing = 2;

    // 计算合适的文件名宽度，避免过度宽泛
    let used_width = icon_width + size_width + chart_width + spacing * 2;
    let available_width = display_width.saturating_sub(used_width);
    let filename_width = if show_chart {
        // 有图表时，文件名宽度适中
        available_width.clamp(20, 50)
    } else {
        // 无图表时，文件名宽度可以稍大但不过度
        available_width.clamp(30, 80)
    };

    // 计算实际使用的总宽度
    let actual_width = icon_width + filename_width + size_width + chart_width + spacing * 2;

    println!("{}", "═".repeat(actual_width).cyan().bold());
    println!("{} {}", "目录:".green().bold(), report.path.yellow());
    println!(
        "{} {}",
        "总大小:".green().bold(),
        format_size(report.total_size).cyan().bold()
    );
    println!("{}", "═".repeat(actual_width).cyan().bold());

    if report.entries.is_empty() {
        println!("{}", "目录为空".yellow());
        return;
    }

    let max_size = report.entries.iter().map(|e| e.size).max().unwrap_or(1);

    for entry in &report.entries {
        let size_str = format_size(entry.size);
        let type_icon = if entry.is_dir { "📁" } else { "📄" };

        let truncated_name = truncate_filename(&entry.name, filename_width);
        let colored_name = if entry.is_dir {
            truncated_name.blue().bold()
        } else {
            truncated_name.white()
        };

        // 以“去色后的可见宽度”计算填充
        let actual_width = strip_ansi_codes(&truncated_name).width();
        let padding_needed = filename_width.saturating_sub(actual_width);
        let padding = " ".repeat(padding_needed);

        if show_chart {
            let bar_length = if max_size > 0 {
                ((entry.size as f64 / max_size as f64) * 40.0).round() as usize
            } else {
                0
            };
            let bar_length = bar_length.min(40);
            let bar = "█".repeat(bar_length);
            let bar_colored = if entry.is_dir {
                bar.blue()
            } else {
                bar.green()
            };

            println!(
                "{} {}{} {:>12} [{}{}]",
                type_icon,
                colored_name,
                padding,
                size_str.cyan(),
                bar_colored,
                " ".repeat(40 - bar_length)
            );
        } else {
            println!(
                "{} {}{} {:>12}",
                type_icon,
                colored_name,
                padding,
                size_str.cyan()
            );
        }
    }

    println!("{}", "═".repeat(actual_width).cyan().bold());
    println!(
        "{} {} 个项目",
        "共计:".green().bold(),
        report.entries.len().to_string().yellow().bold()
    );
}
