use super::{DatabaseCommit, DatabaseRef};
use crate::{interpreter::bytecode::Bytecode, Database, KECCAK_EMPTY};
use crate::{Account, AccountInfo, Log};
use alloc::vec::Vec;
use core::convert::Infallible;
use hashbrown::{hash_map::Entry, HashMap as Map};
use primitive_types::{H160, H256, U256};
use sha3::{Digest, Keccak256};

pub type InMemoryDB = CacheDB<EmptyDB>;

impl InMemoryDB {
    pub fn default() -> Self {
        CacheDB::new(EmptyDB {})
    }
}

/// Memory backend, storing all state values in a `Map` in memory.
#[derive(Debug, Clone)]
pub struct CacheDB<ExtDB: DatabaseRef> {
    /// Account info where None means it is not existing. Not existing state is needed for Pre TANGERINE forks.
    /// `code` is always `None`, and bytecode can be found in `contracts`.
    pub accounts: Map<H160, DbAccount>,
    pub contracts: Map<H256, Bytecode>,
    pub logs: Vec<Log>,
    pub block_hashes: Map<U256, H256>,
    pub db: ExtDB,
}

#[derive(Debug, Clone, Default)]
pub struct DbAccount {
    pub info: AccountInfo,
    /// If account is selfdestructed or newly created, storage will be cleared.
    pub account_state: AccountState,
    /// storage slots
    pub storage: Map<U256, U256>,
}

impl DbAccount {
    pub fn new_not_existing() -> Self {
        Self {
            account_state: AccountState::NotExisting,
            ..Default::default()
        }
    }
    pub fn info(&self) -> Option<AccountInfo> {
        if matches!(self.account_state, AccountState::NotExisting) {
            None
        } else {
            Some(self.info.clone())
        }
    }
}

impl From<Option<AccountInfo>> for DbAccount {
    fn from(from: Option<AccountInfo>) -> Self {
        if let Some(info) = from {
            Self {
                info,
                account_state: AccountState::None,
                ..Default::default()
            }
        } else {
            Self::new_not_existing()
        }
    }
}

impl From<AccountInfo> for DbAccount {
    fn from(info: AccountInfo) -> Self {
        Self {
            info,
            account_state: AccountState::None,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub enum AccountState {
    /// Before Spurious Dragon hardfork there were a difference between empty and not existing.
    /// And we are flaging it here.
    NotExisting,
    /// EVM touched this account. For newer hardfork this means it can be clearead/removed from state.
    Touched,
    /// EVM cleared storage of this account, mostly by selfdestruct, we dont ask database for storage slots
    /// and asume they are U256::zero()
    StorageCleared,
    /// EVM didnt interacted with this account
    #[default]
    None,
}

impl<ExtDB: DatabaseRef> CacheDB<ExtDB> {
    pub fn new(db: ExtDB) -> Self {
        let mut contracts = Map::new();
        contracts.insert(KECCAK_EMPTY, Bytecode::new());
        contracts.insert(H256::zero(), Bytecode::new());
        Self {
            accounts: Map::new(),
            contracts,
            logs: Vec::default(),
            block_hashes: Map::new(),
            db,
        }
    }

    pub fn insert_contract(&mut self, account: &mut AccountInfo) {
        if let Some(code) = &account.code {
            if !code.is_empty() {
                account.code_hash = code.hash();
                self.contracts
                    .entry(account.code_hash)
                    .or_insert_with(|| code.clone());
            }
        }
        if account.code_hash.is_zero() {
            account.code_hash = KECCAK_EMPTY;
        }
    }

    /// Insert account info but not override storage
    pub fn insert_account_info(&mut self, address: H160, mut info: AccountInfo) {
        self.insert_contract(&mut info);
        self.accounts.entry(address).or_default().info = info;
    }

    fn load_account(&mut self, address: H160) -> Result<&mut DbAccount, ExtDB::Error> {
        let db = &self.db;
        match self.accounts.entry(address) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => Ok(entry.insert(
                db.basic(address)?
                    .map(|info| DbAccount {
                        info,
                        ..Default::default()
                    })
                    .unwrap_or_else(DbAccount::new_not_existing),
            )),
        }
    }

    /// insert account storage without overriding account info
    pub fn insert_account_storage(
        &mut self,
        address: H160,
        slot: U256,
        value: U256,
    ) -> Result<(), ExtDB::Error> {
        let account = self.load_account(address)?;
        account.storage.insert(slot, value);
        Ok(())
    }

    /// replace account storage without overriding account info
    pub fn replace_account_storage(
        &mut self,
        address: H160,
        storage: Map<U256, U256>,
    ) -> Result<(), ExtDB::Error> {
        let account = self.load_account(address)?;
        account.account_state = AccountState::StorageCleared;
        account.storage = storage.into_iter().collect();
        Ok(())
    }
}

impl<ExtDB: DatabaseRef> DatabaseCommit for CacheDB<ExtDB> {
    fn commit(&mut self, changes: Map<H160, Account>) {
        for (address, mut account) in changes {
            if account.is_destroyed {
                let db_account = self.accounts.entry(address).or_default();
                db_account.storage.clear();
                db_account.account_state = AccountState::NotExisting;
                db_account.info = AccountInfo::default();
                continue;
            }
            self.insert_contract(&mut account.info);

            let db_account = self.accounts.entry(address).or_default();
            db_account.info = account.info;

            db_account.account_state = if account.storage_cleared {
                db_account.storage.clear();
                AccountState::StorageCleared
            } else {
                AccountState::Touched
            };
            db_account.storage.extend(
                account
                    .storage
                    .into_iter()
                    .map(|(key, value)| (key, value.present_value())),
            );
        }
    }
}

impl<ExtDB: DatabaseRef> Database for CacheDB<ExtDB> {
    type Error = ExtDB::Error;

    fn block_hash(&mut self, number: U256) -> Result<H256, Self::Error> {
        match self.block_hashes.entry(number) {
            Entry::Occupied(entry) => Ok(*entry.get()),
            Entry::Vacant(entry) => {
                let hash = self.db.block_hash(number)?;
                entry.insert(hash);
                Ok(hash)
            }
        }
    }

    fn basic(&mut self, address: H160) -> Result<Option<AccountInfo>, Self::Error> {
        let basic = match self.accounts.entry(address) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(
                self.db
                    .basic(address)?
                    .map(|info| DbAccount {
                        info,
                        ..Default::default()
                    })
                    .unwrap_or_else(DbAccount::new_not_existing),
            ),
        };
        Ok(basic.info())
    }

    /// Get the value in an account's storage slot.
    ///
    /// It is assumed that account is already loaded.
    fn storage(&mut self, address: H160, index: U256) -> Result<U256, Self::Error> {
        match self.accounts.entry(address) {
            Entry::Occupied(mut acc_entry) => {
                let acc_entry = acc_entry.get_mut();
                match acc_entry.storage.entry(index) {
                    Entry::Occupied(entry) => Ok(*entry.get()),
                    Entry::Vacant(entry) => {
                        if matches!(
                            acc_entry.account_state,
                            AccountState::StorageCleared | AccountState::NotExisting
                        ) {
                            Ok(U256::zero())
                        } else {
                            let slot = self.db.storage(address, index)?;
                            entry.insert(slot);
                            Ok(slot)
                        }
                    }
                }
            }
            Entry::Vacant(acc_entry) => {
                // acc needs to be loaded for us to access slots.
                let info = self.db.basic(address)?;
                let (account, value) = if info.is_some() {
                    let value = self.db.storage(address, index)?;
                    let mut account: DbAccount = info.into();
                    account.storage.insert(index, value);
                    (account, value)
                } else {
                    (info.into(), U256::zero())
                };
                acc_entry.insert(account);
                Ok(value)
            }
        }
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Result<Bytecode, Self::Error> {
        match self.contracts.entry(code_hash) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => {
                // if you return code bytes when basic fn is called this function is not needed.
                Ok(entry.insert(self.db.code_by_hash(code_hash)?).clone())
            }
        }
    }
}

impl<ExtDB: DatabaseRef> DatabaseRef for CacheDB<ExtDB> {
    type Error = ExtDB::Error;

    fn basic(&self, address: H160) -> Result<Option<AccountInfo>, Self::Error> {
        match self.accounts.get(&address) {
            Some(acc) => Ok(acc.info()),
            None => self.db.basic(address),
        }
    }

    fn storage(&self, address: H160, index: U256) -> Result<U256, Self::Error> {
        match self.accounts.get(&address) {
            Some(acc_entry) => match acc_entry.storage.get(&index) {
                Some(entry) => Ok(*entry),
                None => {
                    if matches!(
                        acc_entry.account_state,
                        AccountState::StorageCleared | AccountState::NotExisting
                    ) {
                        Ok(U256::zero())
                    } else {
                        self.db.storage(address, index)
                    }
                }
            },
            None => self.db.storage(address, index),
        }
    }

    fn code_by_hash(&self, code_hash: H256) -> Result<Bytecode, Self::Error> {
        match self.contracts.get(&code_hash) {
            Some(entry) => Ok(entry.clone()),
            None => self.db.code_by_hash(code_hash),
        }
    }

    fn block_hash(&self, number: U256) -> Result<H256, Self::Error> {
        match self.block_hashes.get(&number) {
            Some(entry) => Ok(*entry),
            None => self.db.block_hash(number),
        }
    }
}

/// An empty database that always returns default values when queried.
#[derive(Debug, Default, Clone)]
pub struct EmptyDB();

impl DatabaseRef for EmptyDB {
    type Error = Infallible;
    /// Get basic account information.
    fn basic(&self, _address: H160) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(None)
    }
    /// Get account code by its hash
    fn code_by_hash(&self, _code_hash: H256) -> Result<Bytecode, Self::Error> {
        Ok(Bytecode::new())
    }
    /// Get storage value of address at index.
    fn storage(&self, _address: H160, _index: U256) -> Result<U256, Self::Error> {
        Ok(U256::default())
    }

    // History related
    fn block_hash(&self, number: U256) -> Result<H256, Self::Error> {
        let mut buffer: [u8; 4 * 8] = [0; 4 * 8];
        number.to_big_endian(&mut buffer);
        Ok(H256::from_slice(&Keccak256::digest(buffer)))
    }
}

/// Custom benchmarking DB that only has account info for the zero address.
///
/// Any other address will return an empty account.
#[derive(Debug, Default, Clone)]
pub struct BenchmarkDB(pub Bytecode, H256);

impl BenchmarkDB {
    pub fn new_bytecode(bytecode: Bytecode) -> Self {
        let hash = bytecode.hash();
        Self(bytecode, hash)
    }
}

impl Database for BenchmarkDB {
    type Error = Infallible;
    /// Get basic account information.
    fn basic(&mut self, address: H160) -> Result<Option<AccountInfo>, Self::Error> {
        if address == H160::zero() {
            return Ok(Some(AccountInfo {
                nonce: 1,
                balance: U256::from(10000000),
                code: Some(self.0.clone()),
                code_hash: self.1,
            }));
        }
        Ok(None)
    }

    /// Get account code by its hash
    fn code_by_hash(&mut self, _code_hash: H256) -> Result<Bytecode, Self::Error> {
        Ok(Bytecode::default())
    }

    /// Get storage value of address at index.
    fn storage(&mut self, _address: H160, _index: U256) -> Result<U256, Self::Error> {
        Ok(U256::default())
    }

    // History related
    fn block_hash(&mut self, _number: U256) -> Result<H256, Self::Error> {
        Ok(H256::default())
    }
}

#[cfg(test)]
mod tests {
    use primitive_types::H160;

    use crate::{AccountInfo, Database};

    use super::{CacheDB, EmptyDB};

    #[test]
    pub fn test_insert_account_storage() {
        let account = H160::from_low_u64_be(42);
        let nonce = 42;
        let mut init_state = CacheDB::new(EmptyDB::default());
        init_state.insert_account_info(
            account,
            AccountInfo {
                nonce,
                ..Default::default()
            },
        );

        let (key, value) = (123u64.into(), 456u64.into());
        let mut new_state = CacheDB::new(init_state);
        let _ = new_state.insert_account_storage(account, key, value);

        assert_eq!(new_state.basic(account).unwrap().unwrap().nonce, nonce);
        assert_eq!(new_state.storage(account, key), Ok(value));
    }

    #[test]
    pub fn test_replace_account_storage() {
        let account = H160::from_low_u64_be(42);
        let nonce = 42;
        let mut init_state = CacheDB::new(EmptyDB::default());
        init_state.insert_account_info(
            account,
            AccountInfo {
                nonce,
                ..Default::default()
            },
        );

        let (key0, value0) = (123u64.into(), 456u64.into());
        let (key1, value1) = (789u64.into(), 999u64.into());
        let _ = init_state.insert_account_storage(account, key0, value0);

        let mut new_state = CacheDB::new(init_state);
        let _ = new_state.replace_account_storage(account, [(key1, value1)].into());

        assert_eq!(new_state.basic(account).unwrap().unwrap().nonce, nonce);
        assert_eq!(new_state.storage(account, key0), Ok(0.into()));
        assert_eq!(new_state.storage(account, key1), Ok(value1));
    }
}
