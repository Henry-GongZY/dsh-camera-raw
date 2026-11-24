//! # DSH Camera Raw — 插件 ABI 层
//!
//! 把 `dsh-cr-core` 的操作暴露为稳定 C ABI 动态库（契约见
//! `ARCHITECTURE.md` §4）：**消息传递式 ABI**——C ABI 只传 UTF-8 JSON，
//! 二进制产物一律写到宿主指定的文件路径。
//!
//! 导出符号：
//! ```c
//! uint32_t    dsh_cr_abi_version(void);
//! const char* dsh_cr_plugin_manifest(void);              // 静态 JSON
//! const char* dsh_cr_execute(const char* json, uint32_t len);
//! void        dsh_cr_string_free(const char* s);
//! ```
//!
//! 请求（JSON）：`{"op": "...", "input": "...", ...op 专属字段}`；
//! 响应（JSON）：`{"ok": true, "op": "...", "result": {...}}` 或
//! `{"ok": false, "error": {"kind": "...", "message": "..."}}`。
//!
//! 线程安全：`dsh_cr_execute` 内部每次独立打开文档，非并发优化路径；
//! 宿主可串行调用或自行加锁。

use std::ffi::CString;
use std::os::raw::{c_char, c_uint};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use serde_json::{json, Value};

use dsh_cr_core::ops::{convert, info, metadata, preview, render, validate};
use dsh_cr_core::DngDocument;

/// ABI 版本：破坏性变更（请求/响应 schema、符号语义）必须递增。
pub const DSH_CR_ABI_VERSION: u32 = 1;

// ------------------------------------------------------------- 请求模型

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    /// 文件信息。
    Info { input: String },
    /// 完整校验。
    Validate { input: String },
    /// 预览列表 / 提取（out_dir 缺省 = 仅列出）。
    Previews { input: String, out_dir: Option<String> },
    /// 元数据编辑 + 可选另存。
    Edit {
        input: String,
        #[serde(default)]
        edits: Vec<metadata::MetadataEdit>,
        save: Option<String>,
        #[serde(default)]
        uncompressed: bool,
        backward_version: Option<String>,
    },
    /// 渲染导出。
    Render {
        input: String,
        output: String,
        #[serde(default)]
        params: render::RenderParams,
    },
    /// DNG 转换。
    Convert {
        input: String,
        output: String,
        #[serde(default)]
        options: convert::ConvertOptions,
    },
}

/// 错误 kind 与 `DshError` 变体一一对应（宿主可编程匹配）。
fn error_kind(e: &dsh_cr_core::DshError) -> &'static str {
    match e {
        dsh_cr_core::DshError::Dng(_) => "Dng",
        dsh_cr_core::DshError::Io(_) => "Io",
        dsh_cr_core::DshError::NotDng(_) => "NotDng",
        dsh_cr_core::DshError::BadRequest(_) => "BadRequest",
        dsh_cr_core::DshError::Unimplemented(_) => "Unimplemented",
    }
}

/// 执行一个请求（纯 Rust 侧，便于测试）。
fn execute_request(req: &Request) -> Result<Value, dsh_cr_core::DshError> {
    match req {
        Request::Info { input } => {
            let mut doc = DngDocument::open(input)?;
            let summary = info::summarize(&mut doc);
            Ok(json!({ "op": "info", "result": summary }))
        }
        Request::Validate { input } => {
            let report = validate::validate_file(Path::new(input))?;
            Ok(json!({ "op": "validate", "result": report }))
        }
        Request::Previews { input, out_dir } => {
            let mut doc = DngDocument::open(input)?;
            let result = match out_dir {
                Some(dir) => json!({ "previews": preview::extract_previews(&mut doc, Path::new(dir))? }),
                None => json!({ "previews": preview::list_previews(&doc) }),
            };
            Ok(result)
        }
        Request::Edit {
            input,
            edits,
            save,
            uncompressed,
            backward_version,
        } => {
            let mut doc = DngDocument::open(input)?;
            let report = metadata::edit_metadata(&mut doc, edits)?;
            let mut out = json!({ "op": "edit", "edit": report });
            if let Some(out_path) = save {
                let max_backward = dsh_cr_core::backward_version_from_str(backward_version.as_deref())?;
                let save_report = metadata::save_as_dng(
                    &mut doc,
                    Path::new(out_path),
                    &metadata::SaveOptions { uncompressed: *uncompressed, max_backward_version: max_backward },
                )?;
                out["save"] = json!(save_report);
            }
            Ok(out)
        }
        Request::Render {
            input,
            output,
            params,
        } => {
            let mut doc = DngDocument::open(input)?;
            let report = render::render_to(&mut doc, Path::new(output), params)?;
            Ok(json!({ "op": "render", "result": report }))
        }
        Request::Convert {
            input,
            output,
            options,
        } => {
            let mut doc = DngDocument::open(input)?;
            let report = convert::convert_to(&mut doc, Path::new(output), options)?;
            Ok(json!({ "op": "convert", "result": report }))
        }
    }
}

// -------------------------------------------------------------- C ABI

const MANIFEST_JSON: &str = r#"{
  "name": "dsh-camera-raw",
  "version": "0.1.0",
  "abi_version": 1,
  "description": "DSH Camera Raw: Adobe Camera Raw 式 DNG 操作插件（RustDNG 后端）",
  "ops": ["info", "validate", "previews", "edit", "render", "convert"],
  "transport": "json",
  "notes": "二进制产物写入宿主指定路径；edit 的 edits 数组见 dsh-cr-core::ops::metadata::MetadataEdit"
}"#;

/// ABI 版本查询（破坏性变更递增）。
#[no_mangle]
pub extern "C" fn dsh_cr_abi_version() -> c_uint {
    DSH_CR_ABI_VERSION
}

/// 插件清单（静态 JSON，静态存储期，无需释放）。
#[no_mangle]
pub extern "C" fn dsh_cr_plugin_manifest() -> *const c_char {
    // 静态字符串的中间字节本来就是 NUL 结尾的 '\0' 指针语义——
    // 但为了严格的 C 字符串语义，这里用 unwrap 的编译期常量：
    // MANIFEST_JSON 不含内部 NUL，CString 转换恒成功。
    static MANIFEST: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    MANIFEST
        .get_or_init(|| CString::new(MANIFEST_JSON).expect("manifest 不含内部 NUL"))
        .as_ptr()
}

/// 执行操作。返回新分配的 UTF-8 JSON（用 `dsh_cr_string_free` 释放）；
/// 失败时也返回 JSON 错误对象（永不返回 null，除非内存分配失败）。
///
/// # Safety
/// - `json` 必须指向至少 `len` 字节的可读内存。
/// - 返回值必须且只能用 `dsh_cr_string_free` 释放。
#[no_mangle]
pub unsafe extern "C" fn dsh_cr_execute(json: *const c_char, len: c_uint) -> *const c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if json.is_null() {
            return json!({ "ok": false, "error": { "kind": "BadRequest", "message": "请求指针为空" } });
        }
        let slice = std::slice::from_raw_parts(json as *const u8, len as usize);
        let parsed: Result<Request, _> = serde_json::from_slice(slice);
        match parsed {
            Ok(req) => match execute_request(&req) {
                Ok(mut v) => {
                    // 统一响应包装：成功路径一律补 ok:true（各 op 的 result
                    // 结构由 core 决定）。
                    v["ok"] = json!(true);
                    v
                }
                Err(e) => json!({
                    "ok": false,
                    "error": { "kind": error_kind(&e), "message": e.to_string() }
                }),
            },
            Err(e) => json!({
                "ok": false,
                "error": { "kind": "BadRequest", "message": format!("请求 JSON 解析失败: {e}") }
            }),
        }
    }));

    let value = result.unwrap_or_else(|_| {
        json!({ "ok": false, "error": { "kind": "Panic", "message": "插件内部 panic（已拦截）" } })
    });

    let out = match serde_json::to_string(&value) {
        Ok(s) => s,
        Err(_) => r#"{"ok":false,"error":{"kind":"BadRequest","message":"响应序列化失败"}}"#.into(),
    };

    match CString::new(out) {
        Ok(c) => c.into_raw() as *const c_char,
        Err(_) => std::ptr::null(),
    }
}

/// 释放 `dsh_cr_execute` 返回的字符串。
///
/// # Safety
/// `ptr` 必须是 `dsh_cr_execute` 的返回值，且未被释放过。
#[no_mangle]
pub unsafe extern "C" fn dsh_cr_string_free(ptr: *const c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr as *mut c_char));
    }
}

#[cfg(test)]
mod abi_tests {
    use super::*;

    const SAMPLE05: &str = "/Users/shinano/Opensource/RustDNG/vendor/extracted/dng_sdk_1_7_1/sample_files/05_PGTM2_unsigned8.dng";

    #[test]
    fn manifest_is_valid_json() {
        let v: Value = serde_json::from_str(MANIFEST_JSON).unwrap();
        assert_eq!(v["abi_version"], 1);
        assert!(v["ops"].as_array().unwrap().len() == 6);
    }

    #[test]
    fn execute_roundtrip() {
        // info
        let req = json!({ "op": "info", "input": SAMPLE05 });
        let v: Value = serde_json::from_str(&execute_request(&serde_json::from_value(req).unwrap()).unwrap().to_string()).unwrap();
        assert_eq!(v["result"]["is_valid_dng"], true);

        // validate
        let req: Request = serde_json::from_value(json!({ "op": "validate", "input": SAMPLE05 })).unwrap();
        let v = execute_request(&req).unwrap();
        assert_eq!(v["result"]["all_ok_checker"] .as_bool(), None); // 报告不含该字段，仅结构检查
        assert_eq!(v["result"]["is_valid_dng"], true);

        // edit（内存态）
        let req: Request = serde_json::from_value(json!({
            "op": "edit", "input": SAMPLE05,
            "edits": [{ "field": "rating", "value": 3 }]
        })).unwrap();
        let v = execute_request(&req).unwrap();
        assert_eq!(v["edit"]["applied"].as_array().unwrap().len(), 1);

        // 非法 op
        let bad: Result<Request, _> = serde_json::from_value(json!({ "op": "nope", "input": SAMPLE05 }));
        assert!(bad.is_err());
    }
}
