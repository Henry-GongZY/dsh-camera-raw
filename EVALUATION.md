# EVALUATION — RustDNG vs 官方 Adobe DNG SDK 1.7.1

> 结论先行：**对「在应用/插件里做 DNG 操作」这个场景，RustDNG 明显更好用**；
> 官方 SDK 仅在「需要逐字节行为参考」和「需要全量写侧功能（previewList /
> gainMap / 嵌入原片）」时不可替代。依据是本项目（dsh-camera-raw）把 RustDNG
> 作为真实依赖从零集成到出活的全过程（2026-11-23 起），以及 vendor 源码对照。

## 1. 对照总表

| 维度 | 官方 C++ SDK 1.7.1 | RustDNG（纯 Rust 重写） | 结论 |
|---|---|---|---|
| 构建/集成 | 73 个 .cpp（~152k 行）+ C++17 工具链 + **libjpeg / libjxl / zlib / XMP Toolkit 四个外部依赖**，其中 XMP Toolkit 是 Adobe 闭源库 | `cargo add` 一个 `dng-sdk` path/git 依赖即可，零 C 链接、零 `-sys` crate，纯 Rust 依赖树 | RustDNG **胜**（数量级差异） |
| 错误模型 | C++ 异常（`Throw_dng_error`，dng_exceptions.h:104），跨语言边界处理麻烦 | `Result<T, DngError>`（DngError 枚举镜像 dng_errors.h，带编号如 BadFormat=100006） | RustDNG **胜** |
| 内存管理 | 手动 new/delete + 各种 Scoped 类 | Rust 所有权；图像统一 `ImageRef = Arc<RwLock<dyn DngImage>>` | RustDNG **胜** |
| API 迁移成本 | — | 类名/方法名与 C++ 1:1（`dng_negative.cpp` → `dng_negative.rs`），C++ SDK 的所有文档/资料可直接平移 | RustDNG **加分项** |
| 字符串 | `dng_string::Get() → const char*`（裸指针，dng_string.h:59） | `DngString`（自带 UTF-8 修复语义，`get_str() -> Option<&str>`）——但**不是 String 别名**，JSON 边界需要转换（本项目在 core 层统一做了 `dng_string()` 助手） | 各有取舍；RustDNG 类型更安全，但多一步转换 |
| 完整度 | 全量（WriteDNG 带 previewList/gainMapImage/gainMapLossyCompressed 参数，dng_image_writer.h:1375-1390） | 本次发现并修复了 write_dng EXIF IFD 缺口与 XMP 解析路由 bug；剩余诚实桩：previewList 未接、OriginalRawFileData 无写入引用、VarDCT 编码器缺（PORT-NOTE 逐条列明，见 ARCHITECTURE.md §6） | 官方 **胜**（写侧全量），但差距在收敛 |
| 测试 | SDK 自带测试少 | 486 个单测全绿 + dng-validate 14/14 官方样例通过 + wasm 烟雾测试 | RustDNG **胜** |
| 跨平台 | 需要各平台编译脚本 + 依赖分发 | cargo 目标矩阵；**wasm32-unknown-unknown / wasip1 可编译**（浏览器/沙箱内跑 DNG 解码） | RustDNG **胜** |
| 行为保真 | 地面真值 | 移植时逐函数对照 C++（`// PORT-NOTE(<file>:<line>)` 标注偏差），本项目 14 样例 validate 结果与 dng-validate 一致 | 平（设计目标就是保真） |
| 许可 | Adobe DNG SDK License Agreement | 同协议（衍生作品），93 个 .rs 保留 Adobe 版权头 | 平（同源协议） |

## 2. 本次集成中实际踩到的点（正面 + 摩擦）

### 正面（本次实测）

1. **十分钟出活**：从 `cargo build` 到跑通「info + validate 全流程」只花了半天，
   中途零编译环境问题；对比官方 SDK（光 XMP Toolkit 就要单独申请下载、编 CMake）。
2. **错误可编程处理**：`DngError::BadFormat (100006)` / `EndOfFile (100011)`
   枚举直接 match，非法文件场景（本轮 3 个负例）全部优雅返回。
3. **validate 管线一步到位**：`read_stage1_image → validate_raw_image_digest →
   build_stage2/3` 与 dng_validate 完全同构，本项目 14/14 样例 PASS。

### 摩擦与上游 bug（按影响排序；标注 ✅=本次已修复回馈，⚠️=插件层规避）

1. **无类型级 re-export**：`dng-sdk/lib.rs` 只 re-export `DngError/DngResult`，
   其余类型全走模块路径（`dng_sdk::dng_negative::DngNegative`）。上手要翻源码
   找类型位置；建议上游加 `pub use` 扁平层（对插件项目尤其有价值）。
2. **`DngString` 不是 String**：所有字符串字段都要转换。语义上有理由
   （UTF-8 修复），但给 serde 序列化层添了固定样板。
3. **命名陷阱**：`ccLossyJPEG = 34892`（DNG 规范里这是 Lossless JPEG，C++ SDK
   命名如此，RustDNG 保真沿用）——不看源码容易用错。
4. **索引字段是裸 int32**（`main_index/mask_index/...`，无效为 -1），不是
   Option——与 Rust 习惯不符，但与 C++ 保真，可接受。
5. **✅ write_dng 的 EXIF IFD 只发射 3 个 tag**（Make/Model/ExifVersion；注释
   自称"完整发射在 update-metadata 路径"）：C++ `WriteDNGWithMetadata` 走
   exif_tag_set 全量 95 字段。已修复：DNG 写路径接入现成的
   `build_exif_tag_set`（含 GPS IFD 发射），486 测试全绿。顺带修正了
   Software tag 被 writer 硬编码为 "RustDNG 1.7.1" 的偏差（C++ 默认行为是
   发射 exif.software，只有 CleanUpMetadata 才覆盖）。
6. **✅→⚠️ 解析已有 DNG 时 XMP 全部丢失**：`dng_ifd.rs` 的 tcXMP 分支把所有
   IFD0 的 XMP（tag 700）记成 `semantic_xmp_*`（C++ 仅在
   `NewSubFileType == sfSemanticMask` 时如此，主 XMP 由
   `dng_shared::ParseTag` 记到 `shared.xmp_offset/count`）。已修复（ifd 门控
   + shared 补 case）。**遗留**：dng-xmp 路径语义在「活对象」与「解析后的
   对象」之间不一致——写入用带前缀路径（"xmp:Rating"），解析后的节点按无
   前缀名（"Rating"）存储；插件层用 `xmp_read_string` 双路径兜底。
7. **⚠️ sync 会清掉数组型 XMP 属性**：`dng-xmp/src/node.rs:454` 的
   `get_property` 对数组节点返回 `Some("")`，触发 `sync_string` 的「XMP 空
   值即删除」规则——`synchronize_metadata()` 会删掉 `dc:creator` 等数组属
   性。插件层规避：编辑操作不做 sync，改为 EXIF/XMP 双边显式写入
   （`ops/metadata.rs` 有注释）。建议上游：get_property 对非 Simple 节点返
   回 None（数组值应走 get_array_item），并把路径前缀归一化对齐 C++ toolkit。
8. **写侧缺口**（E1/E2/E3，见 ARCHITECTURE.md §6）：嵌入 original raw 的
   tag 无写入引用、previewList 未接、VarDCT 编码器缺。

## 3. 对本项目的最终选型

**采用 RustDNG**。理由：ACR 式插件的核心诉求是「宿主进程内嵌 + 跨平台分发 +
错误可编程处理」，这三点官方 C++ SDK 都处于劣势；RustDNG 唯一的劣势（写侧
全量功能）在本插件的关键路径外，且缺口逐条有 PORT-NOTE 标注、可补偿。

## 4. 评估方法与证据链

- 集成实测：本项目 crates/dsh-cr-core（document/ops/info/validate）。
- 14 样例批量验证：`info`（字段人工核对：JXL/JPEG/CFA RGGB/版本/尺寸）+
  `validate`（14/14 PASS，digest 全 true）。
- 官方 SDK 对照：`RustDNG/vendor/extracted/dng_sdk_1_7_1/dng_sdk/source`
  （73 .cpp；WriteDNG 签名 dng_image_writer.h:1375；dng_string.h:59；
  dng_exceptions.h）。
- 负例：fake.txt（BadFormat）、truncated.dng（EndOfFile）、tiny.bin，均优雅处理。
