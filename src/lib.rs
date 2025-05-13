use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use error::SolanaClientExtError;
use solana_client::{rpc_client::RpcClient, rpc_config::RpcSimulateTransactionConfig};
use solana_compute_budget::compute_budget_limits::ComputeBudgetLimits;
use solana_program_runtime::loaded_programs::{BlockRelation, ForkGraph, ProgramCacheEntry};
use solana_sdk::account::ReadableAccount;
use solana_sdk::clock::Slot;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::fee::FeeStructure;
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::rent_collector::RentCollector;
use solana_sdk::transaction;
use solana_sdk::{
    account::AccountSharedData,
    message::Message,
    signers::Signers,
    transaction::{SanitizedTransaction as SolanaSanitizedTransaction, Transaction},
};
use agave_feature_set::FeatureSet;

use solana_bpf_loader_program::syscalls::create_program_runtime_environment_v1;
use solana_compute_budget::compute_budget::ComputeBudget;
use solana_svm::account_loader::CheckedTransactionDetails;
use solana_svm::transaction_processing_callback::TransactionProcessingCallback;
use solana_svm::transaction_processing_result::ProcessedTransaction;
use solana_svm::transaction_processor::{TransactionBatchProcessor, TransactionProcessingConfig, TransactionProcessingEnvironment};
// use solana_svm_callback::InvokeContextCallback;
use solana_system_program::system_processor;
mod error;

/// # RpcClientExt
///
/// `RpcClientExt` is an extension trait for the rust solana client.
/// This crate provides extensions for the Solana Rust client, focusing on compute unit estimation and optimization.
/// 
/// The crate also provides a robust `ReturnStruct` that includes:
/// * Transaction success/failure status
/// * Compute units used
/// * Detailed result message with success information or error details
/// 
/// /// This function is also a mock. In the Agave validator, the bank pre-checks
/// transactions before providing them to the SVM API. We mock this step in
/// PayTube, since we don't need to perform such pre-checks.
pub(crate) fn get_transaction_check_results(
    len: usize,
) -> Vec<transaction::Result<CheckedTransactionDetails>> {
    let _compute_budget_limit = ComputeBudgetLimits::default();
    vec![
        transaction::Result::Ok(CheckedTransactionDetails::new(
            None,
            5000,
        ));
        len
    ]
}

/// Return structure for rollup transaction processing results
/// 
/// This structure provides information about a transaction's execution:
/// - Whether it was successful
/// - The amount of compute units used
/// - A descriptive message with detailed results or error information
pub struct ReturnStruct{
    /// Whether the transaction completed successfully
    pub success: bool,
    /// The number of compute units used by the transaction
    pub cu: u64,
    /// A descriptive result or error message
    pub result: String,  
}

impl ReturnStruct {
    /// Create a success result with compute units used
    pub fn success(cu: u64) -> Self {
        Self {
            success: true,
            cu,
            result: format!("Transaction executed successfully with {} compute units", cu),
        }
    }

    /// Create a failure result with an error message
    pub fn failure(error: impl ToString) -> Self {
        Self {
            success: false,
            cu: 0,
            result: error.to_string(),
        }
    }

    /// Create a result indicating no transaction results were returned
    pub fn no_results() -> Self {
        Self {
            success: false,
            cu: 0,
            result: "No transaction results returned".to_string(),
        }
    }
}

pub struct RollUpChannel<'a> {
    /// I think you know why this is a bad idea...
    keys: Vec<Pubkey>,
    rpc_client: &'a RpcClient,
}

impl<'a> RollUpChannel<'a> {
    pub fn new(keys: Vec<Pubkey>, rpc_client: &'a RpcClient) -> Self {
        Self { keys, rpc_client }
    }

    pub fn process_rollup_transfers(&self, transactions: &[Transaction]) -> Vec<ReturnStruct> {
        
        let sanitized = transactions.iter().map( |tx|
            SolanaSanitizedTransaction::from_transaction_for_tests(tx.clone())
        ).collect::<Vec<SolanaSanitizedTransaction>>();
        // PayTube default configs.
        //
        // These can be configurable for channel customization, including
        // imposing resource or feature restrictions, but more commonly they
        // would likely be hoisted from the cluster.
        //
        // For example purposes, they are provided as defaults here.
        let compute_budget = ComputeBudget::default();
        let feature_set = Arc::new(FeatureSet::all_enabled());
        let fee_structure = FeeStructure::default();
        let _rent_collector = RentCollector::default();

        // PayTube loader/callback implementation.
        //
        // Required to provide the SVM API with a mechanism for loading
        // accounts.
        let account_loader = RollUpAccountLoader::new(&self.rpc_client);

        // Solana SVM transaction batch processor.
        //
        // Creates an instance of `TransactionBatchProcessor`, which can be
        // used by PayTube to process transactions using the SVM.
        //
        // This allows programs such as the System and Token programs to be
        // translated and executed within a provisioned virtual machine, as
        // well as offers many of the same functionality as the lower-level
        // Solana runtime.
        let fork_graph = Arc::new(RwLock::new(ForkRollUpGraph {}));
        let processor = create_transaction_batch_processor(
            &account_loader,
            &feature_set,
            &compute_budget,
            Arc::clone(&fork_graph),
        );
        println!("transaction batch processor created ");

        // The PayTube transaction processing runtime environment.
        //
        // Again, these can be configurable or hoisted from the cluster.
        let processing_environment = TransactionProcessingEnvironment {
            blockhash: Hash::default(),
            blockhash_lamports_per_signature: fee_structure.lamports_per_signature,
            epoch_total_stake: 0,
            feature_set,
            fee_lamports_per_signature: 5000,
            rent_collector: None,
        };

        // The PayTube transaction processing config for Solana SVM.
        //
        // Extended configurations for even more customization of the SVM API.
        let processing_config = TransactionProcessingConfig::default();

        println!("transaction processing_config created ");

        // Step 1: Convert the batch of PayTube transactions into
        // SVM-compatible transactions for processing.
        //
        // In the future, the SVM API may allow for trait-based transactions.
        // In this case, `PayTubeTransaction` could simply implement the
        // interface, and avoid this conversion entirely.


        // Step 2: Process the SVM-compatible transactions with the SVM API.
        let results = processor.load_and_execute_sanitized_transactions(
            &account_loader,
            &sanitized,
            get_transaction_check_results(transactions.len()),
            &processing_environment,
            &processing_config,
        );
        println!("Executed");

        // Process all transaction results
        let mut return_results = Vec::new();
        
        for (i, transaction_result) in results.processing_results.iter().enumerate() {
            let tx_result = match transaction_result {
                Ok(processed_tx) => {
                    match processed_tx {
                        ProcessedTransaction::Executed(executed_tx) => {
                            let cu = executed_tx.execution_details.executed_units;
                            let logs = executed_tx.execution_details.log_messages.clone();
                            let status = executed_tx.execution_details.status.clone();
                            let is_success = status.is_ok();
                            
                            if is_success {
                                ReturnStruct::success(cu)
                            } else {
                                match status {
                                    Err(err) => {
                                        let error_msg = format!("Transaction {} failed with error: {}", i, err);
                                        let log_msg = logs.map(|logs| logs.join("\n")).unwrap_or_default();
                                        ReturnStruct {
                                            success: false,
                                            cu,
                                            result: format!("{}\nLogs:\n{}", error_msg, log_msg),
                                        }
                                    },
                                    _ => ReturnStruct::success(cu), // This shouldn't happen as we checked is_success
                                }
                            }
                        },
                        ProcessedTransaction::FeesOnly(fees_only) => {
                            ReturnStruct::failure(format!(
                                "Transaction {} failed with error: {}. Only fees were charged.", 
                                i, 
                                fees_only.load_error
                            ))
                        },
                    }
                },
                Err(err) => {
                    ReturnStruct::failure(format!("Transaction {} failed: {}", i, err))
                }
            };
            return_results.push(tx_result);
        }
        
        // If there were no results but transactions were submitted
        if return_results.is_empty() && !transactions.is_empty() {
            return_results.push(ReturnStruct::no_results());
        }
        
        return_results

        // Step 3: Convert the SVM API processor results into a final ledger
        // using `PayTubeSettler`, and settle the resulting balance differences
        // to the Solana base chain.
        //
        // Here the settler is basically iterating over the transaction results
        // to track debits and credits, but only for those transactions which
        // were executed succesfully.
        //
        // The final ledger of debits and credits to each participant can then
        // be packaged into a minimal number of settlement transactions for
        // submission.
    }
}

pub(crate) struct ForkRollUpGraph {}

impl ForkGraph for ForkRollUpGraph {
    fn relationship(&self, _a: Slot, _b: Slot) -> BlockRelation {
        BlockRelation::Unknown
    }
}
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
/// This function encapsulates some initial setup required to tweak the
/// `TransactionBatchProcessor` for use within PayTube.
///
/// We're simply configuring the mocked fork graph on the SVM API's program
/// cache, then adding the System program to the processor's builtins.
pub(crate) fn create_transaction_batch_processor<CB: TransactionProcessingCallback>(
    callbacks: &CB,
    feature_set: &FeatureSet,
    compute_budget: &ComputeBudget,
    fork_graph: Arc<RwLock<ForkRollUpGraph>>,
) -> TransactionBatchProcessor<ForkRollUpGraph> {
    // Create a new transaction batch processor.
    //
    // We're going to use slot 1 specifically because any programs we add will
    // be deployed in slot 0, and they are delayed visibility until the next
    // slot (1).
    // This includes programs owned by BPF Loader v2, which are automatically
    // marked as "depoyed" in slot 0.
    // See `solana_svm::program_loader::load_program_with_pubkey` for more
    // details.
    let processor = TransactionBatchProcessor::<ForkRollUpGraph>::new(
        /* slot */ 1,
        /* epoch */ 1,
        Arc::downgrade(&fork_graph),
        Some(Arc::new(
            create_program_runtime_environment_v1(feature_set, compute_budget, false, false)
                .unwrap(),
        )),
        None,
    );

    // Add the system program builtin.
    processor.add_builtin(
        callbacks,
        solana_system_program::id(),
        "system_program",
        ProgramCacheEntry::new_builtin(
            0,
            b"system_program".len(),
            system_processor::Entrypoint::vm,
        ),
    );

    // Add the BPF Loader v2 builtin, for the SPL Token program.
    processor.add_builtin(
        callbacks,
        solana_sdk::bpf_loader::id(),
        "solana_bpf_loader_program",
        ProgramCacheEntry::new_builtin(
            0,
            b"solana_bpf_loader_program".len(),
            solana_bpf_loader_program::Entrypoint::vm,
        ),
    );

    processor
}
pub trait RpcClientExt {
    /// Estimates compute units for an unsigned transaction
    /// 
    /// Returns a vector of compute unit values for each transaction processed.
    /// If any transaction fails, returns an error with detailed failure information.
    fn estimate_compute_units_unsigned_tx<'a, I: Signers + ?Sized>(
        &self,
        transaction: &Transaction,
        _signers: &'a I,
    ) -> Result<Vec<u64>, Box<dyn std::error::Error + 'static>>;

    /// Estimates compute units for a message
    /// 
    /// Simulates the transaction on the network to determine compute unit usage.
    fn estimate_compute_units_msg<'a, I: Signers + ?Sized>(
        &self,
        msg: &Message,
        signers: &'a I,
    ) -> Result<u64, Box<dyn std::error::Error + 'static>>;

    /// Optimizes compute units for an unsigned transaction
    /// 
    /// Adds a compute budget instruction to the transaction to limit compute units
    /// to the optimal amount needed based on simulation.
    fn optimize_compute_units_unsigned_tx<'a, I: Signers + ?Sized>(
        &self,
        unsigned_transaction: &mut Transaction,
        signers: &'a I,
    ) -> Result<u32, Box<dyn std::error::Error + 'static>>;

    /// Optimizes compute units for a message
    /// 
    /// Adds a compute budget instruction to the message to limit compute units
    /// to the optimal amount needed based on simulation.
    fn optimize_compute_units_msg<'a, I: Signers + ?Sized>(
        &self,
        message: &mut Message,
        signers: &'a I,
    ) -> Result<u32, Box<dyn std::error::Error + 'static>>;
}

impl RpcClientExt for solana_client::rpc_client::RpcClient {
    fn estimate_compute_units_unsigned_tx<'a, I: Signers + ?Sized>(
        &self,
        transaction: &Transaction,
        _signers: &'a I,
    ) -> Result<Vec<u64>, Box<dyn std::error::Error + 'static>> {
        // GET SVM MESSAGE

        let accounts = transaction.message.account_keys.clone();
        let rollup_c = RollUpChannel::new(accounts, self);
        let results = rollup_c.process_rollup_transfers(&[transaction.clone()]);
        
        // Check if all transactions were successful
        let failures: Vec<&ReturnStruct> = results.iter()
            .filter(|r| !r.success)
            .collect();
            
        if !failures.is_empty() {
            let error_messages = failures.iter()
                .map(|r| r.result.clone())
                .collect::<Vec<String>>()
                .join("\n");
                
            return Err(Box::new(SolanaClientExtError::ComputeUnitsError(
                format!("Transaction simulation failed:\n{}", error_messages),
            )));
        }
        
        // Extract compute units from successful transactions
        Ok(results.iter().map(|r| r.cu).collect())
    }

    fn estimate_compute_units_msg<'a, I: Signers + ?Sized>(
        &self,
        message: &Message,
        signers: &'a I,
    ) -> Result<u64, Box<dyn std::error::Error + 'static>> {
        let config = RpcSimulateTransactionConfig {
            sig_verify: true,
            ..RpcSimulateTransactionConfig::default()
        };
        let mut tx = Transaction::new_unsigned(message.clone());
        tx.sign(signers, self.get_latest_blockhash()?);
        let result = self.simulate_transaction_with_config(&tx, config)?;

        let consumed_cu = result.value.units_consumed.ok_or(Box::new(
            SolanaClientExtError::ComputeUnitsError(
                "Missing Compute Units from transaction simulation.".into(),
            ),
        ))?;

        if consumed_cu == 0 {
            return Err(Box::new(SolanaClientExtError::RpcError(
                "Transaction simulation failed.".into(),
            )));
        }

        Ok(consumed_cu)
    }

    fn optimize_compute_units_unsigned_tx<'a, I: Signers + ?Sized>(
        &self,
        transaction: &mut Transaction,
        signers: &'a I,
    ) -> Result<u32, Box<dyn std::error::Error + 'static>> {
        let optimal_cu_vec = self.estimate_compute_units_unsigned_tx(transaction, signers)?;
        let optimal_cu = *optimal_cu_vec.get(0).unwrap() as u32;

        let optimize_ix = ComputeBudgetInstruction::set_compute_unit_limit(
            optimal_cu.saturating_add(optimal_cu),
        );
        transaction
            .message
            .account_keys
            .push(solana_sdk::compute_budget::id());
        let compiled_ix = transaction.message.compile_instruction(&optimize_ix);

        transaction.message.instructions.insert(0, compiled_ix);

        Ok(optimal_cu)
    }

    /// Simulates the transaction to get compute units used for the transaction
    /// and adds an instruction to the message to request
    /// only the required compute units from the ComputeBudget program
    /// to complete the transaction with this Message.
    ///
    /// ```no_run
    /// use solana_client::rpc_client::RpcClient;
    /// use solana_client_ext::RpcClientExt;
    /// use solana_sdk::{
    ///     message::Message, signature::Keypair, signer::Signer, system_instruction,
    ///     transaction::Transaction,
    /// };
    /// fn main() {
    ///     let rpc_client = RpcClient::new("https://api.devnet.solana.com");
    ///     let keypair = Keypair::new();
    ///     let keypair2 = Keypair::new();
    ///     let created_ix = system_instruction::transfer(&keypair.pubkey(), &keypair2.pubkey(), 10000);
    ///     let mut msg = Message::new(&[created_ix], Some(&keypair.pubkey()));
    ///
    ///     let optimized_cu = rpc_client
    ///         .optimize_compute_units_msg(&mut msg, &[&keypair])
    ///         .unwrap();
    ///     println!("Optimized compute units: {}", optimized_cu);
    ///
    ///     let tx = Transaction::new(&[&keypair], msg, rpc_client.get_latest_blockhash().unwrap());
    ///     let result = rpc_client
    ///         .send_and_confirm_transaction_with_spinner(&tx)
    ///         .unwrap();
    ///
    ///     println!(
    ///         "Transaction signature: https://explorer.solana.com/tx/{}?cluster=devnet",
    ///         result
    ///     );
    /// }
    /// ```
    fn optimize_compute_units_msg<'a, I: Signers + ?Sized>(
        &self,
        message: &mut Message,
        signers: &'a I,
    ) -> Result<u32, Box<dyn std::error::Error + 'static>> {
        let optimal_cu = u32::try_from(self.estimate_compute_units_msg(message, signers)?)?;
        let optimize_ix = ComputeBudgetInstruction::set_compute_unit_limit(
            optimal_cu.saturating_add(150 /*optimal_cu.saturating_div(100)*100*/),
        );
        message.account_keys.push(solana_sdk::compute_budget::id());
        let compiled_ix = message.compile_instruction(&optimize_ix);
        message.instructions.insert(0, compiled_ix);

        Ok(optimal_cu)
    }
}

#[cfg(test)]
mod tests {
    use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer, system_instruction};
    use super::*;

    #[test]
    fn cu() {
        let rpc_client = solana_client::rpc_client::RpcClient::new("https://api.devnet.solana.com");
        let new_keypair = Keypair::from_bytes(&[252,148,183,236,100,64,108,105,26,181,229,97,54,43,113,1,253,4,109,80,183,26,222,43,209,246,12,80,15,246,53,149,189,22,176,152,33,128,187,215,121,56,191,187,241,223,7,109,96,88,243,76,92,122,185,245,185,255,80,125,80,157,229,222]).unwrap();

        let transfer_ix =
            system_instruction::transfer(&new_keypair.pubkey(), &Pubkey::new_unique(), 10000);
        let msg = Message::new(&[transfer_ix], Some(&new_keypair.pubkey()));
        let blockhash = rpc_client.get_latest_blockhash().unwrap();
        let mut tx = Transaction::new(&[&new_keypair], msg, blockhash);
        
        // Test direct ReturnStruct results from process_rollup_transfers
        let accounts = tx.message.account_keys.clone();
        let rollup_c = RollUpChannel::new(accounts, &rpc_client);
        let results = rollup_c.process_rollup_transfers(&[tx.clone()]);
        
        println!("Direct rollup results:");
        for (i, result) in results.iter().enumerate() {
            println!("Transaction {}: Success={}, CU={}, Result: {}", 
                i, result.success, result.cu, result.result);
        }
        
        // Test through optimize_compute_units_unsigned_tx
        let optimized_cu = rpc_client
            .optimize_compute_units_unsigned_tx(&mut tx, &[&new_keypair])
            .unwrap();

        println!("Optimized CU: {}", optimized_cu);
        
        // Sign and send the transaction
        tx.sign(&[new_keypair], blockhash);

        let result = rpc_client
            .send_and_confirm_transaction_with_spinner(&tx)
            .unwrap();
        println!(
            "Transaction signature: {} (https://explorer.solana.com/tx/{}?cluster=devnet)",
            result, result
        );
        
        // Get transaction details
        println!("Transaction details: {:?}", tx);
    }
    
    #[test]
    fn test_return_struct() {
        // Test ReturnStruct helper methods
        let success_result = ReturnStruct::success(5000);
        assert_eq!(success_result.success, true);
        assert_eq!(success_result.cu, 5000);
        
        let failure_result = ReturnStruct::failure("Test error message");
        assert_eq!(failure_result.success, false);
        assert_eq!(failure_result.cu, 0);
        assert_eq!(failure_result.result, "Test error message");
        
        let no_results = ReturnStruct::no_results();
        assert_eq!(no_results.success, false);
        assert_eq!(no_results.result, "No transaction results returned");
    }
    
    #[test]
    fn test_failed_transaction() {
        let rpc_client = solana_client::rpc_client::RpcClient::new("https://api.devnet.solana.com");
        
        // Create a new keypair with no funds
        let empty_keypair = Keypair::new();
        
        // Try to transfer more SOL than the account would have (1 SOL)
        let transfer_ix = system_instruction::transfer(
            &empty_keypair.pubkey(),
            &Pubkey::new_unique(),
            1_000_000_000 // 1 SOL in lamports
        );
        
        let msg = Message::new(&[transfer_ix], Some(&empty_keypair.pubkey()));
        let blockhash = rpc_client.get_latest_blockhash().unwrap();
        let tx = Transaction::new(&[&empty_keypair], msg, blockhash);
        
        // Process the transaction - should fail due to insufficient funds
        let accounts = tx.message.account_keys.clone();
        let rollup_c = RollUpChannel::new(accounts, &rpc_client);
        let results = rollup_c.process_rollup_transfers(&[tx.clone()]);
        
        println!("Failed transaction test results:");
        for (i, result) in results.iter().enumerate() {
            println!("Transaction {}: Success={}, CU={}, Result: {}", 
                i, result.success, result.cu, result.result);
            
            // Verify that the transaction failed
            assert!(!result.success, "Transaction should have failed");
            
            // The error message should contain information about the failure
            assert!(result.result.contains("failed"), "Error message should indicate failure");
        }
        
        // Test optimize_compute_units_unsigned_tx with a failing transaction
        let mut failing_tx = tx.clone();
        let result = rpc_client.optimize_compute_units_unsigned_tx(&mut failing_tx, &[&empty_keypair]);
        
        // Should return an error
        assert!(result.is_err(), "optimize_compute_units_unsigned_tx should return an error for a failing transaction");
        
        if let Err(e) = result {
            println!("Expected error from optimize_compute_units_unsigned_tx: {}", e);
            assert!(e.to_string().contains("failed"), "Error message should indicate failure");
        }
    }
}
