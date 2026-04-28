use clap::{Arg, Command};
use colored::*;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use terminal_size::{Width, terminal_size};
use thiserror::Error;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const BAR_MAX_WIDTH: usize = 40;
const WARN_LIMIT: usize = 20;

const CTX_READ_DIR: &str = "无法读取目录";
const CTX_READ_ENTRY: &str = "无法读取目录项";
const CTX_METADATA: &str = "无法读取元数据";

// ---- suffix 截断防退化参数 ----
const ZW_BASE: usize = 8;
const ZW_PER_VISIBLE: usize = 8;
const UTF8_MAX_BYTES: usize = 4;
const SUFFIX_BYTE_BUDGET_PAD: usize = 32;

fn suffix_byte_budget(limit: usize) -> usize {
    limit
        .saturating_mul(UTF8_MAX_BYTES)
        .saturating_add(SUFFIX_BYTE_BUDGET_PAD)
}

// ---- 并行化阈值 ----
static PAR_MIN_ENTRIES: OnceLock<usize> = OnceLock::new();
fn par_min_entries() -> usize {
    *PAR_MIN_ENTRIES.get_or_init(|| {
        std::env::var("YP_PAR_MIN_ENTRIES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&v| v >= 1)
            .unwrap_or(64)
    })
}

// ---- CWD 缓存（词法绝对化，不 canonicalize） ----
static CWD: OnceLock<PathBuf> = OnceLock::new();
fn cwd() -> &'static PathBuf {
    CWD.get_or_init(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// 仅做词法层去除 '.'（CurDir）组件；不处理 '..'（保持"字面路径"策略）。
fn normalize_curdir_only(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        if comp != Component::CurDir {
            out.push(comp.as_os_str());
        }
    }
    out
}

/// 将路径提升到同一坐标系（可注入 cwd，便于测试）：
/// - 绝对路径：仅去掉 '.' 组件
/// - 相对路径：cwd join 后去掉 '.' 组件
///   不做 '..' 归一化。
fn absify_for_compare_with_cwd(p: &Path, cwd: &Path) -> PathBuf {
    if p.is_absolute() {
        normalize_curdir_only(p)
    } else {
        normalize_curdir_only(&cwd.join(p))
    }
}

/// 生产路径使用全局 CWD。
#[inline]
fn absify_for_compare(p: &Path) -> PathBuf {
    absify_for_compare_with_cwd(p, cwd())
}

// ---- ExcludePattern ----
//
// [FIX-BUG-1] 删除 Rel 变体。
// compile_excludes 阶段统一将路径类模式提升为 Abs（使用 absify_for_compare），
// 从而保证 entry.path()（绝对路径）与排除模式始终在同一坐标系下比较。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExcludePattern {
    /// 仅匹配文件名（不含路径分隔符），使用 `OsString` 以支持非 UTF-8 名称
    Name(OsString),
    /// 绝对化后的路径，与 absify_for_compare(entry.path()) 直接比较
    Abs(PathBuf),
}

// ---- ExcludeSet ----
//
// [FIX-MAINT-6] 删除 has_abs 字段，改为方法，避免手动维护不同步。
#[derive(Debug, Clone)]
struct ExcludeSet {
    patterns: Vec<ExcludePattern>,
    /// 缓存是否存在 Abs 模式，避免在热路径重复扫描 patterns。
    has_abs: bool,
}

impl ExcludeSet {
    /// 是否存在 Abs 模式（用于 should_exclude 热路径决策是否执行 absify）
    #[inline]
    fn has_abs(&self) -> bool {
        self.has_abs
    }

    fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

// ---- ScanContext ----
//
// [FIX-MAINT-10] 将 root / root_abs / excludes / warnings 聚合为上下文，
// 减少函数签名中的重复参数。
struct ScanContext<'a> {
    /// 用户指定的根路径（原始，用于相对路径显示等）
    #[allow(dead_code)]
    root: &'a Path,
    /// root 的绝对化形式（预计算，避免热路径重复计算）
    #[allow(dead_code)]
    root_abs: PathBuf,
    excludes: &'a ExcludeSet,
    warnings: &'a WarningTracker,
}

impl<'a> ScanContext<'a> {
    fn new(root: &'a Path, excludes: &'a ExcludeSet, warnings: &'a WarningTracker) -> Self {
        let root_abs = absify_for_compare(root);
        Self {
            root,
            root_abs,
            excludes,
            warnings,
        }
    }
}

// ---- ScanEntry / DirReport ----

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScanEntry {
    name: String,
    size: u64,
    is_dir: bool,
    path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DirReport {
    total_size: u64,
    entries: Vec<ScanEntry>,
    path: String,
}

// ---- AppError ----
//
// [FIX-BUG-3] 删除从未使用的 UnsupportedFileType 变体。
#[derive(Debug, Error)]
enum AppError {
    #[error("路径不存在: {0}")]
    PathNotFound(String),

    #[error("无法读取目录: {path} ({source})")]
    ReadDir {
        path: String,
        #[source]
        source: io::Error,
    },

    #[error("无法读取元数据: {path} ({source})")]
    Metadata {
        path: String,
        #[source]
        source: io::Error,
    },

    #[error("JSON 序列化错误: {0}")]
    Json(#[from] serde_json::Error),
}

type AppResult<T> = Result<T, AppError>;

// ---- WarningTracker ----
//
// [FIX-CONCURRENCY-5] emit 的打印限制策略说明：
//   fetch_add(Relaxed) 后读取旧值 n；当 n < WARN_LIMIT 时打印。
//   多线程竞争下，总计数是精确的，但打印次数在极端情况下可能略超
//   WARN_LIMIT（因 fetch_add 与 eprintln 非原子）。
//   这是"尽力限制"策略：计数精确，打印近似限制，可接受的权衡。
//   若需严格限制打印次数，应使用 Mutex<usize>，但会引入锁竞争。
#[derive(Debug)]
struct WarningTracker {
    total: AtomicUsize,
    io_count: AtomicUsize,
    param_count: AtomicUsize,
}

impl WarningTracker {
    fn new() -> Self {
        Self {
            total: AtomicUsize::new(0),
            io_count: AtomicUsize::new(0),
            param_count: AtomicUsize::new(0),
        }
    }

    fn emit(&self, formatted: String) {
        // fetch_add 返回旧值；旧值 < WARN_LIMIT 时本条消息可以打印
        let n = self.total.fetch_add(1, Ordering::Relaxed);
        if n < WARN_LIMIT {
            eprintln!("{}", formatted);
            // 当本条消息恰好是第 WARN_LIMIT 条时，打印封顶提示
            if n + 1 == WARN_LIMIT {
                eprintln!(
                    "{} 已达到告警上限（{} 条），后续警告将不再逐条打印。",
                    "提示:".yellow().bold(),
                    WARN_LIMIT
                );
            }
        }
    }

    fn warn_io(&self, context: &str, path: &Path, err: &dyn std::fmt::Display) {
        self.io_count.fetch_add(1, Ordering::Relaxed);
        self.emit(format!(
            "{} {}: {} ({})",
            "警告:".yellow().bold(),
            context,
            path.display(),
            err
        ));
    }

    fn warn_msg(&self, msg: &str) {
        self.param_count.fetch_add(1, Ordering::Relaxed);
        self.emit(format!("{} {}", "警告:".yellow().bold(), msg));
    }

    fn warning_total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }
    fn warning_io(&self) -> usize {
        self.io_count.load(Ordering::Relaxed)
    }
    fn warning_param(&self) -> usize {
        self.param_count.load(Ordering::Relaxed)
    }
}

fn print_warning_summary(w: &WarningTracker) {
    let total = w.warning_total();
    if total == 0 {
        return;
    }
    let io_n = w.warning_io();
    let param_n = w.warning_param();

    if param_n > 0 {
        eprintln!(
            "{} 本次运行产生 {} 条警告（IO: {}，参数: {}）。部分结果可能偏小或与预期不符。",
            "提示:".yellow().bold(),
            total,
            io_n,
            param_n
        );
    } else {
        eprintln!(
            "{} 本次运行产生 {} 条 IO 警告，部分结果可能偏小。",
            "提示:".yellow().bold(),
            total
        );
    }
}

// ---- compile_excludes ----
//
// [FIX-BUG-1] Rel 模式在此处统一提升为 Abs，使用 absify_for_compare。
// 这样 should_exclude 只需比较两个绝对路径，无需关心 root 坐标系。
fn compile_excludes(raw: Vec<String>, warnings: &WarningTracker) -> ExcludeSet {
    let mut patterns = Vec::new();

    for p in raw {
        if p.trim().is_empty() {
            warnings.warn_msg("忽略空的 exclude 模式（-e \"\" 或仅空白）");
            continue;
        }

        let is_path_like = p.contains('/') || (cfg!(windows) && p.contains('\\'));
        if is_path_like {
            // 路径类模式：无论相对/绝对，统一提升为 absify_for_compare 坐标系
            let abs = absify_for_compare(Path::new(&p));
            patterns.push(ExcludePattern::Abs(abs));
        } else {
            patterns.push(ExcludePattern::Name(OsString::from(p)));
        }
    }

    let has_abs = patterns.iter().any(|p| matches!(p, ExcludePattern::Abs(_)));
    ExcludeSet { patterns, has_abs }
}

// ---- should_exclude ----
//
// [FIX-PERF-4] 接收预计算的 root_abs（来自 ScanContext），避免热路径重复 normalize。
// [FIX-BUG-1] Rel 变体已删除，所有路径模式均为 Abs，统一 absify 后比较。
fn should_exclude(current_path: &Path, ctx: &ScanContext) -> bool {
    if ctx.excludes.is_empty() {
        return false;
    }

    let name_os = current_path.file_name();

    // 第一遍：仅检查 Name 模式（无需分配）
    for pat in &ctx.excludes.patterns {
        match pat {
            ExcludePattern::Name(n) if name_os == Some(n.as_os_str()) => return true,
            _ => {}
        }
    }

    // 第二遍：若存在 Abs 模式，才执行 absify（可能分配）。
    // 优化：当 current_path 已是绝对路径时，仅做最小化的 normalize；
    // 否则以 ctx.root_abs 作为基准避免反复查询全局 CWD。
    if ctx.excludes.has_abs() {
        let cur_abs = if current_path.is_absolute() {
            normalize_curdir_only(current_path)
        } else {
            absify_for_compare_with_cwd(current_path, ctx.root_abs.as_path())
        };

        for pat in &ctx.excludes.patterns {
            match pat {
                ExcludePattern::Abs(pat_abs) if cur_abs == *pat_abs => return true,
                _ => {}
            }
        }
    }

    false
}

// ---- 文件类型辅助 ----

#[cfg(unix)]
#[allow(dead_code)]
fn file_kind_str(meta: &fs::Metadata) -> &'static str {
    use std::os::unix::fs::FileTypeExt;
    let ft = meta.file_type();
    if ft.is_dir() {
        "directory"
    } else if ft.is_file() {
        "file"
    } else if ft.is_symlink() {
        "symlink"
    } else if ft.is_socket() {
        "socket"
    } else if ft.is_fifo() {
        "fifo"
    } else if ft.is_char_device() {
        "char device"
    } else if ft.is_block_device() {
        "block device"
    } else {
        "other"
    }
}

#[cfg(not(unix))]
#[allow(dead_code)]
fn file_kind_str(meta: &fs::Metadata) -> &'static str {
    let ft = meta.file_type();
    if ft.is_dir() {
        "directory"
    } else if ft.is_file() {
        "file"
    } else if ft.is_symlink() {
        "symlink"
    } else {
        "other"
    }
}

/// 统一"叶子"语义（不跟随 symlink）：
/// - symlink 或 file → Some(len)
/// - directory       → None（需递归）
/// - 其他（socket/fifo/device）→ Some(0)（特殊文件不计入大小）
fn meta_leaf_size_nofollow(meta: &fs::Metadata) -> Option<u64> {
    let ft = meta.file_type();
    if ft.is_symlink() || meta.is_file() {
        Some(meta.len())
    } else if meta.is_dir() {
        None
    } else {
        Some(0)
    }
}

// ---- process_dir_entry ----

fn process_dir_entry(
    entry: fs::DirEntry,
    ctx: &ScanContext,
    size_cache: Option<&HashMap<PathBuf, u64>>,
    top_meta: Option<&HashMap<PathBuf, (bool, u64)>>,
) -> Option<ScanEntry> {
    let p = entry.path();
    if should_exclude(&p, ctx) {
        return None;
    }
    let name = entry.file_name().to_string_lossy().into_owned();

    // 如果在非递归预扫描阶段已经收集到顶层条目的元信息，优先使用以避免重复的 syscalls
    if let Some(meta_map) = top_meta
        && let Some((is_dir, sz)) = meta_map.get(&p)
    {
        return Some(ScanEntry {
            name,
            size: *sz,
            is_dir: *is_dir,
            path: p.to_string_lossy().into_owned(),
        });
    }

    // 否则回退到读取元数据
    let meta = match fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(e) => {
            ctx.warnings.warn_io(CTX_METADATA, &p, &e);
            return None;
        }
    };

    if let Some(sz) = meta_leaf_size_nofollow(&meta) {
        return Some(ScanEntry {
            name,
            size: sz,
            is_dir: false,
            path: p.to_string_lossy().into_owned(),
        });
    }

    // directory: 使用缓存或重新计算
    let size = if let Some(cache) = size_cache {
        cache.get(&p).copied().unwrap_or(0)
    } else {
        // 无缓存时直接递归计算（非 recursive report 模式）
        let mut dummy_cache = HashMap::new();
        dir_size_recursive_serial(&p, ctx, &mut dummy_cache, RecordMode::RecordNone)
    };

    Some(ScanEntry {
        name,
        size,
        is_dir: true,
        path: p.to_string_lossy().into_owned(),
    })
}

// ---- analyze_directory ----
//
// [FIX-BUG-2] 两种模式统一：total_size = 所有叶子文件大小之和。
// 非 recursive 模式下，目录条目的 size 字段表示其子树大小（供排序/显示），
// 但 total_size 不再将其累加（避免重复计算）。
fn analyze_directory(
    path: &str,
    recursive: bool,
    excludes: &ExcludeSet,
    warnings: &WarningTracker,
) -> AppResult<DirReport> {
    let root = Path::new(path);
    let ctx = ScanContext::new(root, excludes, warnings);

    let meta = match fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(AppError::PathNotFound(root.display().to_string()));
        }
        Err(e) => {
            return Err(AppError::Metadata {
                path: root.display().to_string(),
                source: e,
            });
        }
    };

    // root 是叶子（文件/symlink/特殊文件）
    if let Some(sz) = meta_leaf_size_nofollow(&meta) {
        let name = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let entry = ScanEntry {
            name,
            size: sz,
            is_dir: false,
            path: root.to_string_lossy().into_owned(),
        };
        // 若用户对文件使用 --recursive，给出提示
        if recursive {
            warnings.warn_msg("指定路径是文件而非目录，--recursive 无效");
        }
        return Ok(DirReport {
            total_size: sz,
            entries: vec![entry],
            path: root.to_string_lossy().into_owned(),
        });
    }

    if recursive {
        let (_, entries) = scan_dir_recursive(root, &ctx);
        // [FIX-BUG-2] total_size 仅统计叶子文件，与非 recursive 语义一致
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

        // 非 recursive：为每个顶层条目预先读取元信息并为目录计算大小（串行，避免并行递归栈爆炸）
        // 先串行扫描一遍拿到目录大小缓存与顶层元信息，再并行/串行构建 ScanEntry，避免重复的 syscalls。
        let mut size_cache: HashMap<PathBuf, u64> = HashMap::new();
        let mut top_meta: HashMap<PathBuf, (bool, u64)> = HashMap::new();
        for entry in items.iter().flatten() {
            let p = entry.path();
            if should_exclude(&p, &ctx) {
                continue;
            }

            let m = match fs::symlink_metadata(&p) {
                Ok(m) => m,
                Err(e) => {
                    warnings.warn_io(CTX_METADATA, &p, &e);
                    continue;
                }
            };

            if m.is_dir() {
                let sz =
                    dir_size_recursive_serial(&p, &ctx, &mut size_cache, RecordMode::RecordNone);
                size_cache.insert(p.clone(), sz);
                top_meta.insert(p.clone(), (true, sz));
            } else {
                let sz = meta_leaf_size_nofollow(&m).unwrap_or(0);
                top_meta.insert(p.clone(), (false, sz));
            }
        }

        let entries: Vec<ScanEntry> = if items.len() < threshold {
            let mut out = Vec::new();
            for res in items {
                let entry = match res {
                    Ok(v) => v,
                    Err(err) => {
                        warnings.warn_io(CTX_READ_ENTRY, root, &err);
                        continue;
                    }
                };
                if let Some(se) = process_dir_entry(entry, &ctx, Some(&size_cache), Some(&top_meta))
                {
                    out.push(se);
                }
            }
            out
        } else {
            // 并行阶段：size_cache 已经构建完毕，只读引用，安全
            items
                .into_par_iter()
                .filter_map(|res| match res {
                    Ok(v) => Some(v),
                    Err(err) => {
                        warnings.warn_io(CTX_READ_ENTRY, root, &err);
                        None
                    }
                })
                .filter_map(|entry| {
                    process_dir_entry(entry, &ctx, Some(&size_cache), Some(&top_meta))
                })
                .collect()
        };

        // 非 recursive：entries 仅包含根目录下一层条目，目录 size 是各自子树总和，
        // 与同层文件大小互不重叠，因此直接累加全部条目可得到正确总大小。
        let total_size: u64 = entries.iter().map(|e| e.size).sum();
        Ok(DirReport {
            total_size,
            entries,
            path: root.to_string_lossy().into_owned(),
        })
    }
}

// ---- [FIX-MAINT-9] 删除 dir_size_mixed，统一使用 dir_size_recursive_serial ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordMode {
    RecordAllDirs,
    RecordNone,
}

/// 递归目录大小（串行，带可选缓存写入），不跟随 symlink。
fn dir_size_recursive_serial(
    path: &Path,
    ctx: &ScanContext,
    cache: &mut HashMap<PathBuf, u64>,
    record: RecordMode,
) -> u64 {
    if let Some(&v) = cache.get(path) {
        return v;
    }

    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            ctx.warnings.warn_io(CTX_METADATA, path, &e);
            return 0;
        }
    };

    if let Some(sz) = meta_leaf_size_nofollow(&meta) {
        return sz;
    }

    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            ctx.warnings.warn_io(CTX_READ_DIR, path, &e);
            if record == RecordMode::RecordAllDirs {
                cache.insert(path.to_path_buf(), 0);
            }
            return 0;
        }
    };

    let mut sum = 0u64;
    for res in read_dir {
        let entry = match res {
            Ok(v) => v,
            Err(err) => {
                ctx.warnings.warn_io(CTX_READ_ENTRY, path, &err);
                continue;
            }
        };
        let p = entry.path();
        if should_exclude(&p, ctx) {
            continue;
        }
        let m = match fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(e) => {
                ctx.warnings.warn_io(CTX_METADATA, &p, &e);
                continue;
            }
        };

        if let Some(sz) = meta_leaf_size_nofollow(&m) {
            sum += sz;
        } else {
            sum += dir_size_recursive_serial(&p, ctx, cache, record);
        }
    }

    if record == RecordMode::RecordAllDirs {
        cache.insert(path.to_path_buf(), sum);
    }
    sum
}

/// 递归扫描子树，不跟随 symlink。
/// 返回 (本目录叶子总大小, 所有条目（含目录条目）)。
fn scan_dir_recursive(path: &Path, ctx: &ScanContext) -> (u64, Vec<ScanEntry>) {
    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            ctx.warnings.warn_io(CTX_READ_DIR, path, &e);
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
                    ctx.warnings.warn_io(CTX_READ_ENTRY, path, &err);
                    continue;
                }
            };
            out.push(scan_one_recursive(entry, ctx));
        }
        out
    } else {
        children
            .into_par_iter()
            .filter_map(|res| match res {
                Ok(v) => Some(v),
                Err(err) => {
                    ctx.warnings.warn_io(CTX_READ_ENTRY, path, &err);
                    None
                }
            })
            .map(|entry| scan_one_recursive(entry, ctx))
            .collect()
    };

    let mut total = 0u64;
    let total_len: usize = results.iter().map(|(_, v)| v.len()).sum();
    let mut all_entries = Vec::with_capacity(total_len);
    for (sz, list) in results {
        total += sz;
        all_entries.extend(list);
    }

    (total, all_entries)
}

fn scan_one_recursive(entry: fs::DirEntry, ctx: &ScanContext) -> (u64, Vec<ScanEntry>) {
    let p = entry.path();
    if should_exclude(&p, ctx) {
        return (0, Vec::new());
    }

    let name = entry.file_name().to_string_lossy().into_owned();

    let m = match fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(e) => {
            ctx.warnings.warn_io(CTX_METADATA, &p, &e);
            return (0, Vec::new());
        }
    };

    if let Some(sz) = meta_leaf_size_nofollow(&m) {
        let me = ScanEntry {
            name,
            size: sz,
            is_dir: false,
            path: p.to_string_lossy().into_owned(),
        };
        return (sz, vec![me]);
    }

    // directory
    let (sub_size, mut sub_entries) = scan_dir_recursive(&p, ctx);
    let me = ScanEntry {
        name,
        size: sub_size,
        is_dir: true,
        path: p.to_string_lossy().into_owned(),
    };
    sub_entries.push(me);
    (sub_size, sub_entries)
}

// ---- 格式化 ----

fn format_size(size: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let n = size as u128;

    if n < 1024 {
        return format!("{} {}", size, UNITS[0]);
    }

    let mut divisor: u128 = 1024;
    for (unit, unit_name) in UNITS.iter().enumerate().skip(1) {
        let value100 = (n * 100 + divisor / 2) / divisor;
        if value100 < 1024 * 100 || unit + 1 == UNITS.len() {
            return format!("{}.{:02} {}", value100 / 100, value100 % 100, unit_name);
        }
        divisor *= 1024;
    }

    unreachable!("format_size: exhausted units for size={}", size)
}

fn prefix_end_by_width(s: &str, limit: usize) -> usize {
    let mut w = 0usize;
    let mut end = 0usize;
    for (i, ch) in s.char_indices() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > limit {
            break;
        }
        w += cw;
        end = i + ch.len_utf8();
    }
    end
}

fn suffix_start_index_by_width(s: &str, limit: usize) -> usize {
    if limit == 0 {
        return s.len();
    }

    let byte_budget = suffix_byte_budget(limit);

    let mut acc = 0usize;
    let mut visible = 0usize;
    let mut zw_used = 0usize;
    let mut bytes_used = 0usize;

    s.char_indices()
        .rev()
        .take_while(|(_, ch)| {
            let cw = ch.width().unwrap_or(0);
            let clen = ch.len_utf8();

            if bytes_used + clen > byte_budget {
                return false;
            }

            if cw == 0 {
                if acc >= limit {
                    return false;
                }
                let zw_budget = ZW_BASE + visible.saturating_mul(ZW_PER_VISIBLE);
                if zw_used + 1 > zw_budget {
                    return false;
                }
                zw_used += 1;
                bytes_used += clen;
                true
            } else if acc + cw <= limit {
                acc += cw;
                visible += 1;
                bytes_used += clen;
                true
            } else {
                false
            }
        })
        .last()
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

fn truncate_filename(name: &str, max_width: usize) -> String {
    if name.width() <= max_width {
        return name.to_string();
    }
    if max_width <= 3 {
        return "...".to_string();
    }

    let available = max_width - 3;
    let left = available / 2;
    let right = available - left;

    let prefix_end = prefix_end_by_width(name, left);
    let mut suffix_start = suffix_start_index_by_width(name, right);
    if suffix_start < prefix_end {
        suffix_start = prefix_end;
    }

    let mut result = String::new();
    result.push_str(&name[..prefix_end]);
    result.push_str("...");
    result.push_str(&name[suffix_start..]);
    result
}

fn get_terminal_width() -> usize {
    if let Some((Width(w), _)) = terminal_size() {
        (w as usize).clamp(60, 160)
    } else {
        100
    }
}

// ---- 输出函数 ----

fn output_json(report: &DirReport) -> AppResult<()> {
    println!("{}", serde_json::to_string_pretty(report)?);
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

    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn output_summary(report: &DirReport) {
    let w = get_terminal_width();
    println!("{}", "═".repeat(w).cyan().bold());
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
    println!("{}", "═".repeat(w).cyan().bold());
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

    // [FIX-STYLE-8] 改为 unwrap_or(0)，无需依赖前面 is_empty 早返回的隐式保证
    let max_size = report.entries.iter().map(|e| e.size).max().unwrap_or(0);

    for entry in &report.entries {
        let size_str = format_size(entry.size);
        let type_icon = if entry.is_dir { "📁" } else { "📄" };

        let truncated_name = truncate_filename(&entry.name, filename_width);
        let colored_name = if entry.is_dir {
            truncated_name.blue().bold()
        } else {
            truncated_name.white()
        };

        let padding = " ".repeat(filename_width.saturating_sub(truncated_name.width()));

        if show_chart {
            let bar_len = if max_size == 0 {
                0
            } else {
                (((entry.size as u128) * (BAR_MAX_WIDTH as u128)) / (max_size as u128)) as usize
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

// ---- tree 模式 ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheMode {
    AllDirs,
    TopLevel,
}

struct TreePrintConfig<'a> {
    show_icon: bool,
    sort_by_size: bool,
    max_depth: Option<usize>,
    term_width: usize,
    cache: &'a HashMap<PathBuf, u64>,
    warnings: &'a WarningTracker,
}

#[derive(Debug)]
struct TreeItem {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
}

fn run_tree_mode(
    path: &str,
    recursive: bool,
    show_icon: bool,
    sort_by_size: bool,
    excludes: &ExcludeSet,
    warnings: &WarningTracker,
) -> AppResult<()> {
    let root = Path::new(path);
    let ctx = ScanContext::new(root, excludes, warnings);

    let meta = match fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(AppError::PathNotFound(root.display().to_string()));
        }
        Err(e) => {
            return Err(AppError::Metadata {
                path: root.display().to_string(),
                source: e,
            });
        }
    };

    // root 是叶子
    if let Some(sz) = meta_leaf_size_nofollow(&meta) {
        println!(
            "{} {}",
            "路径:".green().bold(),
            root.display().to_string().yellow()
        );
        let name = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let size_str = format_size(sz);
        if show_icon {
            println!("└── 📄 {} {}", name.white(), size_str.cyan());
        } else {
            println!("└── {} {}", name.white(), size_str.cyan());
        }
        print_warning_summary(warnings);
        return Ok(());
    }

    println!(
        "{} {}",
        "目录:".green().bold(),
        root.display().to_string().yellow()
    );

    let max_depth = if recursive { None } else { Some(1) };
    let term_width = get_terminal_width();

    let mut cache: HashMap<PathBuf, u64> = HashMap::new();
    let cache_mode = if recursive {
        CacheMode::AllDirs
    } else {
        CacheMode::TopLevel
    };
    let total_size = build_tree_cache_and_total(root, &ctx, &mut cache, cache_mode);

    println!(
        "{} {}",
        "总大小:".green().bold(),
        format_size(total_size).cyan().bold()
    );

    let cfg = TreePrintConfig {
        show_icon,
        sort_by_size,
        max_depth,
        term_width,
        cache: &cache,
        warnings,
    };

    print_tree_dir(root, "", 0, &cfg, &ctx)?;
    print_warning_summary(warnings);
    Ok(())
}

fn build_tree_cache_and_total(
    root: &Path,
    ctx: &ScanContext,
    cache: &mut HashMap<PathBuf, u64>,
    mode: CacheMode,
) -> u64 {
    match mode {
        CacheMode::AllDirs => {
            dir_size_recursive_serial(root, ctx, cache, RecordMode::RecordAllDirs)
        }
        CacheMode::TopLevel => {
            let read_dir = match fs::read_dir(root) {
                Ok(rd) => rd,
                Err(e) => {
                    ctx.warnings.warn_io(CTX_READ_DIR, root, &e);
                    return 0;
                }
            };

            let mut total = 0u64;
            for res in read_dir {
                let entry = match res {
                    Ok(v) => v,
                    Err(err) => {
                        ctx.warnings.warn_io(CTX_READ_ENTRY, root, &err);
                        continue;
                    }
                };
                let p = entry.path();
                if should_exclude(&p, ctx) {
                    continue;
                }
                let m = match fs::symlink_metadata(&p) {
                    Ok(m) => m,
                    Err(e) => {
                        ctx.warnings.warn_io(CTX_METADATA, &p, &e);
                        continue;
                    }
                };

                if let Some(sz) = meta_leaf_size_nofollow(&m) {
                    total += sz;
                } else {
                    let sz = dir_size_recursive_serial(&p, ctx, cache, RecordMode::RecordNone);
                    cache.insert(p, sz);
                    total += sz;
                }
            }

            cache.insert(root.to_path_buf(), total);
            total
        }
    }
}

fn print_tree_dir(
    path: &Path,
    prefix: &str,
    depth: usize,
    cfg: &TreePrintConfig,
    ctx: &ScanContext,
) -> AppResult<()> {
    if cfg.max_depth.is_some_and(|maxd| depth >= maxd) {
        return Ok(());
    }

    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            cfg.warnings.warn_io(CTX_READ_DIR, path, &e);
            return Ok(());
        }
    };

    let mut items: Vec<TreeItem> = Vec::new();
    for res in read_dir {
        let entry = match res {
            Ok(v) => v,
            Err(err) => {
                cfg.warnings.warn_io(CTX_READ_ENTRY, path, &err);
                continue;
            }
        };

        let p = entry.path();
        if should_exclude(&p, ctx) {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        let m = match fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(e) => {
                cfg.warnings.warn_io(CTX_METADATA, &p, &e);
                continue;
            }
        };

        if let Some(sz) = meta_leaf_size_nofollow(&m) {
            items.push(TreeItem {
                name,
                path: p,
                is_dir: false,
                size: sz,
            });
        } else {
            let sz = cfg.cache.get(&p).copied().unwrap_or(0);
            items.push(TreeItem {
                name,
                path: p,
                is_dir: true,
                size: sz,
            });
        }
    }

    if cfg.sort_by_size {
        items.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    } else {
        items.sort_by(|a, b| a.name.cmp(&b.name));
    }

    let total = items.len();
    for (i, item) in items.into_iter().enumerate() {
        let is_last = i + 1 == total;
        let branch = if is_last { "└──" } else { "├──" };
        let icon = if item.is_dir { "📁" } else { "📄" };
        let size_str = format_size(item.size);

        let mut fixed = prefix.width() + branch.width() + 1;
        if cfg.show_icon {
            fixed += icon.width() + 1;
        }
        fixed += 1 + size_str.width();

        let name_w = cfg.term_width.saturating_sub(fixed).clamp(4, 120);
        let name_trunc = truncate_filename(&item.name, name_w);
        let pad = " ".repeat(name_w.saturating_sub(name_trunc.width()));

        let name_colored = if item.is_dir {
            name_trunc.blue().bold()
        } else {
            name_trunc.white()
        };

        if cfg.show_icon {
            println!(
                "{}{} {} {}{} {}",
                prefix,
                branch,
                icon,
                name_colored,
                pad,
                size_str.cyan()
            );
        } else {
            println!(
                "{}{} {}{} {}",
                prefix,
                branch,
                name_colored,
                pad,
                size_str.cyan()
            );
        }

        if item.is_dir {
            let new_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };
            print_tree_dir(&item.path, &new_prefix, depth + 1, cfg, ctx)?;
        }
    }

    Ok(())
}

// ---- 模式分发 ----

#[allow(clippy::too_many_arguments)]
fn run_report_mode(
    path: &str,
    recursive: bool,
    sort_by_size: bool,
    json_output: bool,
    summary_only: bool,
    show_chart: bool,
    excludes: &ExcludeSet,
    warnings: &WarningTracker,
) -> AppResult<()> {
    let mut report = analyze_directory(path, recursive, excludes, warnings)?;

    if sort_by_size {
        report
            .entries
            .sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    }

    if json_output {
        if summary_only {
            output_json_summary(&report)?;
        } else {
            output_json(&report)?;
        }
    } else if summary_only {
        output_summary(&report);
    } else {
        output_text(&report, show_chart);
    }

    print_warning_summary(warnings);
    Ok(())
}

// ---- CLI ----

fn main() {
    if let Err(e) = run() {
        eprintln!("{} {}", "错误:".red().bold(), e);
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
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
                .help("递归显示所有子目录（tree 模式下展开所有层级；不跟随符号链接）")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("tree")
                .short('t')
                .long("tree")
                .help("以树状方式显示每个文件/目录及其大小（与 -r 结合递归展开；不跟随符号链接）")
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
                .help(
                    "排除指定文件/文件夹（可多次使用）。\n\
                     支持：\n\
                     • 名称模式（如 node_modules）：匹配任意层级同名条目\n\
                     • 路径模式（含 / 则视为路径）：统一绝对化后比较，\n\
                       相对路径以 CWD 为基准；不处理 '..' 归一化。\n\
                     symlink 不跟随，size 取链接自身元数据长度。",
                )
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

    let warnings = WarningTracker::new();

    let excludes_raw: Vec<String> = matches
        .get_many::<String>("exclude")
        .map(|vals| vals.map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let excludes = compile_excludes(excludes_raw, &warnings);

    if tree_mode {
        return run_tree_mode(
            path,
            recursive,
            show_icon,
            sort_by_size,
            &excludes,
            &warnings,
        );
    }

    run_report_mode(
        path,
        recursive,
        sort_by_size,
        json_output,
        summary_only,
        show_chart,
        &excludes,
        &warnings,
    )
}

// ---- tests ----
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let pid = std::process::id();
            let path = std::env::temp_dir().join(format!("{}_{}_{}", prefix, pid, nanos));
            fs::create_dir_all(&path).expect("failed to create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    // [FIX-TEST-7] 使用可注入 cwd 的纯函数，不依赖全局 CWD
    #[test]
    fn test_abs_exclude_dot_slash_path() {
        let fake_cwd = Path::new("/home/user/project");

        // 模拟 compile_excludes 对 "foo/bar" 的处理
        let abs_pat = absify_for_compare_with_cwd(Path::new("foo/bar"), fake_cwd);
        let excludes = ExcludeSet {
            patterns: vec![ExcludePattern::Abs(abs_pat)],
            has_abs: true,
        };

        // entry.path() 通常返回绝对路径
        let entry_path = Path::new("/home/user/project/foo/bar");
        let abs_entry = absify_for_compare_with_cwd(entry_path, fake_cwd);

        assert!(
            excludes
                .patterns
                .iter()
                .any(|p| matches!(p, ExcludePattern::Abs(a) if *a == abs_entry)),
            "绝对路径应匹配排除模式"
        );
    }

    #[test]
    fn test_abs_exclude_dot_slash_prefix() {
        let fake_cwd = Path::new("/home/user/project");

        // 用户输入 "./foo/bar"
        let abs_pat = absify_for_compare_with_cwd(Path::new("./foo/bar"), fake_cwd);
        // entry.path() 返回 /home/user/project/foo/bar
        let abs_entry =
            absify_for_compare_with_cwd(Path::new("/home/user/project/foo/bar"), fake_cwd);

        assert_eq!(
            abs_pat, abs_entry,
            "'./foo/bar' 与 'foo/bar' 应绝对化为同一路径"
        );
    }

    #[test]
    fn test_name_exclude_no_alloc_path() {
        // Name 模式下，has_abs() 返回 false，不触发 absify
        let excludes = ExcludeSet {
            patterns: vec![ExcludePattern::Name(OsString::from("node_modules"))],
            has_abs: false,
        };
        assert!(!excludes.has_abs());
    }

    #[test]
    fn test_has_abs_derived_from_patterns() {
        let excludes_no_abs = ExcludeSet {
            patterns: vec![ExcludePattern::Name(OsString::from("foo"))],
            has_abs: false,
        };
        assert!(!excludes_no_abs.has_abs());

        let excludes_with_abs = ExcludeSet {
            patterns: vec![ExcludePattern::Abs(PathBuf::from("/some/path"))],
            has_abs: true,
        };
        assert!(excludes_with_abs.has_abs());
    }

    #[test]
    fn test_output_text_bar_len_no_div0() {
        // max_size = 0 时 bar_len 应为 0，不 panic
        let report = DirReport {
            total_size: 0,
            entries: vec![
                ScanEntry {
                    name: "a".into(),
                    size: 0,
                    is_dir: true,
                    path: "a".into(),
                },
                ScanEntry {
                    name: "b".into(),
                    size: 0,
                    is_dir: false,
                    path: "b".into(),
                },
            ],
            path: ".".into(),
        };
        output_text(&report, true);
    }

    #[test]
    fn test_output_text_empty_entries() {
        // 空目录不 panic，输出"目录为空"
        let report = DirReport {
            total_size: 0,
            entries: vec![],
            path: ".".into(),
        };
        output_text(&report, true);
        output_text(&report, false);
    }

    #[test]
    fn test_non_recursive_total_size_includes_child_dirs() {
        let tmp = TempDirGuard::new("yp_non_recursive_total_size");
        let sub = tmp.path().join("sub");
        fs::create_dir_all(&sub).expect("failed to create sub dir");
        fs::write(sub.join("a.txt"), b"a").expect("failed to write a.txt");
        fs::write(sub.join("b.txt"), b"bb").expect("failed to write b.txt");

        let warnings = WarningTracker::new();
        let excludes = ExcludeSet {
            patterns: Vec::new(),
            has_abs: false,
        };

        let report = analyze_directory(
            tmp.path().to_str().expect("temp path is not valid UTF-8"),
            false,
            &excludes,
            &warnings,
        )
        .expect("analyze_directory should succeed");

        assert_eq!(report.total_size, 3, "非递归模式应统计子目录文件大小");
        assert_eq!(report.entries.len(), 1, "顶层应只有一个子目录条目");
        assert!(report.entries[0].is_dir, "顶层条目应为目录");
        assert_eq!(report.entries[0].size, 3, "目录条目大小应为子树总和");
    }

    #[test]
    fn test_format_size_boundaries() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.00 KiB");
        assert_eq!(format_size(1024 * 1024), "1.00 MiB");
        assert_eq!(format_size(u64::MAX), "16.00 EiB");
    }

    #[test]
    fn test_truncate_filename_ascii() {
        let s = "hello_world_long_name.txt";
        let t = truncate_filename(s, 10);
        assert!(t.width() <= 10, "截断后宽度应 <= 10，实际：{}", t.width());
        assert!(t.contains("..."), "应包含省略号");
    }

    #[test]
    fn test_truncate_filename_no_truncate_needed() {
        let s = "short.txt";
        assert_eq!(truncate_filename(s, 20), "short.txt");
    }

    #[test]
    fn test_warning_tracker_counts() {
        let w = WarningTracker::new();
        assert_eq!(w.warning_total(), 0);
        w.warn_msg("test param warning");
        assert_eq!(w.warning_total(), 1);
        assert_eq!(w.warning_param(), 1);
        assert_eq!(w.warning_io(), 0);
    }

    #[test]
    fn test_normalize_curdir_only() {
        let p = Path::new("./foo/./bar");
        let norm = normalize_curdir_only(p);
        assert_eq!(norm, PathBuf::from("foo/bar"));
    }

    #[test]
    fn test_normalize_preserves_dotdot() {
        // '..' 不应被处理
        let p = Path::new("foo/../bar");
        let norm = normalize_curdir_only(p);
        assert_eq!(norm, PathBuf::from("foo/../bar"));
    }
}
