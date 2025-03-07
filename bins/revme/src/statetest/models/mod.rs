use bytes::Bytes;
use primitive_types::{H160, H256, U256};
use std::collections::{BTreeMap, HashMap};
mod deserializer;
mod spec;

use deserializer::*;

use serde_derive::*;

pub use self::spec::SpecName;

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct TestSuit(pub BTreeMap<String, TestUnit>);

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct TestUnit {
    pub env: Env,
    pub pre: HashMap<H160, AccountInfo>,
    pub post: HashMap<SpecName, Vec<Test>>,
    pub transaction: TransactionParts,
}

/// State test indexed state result deserialization.
#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct Test {
    /// Post state hash
    pub hash: H256,
    /// Indexes
    pub indexes: TxPartIndices,
    // logs
    pub logs: H256,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_str_as_bytes")]
    pub txbytes: Option<Bytes>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct TxPartIndices {
    pub data: usize,
    pub gas: usize,
    pub value: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub balance: U256,
    #[serde(deserialize_with = "deserialize_str_as_bytes")]
    pub code: Bytes,
    #[serde(deserialize_with = "deserialize_str_as_u64")]
    pub nonce: u64,
    pub storage: HashMap<U256, U256>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Env {
    pub current_coinbase: H160,
    #[serde(default, deserialize_with = "deserialize_str_as_u256")]
    pub current_difficulty: U256,
    #[serde(deserialize_with = "deserialize_str_as_u256")]
    pub current_gas_limit: U256,
    #[serde(deserialize_with = "deserialize_str_as_u256")]
    pub current_number: U256,
    #[serde(deserialize_with = "deserialize_str_as_u256")]
    pub current_timestamp: U256,
    pub current_base_fee: Option<U256>,
    pub previous_hash: H256,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionParts {
    #[serde(deserialize_with = "deserialize_vec_as_vec_bytes")]
    pub data: Vec<Bytes>,
    pub access_lists: Option<Vec<Option<AccessList>>>,
    pub gas_limit: Vec<U256>,
    pub gas_price: Option<U256>,
    pub nonce: U256,
    pub secret_key: Option<H256>,
    #[serde(deserialize_with = "deserialize_maybe_empty")]
    pub to: Option<H160>,
    pub value: Vec<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccessListItem {
    pub address: H160,
    pub storage_keys: Vec<H256>,
}

pub type AccessList = Vec<AccessListItem>;

#[cfg(test)]
mod tests {

    use super::*;
    use serde_json::Error;

    #[test]
    pub fn serialize_u256() -> Result<(), Error> {
        let json = r#"{"_item":"0x10"}"#;

        #[derive(Deserialize, Debug)]
        pub struct Test {
            _item: Option<U256>,
        }

        let out: Test = serde_json::from_str(json)?;
        println!("out:{:?}", out);
        Ok(())
    }
}
