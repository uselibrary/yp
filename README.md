# YP — Directory Space Viewer

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white" />
  <a href="https://www.gnu.org/licenses/gpl-3.0">
    <img alt="License: GPL v3" src="https://img.shields.io/badge/License-GPLv3-blue.svg" />
  </a>
  <a href="https://github.com/uselibrary/yp/blob/master/README_zh-CN.md">
    <img alt="Chinese" src="https://img.shields.io/badge/Language-中文-success.svg" />
  </a>
  <a href="https://github.com/uselibrary/yp/blob/master/README.md">
    <img alt="Chinese" src="https://img.shields.io/badge/Language-English-success.svg" />
  </a>
</p>

> 硬盘 (Chinese Pinyin: yìng pán, Hard disk) -> **y**ing**p**an -> yp

YP is a fast and flexible directory space usage viewer written in Rust. It supports multiple output formats and visualization options.

## ✨ Features

- 📊 Intuitive display: ASCII-art style bar charts and colored output
- 🗂️ Flexible traversal: analyze the current directory or recurse into subdirectories
- 📏 Smart units: automatically format sizes as B/KB/MB/GB/TB
- 🔄 Multiple sorting options: sort entries by size
- 📝 Multi-format output: plain text and JSON
- 📊 Summary mode: show only directories and total size
- 🚫 Smart exclusion: exclude specified files or folders (root-level only)
- 🚀 High performance: implemented in Rust for speed
- 🔧 Static build: optional musl static linking for dependency-free deployment
- 🎯 Smart layout: adapts to terminal width and handles long names with perfect alignment


## 📦 Installation

### Prebuilt binaries

Download a prebuilt binary from the releases page: https://github.com/uselibrary/yp/releases and pick the one that matches your platform. Currently, there is a build for `x86_64-unknown-linux-musl`.

Place the downloaded binary in `/usr/local/bin` and make it executable. Example:
```
wget https://github.com/uselibrary/yp/releases/download/v0.2.2/yp-x86_64-unknown-linux-musl
sudo mv yp-x86_64-unknown-linux-musl /usr/local/bin/yp
sudo chmod +x /usr/local/bin/yp
```

### Build from source

```bash
# clone
git clone https://github.com/uselibrary/yp
cd yp

# build (normal Linux)
cargo build --release

# static Linux build (musl)
rustup target add x86_64-unknown-linux-musl
cargo build --target x86_64-unknown-linux-musl --release

# Windows
rustup target add x86_64-pc-windows-gnu
cargo build --target x86_64-pc-windows-gnu --release

# macOS
rustup target add x86_64-apple-darwin
cargo build --target x86_64-apple-darwin --release
```

Built binaries are located at:
- Normal build: `target/release/yp`
- Static musl build: `target/x86_64-unknown-linux-musl/release/yp` (≈ 1.3 MB)

## 🚀 Usage

### Basic usage

```bash
# analyze current directory
yp

# analyze a specific path
yp -p /path/to/directory

# sort by size
yp -s

# show ASCII bar chart
yp -c

# combine options
yp -p /home -s -c

# summary mode (show only total)
yp -S

# exclude specified folder
yp -e target

# exclude multiple folders
yp -e target -e .git -e node_modules
```

### Advanced options

```bash
# recurse into subdirectories
yp -r

# JSON output
yp -j

# summary mode in JSON
yp -S -j

# exclude directories and show (only excludes current directory matches)
yp -s -c -e target -e assets

# full example
yp -p /usr -s -c -r
```

## 📋 Command-line options

| Option | Long option | Description |
|--------|-------------|-------------|
| `-p` | `--path <PATH>` | Path to analyze (default: current directory) |
| `-s` | `--sort` | Sort entries by size |
| `-j` | `--json` | Output JSON; in recursive mode includes all nested entries |
| `-c` | `--chart` | Show ASCII-art bar chart |
| `-r` | `--recursive` | Recurse into all subdirectories |
| `-S` | `--summary` | Show only directories and total size. In JSON mode this adds `file_count` and `dir_count` fields. |
| `-e` | `--exclude <PATTERN>` | Exclude specified files or folders (can be used multiple times, current directory only) |
| `-h` | `--help` | Show help |
| `-V` | `--version` | Show version |

## 📊 Output examples

### Text output

Default mode:

![text output](https://raw.githubusercontent.com/uselibrary/yp/refs/heads/master/assets/yp.png)

ASCII-art bar chart:

![chart output](https://raw.githubusercontent.com/uselibrary/yp/refs/heads/master/assets/yp-c.png)

### JSON output

```json
{
  "total_size": 212604987,
  "entries": [
    {
      "name": "target",
      "size": 212578344,
      "is_dir": true,
      "path": "./target"
    },
    {
      "name": "Cargo.lock",
      "size": 11674,
      "is_dir": false,
      "path": "./Cargo.lock"
    }
  ],
  "path": "."
}
```

### Summary output

Text (`./yp -S`):
```
════════════════════════════════════════════════════════════════════════════════
Path: /home/user/project
Total size: 261.11 MB
Items: 7 items
════════════════════════════════════════════════════════════════════════════════
```

JSON (`./yp -S -j`):
```json
{
  "file_count": 7,
  "path": "/home/user/project",
  "total_size": 273797834
}
```

## 🚫 Exclusion Feature Details

### Exclusion Strategy
The exclusion feature uses a **root-level exclusion strategy**, which only excludes files or folders at the root level of the specified directory and does not affect same-named items in subdirectories.

### Usage Examples

```bash
# Exclude a single directory
yp -e target

# Exclude multiple directories
yp -e target -e .git -e node_modules

# Combine with other options
yp -s -c -e target -e assets
yp -r -e .git -e target  # Recursive mode also supports exclusion
```

### Exclusion Behavior

Assuming the following directory structure:
```
project/
├── target/          # Will be excluded
├── src/
│   └── target/      # Will NOT be excluded
└── tests/
    └── target/      # Will NOT be excluded
```

Using `yp -e target` command:
- ✅ **Will exclude**: `target` folder in the root directory
- ❌ **Will NOT exclude**: `src/target` and `tests/target` folders

### Common Use Cases

```bash
# Rust projects: exclude build artifacts
yp -e target

# Git repositories: exclude version control directory
yp -e .git

# Node.js projects: exclude dependencies and build artifacts
yp -e node_modules -e dist -e build

# Multi-language projects: comprehensive exclusion
yp -e target -e .git -e node_modules -e __pycache__
```

## 🎯 Smart display features

### Adaptive layout
- Terminal width detection: automatically detects terminal width and adjusts layout
- Perfect alignment: size column and chart stay aligned even for long names
- Minimum width safety: still displays sensibly in narrow terminals

### Smart name handling
- Long name truncation: keeps start and end of very long names
- Information preservation: truncation strategy retains key parts of names
- Visual hint: uses `...` to indicate omitted sections

### Example
```
📄 libserde-2b6650dbf0c6568b.rlib                     5.57 MB [████████████████████████]
📄 libserde-2b6650dbf0c6568b.rmeta                    5.47 MB [███████████████████████ ]
```

### Implementation details
- Uses the `terminal_size` crate to detect terminal dimensions
- Dynamically calculates column widths for the best fit
- Smart truncation algorithm to preserve important parts of file names

## 🔧 Technical details

### Dependencies

- **clap**: command-line argument parsing
- **colored**: colored terminal output
- **serde**: serialization support
- **serde_json**: JSON output
- **walkdir**: directory traversal
- **terminal_size**: terminal width detection
- **unicode-width**: string display width calculation

### Performance characteristics

- Efficient directory traversal algorithms
- Memory usage optimizations
- Static musl build is about 1.3 MB and has no runtime dependencies

## 🌐 Platform compatibility

### ✅ Fully supported
- **Linux**: all features are supported
- **macOS**: all features are supported
- **Unix systems**: supported

### ⚠️ Partial support
- **Windows 10/11**: core features work; emoji may not display correctly; `Windows Terminal` fully supports all features
- **Older Windows**: core features work; colored output may be unavailable

### Core features (all platforms)
- ✅ Directory size calculation
- ✅ Entry sorting
- ✅ JSON output
- ✅ Basic text output

### Display features (platform dependent)
- 🎨 Colored output (modern terminals)
- 📊 Unicode bar charts (terminals with UTF-8)
- 📁 Emoji icons (terminals with Unicode support)

## 🔄 Build targets

```bash
# Static Linux build (recommended)
cargo build --target x86_64-unknown-linux-musl --release

# Other targets
cargo build --target x86_64-pc-windows-gnu --release
cargo build --target x86_64-apple-darwin --release
```

## 🐛 Troubleshooting

### Incorrect characters or garbled output
If you see garbled characters in some terminals, make sure:
1. The terminal is using UTF-8 encoding
2. The terminal font supports Unicode characters
3. On older Windows, consider using plain text mode

### Colored output issues
If colors don't appear correctly:
1. Ensure the terminal supports ANSI color codes
2. On Windows you may need to enable ANSI support

## 📈 Changelog


### v0.2.2 (latest)
- ✨ **Added**: Exclusion feature (`-e/--exclude`) - support for excluding specified files or folders
- 🎯 **Smart exclusion**: Only applies to root directory, doesn't affect same-named items in subdirectories
- 🔄 **Multiple exclusions**: Support using multiple `-e` parameters to exclude multiple items simultaneously
- 💡 **Use cases**: Quickly exclude common large directories like `target`, `.git`, `node_modules`

### v0.2.1
- 🎯 UX: UI/UX adjustments

### v0.2.0
- ✨ Added: parallel directory scanning. Uses rayon to parallelize child directory and file processing, improving performance on large trees.
- ✨ Added: single-pass aggregation. In recursive mode directory sizes are aggregated in one pass to avoid the previous O(N²) behavior.
- ✨ Added: enhanced JSON summary. Adds `file_count` and `dir_count` fields to better distinguish files and directories.
- ✨ Added: stable sorting. When sizes are equal entries are sorted by name to ensure stable output.
- ✨ Improved: unified error handling. Unreadable files or directories are skipped instead of aborting, increasing robustness.
- ✨ Improved: terminal width usage. The display width cap was changed from a fixed 80 to clamp(60,160) to use wider terminals more effectively.
- ✨ Improved: non-recursive mode optimizations. Direct child directory sizes are computed in parallel for better performance.
- 🎯 UX: CLI compatibility preserved. Command-line options remain compatible with previous versions.

### v0.1.2
- ✨ Added: summary mode (`-S/--summary`)
- ✨ Added: show only path, total size and item count
- ✨ Added: JSON support for summary mode
- 🎯 UX: useful for scripts and quick checks

### v0.1.1
- ✨ Added: smart file name handling
- ✨ Added: adaptive terminal width detection
- ✨ Improved: perfect column alignment
- ✨ Improved: smart truncation for long names
- 🔧 Technical: added `terminal_size` dependency
- 🎯 UX: fixed display issues caused by long file names

### v0.1.0
- 🎉 First release: basic directory space viewer
- ✨ Features: text and JSON output
- ✨ Features: ASCII-art bar chart
- ✨ Features: colored terminal output
- ✨ Features: recursive traversal
- ✨ Features: size sorting
- 🔧 Build: musl static linking support


## 📊 Benchmarks

| Test directory | File count | Time | Memory |
|---------------|------------|------|--------|
| Small project | ~100 files | <10 ms | ~2 MB |
| Medium project | ~1K files | ~50 ms | ~5 MB |
| Large project | ~10K files | ~200 ms | ~15 MB |

> Test environment: VPS E5V3, SSD, Linux


## 📝 License

This project is licensed under GPLv3 — see the [LICENSE](LICENSE) file for details.

## 🤝 Contributing

Contributions, issues and pull requests are welcome!
