use ff::Field;
use gkr::structs::Circuit;
use goldilocks::SmallField;
use paste::paste;
use simple_frontend::structs::{CircuitBuilder, MixedCell};
use std::sync::Arc;

use crate::{
    constants::OpcodeType,
    error::ZKVMError,
    utils::{
        chip_handler::{
            BytecodeChipOperations, ChipHandler, GlobalStateChipOperations, RangeChipOperations,
            StackChipOperations,
        },
        uint::{PCUInt, StackUInt, TSUInt, UIntAddSub, UIntCmp},
    },
};

use super::{ChipChallenges, InstCircuit, InstCircuitLayout, Instruction, InstructionGraph};
pub struct PopInstruction;

impl InstructionGraph for PopInstruction {
    type InstType = Self;
}

register_witness!(
    PopInstruction,
    phase0 {
        pc => PCUInt::N_OPRAND_CELLS,
        stack_ts => TSUInt::N_OPRAND_CELLS,
        memory_ts => TSUInt::N_OPRAND_CELLS,
        stack_top => 1,
        clk => 1,

        pc_add => UIntAddSub::<PCUInt>::N_NO_OVERFLOW_WITNESS_UNSAFE_CELLS,

        old_stack_ts => TSUInt::N_OPRAND_CELLS,
        old_stack_ts_lt => UIntCmp::<TSUInt>::N_NO_OVERFLOW_WITNESS_CELLS,
        stack_values => StackUInt::N_OPRAND_CELLS
    }
);

impl PopInstruction {
    const OPCODE: OpcodeType = OpcodeType::POP;
}

impl Instruction for PopInstruction {
    fn construct_circuit<F: SmallField>(
        challenges: ChipChallenges,
    ) -> Result<InstCircuit<F>, ZKVMError> {
        let mut circuit_builder = CircuitBuilder::new();
        let (phase0_wire_id, phase0) = circuit_builder.create_witness_in(Self::phase0_size());
        let mut global_state_in_handler = ChipHandler::new(challenges.global_state());
        let mut global_state_out_handler = ChipHandler::new(challenges.global_state());
        let mut bytecode_chip_handler = ChipHandler::new(challenges.bytecode());
        let mut stack_pop_handler = ChipHandler::new(challenges.stack());
        let mut range_chip_handler = ChipHandler::new(challenges.range());

        // State update
        let pc = PCUInt::try_from(&phase0[Self::phase0_pc()])?;
        let stack_ts = TSUInt::try_from(&phase0[Self::phase0_stack_ts()])?;
        let memory_ts = &phase0[Self::phase0_memory_ts()];
        let stack_top = phase0[Self::phase0_stack_top().start];
        let stack_top_expr = MixedCell::Cell(stack_top);
        let clk = phase0[Self::phase0_clk().start];
        let clk_expr = MixedCell::Cell(clk);
        global_state_in_handler.state_in(
            &mut circuit_builder,
            pc.values(),
            stack_ts.values(),
            &memory_ts,
            stack_top,
            clk,
        );

        let next_pc = ChipHandler::add_pc_const(
            &mut circuit_builder,
            &pc,
            1,
            &phase0[Self::phase0_pc_add()],
        )?;
        global_state_out_handler.state_out(
            &mut circuit_builder,
            next_pc.values(),
            stack_ts.values(),
            memory_ts,
            stack_top_expr.sub(F::BaseField::from(1)),
            clk_expr.add(F::BaseField::ONE),
        );

        // Check the range of stack_top - 1 is within [0, 1 << STACK_TOP_BIT_WIDTH).
        range_chip_handler
            .range_check_stack_top(&mut circuit_builder, stack_top_expr.sub(F::BaseField::ONE))?;

        // Pop rlc from stack
        let old_stack_ts = (&phase0[Self::phase0_old_stack_ts()]).try_into()?;
        UIntCmp::<TSUInt>::assert_lt(
            &mut circuit_builder,
            &mut range_chip_handler,
            &old_stack_ts,
            &stack_ts,
            &phase0[Self::phase0_old_stack_ts_lt()],
        )?;
        let stack_values = &phase0[Self::phase0_stack_values()];
        stack_pop_handler.stack_pop(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(1)),
            old_stack_ts.values(),
            stack_values,
        );

        // Bytecode check for (pc, POP)
        bytecode_chip_handler.bytecode_with_pc_opcode(
            &mut circuit_builder,
            pc.values(),
            Self::OPCODE,
        );

        let global_state_in_id = global_state_in_handler
            .finalize_with_const_pad(&mut circuit_builder, F::BaseField::ONE);
        let global_state_out_id = global_state_out_handler
            .finalize_with_const_pad(&mut circuit_builder, F::BaseField::ONE);
        let bytecode_chip_id =
            bytecode_chip_handler.finalize_with_repeated_last(&mut circuit_builder);
        let stack_pop_id =
            stack_pop_handler.finalize_with_const_pad(&mut circuit_builder, F::BaseField::ONE);
        let range_chip_id = range_chip_handler.finalize_with_repeated_last(&mut circuit_builder);
        circuit_builder.configure();

        let outputs_wire_id = [
            Some(global_state_in_id),
            Some(global_state_out_id),
            Some(bytecode_chip_id),
            Some(stack_pop_id),
            None,
            Some(range_chip_id),
            None,
            None,
            None,
        ];

        Ok(InstCircuit {
            circuit: Arc::new(Circuit::new(&circuit_builder)),
            layout: InstCircuitLayout {
                chip_check_wire_id: outputs_wire_id,
                phases_wire_id: vec![phase0_wire_id],
                ..Default::default()
            },
        })
    }
}

#[cfg(test)]
mod test {
    use core::ops::Range;
    use std::collections::BTreeMap;

    use crate::instructions::{ChipChallenges, Instruction, PopInstruction};
    use crate::test::{get_uint_params, test_opcode_circuit};
    use crate::utils::uint::TSUInt;
    use goldilocks::Goldilocks;
    use simple_frontend::structs::CellId;

    impl PopInstruction {
        #[inline]
        fn phase0_idxes_map() -> BTreeMap<String, Range<CellId>> {
            let mut map = BTreeMap::new();
            map.insert("phase0_pc".to_string(), Self::phase0_pc());
            map.insert("phase0_stack_ts".to_string(), Self::phase0_stack_ts());
            map.insert("phase0_memory_ts".to_string(), Self::phase0_memory_ts());
            map.insert("phase0_stack_top".to_string(), Self::phase0_stack_top());
            map.insert("phase0_clk".to_string(), Self::phase0_clk());
            map.insert("phase0_pc_add".to_string(), Self::phase0_pc_add());
            map.insert(
                "phase0_old_stack_ts".to_string(),
                Self::phase0_old_stack_ts(),
            );
            map.insert(
                "phase0_old_stack_ts_lt".to_string(),
                Self::phase0_old_stack_ts_lt(),
            );
            map.insert(
                "phase0_stack_values".to_string(),
                Self::phase0_stack_values(),
            );

            map
        }
    }

    #[test]
    fn test_pop_construct_circuit() {
        let challenges = ChipChallenges::default();

        // initialize general test inputs associated with push1
        let inst_circuit = PopInstruction::construct_circuit::<Goldilocks>(challenges).unwrap();

        println!("{:?}", inst_circuit);

        let phase0_idx_map = PopInstruction::phase0_idxes_map();
        println!("{:?}", &phase0_idx_map);
        let phase0_witness_size = PopInstruction::phase0_size();
        let mut phase0_values_map = BTreeMap::<String, Vec<Goldilocks>>::new();
        phase0_values_map.insert("phase0_pc".to_string(), vec![Goldilocks::from(1u64)]);
        phase0_values_map.insert("phase0_stack_ts".to_string(), vec![Goldilocks::from(2u64)]);
        phase0_values_map.insert("phase0_memory_ts".to_string(), vec![Goldilocks::from(1u64)]);
        phase0_values_map.insert(
            "phase0_stack_top".to_string(),
            vec![Goldilocks::from(100u64)],
        );
        phase0_values_map.insert("phase0_clk".to_string(), vec![Goldilocks::from(1u64)]);
        phase0_values_map.insert(
            "phase0_pc_add".to_string(),
            vec![], // carry is 0, may test carry using larger values in PCUInt
        );
        phase0_values_map.insert(
            "phase0_old_stack_ts".to_string(),
            vec![Goldilocks::from(1u64)],
        );
        let m: u64 = 1 << get_uint_params::<TSUInt>().1 - 1;
        phase0_values_map.insert(
            "phase0_old_stack_ts_lt".to_string(),
            vec![
                Goldilocks::from(m),
                -Goldilocks::from(1u64),
                Goldilocks::from(0u64),
                Goldilocks::from(1u64),
            ],
        );
        phase0_values_map.insert(
            "phase0_stack_values".to_string(),
            vec![
                Goldilocks::from(7u64),
                Goldilocks::from(6u64),
                Goldilocks::from(5u64),
                Goldilocks::from(4u64),
                Goldilocks::from(3u64),
                Goldilocks::from(2u64),
                Goldilocks::from(1u64),
            ],
        );

        let circuit_witness_challenges = vec![
            Goldilocks::from(2),
            Goldilocks::from(2),
            Goldilocks::from(2),
        ];

        test_opcode_circuit(
            &inst_circuit,
            &phase0_idx_map,
            phase0_witness_size,
            &phase0_values_map,
            circuit_witness_challenges,
        );
    }
}
