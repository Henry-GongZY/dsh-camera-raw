//! DSH Camera Raw CLI 宿主。
//!
//! 这是插件引擎的第一个「宿主」：既面向用户（等价 Adobe DNG Converter 的
//! 命令行子集），也用于每次迭代的自验收（对样例库批量跑操作并比对结果）。
//!
//! 用法：
//! ```text
//! dsh-cr-cli info     <file.dng>... [--pretty]
//! dsh-cr-cli validate <file.dng>... [--pretty]
//! dsh-cr-cli previews <file.dng> [out-dir] [--pretty]     # 无 out-dir 仅列出
//! dsh-cr-cli edit     <file.dng> [编辑项...] [--save out.dng]
//!                     [--uncompressed] [--backward-version 1.4] [--pretty]
//! dsh-cr-cli convert  <file.dng> <out.dng> [--compression auto|uncompressed]
//!                     [--backward-version 1.1|1.4|1.6|1.7|save] [--pretty]
//! dsh-cr-cli render   <file.dng> <out.tif|out.ppm> [--cs srgb|adobe_rgb|...]
//!                     [--depth u8|u16|f32] [--exposure F] [--shadows F]
//!                     [--max-size N] [--pretty]
//!
//! 编辑项：
//!   --description S   --artist S   --copyright S  --software S
//!   --user-comment S  --date-original S(ISO 或 EXIF 格式)
//!   --rating N        --label S
//! ```

use std::process::ExitCode;

use dsh_cr_core::ops::convert;
use dsh_cr_core::ops::{info, metadata, preview, render, validate};

const USAGE: &str = "用法:\n  dsh-cr-cli info     <file.dng>... [--pretty]\n  dsh-cr-cli validate <file.dng>... [--pretty]\n  dsh-cr-cli previews <file.dng> [out-dir] [--pretty]\n  dsh-cr-cli edit     <file.dng> [编辑项...] [--save out.dng] [--uncompressed] [--backward-version 1.1|1.4|1.6|1.7|save]\n  dsh-cr-cli render   <file.dng> <out.tif|out.ppm> [--cs srgb|adobe_rgb|pro_photo|gray18|gray22|display_p3|rec2020] [--depth u8|u16|f32] [--exposure F] [--max-size N]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("{msg}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// 解析后的命令行。
struct Parsed {
    op: String,
    files: Vec<String>,
    pretty: bool,
    /// previews 子命令的输出目录。
    out_dir: Option<String>,
    edits: Vec<dsh_cr_core::ops::metadata::MetadataEdit>,
    save: Option<String>,
    save_opts: metadata::SaveOptions,
    render_out: Option<String>,
    render_params: render::RenderParams,
    convert_out: Option<String>,
    convert_opts: convert::ConvertOptions,
}

fn parse_args(args: &[String]) -> Result<Parsed, String> {
    use dsh_cr_core::ops::metadata::{MetadataEdit, SaveOptions};

    let mut p = Parsed {
        op: String::new(),
        files: Vec::new(),
        pretty: false,
        out_dir: None,
        edits: Vec::new(),
        save: None,
        save_opts: SaveOptions::default(),
        render_out: None,
        render_params: render::RenderParams::default(),
        convert_out: None,
        convert_opts: convert::ConvertOptions::default(),
    };

    let mut i = 0;
    // 首个位置参数是子命令
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "info" | "validate" | "previews" | "edit" | "render" | "convert" if p.op.is_empty() => {
                p.op = a.clone()
            }
            "--pretty" => p.pretty = true,
            "--description" => p.edits.push(MetadataEdit::ImageDescription(next(args, &mut i, a)?.clone())),
            "--artist" => p.edits.push(MetadataEdit::Artist(next(args, &mut i, a)?.clone())),
            "--copyright" => p.edits.push(MetadataEdit::Copyright(next(args, &mut i, a)?.clone())),
            "--software" => p.edits.push(MetadataEdit::Software(next(args, &mut i, a)?.clone())),
            "--user-comment" => p.edits.push(MetadataEdit::UserComment(next(args, &mut i, a)?.clone())),
            "--date-original" => p.edits.push(MetadataEdit::DateTimeOriginal(next(args, &mut i, a)?.clone())),
            "--rating" => {
                let v = next(args, &mut i, a)?;
                let n: i32 = v.parse().map_err(|_| format!("--rating 需要整数，得到: {v}"))?;
                p.edits.push(MetadataEdit::Rating(n));
            }
            "--label" => p.edits.push(MetadataEdit::Label(next(args, &mut i, a)?.clone())),
            "--save" => p.save = Some(next(args, &mut i, a)?.clone()),
            "--uncompressed" => {
                // convert 子命令用 --compression；edit --save 沿用本开关
                p.save_opts.uncompressed = true;
                p.convert_opts.compression = Some("uncompressed".into());
            }
            "--compression" => p.convert_opts.compression = Some(next(args, &mut i, a)?.clone()),
            "--embed-original" => p.convert_opts.embed_original = Some(true),
            "--cs" => p.render_params.color_space = Some(next(args, &mut i, a)?.clone()),
            "--depth" => p.render_params.depth = Some(next(args, &mut i, a)?.clone()),
            "--exposure" => {
                let v = next(args, &mut i, a)?;
                let f: f64 = v
                    .parse()
                    .map_err(|_| format!("--exposure 需要数字，得到: {v}"))?;
                p.render_params.exposure = Some(f);
            }
            "--shadows" => {
                let v = next(args, &mut i, a)?;
                let f: f64 = v
                    .parse()
                    .map_err(|_| format!("--shadows 需要数字，得到: {v}"))?;
                p.render_params.shadows = Some(f);
            }
            "--max-size" => {
                let v = next(args, &mut i, a)?;
                let n: u32 = v
                    .parse()
                    .map_err(|_| format!("--max-size 需要整数，得到: {v}"))?;
                p.render_params.max_size = Some(n);
            }
            "--backward-version" => {
                let v = next(args, &mut i, a)?;
                use dng_core::dng_tag_values::{
                    dngVersion_1_1_0_0, dngVersion_1_4_0_0, dngVersion_1_6_0_0, dngVersion_1_7_0_0,
                };
                let ver = match v.as_str() {
                    "1.1" => Some(dngVersion_1_1_0_0),
                    "1.4" => Some(dngVersion_1_4_0_0),
                    "1.6" => Some(dngVersion_1_6_0_0),
                    "1.7" => Some(dngVersion_1_7_0_0),
                    "save" | "auto" => None,
                    _ => return Err(format!("未知版本: {v}（可选 1.1/1.4/1.6/1.7/save）")),
                };
                p.save_opts.max_backward_version = ver;
            }
            s if s.starts_with('-') => return Err(format!("未知选项: {s}")),
            _ => match (
                p.op.as_str(),
                p.out_dir.is_none(),
                p.render_out.is_none(),
                p.convert_out.is_none(),
                p.files.is_empty(),
            ) {
                ("previews", true, _, _, false) => p.out_dir = Some(a.clone()),
                ("render", _, true, _, true) => p.files.push(a.clone()),
                ("render", _, _, _, false) => p.render_out = Some(a.clone()),
                ("convert", _, _, true, true) => p.files.push(a.clone()),
                ("convert", _, _, _, false) => p.convert_out = Some(a.clone()),
                _ => p.files.push(a.clone()),
            },
        }
        i += 1;
    }

    if p.op.is_empty() {
        return Err("缺少子命令".into());
    }
    if p.files.is_empty() {
        return Err(format!("{} 至少需要一个文件", p.op));
    }
    Ok(p)
}

fn next<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a String, String> {
    *i += 1;
    args.get(*i).ok_or_else(|| format!("{flag} 缺少值"))
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let p = parse_args(args)?;
    let pretty = p.pretty;
    let mut had_error = false;

    for (n, f) in p.files.iter().enumerate() {
        if n > 0 {
            println!();
        }
        let result = dispatch(&p, f);
        match result {
            Ok(v) => println!(
                "{}",
                if pretty {
                    serde_json::to_string_pretty(&v).unwrap()
                } else {
                    v.to_string()
                }
            ),
            Err(e) => {
                had_error = true;
                eprintln!("错误 [{f}]: {e}");
            }
        }
    }

    if had_error {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn dispatch(p: &Parsed, f: &str) -> Result<serde_json::Value, String> {
    use dsh_cr_core::{ops::convert, ops::metadata, ops::preview, ops::render, DngDocument};
    match p.op.as_str() {
        "info" => DngDocument::open(f)
            .map(|mut doc| serde_json::to_value(info::summarize(&mut doc)).unwrap())
            .map_err(|e| e.to_string()),
        "validate" => validate::validate_file(f)
            .map(|report| serde_json::to_value(&report).unwrap())
            .map_err(|e| e.to_string()),
        "previews" => DngDocument::open(f)
            .map_err(|e| e.to_string())
            .and_then(|mut doc| match &p.out_dir {
                Some(dir) => preview::extract_previews(&mut doc, dir)
                    .map(|r| serde_json::to_value(&r).unwrap())
                    .map_err(|e| e.to_string()),
                None => Ok(serde_json::to_value(preview::list_previews(&doc)).unwrap()),
            }),
        "render" => {
            let out = p
                .render_out
                .as_ref()
                .ok_or_else(|| "render 需要输出路径（.tif 或 .ppm）".to_string())?;
            DngDocument::open(f)
                .map_err(|e| e.to_string())
                .and_then(|mut doc| {
                    render::render_to(&mut doc, out, &p.render_params).map_err(|e| e.to_string())
                })
                .map(|r| serde_json::to_value(&r).unwrap())
        }
        "convert" => {
            let out = p
                .convert_out
                .as_ref()
                .ok_or_else(|| "convert 需要输出路径（out.dng）".to_string())?;
            DngDocument::open(f)
                .map_err(|e| e.to_string())
                .and_then(|mut doc| {
                    convert::convert_to(&mut doc, out, &p.convert_opts)
                        .map_err(|e| e.to_string())
                })
                .map(|r| serde_json::to_value(&r).unwrap())
        }
        "edit" => {
            let mut doc = DngDocument::open(f).map_err(|e| e.to_string())?;
            let report = metadata::edit_metadata(&mut doc, &p.edits).map_err(|e| e.to_string())?;
            let value = match &p.save {
                Some(out) => {
                    let sr = metadata::save_as_dng(&mut doc, out, &p.save_opts)
                        .map_err(|e| e.to_string())?;
                    let mut obj = serde_json::Map::new();
                    obj.insert("edit".into(), serde_json::to_value(&report).unwrap());
                    obj.insert("save".into(), serde_json::to_value(&sr).unwrap());
                    serde_json::Value::Object(obj)
                }
                None => serde_json::to_value(&report).unwrap(),
            };
            Ok(value)
        }
        other => Err(format!("未知子命令: {other}")),
    }
}
