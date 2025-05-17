/// Return structure for rollup transaction processing results
///
/// -> This structure provides information about a transaction's execution:
/// - Whether it was successful
/// - The amount of compute units used
/// - A descriptive message with detailed results or error information
pub struct ReturnStruct {
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
            result: format!(
                "Transaction executed successfully with {} compute units",
                cu
            ),
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
