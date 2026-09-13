use std::fmt;

#[derive(Debug)]
pub struct AppError {
    pub code: i32,
    pub msg: String,
}

pub type Res<T> = Result<T, AppError>;

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl std::error::Error for AppError {}

impl AppError {
    pub fn new(code: i32, msg: impl Into<String>) -> AppError {
        AppError { code, msg: msg.into() }
    }
}

pub fn fail<T>(code: i32, msg: impl Into<String>) -> Res<T> {
    Err(AppError::new(code, msg))
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::new(5, format!("IO 错误: {}", e))
    }
}

impl From<ureq::Error> for AppError {
    fn from(e: ureq::Error) -> Self {
        let code = match &e {
            ureq::Error::Status(_, _) => 4,
            _ => 3,
        };
        AppError::new(code, format!("网络错误: {}", e))
    }
}

impl From<native_tls::Error> for AppError {
    fn from(e: native_tls::Error) -> Self {
        AppError::new(3, format!("TLS 错误: {}", e))
    }
}
