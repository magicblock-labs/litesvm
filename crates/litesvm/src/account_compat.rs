//! Conversion shims between the forked `solana_account` (alias of
//! `magicblock-solana-account`) used throughout litesvm and the stock
//! `solana_account` (`solana_account_stock`) used by the agave runtime
//! crates pulled from crates.io.
//!
//! These are only needed at the boundaries where litesvm hands accounts to,
//! or reads accounts back from, the agave runtime. The fork's extra flags
//! (delegated/undelegating/privileged/...) are not represented in the stock
//! type, so they default off when converting stock -> fork.

use {
    solana_account::{
        Account as ForkAccount, AccountBuilder, AccountMode,
        AccountSharedData as ForkAccountSharedData, ReadableAccount,
    },
    solana_account_stock::{
        Account as StockAccount, AccountSharedData as StockAccountSharedData,
        ReadableAccount as StockReadableAccount,
    },
};

/// Convert a fork account into the stock account expected by the agave runtime.
pub fn fork_to_stock(account: &ForkAccountSharedData) -> StockAccountSharedData {
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
pub fn stock_to_fork(account: &StockAccountSharedData) -> ForkAccountSharedData {
    ForkAccountSharedData::from(ForkAccount {
        lamports: StockReadableAccount::lamports(account),
        data: StockReadableAccount::data(account).to_vec(),
        owner: *StockReadableAccount::owner(account),
        executable: StockReadableAccount::executable(account),
        rent_epoch: StockReadableAccount::rent_epoch(account),
    })
}

/// Convert a stock account back into a fork account and restore its lifecycle
/// mode, which the stock type cannot represent.
pub fn stock_to_fork_with_mode(
    account: &StockAccountSharedData,
    mode: AccountMode,
) -> ForkAccountSharedData {
    AccountBuilder::from(stock_to_fork(account))
        .mode(mode)
        .build()
}

/// Copies MagicBlock fork flags from `pre` onto `post` when the stock runtime
/// round-trip dropped them. Post-transaction lamports, data, owner, and
/// executable are kept. Live ephemeral accounts stay ephemeral even when a
/// transaction returns them to zero lamports (create → credit → debit).
pub(crate) fn preserve_mode(pre: &ForkAccountSharedData, post: &mut ForkAccountSharedData) {
    *post = AccountBuilder::from(std::mem::take(post))
        .mode(pre.mode())
        .build();
}
