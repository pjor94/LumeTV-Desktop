// SPDX-License-Identifier: GPL-3.0-or-later
mod install;
mod network;
pub mod policy;
mod service;
#[cfg(test)]
mod tests;
pub use install::run_helper;
use serde::Serialize;
pub use service::UpdateService;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Error {
    Network,
    Metadata,
    Signature,
    Checksum,
    RateLimited,
    Disk,
    Busy,
    NotReady,
    Install,
    Cancelled,
    Unsupported,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
