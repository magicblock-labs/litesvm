//! Conversion shims between the forked `solana_account` (alias of
//! `magicblock-account`) used throughout litesvm and the stock
//! `solana_account` (`solana_account_stock`) used by the agave runtime
//! crates pulled from crates.io.
//!
//! These are only needed at the boundaries where litesvm hands accounts to,
//! or reads accounts back from, the agave runtime. The fork's extra flags
//! (delegated/undelegating/privileged/...) are not represented in the stock
//! type, so they default off when converting stock -> fork.

use {
    solana_account::{AccountSharedData as ForkAccountSharedData, ReadableAccount},
    solana_account_stock::{
        Account as StockAccount, AccountSharedData as StockAccountSharedData,
        ReadableAccount as StockReadableAccount,
    },
};

/// Convert a fork account into the stock account expected by the agave runtime.
pub(crate) fn fork_to_stock(account: &ForkAccountSharedData) -> StockAccountSharedData {
    StockAccountSharedData::from(StockAccount {
        lamports: account.lamports(),
        data: account.data().to_vec(),
        owner: *account.owner(),
        executable: account.executable(),
        rent_epoch: account.rent_epoch(),
    })
}

/// Convert a stock account returned by the agave runtime back into a fork
/// account. Fork-specific flags default off.
pub(crate) fn stock_to_fork(account: &StockAccountSharedData) -> ForkAccountSharedData {
    ForkAccountSharedData::from(solana_account::Account {
        lamports: StockReadableAccount::lamports(account),
        data: StockReadableAccount::data(account).to_vec(),
        owner: *StockReadableAccount::owner(account),
        executable: StockReadableAccount::executable(account),
        rent_epoch: StockReadableAccount::rent_epoch(account),
    })
}
