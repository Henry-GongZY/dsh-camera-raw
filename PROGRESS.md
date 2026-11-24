# PROGRESS — DSH Camera Raw 执行台账

> 跨会话事实来源。每完成一步更新本文件；不可验证事项累积到「待用户验证」清单。

## 当前状态

**迭代 0-5 全部完成**：插件 MVP 闭环（6 个 op + C ABI dylib + dlopen 宿主 11/11）。

## 已完成

### 2026-11-23 — 迭代 0：API 探查 + 工程骨架

- RustDNG 公开 API 面探查完成（读写管线、元数据访问、渲染、写侧缺口）。
  关键结论已固化到 `ARCHITECTURE.md` §3 §6。
- workspace 骨架：`crates/dsh-cr-core` + `crates/dsh-cr-cli`（`dsh-cr-plugin`
  按契约在迭代 5 落地）。
- EVALUATION.md 初版（含官方 SDK 源码取证）。

### 2026-11-23 — 迭代 1：info + validate（自验收通过 ✅）

- `DngDocument`（严格/宽松双模式）、`ops::info`、`ops::validate`、CLI。
- 自验收：validate × 14 = 14/14 PASS（与 dng-validate ground truth 一致）；
  info × 14 字段核对正确；负例 × 3 优雅处理。

### 2026-11-23 — 迭代 2：元数据操作 + 预览提取（自验收通过 ✅）

- `ops/metadata.rs`：8 种编辑项（description/artist/copyright/software/
  user_comment/date_original/rating/label）+ `save_as_dng`（另存 round-trip）。
- `ops/preview.rs`：预览列表 + 提取（JPEG/JXL 直接 dump、未压缩 8 位缩略图
  → PPM 转写，256×160 缩略图用 sips 验证为合法图片）。
- CLI 新增 `previews` / `edit` 子命令。
- **向上游 RustDNG 回馈 3 个修复**（本地已改、未提交，见下方清单）：
  1. write_dng EXIF IFD 只发射 Make/Model/ExifVersion → 接入完整
     `build_exif_tag_set`（95 字段 + GPS IFD），并移除 Software 硬编码
     （dng_image_writer.rs）。
  2. 解析已有 DNG 时 XMP 全丢：dng_ifd.rs 的 tcXMP 分支缺
     `sfSemanticMask` 门控 + dng_shared.rs 缺主 XMP case（对照 C++
     dng_ifd.cpp:2927 / dng_shared.cpp:1863）。
  3. 回归：RustDNG 486/486 测试全绿 + 14 样例 dng-validate 全 PASS。
- 插件层规避 1 个上游 bug：dng-xmp get_property 对数组节点返回 Some("")
  → sync 清空 dc:creator 等数组属性 → 编辑操作不做 synchronize_metadata，
  改为 EXIF/XMP 双边显式写入（`xmp_read_string` 双路径读取兜底）。
- 自验收：`tests/iter2_roundtrip.rs` 4/4（artist 全链路、全字段持久化、
  编辑后 validate 全绿、digest 不变）；CLI 端到端 8 项编辑全部持久化。

### 2026-11-23 — 迭代 3：渲染管线（自验收通过 ✅）

- `ops/render.rs`：`render_to(doc, out, params)` — stage1→digest→stage2→
  stage3→`DngRender`；参数：色彩空间（srgb/adobe_rgb/pro_photo/gray18/
  gray22/display_p3/rec2020，编号对齐 dng_validate -cs1..8）、位深
  u8/u16/f32、曝光、阴影、最大边长；输出 .tif（write_tiff_basic）/.ppm。
- CLI `render` 子命令。
- **向上游回馈第 4 个修复**：`image_samples_u16` 的 buffer pixel_type 用
  `image.pixel_type()`（ttByte 图必 panic "bad element width"）→ 改为
  ttShort，由 `DngImage::get` 转换（dng_image_writer.rs:157）。486/486 回归。
- 自验收：
  - **PPM 与 dng_validate ground truth 逐字节一致**（05 样例，sRGB u8）——
    渲染管线同构性的最强证据。
  - TIFF 输出 sips 验证合法；u16 位深字节数翻倍正确；曝光/色彩空间参数
    各自起效（输出产生差异）。
  - 大样例 01（9504×6336 JXL）渲染通过（见台账末尾）。

### 2026-11-23 — 迭代 4：DNG Converter（自验收通过 ✅）

- `ops/convert.rs`：`convert_to/convert_file` — 压缩 auto/uncompressed、
  向后兼容版本（1.1/1.4/1.6/1.7/save）、embed-original 诚实桩（E1 →
  `Unimplemented`，不静默降级）。
- **向上游回馈第 5 个修复（本次最大）**：writer 单遍整图编码但 IFD 按
  `find_tile_size(128KB/512KB)` 声明多 tile 网格 → tile 布局与数据错位，
  **任何 >128KB 的压缩 DNG 写出都不可解析**（上游 486 测试用 64×48 合成图
  恰好单 tile，未覆盖）。修复：压缩路径强制 1×1 tile 网格
  （`set_single_tile_grid`），486/486 回归。
- 自验收（全部双重验证：本插件 validate + RustDNG dng-validate）：
  - 05 u16 → lossless JPEG（240KB→17.5KB）：双验证 PASS ✅
  - 05 → uncompressed 1.1/1.4/save 三种版本：dng-validate 全 PASS ✅
    （backward tag 的 min 规则 = C++ 保真行为）
  - 08（u16 像素 + PGTM2 float32 元数据）→ lossless JPEG：双验证 PASS ✅
  - embed-original → 显式 Unimplemented 报错 ✅
  - 01（9504×6336 JXL）→ uncompressed 转换 + 双验证（见台账末尾）

### 2026-11-23 — 迭代 5：插件 ABI + dlopen 宿主（自验收通过 ✅）

- `crates/dsh-cr-plugin`（cdylib + rlib）：4 个导出符号（`dsh_cr_abi_version`
  / `dsh_cr_plugin_manifest` / `dsh_cr_execute` / `dsh_cr_string_free`），
  JSON 消息传递式 ABI；panic 用 `catch_unwind` 拦截（C ABI 不逃逸）。
- `dsh-cr-host`（libloading dlopen 宿主）：模拟第三方应用加载 dylib，
  11 项检查全过——版本协商、manifest、6 个 op 全套（info/validate/previews/
  edit/edit+save/render/convert）、错误路径 4 种（BadRequest/Unimplemented/
  坏 JSON/空指针）。debug 与 release dylib 均通过。
- release 产物：`dsh-cr-cli` / `dsh-cr-host` / `libdsh_cr_plugin.dylib`（5.5MB）。

## 进行中

- （MVP 完成；后续方向见「下一步」）

## 下一步（可选迭代）

1. ACR 调整参数（.xmp 配方）读写（`ops/settings.rs`，XMP CRSS 命名空间）
2. E1 补偿：插件层写 OriginalRawFileData 三 tag（嵌入原片）
3. 会话句柄 ABI（`dsh_cr_session_open/close`，避免每次 op 重开文档）
4. 上游补丁 PR 化（RustDNG 5 个修复，见下表）

## RustDNG 本地补丁清单（可回馈上游）

仓库 `/Users/shinano/Opensource/RustDNG`，基线 commit `d768160`，改动未提交：

| 文件 | 修复 |
|---|---|
| `crates/dng-sdk/src/dng_image_writer.rs` | write_dng_with_metadata 接入 build_exif_tag_set（EXIF IFD 全量 + GPS IFD），移除 Software 硬编码 |
| `crates/dng-sdk/src/dng_image_writer.rs` | image_samples_u16 buffer 类型改为 ttShort（u8 图写 TIFF 必 panic） |
| `crates/dng-sdk/src/dng_image_writer.rs` | 压缩写路径强制 1×1 tile 网格（单遍整图编码 × 多 tile 声明 = 产物不可解析） |
| `crates/dng-core/src/dng_ifd.rs` | tcXMP 分支加 sfSemanticMask 门控（非语义掩码 IFD 交给 shared 处理） |
| `crates/dng-core/src/dng_shared.rs` | DngShared::parse_tag 补 tcXMP case（主 XMP 记入 shared.xmp_offset/count） |

回归证据：`cargo test --workspace` 486/486；14 样例 dng-validate 全 PASS；
dsh-cr-core 4/4 round-trip 测试。

## 待用户验证

- （暂无）

## 待用户验证

- （暂无）

## 环境备忘

- Rust 1.99.1 arm64；RustDNG 在 `../RustDNG`（path 依赖）。
- 样例库：`../RustDNG/vendor/extracted/dng_sdk_1_7_1/sample_files/`。
- RustDNG debug 构建有 ~111 个 unused-import 警告（上游问题，与本仓库无关）。
