# DSH Camera Raw

复刻 Adobe Camera Raw 主要 DNG 操作功能的插件，底层依赖
[RustDNG](https://github.com/Henry-GongZY/RustDNG)（Adobe DNG SDK 1.7.1 的
100% 纯 Rust 重写）。

```
宿主应用 → dsh-cr-plugin (C ABI dylib) → dsh-cr-core (引擎) → RustDNG
```

## 构建

```sh
cargo build --release
# 产物：
#   target/release/dsh-cr-cli                  CLI 宿主（用户工具）
#   target/release/dsh-cr-host                 dlopen 宿主（插件验证）
#   target/release/libdsh_cr_plugin.dylib      插件动态库（给任意宿主加载）
```

依赖：Rust 工具链 + 本地 `../RustDNG` 仓库（path 依赖，见 `ARCHITECTURE.md` §5）。

## CLI 用法

```sh
dsh-cr-cli info     <file.dng>... [--pretty]            # 文件信息（ACR 信息面板）
dsh-cr-cli validate <file.dng>... [--pretty]            # 完整校验（dng_validate 等价）
dsh-cr-cli previews <file.dng> [out-dir] [--pretty]     # 内嵌预览列表/提取
dsh-cr-cli edit     <file.dng> [编辑项...] [--save out.dng]
                    [--uncompressed] [--backward-version 1.1|1.4|1.6|1.7|save]
                    # 编辑项：--description --artist --copyright --software
                    #         --user-comment --date-original --rating --label
dsh-cr-cli render   <file.dng> <out.tif|out.ppm>
                    [--cs srgb|adobe_rgb|pro_photo|gray18|gray22|display_p3|rec2020]
                    [--depth u8|u16|f32] [--exposure F] [--shadows F] [--max-size N]
dsh-cr-cli convert  <file.dng> <out.dng> [--compression auto|uncompressed]
                    [--backward-version 1.1|1.4|1.6|1.7|save]
```

## 插件 ABI（给宿主应用）

C ABI 动态库，消息传递式（JSON 进/JSON 出，二进制产物走文件路径）：

```c
uint32_t    dsh_cr_abi_version(void);            // 当前 1
const char* dsh_cr_plugin_manifest(void);        // 静态 JSON 清单
const char* dsh_cr_execute(const char* json, uint32_t len);
void        dsh_cr_string_free(const char* s);
```

请求：`{"op": "info|validate|previews|edit|render|convert", "input": "...", ...}`；
响应：`{"ok": true, ..., "result": {...}}` 或
`{"ok": false, "error": {"kind": "Dng|Io|NotDng|BadRequest|Unimplemented", "message": "..."}}`。

`dsh-cr-host` 用 libloading 演示完整的宿主加载流程（11 项检查）：

```sh
dsh-cr-host target/release/libdsh_cr_plugin.dylib
```

## 自验收基线

- `validate` × 14 官方样例 = 14/14 PASS，与 RustDNG `dng-validate` 一致。
- 渲染 PPM 与 `dng_validate --render --ppm` **逐字节一致**。
- 转换产物（压缩/版本变体）双重验证（本插件 + dng-validate）全过。
- 9504×6336 JXL：渲染 12.5s（--max-size 2048）、转换 3.4s（arm64 release）。
- dlopen 宿主 11/11（含错误路径：BadRequest / Unimplemented / 坏 JSON / 空指针）。

## 文档

- `ARCHITECTURE.md` — 设计契约（模块边界、插件 ABI、ACR 功能映射、上游缺口）
- `PROGRESS.md` — 执行台账（跨会话事实来源）
- `EVALUATION.md` — RustDNG vs 官方 C++ DNG SDK 易用性评估

## 许可

本仓库依赖 RustDNG（Adobe DNG SDK License Agreement 下的衍生作品）。
涉及 DNG 的分发需遵循该协议，详见 RustDNG 仓库的 `LICENSE` / `NOTICE.md`。
