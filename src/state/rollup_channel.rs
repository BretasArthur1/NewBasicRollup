use std::sync::{Arc, RwLock};

use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::{Transaction, SanitizedTransaction as SolanaSanitizedTransaction};
use solana_sdk::fee::FeeStructure;
use solana_sdk::rent_collector::RentCollector;
use solana_compute_budget::compute_budget::ComputeBudget;
use solana_client::rpc_client::RpcClient;

use agave_feature_set::FeatureSet;
use solana_svm::transaction_processing_result::ProcessedTransaction;
use solana_svm::transaction_processor::{TransactionProcessingConfig, TransactionProcessingEnvironment};

use crate::state::rollup_account_loader::RollUpAccountLoader;
use crate::{ForkRollUpGraph, ReturnStruct};
use crate::utils::helpers::{
    get_transaction_check_results,
    create_transaction_batch_processor,
};


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