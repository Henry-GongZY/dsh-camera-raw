//! 统一错误类型：上层（插件 ABI / CLI / 宿主）只看得到 `DshError`。

use std::fmt;

/// DSH Camera Raw 插件引擎的统一错误。
#[derive(Debug)]
pub enum DshError {
    /// 底层 RustDNG 返回的错误（镜像 C++ SDK 的 `dng_exception` / `dng_errors.h`）。
    Dng(dng_core::DngError),
    /// 文件系统 / IO 错误。
    Io(std::io::Error),
    /// 文件能解析但不是合法 DNG（`is_valid_dng` 失败）。
    NotDng(String),
    /// 请求参数不合法（插件 ABI 的 JSON 请求由宿主负责构造）。
    BadRequest(String),
    /// 功能尚未实现（诚实标注缺口，不允许静默降级）。
    Unimplemented(String),
}

impl fmt::Display for DshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DshError::Dng(e) => write!(f, "DNG SDK 错误: {e}"),
            DshError::Io(e) => write!(f, "IO 错误: {e}"),
            DshError::NotDng(m) => write!(f, "不是合法的 DNG 文件: {m}"),
            DshError::BadRequest(m) => write!(f, "请求参数不合法: {m}"),
            DshError::Unimplemented(m) => write!(f, "功能未实现: {m}"),
        }
    }
}

impl std::error::Error for DshError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DshError::Dng(e) => Some(e),
            DshError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<dng_core::DngError> for DshError {
    fn from(e: dng_core::DngError) -> Self {
        DshError::Dng(e)
    }
}

impl From<std::io::Error> for DshError {
    fn from(e: std::io::Error) -> Self {
        DshError::Io(e)
    }
}

pub type DshResult<T> = Result<T, DshError>;
