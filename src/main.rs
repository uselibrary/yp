// src/main.rs
use clap::{Arg, Command};
use colored::*;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use terminal_size::{Width, terminal_size};
use thiserror::Error;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const BAR_MAX_WIDTH: usize = 40;
const WARN_LIMIT: usize = 20;

/// 并行化阈值：目录项数量小于该值时走串行，避免递归 into_par_iter 造成任务膨胀。
/// 这是经验值：IO 密集型遍历受磁盘带宽影响，合理阈值应通过 benchmark 决定。
/// 允许用环境变量覆盖：YP_PAR_MIN_ENTRIES=256 ./yp ...
fn par_min_entries() -> usize {
    std::env::var("YP_PAR_MIN_ENTRIES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&v| v >= 1)
        .unwrap_or(64)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScanEntry {
    name: String,
    size: u64,
    is_dir: bool,
    path: String, // JSON 输出友好；非 UTF-8 路径会 lossy，这里接受这一限制
}

#[derive(Debug, Serialize, Deserialize)]
struct DirReport {
    total_size: u64,
    entries: Vec<ScanEntry>,
    path: String,
}

#[derive(Debug, Error)]
enum AppError {
    #[error("路径不存在: {0}")]
    PathNotFound(String),

    #[error("无法读取目录: {path} ({source})")]
    ReadDir {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("无法读取元数据: {path} ({source})")]
    Metadata {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("JSON 序列化错误: {0}")]
    Json(#[from] serde_json::Error),
}

type AppResult<T> = Result<T, AppError>;

/// 控制告警输出，避免并行遍历时刷屏
#[derive(Debug)]
struct WarningTracker {
    count: AtomicUsize,
}

impl WarningTracker {
    fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
        }
    }

    fn warn_io(&self, context: &str, path: &Path, err: &dyn std::fmt::Display) {
        let n = self.count.fetch_add(1, Ordering::Relaxed);
        if n < WARN_LIMIT {
            eprintln!(
                "{} {}: {} ({})",
                "警告:".yellow().bold(),
                context,
                path.display(),
                err
            );
            if n + 1 == WARN_LIMIT {
                eprintln!(
                    "{} 已达到告警上限（{} 条），后续错误将不再逐条打印。",
                    "提示:".yellow().bold(),
                    WARN_LIMIT
                );
            }
        }
    }

    fn has_warnings(&self) -> bool {
        self.count.load(Ordering::Relaxed) > 0
    }

    fn warning_count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
}

/// 目录大小 cache：
/// - key 使用 PathBuf（来自实际遍历，不依赖 lossy 字符串），避免非 UTF-8 路径 miss
/// - 用 Mutex 简化；tree 打印通常是串行递归，锁竞争很低
type SizeCache = Mutex<HashMap<PathBuf, u64>>;

fn main() {
    let matches = Command::new("yp")
        .name("YP - 目录空间查看器")
        .version(env!("CARGO_PKG_VERSION"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(
            Arg::new("path")
                .short('p')
                .long("path")
                .value_name("PATH")
                .help("指定要分析的目录路径")
                .default_value("."),
        )
        .arg(
            Arg::new("no-sort")
                .long("no-sort")
                .help("禁用按大小排序（默认启用排序）")
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
            Arg::new("no-chart")
                .long("no-chart")
                .help("禁用 ASCII 条形图（默认启用）")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("recursive")
                .short('r')
                .long("recursive")
                .help("递归显示所有子目录（tree 模式下表示展开所有层级）")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("tree")
                .short('t')
                .long("tree")
                .help("以树状（tree）方式显示每个文件/目录及其大小（默认只显示一层；与 -r 结合递归展开）")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("icon")
                .long("icon")
                .help("tree 模式显示图标（📁/📄）")
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
                .help("排除指定的文件或文件夹（可多次使用；支持名称或相对/绝对路径）")
                .action(clap::ArgAction::Append),
        )
        .get_matches();

    let path = matches.get_one::<String>("path").unwrap();
    let sort_by_size = !matches.get_flag("no-sort");
    let json_output = matches.get_flag("json");
    let show_chart = !matches.get_flag("no-chart");
    let tree_mode = matches.get_flag("tree");
    let recursive = matches.get_flag("recursive");
    let show_icon = matches.get_flag("icon");
    let summary_only = matches.get_flag("summary");

    let excludes: Vec<String> = matches
        .get_many::<String>("exclude")
        .map(|vals| vals.map(|s| s.to_string()).collect())
        .unwrap_or_default();

    let warnings = WarningTracker::new();

    if tree_mode {
        // tree 模式：不依赖 analyze_directory 产出 entries；目录大小由 cache 驱动
        let root = Path::new(path);
        if !root.exists() {
            eprintln!(
                "{} {}",
                "错误:".red().bold(),
                AppError::PathNotFound(root.display().to_string())
            );
            std::process::exit(1);
        }

        println!(
            "{} {}",
            "目录:".green().bold(),
            root.display().to_string().yellow()
        );

        let cache: SizeCache = Mutex::new(HashMap::new());

        // ✅ 关键修复：-t -r 时先预扫描整棵树，cache 命中率才有意义
        let total_size = if recursive {
            compute_dir_size_cached(root, root, &excludes, &warnings, &cache)
        } else {
            // 只显示一层时，总大小仍可给 root 子树大小（可选：也可只算一层）
            compute_dir_size_cached(root, root, &excludes, &warnings, &cache)
        };

        println!(
            "{} {}",
            "总大小:".green().bold(),
            format_size(total_size).cyan().bold()
        );

        // ✅ -t 默认只显示一层；-t -r 无限深度
        let max_depth = if recursive { None } else { Some(1) };

        if let Err(e) = print_tree_dir(
            root,
            root,
            &excludes,
            "",
            show_icon,
            sort_by_size,
            max_depth,
            0,
            &cache,
            &warnings,
        ) {
            eprintln!("{} 打印树状视图时出错: {}", "错误:".red().bold(), e);
            std::process::exit(1);
        }

        if warnings.has_warnings() {
            eprintln!(
                "{} 本次扫描存在 {} 条访问/读取失败，输出结果可能偏小。",
                "提示:".yellow().bold(),
                warnings.warning_count()
            );
        }
        return;
    }

    // 非 tree 模式：走 report 输出逻辑
    match analyze_directory(path, recursive, &excludes, &warnings) {
        Ok(mut report) => {
            if sort_by_size {
                report
                    .entries
                    .sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
            }

            if json_output {
                if summary_only {
                    if let Err(e) = output_json_summary(&report) {
                        eprintln!("{} {}", "错误:".red().bold(), e);
                        std::process::exit(1);
                    }
                } else if let Err(e) = output_json(&report) {
                    eprintln!("{} {}", "错误:".red().bold(), e);
                    std::process::exit(1);
                }
            } else if summary_only {
                output_summary(&report);
            } else {
                output_text(&report, show_chart);
            }

            if warnings.has_warnings() {
                eprintln!(
                    "{} 本次扫描存在 {} 条访问/读取失败，输出结果可能偏小。",
                    "提示:".yellow().bold(),
                    warnings.warning_count()
                );
            }
        }
        Err(e) => {
            eprintln!("{} 分析目录时出错: {}", "错误:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

/// 检查路径是否应该被排除
fn should_exclude(current_path: &Path, root_path: &Path, excludes: &[String]) -> bool {
    let name = current_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    excludes.iter().any(|pattern| {
        if pattern.contains('/') || pattern.contains('\\') {
            let pattern_path = Path::new(pattern);

            if pattern_path.is_absolute() {
                return current_path == pattern_path;
            }

            if let Ok(relative) = current_path.strip_prefix(root_path) {
                return relative == pattern_path;
            }
            false
        } else {
            name == pattern
        }
    })
}

/// 抽取公共逻辑，串/并行分支都复用
fn process_dir_entry(
    entry: fs::DirEntry,
    root: &Path,
    excludes: &[String],
    warnings: &WarningTracker,
) -> Option<ScanEntry> {
    let p = entry.path();
    if should_exclude(&p, root, excludes) {
        return None;
    }

    let name = entry.file_name().to_string_lossy().into_owned();

    match entry.metadata() {
        Ok(m) => {
            if m.is_file() {
                Some(ScanEntry {
                    name,
                    size: m.len(),
                    is_dir: false,
                    path: p.to_string_lossy().into_owned(),
                })
            } else if m.is_dir() {
                let size = dir_size_mixed(&p, root, excludes, warnings);
                Some(ScanEntry {
                    name,
                    size,
                    is_dir: true,
                    path: p.to_string_lossy().into_owned(),
                })
            } else {
                None
            }
        }
        Err(err) => {
            warnings.warn_io("无法读取元数据", &p, &err);
            None
        }
    }
}

/// 顶层分析入口（非 tree 模式）
fn analyze_directory(
    path: &str,
    recursive: bool,
    excludes: &[String],
    warnings: &WarningTracker,
) -> AppResult<DirReport> {
    let root = Path::new(path);
    if !root.exists() {
        return Err(AppError::PathNotFound(root.display().to_string()));
    }

    if !root.is_dir() {
        let meta = fs::metadata(root).map_err(|e| AppError::Metadata {
            path: root.display().to_string(),
            source: e,
        })?;
        let size = meta.len();
        let entry = ScanEntry {
            name: root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string()),
            size,
            is_dir: false,
            path: root.to_string_lossy().into_owned(),
        };
        return Ok(DirReport {
            total_size: size,
            entries: vec![entry],
            path: root.to_string_lossy().into_owned(),
        });
    }

    if recursive {
        // ⚠️ &WarningTracker 可跨线程共享：
        // WarningTracker 只包含 AtomicUsize（Sync），因此 &WarningTracker 可以被 rayon 闭包安全捕获。
        let (_, entries) = scan_dir_recursive(root, root, excludes, warnings);
        let total_size: u64 = entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
        Ok(DirReport {
            total_size,
            entries,
            path: root.to_string_lossy().into_owned(),
        })
    } else {
        let read_dir = fs::read_dir(root).map_err(|e| AppError::ReadDir {
            path: root.display().to_string(),
            source: e,
        })?;
        let items: Vec<_> = read_dir.collect();

        let threshold = par_min_entries();

        let entries: Vec<ScanEntry> = if items.len() < threshold {
            let mut out = Vec::new();
            for res in items {
                let entry = match res {
                    Ok(v) => v,
                    Err(err) => {
                        warnings.warn_io("无法读取目录项", root, &err);
                        continue;
                    }
                };
                if let Some(se) = process_dir_entry(entry, root, excludes, warnings) {
                    out.push(se);
                }
            }
            out
        } else {
            items
                .into_par_iter()
                .filter_map(|res| match res {
                    Ok(v) => Some(v),
                    Err(err) => {
                        warnings.warn_io("无法读取目录项", root, &err);
                        None
                    }
                })
                .filter_map(|entry| process_dir_entry(entry, root, excludes, warnings))
                .collect()
        };

        let total_size: u64 = entries.iter().map(|e| e.size).sum();
        Ok(DirReport {
            total_size,
            entries,
            path: root.to_string_lossy().into_owned(),
        })
    }
}

/// 计算某目录子树的总文件大小（串/并行混合）
/// 遇到 IO 错误告警并按 0 继续
fn dir_size_mixed(path: &Path, root: &Path, excludes: &[String], warnings: &WarningTracker) -> u64 {
    if !path.is_dir() {
        match fs::metadata(path) {
            Ok(meta) => return if meta.is_file() { meta.len() } else { 0 },
            Err(e) => {
                warnings.warn_io("无法读取元数据", path, &e);
                return 0;
            }
        }
    }

    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            warnings.warn_io("无法读取目录", path, &e);
            return 0;
        }
    };

    let entries: Vec<_> = read_dir.collect();
    let threshold = par_min_entries();

    if entries.len() < threshold {
        let mut sum = 0u64;
        for res in entries {
            let entry = match res {
                Ok(v) => v,
                Err(err) => {
                    warnings.warn_io("无法读取目录项", path, &err);
                    continue;
                }
            };
            let p = entry.path();
            if should_exclude(&p, root, excludes) {
                continue;
            }
            match entry.metadata() {
                Ok(m) => {
                    if m.is_file() {
                        sum += m.len();
                    } else if m.is_dir() {
                        sum += dir_size_mixed(&p, root, excludes, warnings);
                    }
                }
                Err(err) => warnings.warn_io("无法读取元数据", &p, &err),
            }
        }
        sum
    } else {
        entries
            .into_par_iter()
            .filter_map(|e| e.ok())
            .map(|entry| {
                let p = entry.path();
                if should_exclude(&p, root, excludes) {
                    return 0;
                }
                match entry.metadata() {
                    Ok(m) => {
                        if m.is_file() {
                            m.len()
                        } else if m.is_dir() {
                            dir_size_mixed(&p, root, excludes, warnings)
                        } else {
                            0
                        }
                    }
                    Err(err) => {
                        warnings.warn_io("无法读取元数据", &p, &err);
                        0
                    }
                }
            })
            .sum()
    }
}

/// 递归扫描子树（补齐阈值策略：小目录串行，大目录并行）
/// 返回 (该子树文件总大小, 子树所有条目列表)
fn scan_dir_recursive(
    path: &Path,
    root: &Path,
    excludes: &[String],
    warnings: &WarningTracker,
) -> (u64, Vec<ScanEntry>) {
    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            warnings.warn_io("无法读取目录", path, &e);
            return (0, Vec::new());
        }
    };

    let children: Vec<_> = read_dir.collect();
    let threshold = par_min_entries();

    let results: Vec<(u64, Vec<ScanEntry>)> = if children.len() < threshold {
        let mut out = Vec::new();
        for res in children {
            let entry = match res {
                Ok(v) => v,
                Err(err) => {
                    warnings.warn_io("无法读取目录项", path, &err);
                    continue;
                }
            };
            out.push(scan_one_recursive(entry, root, excludes, warnings));
        }
        out
    } else {
        // ⚠️ &WarningTracker 可跨线程共享（见上面注释）
        children
            .into_par_iter()
            .filter_map(|res| match res {
                Ok(v) => Some(v),
                Err(err) => {
                    warnings.warn_io("无法读取目录项", path, &err);
                    None
                }
            })
            .map(|entry| scan_one_recursive(entry, root, excludes, warnings))
            .collect()
    };

    let mut total = 0u64;
    let mut all_entries = Vec::new();
    for (sz, mut list) in results {
        total += sz;
        all_entries.append(&mut list);
    }
    (total, all_entries)
}

fn scan_one_recursive(
    entry: fs::DirEntry,
    root: &Path,
    excludes: &[String],
    warnings: &WarningTracker,
) -> (u64, Vec<ScanEntry>) {
    let p = entry.path();
    if should_exclude(&p, root, excludes) {
        return (0, Vec::new());
    }

    let name = entry.file_name().to_string_lossy().into_owned();

    match entry.metadata() {
        Ok(m) => {
            if m.is_file() {
                let size = m.len();
                let me = ScanEntry {
                    name,
                    size,
                    is_dir: false,
                    path: p.to_string_lossy().into_owned(),
                };
                (size, vec![me])
            } else if m.is_dir() {
                let (sub_size, mut sub_entries) = scan_dir_recursive(&p, root, excludes, warnings);
                let me = ScanEntry {
                    name,
                    size: sub_size,
                    is_dir: true,
                    path: p.to_string_lossy().into_owned(),
                };
                sub_entries.push(me);
                (sub_size, sub_entries)
            } else {
                (0, Vec::new())
            }
        }
        Err(e) => {
            warnings.warn_io("无法读取元数据", &p, &e);
            (0, Vec::new())
        }
    }
}

/// ✅ 目录大小预扫描 + cache 填充：
/// - 计算该目录子树大小
/// - 将每个目录的大小写入 cache（key=PathBuf）
/// - 遇到错误告警并按 0 继续
fn compute_dir_size_cached(
    path: &Path,
    root: &Path,
    excludes: &[String],
    warnings: &WarningTracker,
    cache: &SizeCache,
) -> u64 {
    // cache hit
    if let Some(v) = cache.lock().unwrap().get(path).copied() {
        return v;
    }

    if !path.is_dir() {
        let sz = match fs::metadata(path) {
            Ok(m) => {
                if m.is_file() {
                    m.len()
                } else {
                    0
                }
            }
            Err(e) => {
                warnings.warn_io("无法读取元数据", path, &e);
                0
            }
        };
        return sz;
    }

    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            warnings.warn_io("无法读取目录", path, &e);
            // 写入 cache，避免反复告警/反复尝试
            cache.lock().unwrap().insert(path.to_path_buf(), 0);
            return 0;
        }
    };

    let entries: Vec<_> = read_dir.collect();
    let threshold = par_min_entries();

    let sum: u64 = if entries.len() < threshold {
        let mut acc = 0u64;
        for res in entries {
            let entry = match res {
                Ok(v) => v,
                Err(err) => {
                    warnings.warn_io("无法读取目录项", path, &err);
                    continue;
                }
            };
            let p = entry.path();
            if should_exclude(&p, root, excludes) {
                continue;
            }
            match entry.metadata() {
                Ok(m) => {
                    if m.is_file() {
                        acc += m.len();
                    } else if m.is_dir() {
                        acc += compute_dir_size_cached(&p, root, excludes, warnings, cache);
                    }
                }
                Err(err) => warnings.warn_io("无法读取元数据", &p, &err),
            }
        }
        acc
    } else {
        entries
            .into_par_iter()
            .filter_map(|r| r.ok())
            .map(|entry| {
                let p = entry.path();
                if should_exclude(&p, root, excludes) {
                    return 0;
                }
                match entry.metadata() {
                    Ok(m) => {
                        if m.is_file() {
                            m.len()
                        } else if m.is_dir() {
                            compute_dir_size_cached(&p, root, excludes, warnings, cache)
                        } else {
                            0
                        }
                    }
                    Err(err) => {
                        warnings.warn_io("无法读取元数据", &p, &err);
                        0
                    }
                }
            })
            .sum()
    };

    cache.lock().unwrap().insert(path.to_path_buf(), sum);
    sum
}

/// ✅ 无 f64 精度损失的 1024 基单位格式化（两位小数）
/// - unit=0：整数 B
/// - unit>0：整数部分 + 小数部分（rem*100/divisor）
fn format_size(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    if size < 1024 {
        return format!("{} {}", size, UNITS[0]);
    }

    let mut unit = 0usize;
    let mut divisor: u128 = 1;

    // 找到最大 unit，使得 size/divisor >= 1 且 unit 不越界
    for i in 1..UNITS.len() {
        let next = divisor * 1024;
        if (size as u128) < next {
            break;
        }
        divisor = next;
        unit = i;
    }

    let n = size as u128;
    let int_part = n / divisor;
    let rem = n % divisor;
    let frac = (rem * 100) / divisor; // 两位小数

    format!("{}.{} {}", int_part, format!("{:02}", frac), UNITS[unit])
}

fn get_terminal_width() -> usize {
    if let Some((Width(w), _)) = terminal_size() {
        (w as usize).clamp(60, 160)
    } else {
        100
    }
}

fn output_json(report: &DirReport) -> AppResult<()> {
    let json = serde_json::to_string_pretty(report)?;
    println!("{}", json);
    Ok(())
}

fn output_json_summary(report: &DirReport) -> AppResult<()> {
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

    let json = serde_json::to_string_pretty(&summary)?;
    println!("{}", json);
    Ok(())
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

/// 用 strip-ansi-escapes：覆盖完整 ANSI escape
fn strip_ansi_codes(text: &str) -> String {
    strip_ansi_escapes::strip_str(text)
}

/// 按显示宽度截取前缀（不超过 limit）
fn take_prefix_by_width(s: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > limit {
            break;
        }
        w += cw;
        out.push(ch);
    }
    out
}

/// 按显示宽度截取后缀（不超过 limit）
fn take_suffix_by_width(s: &str, limit: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut w = 0usize;
    let mut start = chars.len();
    for (i, ch) in chars.iter().enumerate().rev() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > limit {
            break;
        }
        w += cw;
        start = i;
    }
    chars[start..].iter().collect()
}

/// “中间省略”截断：严格基于显示宽度
fn truncate_filename(name: &str, max_width: usize) -> String {
    let display_width = name.width();
    if display_width <= max_width {
        return name.to_string();
    }
    if max_width <= 3 {
        return "...".to_string();
    }

    let available = max_width - 3;
    if available < 2 {
        return "...".to_string();
    }

    let left = available / 2;
    let right = available - left;

    let mut result = String::new();
    result.push_str(&take_prefix_by_width(name, left));
    result.push_str("...");
    result.push_str(&take_suffix_by_width(name, right));
    result
}

fn output_text(report: &DirReport, show_chart: bool) {
    let display_width = get_terminal_width();

    let size_width = 12;
    let chart_width = if show_chart { BAR_MAX_WIDTH + 2 } else { 0 };
    let icon_width = 3;
    let spacing = 2;

    let used_width = icon_width + size_width + chart_width + spacing * 2;
    let available_width = display_width.saturating_sub(used_width);
    let filename_width = if show_chart {
        available_width.clamp(20, 50)
    } else {
        available_width.clamp(30, 80)
    };

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

        let visible_width = strip_ansi_codes(&truncated_name).width();
        let padding_needed = filename_width.saturating_sub(visible_width);
        let padding = " ".repeat(padding_needed);

        if show_chart {
            let bar_len = if max_size > 0 {
                ((entry.size as f64 / max_size as f64) * BAR_MAX_WIDTH as f64).round() as usize
            } else {
                0
            }
            .min(BAR_MAX_WIDTH);

            let bar = "█".repeat(bar_len);
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
                " ".repeat(BAR_MAX_WIDTH - bar_len)
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

/// 打印树状结构（类似 tree）
/// max_depth: Some(n) 表示最多进入 n 层子目录（root 的直接子项深度=1）；None 表示无限深度
fn print_tree_dir(
    path: &Path,
    root: &Path,
    excludes: &[String],
    prefix: &str,
    show_icon: bool,
    sort_by_size: bool,
    max_depth: Option<usize>,
    depth: usize,
    cache: &SizeCache,
    warnings: &WarningTracker,
) -> AppResult<()> {
    if let Some(maxd) = max_depth {
        if depth >= maxd {
            return Ok(());
        }
    }

    // ✅ 不再构造 AppError 又降级：直接 read_dir，失败告警后继续
    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            warnings.warn_io("无法读取目录", path, &e);
            return Ok(());
        }
    };

    let mut items: Vec<(String, PathBuf, bool, u64)> = Vec::new();

    for res in read_dir {
        let entry = match res {
            Ok(v) => v,
            Err(err) => {
                warnings.warn_io("无法读取目录项", path, &err);
                continue;
            }
        };

        let p = entry.path();
        if should_exclude(&p, root, excludes) {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        match entry.metadata() {
            Ok(m) => {
                if m.is_file() {
                    items.push((name, p, false, m.len()));
                } else if m.is_dir() {
                    // cache 命中则用；miss 则 lazy 计算并写入 cache（只算一次）
                    let sz = {
                        if let Some(v) = cache.lock().unwrap().get(&p).copied() {
                            v
                        } else {
                            compute_dir_size_cached(&p, root, excludes, warnings, cache)
                        }
                    };
                    items.push((name, p, true, sz));
                }
            }
            Err(err) => warnings.warn_io("无法读取元数据", &p, &err),
        }
    }

    if sort_by_size {
        items.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));
    } else {
        items.sort_by(|a, b| a.0.cmp(&b.0));
    }

    let total = items.len();
    for (i, (name, p, is_dir, sz)) in items.into_iter().enumerate() {
        let is_last = i + 1 == total;
        let branch = if is_last { "└──" } else { "├──" };
        let icon = if is_dir { "📁" } else { "📄" };
        let size_str = format_size(sz);

        if show_icon {
            if is_dir {
                println!(
                    "{}{} {} {} {}",
                    prefix,
                    branch,
                    icon,
                    name.blue().bold(),
                    size_str.cyan()
                );
            } else {
                println!(
                    "{}{} {} {} {}",
                    prefix,
                    branch,
                    icon,
                    name.white(),
                    size_str.cyan()
                );
            }
        } else if is_dir {
            println!(
                "{}{} {} {}",
                prefix,
                branch,
                name.blue().bold(),
                size_str.cyan()
            );
        } else {
            println!("{}{} {} {}", prefix, branch, name.white(), size_str.cyan());
        }

        if is_dir {
            let new_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };

            print_tree_dir(
                &p,
                root,
                excludes,
                &new_prefix,
                show_icon,
                sort_by_size,
                max_depth,
                depth + 1,
                cache,
                warnings,
            )?;
        }
    }

    Ok(())
}
