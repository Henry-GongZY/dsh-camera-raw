# ARCHITECTURE — DSH Camera Raw 设计契约

> 本文档是跨会话的**接口契约与设计决策记录**。执行状态见 `PROGRESS.md`。
> 修改模块边界 / ABI / 报告 schema 前先更新本文档。

## 1. 项目定位

**DSH Camera Raw**：复刻 Adobe Camera Raw（ACR）主要 DNG 操作功能的插件。

- 底层依赖 **RustDNG**（Adobe DNG SDK 1.7.1 的 100% 纯 Rust 重写，本地仓库
  `/Users/shinano/Opensource/RustDNG`，GitHub `Henry-GongZY/RustDNG`）。
- 对官方 C++ DNG SDK 的易用性评估随迭代记录在 `EVALUATION.md`。
- 语言：Rust（edition 2021，与 RustDNG 一致）。文档与代码注释：简体中文。

## 2. 三层结构

```
宿主应用（任意语言/任意进程）
    │  C ABI 动态库加载（dlopen / LoadLibrary）
    ▼
dsh-cr-plugin   cdylib：插件 ABI 层（JSON 请求/响应 + 版本协商）
    │  Rust 调用
    ▼
dsh-cr-core     引擎层：DngDocument + ACR 式操作（info/validate/metadata/render/convert）
    │  path 依赖（后续可一行切 git）
    ▼
RustDNG (dng-sdk / dng-core / dng-jpeg / dng-jxl / dng-xmp)
```

| crate | 类型 | 职责 |
|---|---|---|
| `dsh-cr-core` | lib | 引擎：`DngDocument`（打开/持有解析状态）+ `ops/*`（各操作 + serde 报告结构体） |
| `dsh-cr-plugin` | cdylib + lib | 插件 ABI：把 core 的操作暴露成稳定 C ABI |
| `dsh-cr-cli` | bin | CLI 宿主：面向用户的命令行工具，兼作迭代自验收驱动 |

## 3. 操作集与 ACR 功能映射（核心契约）

每个操作在 core 中是一个函数 `fn op(&mut DngDocument, …) -> DshResult<Report>`，
报告结构体全部 `serde::Serialize`（插件 ABI 用 JSON 传输）。映射表：

| ACR / Adobe DNG Converter 功能 | 插件 op | core 模块 | 底层 RustDNG API | 状态 |
|---|---|---|---|---|
| 打开文件、信息面板 | `info` | `ops/info.rs` | `DngInfo::parse/post_parse/is_valid_dng`、`DngNegative::parse/post_parse` | ✅ |
| 校验（dng_validate 等价） | `validate` | `ops/validate.rs` | `read_stage1_image` → `validate_raw_image_digest` → `build_stage2_image` → `build_stage3_image` | ✅ |
| 元数据面板编辑（EXIF/XMP） | `edit`（CLI）/ `edit_metadata` | `ops/metadata.rs` | `metadata_mut().get_exif_mut()/get_xmp_mut()`；**不调 sync**（缺口 E4） | ✅ |
| 另存 DNG（编辑落盘） | `edit --save` / `save_as_dng` | `ops/metadata.rs` | `read_stage1_image` + `DngImageWriter::write_dng` | ✅ |
| 预览/缩略图提取 | `previews`（CLI） | `ops/preview.rs` | IFD 遍历（chained/subifd）+ JPEG/JXL dump + 未压缩缩略图→PPM | ✅ |
| 渲染（去马赛克 → 成品图） | `render` | `ops/render.rs` | `DngRender` + 7 色彩空间 + u8/u16/f32 + 曝光/阴影/最大边长 → TIFF/PPM | ✅ |
| DNG Converter（转码/压缩/版本） | `convert` | `ops/convert.rs` | `DngImageWriter::write_dng`（压缩 auto/uncompressed，版本 1.1-1.7） | ✅ |
| 嵌入 original raw | `convert embed_original` | `ops/convert.rs` | writer 未接三个 tag（缺口 E1）→ 诚实桩 `Unimplemented` | ⚠️ 诚实桩 |
| ACR 调整参数（.xmp 配方）读写 | `settings` | `ops/settings.rs`（计划） | XMP CRSS 命名空间（`dng-xmp`） | 📋 计划 |

## 4. 插件 ABI（已实现，迭代 5）

原则：**消息传递式 ABI**——C ABI 只传 UTF-8 JSON 与文件路径，二进制产物一律
写到宿主指定的文件（路径走 JSON），避免跨语言传二进制缓冲区的复杂度。

### 4.1 导出符号

```c
// ABI 版本（整数，破坏性变更必须递增）
uint32_t dsh_cr_abi_version(void);

// 插件清单（JSON，静态存储期，无需释放）
const char* dsh_cr_plugin_manifest(void);

// 执行操作：请求 JSON 进，响应 JSON 出。返回的指针用 dsh_cr_string_free 释放。
const char* dsh_cr_execute(const char* request_json, uint32_t len);
void        dsh_cr_string_free(const char* s);
```

### 4.2 请求/响应 schema（v1）

```json
// 请求
{ "op": "info" | "validate" | "edit_metadata" | "extract_preview" | "render" | "convert" | "settings",
  "input": "<dng 路径>",
  ...op 专属参数 }

// 响应（成功）
{ "ok": true, "op": "info", "result": <该操作的报告对象> }

// 响应（失败）
{ "ok": false, "error": { "kind": "Dng|Io|NotDng|BadRequest|Unimplemented", "message": "…" } }
```

- 报告对象 = core 中对应 `ops/*` 的 serde 结构体（单一事实来源，CLI 与 ABI 共用）。
- 错误 kind 与 `DshError` 变体一一对应。

### 4.3 生命周期约束

- `dsh_cr_execute` 非线程安全（每次调用独立打开文档）；并发需求后续加
  会话句柄 API（`dsh_cr_session_open/close`），先不做。
- ABI 层不做任何解析逻辑——只做 JSON ↔ core 调用的翻译与错误映射。

## 5. 关键设计决策

1. **path 依赖 RustDNG**：本地迭代期间 `path = "../../../RustDNG/crates/*"`；
   切换 git 依赖只改 `Cargo.toml`。RustDNG crate 单一入口 `dng-sdk`。
2. **`DngString` 不是 String**：RustDNG 的 `DngString` 自带 UTF-8 修复语义
   （`get_str() -> Option<&str>`）。core 在 JSON 边界统一转换为 `String`
   （无效序列丢弃），保证上层永远拿到合法 UTF-8。
3. **宽松/严格双模式打开**：`DngDocument::open`（严格，`is_valid_dng` 不过即
   `NotDng`）与 `open_lenient`（宽松，供 `validate` 收集完整诊断）。
4. **错误不跨 ABI 逃逸**：`validate` 等诊断型操作把错误收进报告；`info` 等
   操作型操作向上抛 `DshError`，由 ABI 层统一映射为 `{"ok":false,...}`。
5. **诚实桩原则**（继承自 RustDNG）：任何未实现的功能必须以
   `DshError::Unimplemented` 或报告中的显式字段呈现，不允许静默降级。

## 6. RustDNG 已知缺口（影响本插件的）

| 编号 | 缺口 | 影响 | 插件层对策 |
|---|---|---|---|
| E1 | `OriginalRawFileData` 三个 tag（50827/50828/50973）negative 端有数据模型、writer 端无写入引用 | 「嵌入 original raw」转换无法走 SDK | 迭代 4 评估：插件层自己发这三个 tag（TIFF 目录层操作），或诚实桩 |
| E2 | `DngImageWriter::write_dng` 的 previewList / gainMap 参数未接（C++ `WriteDNG` 有） | 转换时不能写入新预览 | 迭代 4 用现有 preview 或跳过，PORT-NOTE 口径 |
| E3 | JXL VarDCT 编码器未实现（用户已推迟） | 只能写模块化模式 JXL | 与 ACR 无关（ACR 不产 JXL），不受影响 |
| E4（已规避） | dng-xmp `get_property` 对数组节点返回 `Some("")` → `sync_string` 清掉 `dc:creator` 等数组属性；且 XMP 路径前缀语义在活对象/解析对象间不一致 | 编辑后调 `synchronize_metadata` 会丢数据 | `ops/metadata.rs` 不调 sync，EXIF/XMP 双边显式写入；读取用 `xmp_read_string` 双路径兜底。建议回馈上游 |

### 已回馈上游的修复（2026-11-23，本地已改未提交，清单见 PROGRESS.md）

1. **write_dng EXIF IFD 全量发射**（原只有 Make/Model/ExifVersion，且 Software
   被硬编码为 "RustDNG 1.7.1"）——接入 `build_exif_tag_set`。
2. **XMP tag 700 解析路由**——dng_ifd 缺 sfSemanticMask 门控 +
   dng_shared 缺主 XMP case，导致解析任何已有 DNG 时 XMP 全丢。

## 7. 验证基线

- 样例库：`RustDNG/vendor/extracted/dng_sdk_1_7_1/sample_files/`（14 个官方样例）。
- 每次迭代的自验收：
  - `info` × 14：字段合理性与文件内容人工核对（相机型号、尺寸、压缩名）。
  - `validate` × 14：与 RustDNG `dng-validate` 结果一致性对照（ground truth）。
  - 非法输入：非 DNG 文件必须得到 `NotDng` / 报告内显式失败，不允许 panic。
