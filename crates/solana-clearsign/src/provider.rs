//! Host-supplied raw account lookup boundary.

use std::future::Future;
use std::pin::Pin;

/// Raw account bytes fetched by the host. The strict sRFC 39 renderer carries
/// the owner through its boundary but leaves owner validation to host policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountData {
    /// Base58 address of the program that owns the account.
    pub owner: String,
    /// The account's full raw data.
    pub data: Vec<u8>,
}

pub(crate) type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Account-only provider used by the native sRFC 39 engine.
///
/// The host owns all I/O and policy. The renderer asks only for raw account
/// bytes required by links in the supplied IDL.
pub trait Srf39AccountProvider: Send + Sync {
    fn resolve_account<'a>(&'a self, address: &'a str) -> Fut<'a, Option<AccountData>>;
}
