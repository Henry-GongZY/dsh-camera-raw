//! 操作：校验 DNG（对应 RustDNG `dng-validate` / C++ `dng_validate` 的完整流程：
//! 元数据解析 → stage1 读图 → RAW digest 校验 → stage2 线性化 → stage3 去马赛克）。

use serde::Serialize;

use crate::document::DngDocument;
use crate::error::DshResult;

/// 单阶段结果。
#[derive(Debug, Serialize)]
pub struct StageResult {
    pub ok: bool,
    pub error: Option<String>,
}

impl StageResult {
    fn ok() -> Self {
        StageResult { ok: true, error: None }
    }
    fn err(e: impl std::fmt::Display) -> Self {
        StageResult { ok: false, error: Some(e.to_string()) }
    }
}

/// DNG 校验报告（JSON）。
#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub file: String,
    /// 元数据（IFD + negative）解析是否成功。
    pub metadata_parsed: bool,
    pub is_valid_dng: bool,
    pub stage1_read: StageResult,
    /// RAW 数据 digest 校验：Some(true/false)；跳过时为 None。
    pub raw_digest_ok: Option<bool>,
    pub stage2_built: StageResult,
    pub stage3_built: StageResult,
}

impl ValidationReport {
    /// 全部通过的校验报告才算合格。
    pub fn all_ok(&self) -> bool {
        self.metadata_parsed
            && self.is_valid_dng
            && self.stage1_read.ok
            && self.raw_digest_ok.unwrap_or(false)
            && self.stage2_built.ok
            && self.stage3_built.ok
    }
}

/// 对一个 DNG 文件执行完整校验，返回报告（不抛异常——错误都进报告）。
pub fn validate_file(path: impl AsRef<std::path::Path>) -> DshResult<ValidationReport> {
    let mut report = ValidationReport {
        file: path.as_ref().display().to_string(),
        metadata_parsed: false,
        is_valid_dng: false,
        stage1_read: StageResult { ok: false, error: None },
        raw_digest_ok: None,
        stage2_built: StageResult { ok: false, error: None },
        stage3_built: StageResult { ok: false, error: None },
    };

    // 宽松打开：即使不是合法 DNG 也要给出诊断报告
    let mut doc = match DngDocument::open_lenient(&path) {
        Ok(doc) => doc,
        Err(e) => {
            report.stage1_read = StageResult::err(&e);
            return Ok(report);
        }
    };
    report.metadata_parsed = true;
    report.is_valid_dng = doc.info.is_valid_dng();

    match doc.negative.read_stage1_image(&mut doc.host, &mut doc.stream, &mut doc.info) {
        Ok(()) => report.stage1_read = StageResult::ok(),
        Err(e) => {
            report.stage1_read = StageResult::err(&e);
            return Ok(report);
        }
    }

    match doc.negative.validate_raw_image_digest(&mut doc.host) {
        Ok(()) => report.raw_digest_ok = Some(true),
        Err(e) => {
            report.raw_digest_ok = Some(false);
            report.stage2_built = StageResult::err(&e);
            return Ok(report);
        }
    }

    match doc.negative.build_stage2_image(&mut doc.host) {
        Ok(()) => report.stage2_built = StageResult::ok(),
        Err(e) => {
            report.stage2_built = StageResult::err(&e);
            return Ok(report);
        }
    }

    match doc.negative.build_stage3_image(&mut doc.host, -1) {
        Ok(()) => report.stage3_built = StageResult::ok(),
        Err(e) => report.stage3_built = StageResult::err(&e),
    }

    Ok(report)
}
