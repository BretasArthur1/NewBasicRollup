use std::sync::{Arc, RwLock};

use solana_bpf_loader_program::syscalls::create_program_runtime_environment_v1;
use solana_compute_budget::{
    compute_budget::ComputeBudget, compute_budget_limits::ComputeBudgetLimits,
};
use solana_program_runtime::loaded_programs::ProgramCacheEntry;
use solana_sdk::transaction;
use solana_svm::account_loader::CheckedTransactionDetails;
use solana_svm::transaction_processing_callback::TransactionProcessingCallback;
use solana_svm::transaction_processor::TransactionBatchProcessor;
use solana_system_program::system_processor;

use crate::ForkRollUpGraph;
use agave_feature_set::FeatureSet;

/// This function is also a mock. In the Agave validator, the bank pre-checks
/// transactions before providing them to the SVM API. We mock this step in
/// PayTube, since we don't need to perform such pre-checks.
pub(crate) fn get_transaction_check_results(
    len: usize,
) -> Vec<transaction::Result<CheckedTransactionDetails>> {
    let _compute_budget_limit = ComputeBudgetLimits::default();
    vec![transaction::Result::Ok(CheckedTransactionDetails::new(None, 5000,)); len]
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
