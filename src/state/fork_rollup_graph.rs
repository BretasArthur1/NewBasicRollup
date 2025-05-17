use solana_program_runtime::loaded_programs::{BlockRelation, ForkGraph};
use solana_sdk::clock::Slot;

pub(crate) struct ForkRollUpGraph {}

impl ForkGraph for ForkRollUpGraph {
    fn relationship(&self, _a: Slot, _b: Slot) -> BlockRelation {
        BlockRelation::Unknown
    }
}
