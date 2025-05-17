use solana_client::rpc_client::RpcClient;
use solana_sdk::account::ReadableAccount;
use solana_sdk::{account::AccountSharedData, pubkey::Pubkey};
use solana_svm::transaction_processing_callback::TransactionProcessingCallback;
use std::collections::HashMap;
use std::sync::RwLock;

pub struct RollUpAccountLoader<'a> {
    cache: RwLock<HashMap<Pubkey, AccountSharedData>>,
    rpc_client: &'a RpcClient,
}

impl<'a> RollUpAccountLoader<'a> {
    pub fn new(rpc_client: &'a RpcClient) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            rpc_client,
        }
    }
}

// impl InvokeContextCallback for RollUpAccountLoader<'_> {}
impl TransactionProcessingCallback for RollUpAccountLoader<'_> {
    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData> {
        if let Some(account) = self.cache.read().unwrap().get(pubkey) {
            return Some(account.clone());
        }

        let account: AccountSharedData = self.rpc_client.get_account(pubkey).ok()?.into();
        self.cache.write().unwrap().insert(*pubkey, account.clone());

        Some(account)
    }

    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize> {
        self.get_account_shared_data(account)
            .and_then(|account| owners.iter().position(|key| account.owner().eq(key)))
    }
}
