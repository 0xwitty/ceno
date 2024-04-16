use gkr::structs::Circuit;
use goldilocks::SmallField;
use paste::paste;
use simple_frontend::structs::CircuitBuilder;
use singer_utils::{
    chip_handler::ROMOperations,
    chips::IntoEnumIterator,
    constants::OpcodeType,
    register_witness,
    structs::{ChipChallenges, InstOutChipType, ROMHandler, StackUInt, TSUInt},
    uint::UIntAddSub,
};
use std::sync::Arc;

use crate::{
    component::{FromPredInst, FromWitness, InstCircuit, InstLayout, ToSuccInst},
    error::ZKVMError,
    utils::add_assign_each_cell,
};

use super::{Instruction, InstructionGraph};

pub struct AddInstruction;

impl<F: SmallField> InstructionGraph<F> for AddInstruction {
    type InstType = Self;
}

register_witness!(
    AddInstruction,
    phase0 {
        // Witness for addend_0 + addend_1
        instruction_add => UIntAddSub::<StackUInt>::N_WITNESS_CELLS
    }
);

impl<F: SmallField> Instruction<F> for AddInstruction {
    const OPCODE: OpcodeType = OpcodeType::ADD;
    const NAME: &'static str = "ADD";
    fn construct_circuit(challenges: ChipChallenges) -> Result<InstCircuit<F>, ZKVMError> {
        let mut circuit_builder = CircuitBuilder::new();

        // From witness
        let (phase0_wire_id, phase0) = circuit_builder.create_witness_in(Self::phase0_size());

        // From predesessor instruction
        let (memory_ts_id, memory_ts) = circuit_builder.create_witness_in(TSUInt::N_OPRAND_CELLS);
        let (addend_0_id, addend_0) = circuit_builder.create_witness_in(StackUInt::N_OPRAND_CELLS);
        let (addend_1_id, addend_1) = circuit_builder.create_witness_in(StackUInt::N_OPRAND_CELLS);

        let mut rom_handler = ROMHandler::new(&challenges);

        // Execution result = addend0 + addend1, with carry.
        let addend_0 = addend_0.try_into()?;
        let addend_1 = addend_1.try_into()?;
        let result = UIntAddSub::<StackUInt>::add(
            &mut circuit_builder,
            &mut rom_handler,
            &addend_0,
            &addend_1,
            &phase0[Self::phase0_instruction_add()],
        )?;
        // To successor instruction
        let stack_result_id = circuit_builder.create_witness_out_from_cells(result.values());
        let (next_memory_ts_id, next_memory_ts) =
            circuit_builder.create_witness_out(TSUInt::N_OPRAND_CELLS);
        add_assign_each_cell(&mut circuit_builder, &next_memory_ts, &memory_ts);

        // To chips
        let rom_id = rom_handler.finalize(&mut circuit_builder);
        circuit_builder.configure();

        let mut to_chip_ids = vec![None; InstOutChipType::iter().count()];
        to_chip_ids[InstOutChipType::ROMInput as usize] = rom_id;

        Ok(InstCircuit {
            circuit: Arc::new(Circuit::new(&circuit_builder)),
            layout: InstLayout {
                from_pred_inst: FromPredInst {
                    memory_ts_id,
                    stack_operand_ids: vec![addend_0_id, addend_1_id],
                },
                from_witness: FromWitness {
                    phase_ids: vec![phase0_wire_id],
                },
                from_public_io: None,

                to_chip_ids,
                to_succ_inst: ToSuccInst {
                    next_memory_ts_id,
                    stack_result_ids: vec![stack_result_id],
                },
                to_bb_final: None,
                to_acc_dup: None,
                to_acc_ooo: None,
            },
        })
    }
}

#[cfg(test)]
mod test {
    use crate::instructions::{AddInstruction, ChipChallenges};
    use core::ops::Range;
    use simple_frontend::structs::CellId;
    use std::collections::BTreeMap;

    impl AddInstruction {
        #[inline]
        fn phase0_idxes_map() -> BTreeMap<String, Range<CellId>> {
            let mut map = BTreeMap::new();
            map.insert(
                "phase0_instruction_add".to_string(),
                Self::phase0_instruction_add(),
            );

            map
        }
    }

    #[test]
    fn test_add_construct_circuit() {
        let challenges = ChipChallenges::default();

        let phase0_idx_map = AddInstruction::phase0_idxes_map();
        let phase0_witness_size = AddInstruction::phase0_size();

        #[cfg(feature = "witness-count")]
        {
            println!("ADD: {:?}", &phase0_idx_map);
            println!("ADD witness_size: {:?}", phase0_witness_size);
        }
    }
}
