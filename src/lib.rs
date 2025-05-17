use error::SolanaClientExtError;
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::{message::Message, signers::Signers, transaction::Transaction};
// use solana_svm_callback::InvokeContextCallback;
mod error;
pub mod state;
mod utils;

use crate::state::fork_rollup_graph::ForkRollUpGraph;

pub use state::{return_struct::ReturnStruct, rollup_channel::RollUpChannel};

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
        let failures: Vec<&ReturnStruct> = results.iter().filter(|r| !r.success).collect();

        if !failures.is_empty() {
            let error_messages = failures
                .iter()
                .map(|r| r.result.clone())
                .collect::<Vec<String>>()
                .join("\n");

            return Err(Box::new(SolanaClientExtError::ComputeUnitsError(format!(
                "Transaction simulation failed:\n{}",
                error_messages
            ))));
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

        let optimize_ix =
            ComputeBudgetInstruction::set_compute_unit_limit(optimal_cu.saturating_add(optimal_cu));
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
