use {
    super::error::CoreBpfMigrationError,
    crate::bank::Bank,
    solana_account::{AccountSharedData, ReadableAccount},
    solana_loader_v4_interface::state::{LoaderV4State, LoaderV4Status},
    solana_pubkey::Pubkey,
    solana_sdk_ids::loader_v4,
};

/// The account details of a buffer account slated to replace a program.
#[derive(Debug)]
pub(crate) struct SourceBufferV4 {
    pub buffer_address: Pubkey,
    pub buffer_account: AccountSharedData,
}

impl SourceBufferV4 {
    /// Collects the details of a buffer account and verifies it exists, is
    /// owned by the Loader v4, and has the correct state.
    pub(crate) fn new_checked(
        bank: &Bank,
        buffer_address: &Pubkey,
    ) -> Result<Self, CoreBpfMigrationError> {
        // The buffer account should exist.
        let buffer_account = bank
            .get_account_with_fixed_root(buffer_address)
            .ok_or(CoreBpfMigrationError::AccountNotFound(*buffer_address))?;

        // The buffer account should be owned by Loader v4.
        if buffer_account.owner() != &loader_v4::id() {
            return Err(CoreBpfMigrationError::IncorrectOwner(*buffer_address));
        }

        // The buffer account should have the correct state.
        let buffer_metadata_size = LoaderV4State::program_data_offset();
        if buffer_account.data().len() >= buffer_metadata_size {
            let state = unsafe {
                std::mem::transmute::<&[u8; LoaderV4State::program_data_offset()], &LoaderV4State>(
                    buffer_account.data()[..buffer_metadata_size]
                        .try_into()
                        .unwrap(),
                )
            };

            if matches!(state.status, LoaderV4Status::Retracted) {
                return Ok(Self {
                    buffer_address: *buffer_address,
                    buffer_account,
                });
            }
        }
        Err(CoreBpfMigrationError::InvalidBufferAccount(*buffer_address))
    }

    /*
    /// [`SourceBufferV4::new_checked`] but also verifies the build hash
    /// https://github.com/Ellipsis-Labs/solana-verifiable-build
    pub(crate) fn new_checked_with_verified_build_hash(
        bank: &Bank,
        buffer_address: &Pubkey,
        expected_hash: Hash,
    ) -> Result<Self, CoreBpfMigrationError> {
        let buffer = Self::new_checked(bank, buffer_address)?;
        let data = buffer.buffer_account.data();

        let offset = LoaderV4State::program_data_offset();
        let end_offset = data.iter().rposition(|&x| x != 0).map_or(offset, |i| i + 1);
        let buffer_program_data = &data[offset..end_offset];
        let hash = solana_sha256_hasher::hash(buffer_program_data);

        if hash != expected_hash {
            return Err(CoreBpfMigrationError::BuildHashMismatch(
                hash,
                expected_hash,
            ));
        }

        Ok(buffer)
    }
    */
}

#[cfg(test)]
mod tests {
    use {
        super::*, crate::bank::tests::create_simple_test_bank, assert_matches::assert_matches,
        solana_account::WritableAccount,
    };

    fn store_account(bank: &Bank, address: &Pubkey, data: &[u8], owner: &Pubkey) {
        let space = data.len();
        let lamports = bank.get_minimum_balance_for_rent_exemption(space);
        let mut account = AccountSharedData::new(lamports, space, owner);
        account.data_as_mut_slice().copy_from_slice(data);
        bank.store_account_and_update_capitalization(address, &account);
    }

    #[test]
    fn test_source_buffer_v4() {
        let bank = create_simple_test_bank(0);

        let buffer_address = Pubkey::new_unique();

        // Fail if the buffer account does not exist
        assert_matches!(
            SourceBufferV4::new_checked(&bank, &buffer_address).unwrap_err(),
            CoreBpfMigrationError::AccountNotFound(..)
        );

        // Fail if the buffer account is not owned by the upgradeable loader.
        store_account(
            &bank,
            &buffer_address,
            &[4u8; 200],
            &Pubkey::new_unique(), // Not the upgradeable loader
        );
        assert_matches!(
            SourceBufferV4::new_checked(&bank, &buffer_address).unwrap_err(),
            CoreBpfMigrationError::IncorrectOwner(..)
        );

        // Fail if the buffer account does not have the correct state.
        store_account(
            &bank,
            &buffer_address,
            &[4u8; 200], // Not the correct state
            &loader_v4::id(),
        );
        assert_matches!(
            SourceBufferV4::new_checked(&bank, &buffer_address).unwrap_err(),
            CoreBpfMigrationError::InvalidBufferAccount(..)
        );

        let mut metadata = [0u8; LoaderV4State::program_data_offset()];
        let state = unsafe {
            std::mem::transmute::<&mut [u8; LoaderV4State::program_data_offset()], &mut LoaderV4State>(
                &mut metadata,
            )
        };
        *state = LoaderV4State {
            slot: 0,
            authority_address_or_next_version: buffer_address,
            status: LoaderV4Status::Deployed,
        };

        // Fail if the buffer account does not have the correct state.
        // This time, valid `LoaderV4Status` but not a 'Retracted' value.
        store_account(&bank, &buffer_address, &metadata, &loader_v4::id());
        assert_matches!(
            SourceBufferV4::new_checked(&bank, &buffer_address).unwrap_err(),
            CoreBpfMigrationError::InvalidBufferAccount(..)
        );

        // Success
        let mut metadata = [0u8; LoaderV4State::program_data_offset()];
        let state = unsafe {
            std::mem::transmute::<&mut [u8; LoaderV4State::program_data_offset()], &mut LoaderV4State>(
                &mut metadata,
            )
        };
        *state = LoaderV4State {
            slot: 0,
            authority_address_or_next_version: buffer_address,
            status: LoaderV4Status::Retracted,
        };

        let elf = vec![4u8; 200];
        // Loader v4 always writes ELF bytes after `LoaderV4State::program_data_offset()`.
        let data_len = LoaderV4State::program_data_offset() + elf.len();
        let mut data = Vec::with_capacity(data_len);
        data.extend_from_slice(&metadata);
        data.extend_from_slice(&elf);

        store_account(&bank, &buffer_address, &data, &loader_v4::id());

        let source_buffer = SourceBufferV4::new_checked(&bank, &buffer_address).unwrap();

        assert_eq!(source_buffer.buffer_address, buffer_address);

        let state = unsafe {
            std::mem::transmute::<&[u8; LoaderV4State::program_data_offset()], &LoaderV4State>(
                source_buffer.buffer_account.data()[..LoaderV4State::program_data_offset()]
                    .try_into()
                    .unwrap(),
            )
        };
        assert_eq!(
            state,
            &LoaderV4State {
                slot: 0,
                authority_address_or_next_version: buffer_address,
                status: LoaderV4Status::Retracted,
            }
        );
        assert_eq!(
            &source_buffer.buffer_account.data()[LoaderV4State::program_data_offset()..],
            elf.as_slice()
        );
    }
}
