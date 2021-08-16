
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("invalid token provided to internal function")]
    InvalidToken,

    #[error("underlying IO error")]
    IoError(#[from] std::io::Error),

    #[error("error generated in event callback")]
    CallbackError(#[from] Box<dyn std::error::Error>),

    #[error("error generated in event callback")]
    CallbackErrorSync(#[from] Box<dyn std::error::Error + Sync + Send>)
}

impl From<nix::errno::Errno> for Error {
    fn from(err: nix::errno::Errno) -> Self {
        Into::<std::io::Error>::into(err).into()
    }
}

impl From<Error> for std::io::Error {
    fn from(err: Error) -> Self {
        match err {
            Error::IoError(source) => Self::new(source.kind(), source),
            Error::InvalidToken => Self::new(std::io::ErrorKind::InvalidInput, err.to_string()),
            Error::CallbackError(src) => Self::new(std::io::ErrorKind::Other, src.to_string()),
            Error::CallbackErrorSync(src) => Self::new(std::io::ErrorKind::Other, src),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;
