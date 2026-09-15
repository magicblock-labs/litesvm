mod error;
mod types;

pub use error::PersistenceError;
use {
    litesvm::LiteSVM,
    std::{
        fs::File,
        io::{BufWriter, Read, Write},
        path::Path,
    },
    types::{
        AccountEntryWire, AccountEntryWireV4, FeatureSetSnapshot, LiteSvmSnapshotV1,
        LiteSvmSnapshotV2, LiteSvmSnapshotV3, LiteSvmSnapshotV4, TxResult,
    },
    wincode::{Deserialize, Serialize},
};

const V1_STATE_VERSION: u8 = 1;
const V2_STATE_VERSION: u8 = 2;
const V3_STATE_VERSION: u8 = 3;
const STATE_VERSION: u8 = 4;

fn extract_snapshot_v2(svm: &LiteSVM) -> LiteSvmSnapshotV2 {
    LiteSvmSnapshotV2 {
        // AccountSharedData::clone is an Arc bump — no underlying data copy.
        // The actual data bytes are written once during serialization via AccountSchema.
        accounts: svm
            .accounts_db()
            .inner
            .iter()
            .map(|(k, v)| AccountEntryWire::from((*k, litesvm::account_compat::fork_to_stock(v))))
            .collect(),
        airdrop_kp: *svm.airdrop_keypair_bytes(),
        feature_set: FeatureSetSnapshot::from_feature_set(svm.get_feature_set_ref()),
        latest_blockhash: svm.latest_blockhash(),
        history: svm
            .transaction_history_entries()
            .iter()
            .map(|(k, v)| (*k, TxResult::from_result(v.clone())))
            .collect(),
        history_capacity: svm.transaction_history_capacity() as u64,
        compute_budget: svm.get_compute_budget(),
        sigverify: svm.get_sigverify(),
        blockhash_check: svm.get_blockhash_check(),
        fee_structure: svm.get_fee_structure().clone(),
        log_bytes_limit: svm.get_log_bytes_limit().map(|v| v as u64),
    }
}

fn extract_snapshot(svm: &LiteSVM) -> LiteSvmSnapshotV4 {
    let mut epoch_vote_stakes: Vec<_> = svm
        .epoch_vote_stakes()
        .map(|(vote_account, stake)| (*vote_account, *stake))
        .collect();
    epoch_vote_stakes.sort_unstable_by_key(|(vote_account, _)| *vote_account);
    let mut snapshot = LiteSvmSnapshotV4::from(LiteSvmSnapshotV3 {
        state: extract_snapshot_v2(svm),
        epoch_vote_stakes,
    });
    for entry in &mut snapshot.accounts {
        if let Some(account) = svm.accounts_db().inner.get(&entry.address) {
            entry.mode = account.mode();
        }
    }
    snapshot
}

fn restore_from_snapshot(snapshot: LiteSvmSnapshotV4) -> Result<LiteSVM, PersistenceError> {
    let LiteSvmSnapshotV4 {
        accounts,
        airdrop_kp,
        feature_set,
        latest_blockhash,
        history,
        history_capacity,
        compute_budget,
        sigverify,
        blockhash_check,
        fee_structure,
        log_bytes_limit,
        mut epoch_vote_stakes,
    } = snapshot;
    let feature_set = feature_set.into_feature_set();
    let mut svm = LiteSVM::default().with_feature_set(feature_set);

    svm = svm
        .with_sigverify(sigverify)
        .with_blockhash_check(blockhash_check)
        .with_log_bytes_limit(log_bytes_limit.map(|v| v as usize));

    if let Some(cb) = compute_budget {
        svm = svm.with_compute_budget(cb);
    }

    svm.set_fee_structure(fee_structure);
    svm.set_latest_blockhash(latest_blockhash);
    svm.set_airdrop_keypair(airdrop_kp);
    epoch_vote_stakes.sort_unstable_by_key(|(vote_account, _)| *vote_account);
    if let Some(entries) = epoch_vote_stakes
        .windows(2)
        .find(|entries| entries[0].0 == entries[1].0)
    {
        return Err(PersistenceError::DuplicateEpochStake(entries[0].0));
    }
    svm.set_epoch_stakes(epoch_vote_stakes)
        .map_err(PersistenceError::InvalidEpochStakes)?;

    for AccountEntryWireV4 {
        address,
        account,
        mode,
    } in accounts
    {
        svm.set_account_no_checks(
            address,
            litesvm::account_compat::stock_to_fork_with_mode(&account, mode),
        );
    }

    svm.restore_transaction_history(
        history
            .into_iter()
            .map(|(k, v)| (k, v.into_result()))
            .collect(),
        history_capacity as usize,
    );

    svm.rebuild_caches()?;

    Ok(svm)
}

fn deserialize_snapshot(version: u8, bytes: &[u8]) -> Result<LiteSvmSnapshotV4, PersistenceError> {
    match version {
        V1_STATE_VERSION => {
            let snapshot: LiteSvmSnapshotV2 = LiteSvmSnapshotV1::deserialize(bytes)?.into();
            Ok(LiteSvmSnapshotV3::from(snapshot).into())
        }
        V2_STATE_VERSION => {
            Ok(LiteSvmSnapshotV3::from(LiteSvmSnapshotV2::deserialize(bytes)?).into())
        }
        V3_STATE_VERSION => Ok(LiteSvmSnapshotV3::deserialize(bytes)?.into()),
        STATE_VERSION => Ok(LiteSvmSnapshotV4::deserialize(bytes)?),
        version => Err(PersistenceError::UnsupportedVersion(version)),
    }
}

/// Saves the full LiteSVM state to a file.
pub fn save_to_file(svm: &LiteSVM, path: impl AsRef<Path>) -> Result<(), PersistenceError> {
    let snapshot = extract_snapshot(svm);
    let mut writer = BufWriter::new(File::create(path)?);
    let payload_size = LiteSvmSnapshotV4::serialized_size(&snapshot)? as usize;
    let mut payload = Vec::with_capacity(payload_size);
    LiteSvmSnapshotV4::serialize_into(&mut payload, &snapshot)?;
    writer.write_all(&[STATE_VERSION])?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

/// Loads a full LiteSVM state from a file.
pub fn load_from_file(path: impl AsRef<Path>) -> Result<LiteSVM, PersistenceError> {
    let mut reader = File::open(path)?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    let (version, rest) = bytes.split_first().ok_or(PersistenceError::EmptyInput)?;
    let snapshot = deserialize_snapshot(*version, rest)?;
    restore_from_snapshot(snapshot)
}

/// Serializes the full LiteSVM state to bytes.
pub fn to_bytes(svm: &LiteSVM) -> Result<Vec<u8>, PersistenceError> {
    let snapshot = extract_snapshot(svm);
    let payload_size = LiteSvmSnapshotV4::serialized_size(&snapshot)? as usize;
    let mut buf = Vec::with_capacity(1 + payload_size);
    buf.push(STATE_VERSION);
    LiteSvmSnapshotV4::serialize_into(&mut buf, &snapshot)?;
    Ok(buf)
}

/// Deserializes the full LiteSVM state from bytes.
pub fn from_bytes(bytes: &[u8]) -> Result<LiteSVM, PersistenceError> {
    let (version, rest) = bytes.split_first().ok_or(PersistenceError::EmptyInput)?;
    let snapshot = deserialize_snapshot(*version, rest)?;
    restore_from_snapshot(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serialize_v2(snapshot: &LiteSvmSnapshotV2) -> Vec<u8> {
        let payload_size = LiteSvmSnapshotV2::serialized_size(snapshot).unwrap() as usize;
        let mut bytes = Vec::with_capacity(1 + payload_size);
        bytes.push(V2_STATE_VERSION);
        LiteSvmSnapshotV2::serialize_into(&mut bytes, snapshot).unwrap();
        bytes
    }

    fn serialize_v3(snapshot: &LiteSvmSnapshotV3) -> Vec<u8> {
        let payload_size = LiteSvmSnapshotV3::serialized_size(snapshot).unwrap() as usize;
        let mut bytes = Vec::with_capacity(1 + payload_size);
        bytes.push(V3_STATE_VERSION);
        LiteSvmSnapshotV3::serialize_into(&mut bytes, snapshot).unwrap();
        bytes
    }

    #[test]
    fn version_two_snapshot_is_still_loadable() {
        let mut svm = LiteSVM::new();
        svm.set_epoch_stake(solana_address::Address::new_unique(), 456)
            .unwrap();

        let restored = from_bytes(&serialize_v2(&extract_snapshot_v2(&svm))).unwrap();
        assert_eq!(restored.epoch_total_stake(), 0);
    }

    #[test]
    fn duplicate_epoch_stakes_are_rejected() {
        let vote_account = solana_address::Address::new_unique();
        let snapshot = LiteSvmSnapshotV3 {
            state: extract_snapshot_v2(&LiteSVM::new()),
            epoch_vote_stakes: vec![(vote_account, 100), (vote_account, 200)],
        };

        assert!(matches!(
            from_bytes(&serialize_v3(&snapshot)),
            Err(PersistenceError::DuplicateEpochStake(address)) if address == vote_account
        ));
    }

    #[test]
    fn overflowing_unique_epoch_stakes_are_rejected() {
        let snapshot = LiteSvmSnapshotV3 {
            state: extract_snapshot_v2(&LiteSVM::new()),
            epoch_vote_stakes: vec![
                (solana_address::Address::new_unique(), u64::MAX),
                (solana_address::Address::new_unique(), 1),
            ],
        };

        assert!(matches!(
            from_bytes(&serialize_v3(&snapshot)),
            Err(PersistenceError::InvalidEpochStakes(
                litesvm::error::LiteSVMError::EpochStakeOverflow
            ))
        ));
    }
}
