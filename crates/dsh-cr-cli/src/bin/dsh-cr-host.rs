//! dlopen 宿主：模拟第三方应用加载 libdsh_cr_plugin.dylib，
//! 走纯 C ABI 跑全套操作——「插件可被任意语言宿主使用」的最终验证。

use libloading::{Library, Symbol};
use std::ffi::{c_char, CStr, CString};
use std::process::ExitCode;

const SAMPLE05: &str = "/Users/shinano/Opensource/RustDNG/vendor/extracted/dng_sdk_1_7_1/sample_files/05_PGTM2_unsigned8.dng";
const SAMPLE01: &str = "/Users/shinano/Opensource/RustDNG/vendor/extracted/dng_sdk_1_7_1/sample_files/01_jxl_linear_raw_integer.dng";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dylib = args
        .first()
        .map(|s| s.as_str())
        .unwrap_or("target/debug/libdsh_cr_plugin.dylib");

    unsafe {
        let lib = match Library::new(dylib) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("dlopen {dylib} 失败: {e}");
                return ExitCode::FAILURE;
            }
        };
        println!("✅ dlopen {dylib}");

        let abi_version: Symbol<unsafe extern "C" fn() -> u32> =
            lib.get(b"dsh_cr_abi_version").unwrap();
        let v = abi_version();
        println!("✅ dsh_cr_abi_version() = {v}");
        if v != 1 {
            eprintln!("ABI 版本不匹配（期望 1）");
            return ExitCode::FAILURE;
        }

        let manifest_fn: Symbol<unsafe extern "C" fn() -> *const c_char> =
            lib.get(b"dsh_cr_plugin_manifest").unwrap();
        let manifest = CStr::from_ptr(manifest_fn()).to_str().unwrap();
        let m: serde_json::Value = serde_json::from_str(manifest).unwrap();
        println!("✅ manifest: name={} ops={:?}", m["name"], m["ops"]);

        let exec_fn: Symbol<unsafe extern "C" fn(*const c_char, u32) -> *const c_char> =
            lib.get(b"dsh_cr_execute").unwrap();
        let free_fn: Symbol<unsafe extern "C" fn(*const c_char)> =
            lib.get(b"dsh_cr_string_free").unwrap();

        let mut pass = 0;
        let mut fail = 0;
        let run = |exec_fn: &Symbol<unsafe extern "C" fn(*const c_char, u32) -> *const c_char>,
                   free_fn: &Symbol<unsafe extern "C" fn(*const c_char)>,
                   label: &str,
                   req: serde_json::Value,
                   check: &dyn Fn(&serde_json::Value) -> Result<(), String>|
         -> (usize, usize) {
            let s = serde_json::to_string(&req).unwrap();
            let c = CString::new(s).unwrap();
            let out_ptr = exec_fn(c.as_ptr(), c.as_bytes().len() as u32);
            let out = CStr::from_ptr(out_ptr).to_str().unwrap().to_string();
            free_fn(out_ptr);
            let v: serde_json::Value = serde_json::from_str(&out).unwrap();
            match check(&v) {
                Ok(()) => {
                    println!("✅ {label}");
                    (1, 0)
                }
                Err(msg) => {
                    println!("❌ {label}: {msg}\n   响应: {}", &out[..out.len().min(300)]);
                    (0, 1)
                }
            }
        };

        // 1. info
        let (p, f) = run(
            &exec_fn,
            &free_fn,
            "info",
            serde_json::json!({ "op": "info", "input": SAMPLE05 }),
            &|v| {
                if v["ok"] == true && v["result"]["is_valid_dng"] == true {
                    Ok(())
                } else {
                    Err("校验失败".into())
                }
            },
        );
        pass += p;
        fail += f;

        // 2. validate
        let (p, f) = run(
            &exec_fn,
            &free_fn,
            "validate",
            serde_json::json!({ "op": "validate", "input": SAMPLE05 }),
            &|v| {
                let r = &v["result"];
                if v["ok"] == true
                    && r["is_valid_dng"] == true
                    && r["raw_digest_ok"] == true
                    && r["stage3_built"]["ok"] == true
                {
                    Ok(())
                } else {
                    Err("校验报告不全绿".into())
                }
            },
        );
        pass += p;
        fail += f;

        // 3. previews（列表）
        let (p, f) = run(
            &exec_fn,
            &free_fn,
            "previews",
            serde_json::json!({ "op": "previews", "input": SAMPLE01 }),
            &|v| {
                if v["ok"] == true
                    && v["previews"]
                        .as_array()
                        .map(|a| !a.is_empty())
                        .unwrap_or(false)
                {
                    Ok(())
                } else {
                    Err("样例 01 应有一个 256×160 缩略图".into())
                }
            },
        );
        pass += p;
        fail += f;

        // 4. edit（内存态）
        let (p, f) = run(
            &exec_fn,
            &free_fn,
            "edit(内存)",
            serde_json::json!({
                "op": "edit", "input": SAMPLE05,
                "edits": [{ "field": "rating", "value": 4 }, { "field": "label", "value": "Host" }]
            }),
            &|v| {
                if v["ok"] == true && v["edit"]["applied"].as_array().unwrap().len() == 2 {
                    Ok(())
                } else {
                    Err("applied != 2".into())
                }
            },
        );
        pass += p;
        fail += f;

        // 5. edit + 另存
        let (p, f) = run(
            &exec_fn,
            &free_fn,
            "edit+save",
            serde_json::json!({
                "op": "edit", "input": SAMPLE05,
                "edits": [{ "field": "artist", "value": "dlopen 宿主" }],
                "save": "/tmp/dsh-cr-test/host_save.dng", "uncompressed": true
            }),
            &|v| {
                if v["ok"] == true && v["save"]["file_size_bytes"].as_u64().unwrap_or(0) > 100000 {
                    Ok(())
                } else {
                    Err("save 报告异常".into())
                }
            },
        );
        pass += p;
        fail += f;

        // 6. render
        let (p, f) = run(
            &exec_fn,
            &free_fn,
            "render(ppm)",
            serde_json::json!({
                "op": "render", "input": SAMPLE05, "output": "/tmp/dsh-cr-test/host_render.ppm",
                "params": { "color_space": "srgb", "depth": "u8" }
            }),
            &|v| {
                if v["ok"] == true && v["result"]["width"] == 200 {
                    Ok(())
                } else {
                    Err("尺寸不符".into())
                }
            },
        );
        pass += p;
        fail += f;

        // 7. convert
        let (p, f) = run(
            &exec_fn,
            &free_fn,
            "convert(无损)",
            serde_json::json!({
                "op": "convert", "input": SAMPLE05, "output": "/tmp/dsh-cr-test/host_conv.dng"
            }),
            &|v| {
                if v["ok"] == true && v["result"]["compression"] == "lossless_jpeg" {
                    Ok(())
                } else {
                    Err("压缩类型不符".into())
                }
            },
        );
        pass += p;
        fail += f;

        // 8. 错误路径：非法 op
        let (p, f) = run(
            &exec_fn,
            &free_fn,
            "错误路径 BadRequest",
            serde_json::json!({ "op": "nope", "input": SAMPLE05 }),
            &|v| {
                if v["ok"] == false && v["error"]["kind"] == "BadRequest" {
                    Ok(())
                } else {
                    Err("应得 BadRequest".into())
                }
            },
        );
        pass += p;
        fail += f;

        // 9. 错误路径：Unimplemented（嵌入原片）
        let (p, f) = run(
            &exec_fn,
            &free_fn,
            "错误路径 Unimplemented",
            serde_json::json!({
                "op": "convert", "input": SAMPLE05, "output": "/tmp/dsh-cr-test/x.dng",
                "options": { "embed_original": true }
            }),
            &|v| {
                if v["ok"] == false && v["error"]["kind"] == "Unimplemented" {
                    Ok(())
                } else {
                    Err("应得 Unimplemented".into())
                }
            },
        );
        pass += p;
        fail += f;

        // 10. 坏 JSON
        let bad = CString::new("{not json").unwrap();
        let out_ptr = exec_fn(bad.as_ptr(), bad.as_bytes().len() as u32);
        let out = CStr::from_ptr(out_ptr).to_str().unwrap().to_string();
        free_fn(out_ptr);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        if v["ok"] == false && v["error"]["kind"] == "BadRequest" {
            println!("✅ 错误路径 坏JSON");
            pass += 1;
        } else {
            println!("❌ 错误路径 坏JSON");
            fail += 1;
        }

        // 11. 空指针防护
        let out_ptr = exec_fn(std::ptr::null(), 0);
        let out = CStr::from_ptr(out_ptr).to_str().unwrap().to_string();
        free_fn(out_ptr);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        if v["ok"] == false && v["error"]["kind"] == "BadRequest" {
            println!("✅ 错误路径 空指针");
            pass += 1;
        } else {
            println!("❌ 错误路径 空指针");
            fail += 1;
        }

        println!("\n宿主验证结果: {pass} 通过, {fail} 失败");
        if fail == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}
