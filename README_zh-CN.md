# YP - 目录空间查看器

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

> 硬盘 --> **y**ing**p**an --> yp

一个功能强大的目录空间占用空间查看工具，由rust编写，支持多种输出格式和可视化选项。

## ✨ 功能特性

- 📊 **直观显示**: 支持ASCII艺术风格条形图和彩色输出
- 🗂️ **灵活遍历**: 支持当前目录和递归遍历模式
- 📏 **智能单位**: 自动转换文件大小单位（B/KB/MB/GB/TB）
- 🔄 **多种排序**: 支持按文件大小排序
- 📝 **多格式输出**: 支持文本和JSON格式输出
- 📊 **简洁模式**: 支持只显示目录和总大小的简洁输出
- 🚫 **智能排除**: 支持排除指定文件或文件夹（仅作用于根目录）
- 🚀 **高性能**: 使用Rust编写，性能优异
- 🔧 **静态编译**: 支持musl静态链接，无依赖部署
- 🎯 **智能显示**: 自适应终端宽度，智能处理长文件名，确保完美对齐


## 📦 安装

### 安装二进制文件

从[发布页面](https://github.com/uselibrary/yp/releases)下载预编译的二进制文件，选择适合您系统的版本。当前仅提供`x86_64-unknown-linux-musl`版本。
将下载的二进制文件放置到`/usr/local/bin`中，并赋予可执行权限。示例操作如下：
```
wget https://github.com/uselibrary/yp/releases/download/v0.2.2/yp-x86_64-unknown-linux-musl
sudo mv yp-x86_64-unknown-linux-musl /usr/local/bin/yp
sudo chmod +x /usr/local/bin/yp
```

### 从源码编译

```bash
# 克隆仓库
git clone [<repository-url>](https://github.com/uselibrary/yp)
cd yp

# Linux 普通编译
cargo build --release


rustup target add x86_64-unknown-linux-musl # Linux静态链接编译
cargo build --target x86_64-unknown-linux-musl --release

rustup target add x86_64-pc-windows-gnu # Windows编译
cargo build --target x86_64-pc-windows-gnu --release

rustup target add x86_64-apple-darwin # macOS编译
cargo build --target x86_64-apple-darwin --release
```

编译后的二进制文件位于：
- 普通版本: `target/release/yp`
- 静态版本: `target/x86_64-unknown-linux-musl/release/yp` (约1.3MB)

## 🚀 使用方法

### 基本用法

```bash
# 查看当前目录
yp

# 查看指定目录
yp -p /path/to/directory

# 按大小排序显示（默认启用；传入 `-s` 将禁用排序）
yp -s

# 显示条形图（默认启用；传入 `-c` 将禁用图表）
yp -c

# 组合使用
yp -p /home -s -c

# 简洁模式（只显示总大小）
yp -S

# 排除指定文件夹
yp -e target

# 排除多个文件夹
yp -e target -e .git -e node_modules
```

### 高级选项

```bash
# 递归显示所有子目录
yp -r

# JSON格式输出
yp -j

# 简洁模式JSON输出
yp -S -j

# 排除目录并显示（仅排除当前目录下的匹配项）
# 说明：排序和图表默认启用；使用 `-s` 或 `-c` 可分别禁用
yp -e target -e assets

# 完整功能演示
# 说明：排序和图表默认启用（等同于 `-s -c`）
yp -p /usr -r
```

## 📋 命令行选项

| 选项 | 长选项 | 描述 |
|------|--------|------|
| `-p` | `--path <PATH>` | 指定要分析的目录路径（默认: 当前目录） |
| `-s` | `--sort` | 按大小排序显示（默认启用；传入 `-s` 将禁用排序） |
| `-j` | `--json` | 以JSON格式输出，递归模式下包含所有子目录和文件条目。 |
| `-c` | `--chart` | 显示ASCII艺术风格条形图（默认启用；传入 `-c` 将禁用图表） |
| `-r` | `--recursive` | 递归显示所有子目录 |
| `-S` | `--summary` | 只显示目录和总大小，不显示详细内容。在 JSON 模式下，会额外输出 file_count 与 dir_count 字段。 |
| `-e` | `--exclude <PATTERN>` | 排除指定的文件或文件夹（可多次使用，仅作用于当前目录） |
| `-h` | `--help` | 显示帮助信息 |
| `-V` | `--version` | 显示版本信息 |

## 📊 输出示例

### 文本模式输出

**默认模式：**

![文本模式输出](https://raw.githubusercontent.com/uselibrary/yp/refs/heads/master/assets/yp.png)

**ASCII艺术风格条形图：**

![文本模式输出（条形图）](https://raw.githubusercontent.com/uselibrary/yp/refs/heads/master/assets/yp-c.png)

### JSON模式输出

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

### 简洁模式输出

**文本格式** (`./yp -S`):
```
════════════════════════════════════════════════════════════════════════════════
目录: /home/user/project
总大小: 261.11 MB
项目数: 7 个项目
════════════════════════════════════════════════════════════════════════════════
```

**JSON格式** (`./yp -S -j`):
```json
{
  "file_count": 7,
  "path": "/home/user/project",
  "total_size": 273797834
}
```

## 🚫 排除功能详解

### 排除策略
排除功能采用**根目录排除策略**，只会排除指定目录的根级别的文件或文件夹，不会影响子目录中的同名项。

### 使用示例

```bash
# 排除单个目录
yp -e target

# 排除多个目录
yp -e target -e .git -e node_modules

# 结合其他选项使用
yp -s -c -e target -e assets
yp -r -e .git -e target  # 递归模式也支持排除
```

### 排除行为说明

假设目录结构如下：
```
project/
├── target/          # 会被排除
├── src/
│   └── target/      # 不会被排除
└── tests/
    └── target/      # 不会被排除
```

使用 `yp -e target` 命令：
- ✅ **会排除**: 根目录下的 `target` 文件夹
- ❌ **不会排除**: `src/target` 和 `tests/target` 文件夹

### 常见使用场景

```bash
# Rust 项目：排除编译产物
yp -e target

# Git 仓库：排除版本控制目录
yp -e .git

# 按照路径排除
yp -e path/to/exclude

# Node.js 项目：排除依赖和构建产物
yp -e node_modules -e dist -e build

# 多语言项目：综合排除
yp -e target -e .git -e node_modules -e __pycache__
```

## 🎯 智能显示功能

### 自适应布局
- **终端宽度检测**: 自动检测当前终端宽度，动态调整显示布局
- **完美对齐**: 无论文件名多长，大小列和条形图始终保持完美对齐
- **最小宽度保护**: 在极窄终端中也能正常显示

### 智能文件名处理
- **长文件名截断**: 对于超长文件名，智能保留开头和结尾部分
- **信息保留**: 截断策略确保文件名的关键信息得以保留
- **视觉提示**: 使用`...`清晰表示省略部分

### 显示效果
```
📄 libserde-2b6650dbf0c6568b.rlib                     5.57 MB [████████████████████████]
📄 libserde-2b6650dbf0c6568b.rmeta                    5.47 MB [███████████████████████ ]
```

### 技术实现
- 使用`terminal_size` crate检测终端尺寸
- 动态计算各列的最佳宽度分配
- 智能截断算法保留文件名关键信息

## 🔧 技术细节

### 依赖项

- **clap**: 命令行参数解析
- **colored**: 彩色终端输出
- **serde**: 序列化支持
- **serde_json**: JSON格式输出
- **walkdir**: 目录遍历
- **terminal_size**: 终端宽度检测
- **unicode-width**: 计算字符串显示宽度

### 性能特点

- 使用高效的目录遍历算法
- 内存使用优化
- 静态链接版本约1.3MB，无运行时依赖

## 🌐 跨平台兼容性

### ✅ 完全支持
- **Linux**: 所有功能完整支持
- **macOS**: 所有功能完整支持  
- **Unix系统**: 完整支持

### ⚠️ 部分支持
- **Windows 10/11**: 核心功能完整，emoji显示可能异常；`Windows Terminal`完美支持所有功能
- **旧版Windows**: 核心功能正常，彩色输出可能不支持

### 核心功能（所有平台）
- ✅ 目录大小计算
- ✅ 文件排序
- ✅ JSON输出
- ✅ 基本文本输出

### 显示功能（平台相关）
- 🎨 彩色输出（现代终端）
- 📊 Unicode条形图（支持UTF-8的终端）
- 📁 Emoji图标（支持Unicode的终端）

## 🔄 编译目标

```bash
# Linux静态链接版本（推荐）
cargo build --target x86_64-unknown-linux-musl --release

# 其他目标
cargo build --target x86_64-pc-windows-gnu --release
cargo build --target x86_64-apple-darwin --release
```

## 🐛 问题排查

### 字符显示异常
如果在某些终端中看到乱码，请确保：
1. 终端支持UTF-8编码
2. 终端字体支持Unicode字符
3. 在旧版本Windows中考虑使用纯文本模式

### 彩色输出问题
如果彩色输出不正常：
1. 确保终端支持ANSI颜色代码
2. 在Windows中可能需要启用ANSI支持

## 📈 更新日志

### v0.2.3 (最新)
- 💡 **按照路径排除**: 支持按完整路径排除文件或文件夹

### v0.2.2
- ✨ **新增**: 排除功能 (`-e/--exclude`) - 支持排除指定的文件或文件夹
- 🎯 **智能排除**: 仅作用于根目录，不影响子目录中的同名文件/文件夹
- 🔄 **多重排除**: 支持使用多个 `-e` 参数同时排除多个项目
- 💡 **使用场景**: 快速排除 `target`、`.git`、`node_modules` 等常见大型目录

### v0.2.1
- 🎯 **用户体验**: 调整UI/UX。

### v0.2.0 
- ✨ **新增**: 并行化目录扫描。使用 rayon 并行处理子目录和文件，大幅提升大目录下的性能。
- ✨ **新增**: 一次遍历聚合。递归模式下通过单次遍历聚合目录大小，避免原来 O(N²) 的重复计算。
- ✨ **新增**: JSON 摘要增强。新增 file_count 和 dir_count 字段，更直观地区分文件与目录数量。
- ✨ **新增**: 排序稳定化。当大小相同时，按名称排序，保证输出一致性。
- ✨ **改进**: 错误处理统一。不可访问的文件或目录将被跳过而非中止，提高健壮性。
- ✨ **改进**: 终端宽度利用。显示宽度上限从固定 80 调整为 clamp(60,160)，在大屏终端显示更充分。
- ✨ **改进**: 非递归模式优化。直接子目录大小使用并行统计，效率更高。
- 🎯 **用户体验**: 保持兼容。CLI 参数与原版保持一致，原有使用方式不受影响。
- ✨ **功能**: 输出格式在增强的同时，保持了对原有字段和行为的兼容。

### v0.1.2
- ✨ **新增**: 简洁模式 (`-S/--summary`) 功能
- ✨ **新增**: 只显示目录路径、总大小和文件数量
- ✨ **新增**: 简洁模式的JSON输出支持
- 🎯 **用户体验**: 适用于脚本和快速查看场景

### v0.1.1
- ✨ **新增**: 智能文件名处理功能
- ✨ **新增**: 自适应终端宽度检测
- ✨ **改进**: 完美的列对齐显示
- ✨ **改进**: 长文件名智能截断
- 🔧 **技术**: 添加 `terminal_size` 依赖
- 🎯 **用户体验**: 解决长文件名导致的显示混乱问题

### v0.1.0
- 🎉 **首次发布**: 基础目录空间查看功能
- ✨ **功能**: 支持文本和JSON输出格式
- ✨ **功能**: ASCII艺术风格条形图
- ✨ **功能**: 彩色终端输出
- ✨ **功能**: 递归目录遍历
- ✨ **功能**: 文件大小排序
- 🔧 **编译**: 支持musl静态链接


## 📊 性能基准

| 测试目录 | 文件数量 | 执行时间 | 内存使用 |
|----------|----------|----------|----------|
| 小型项目 | ~100文件 | <10ms | ~2MB |
| 中型项目 | ~1K文件 | ~50ms | ~5MB |
| 大型项目 | ~10K文件 | ~200ms | ~15MB |

> 测试环境: VPS E5V3, SSD, Linux


## 📝 许可证

本项目使用 GPLv3 许可证 - 查看 [LICENSE](LICENSE) 文件了解详情。

## 🤝 贡献

欢迎提交问题和拉取请求！