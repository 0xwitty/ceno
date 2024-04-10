use ff::Field;
use gkr::structs::Circuit;
use goldilocks::SmallField;
use itertools::izip;
use paste::paste;
use simple_frontend::structs::{CircuitBuilder, MixedCell};
use singer_utils::{
    chip_handler::{
        BytecodeChipOperations, GlobalStateChipOperations, OAMOperations, ROMOperations,
        RangeChipOperations, StackChipOperations,
    },
    constants::OpcodeType,
    register_witness,
    structs::{PCUInt, RAMHandler, ROMHandler, StackUInt, TSUInt},
    uint::{UIntAddSub, UIntCmp},
};
use std::sync::Arc;

use crate::error::ZKVMError;

use super::{ChipChallenges, InstCircuit, InstCircuitLayout, Instruction, InstructionGraph};

pub struct JumpiInstruction;

impl<F: SmallField> InstructionGraph<F> for JumpiInstruction {
    type InstType = Self;
}

register_witness!(
    JumpiInstruction,
    phase0 {
        pc => PCUInt::N_OPRAND_CELLS ,
        stack_ts => TSUInt::N_OPRAND_CELLS,
        memory_ts => TSUInt::N_OPRAND_CELLS,
        stack_top => 1,
        clk => 1,

        old_stack_ts_dest => TSUInt::N_OPRAND_CELLS,
        old_stack_ts_dest_lt => UIntCmp::<TSUInt>::N_WITNESS_CELLS,
        old_stack_ts_cond => TSUInt::N_OPRAND_CELLS,
        old_stack_ts_cond_lt => UIntCmp::<TSUInt>::N_WITNESS_CELLS,

        dest_values => StackUInt::N_OPRAND_CELLS,
        cond_values => StackUInt::N_OPRAND_CELLS,
        cond_values_inv => StackUInt::N_OPRAND_CELLS,
        cond_non_zero_or_inv => 1,

        pc_add => UIntAddSub::<PCUInt>::N_NO_OVERFLOW_WITNESS_UNSAFE_CELLS,
        pc_plus_1_opcode => 1
    }
);

impl JumpiInstruction {
    const OPCODE: OpcodeType = OpcodeType::JUMPI;
}

impl<F: SmallField> Instruction<F> for JumpiInstruction {
    fn construct_circuit(challenges: ChipChallenges) -> Result<InstCircuit<F>, ZKVMError> {
        let mut circuit_builder = CircuitBuilder::new();
        let (phase0_wire_id, phase0) = circuit_builder.create_witness_in(Self::phase0_size());
        let mut ram_handler = RAMHandler::new(&challenges);
        let mut rom_handler = ROMHandler::new(&challenges);

        // State update
        let pc = PCUInt::try_from(&phase0[Self::phase0_pc()])?;
        let stack_ts = TSUInt::try_from(&phase0[Self::phase0_stack_ts()])?;
        let memory_ts = &phase0[Self::phase0_memory_ts()];
        let stack_top = phase0[Self::phase0_stack_top().start];
        let stack_top_expr = MixedCell::Cell(stack_top);
        let clk = phase0[Self::phase0_clk().start];
        let clk_expr = MixedCell::Cell(clk);
        ram_handler.state_in(
            &mut circuit_builder,
            pc.values(),
            stack_ts.values(),
            &memory_ts,
            stack_top,
            clk,
        );

        // Range check stack_top - 2
        rom_handler.range_check_stack_top(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(2)),
        )?;

        // Pop the destination pc from stack.
        let dest_values = &phase0[Self::phase0_dest_values()];
        let dest_stack_addr = stack_top_expr.sub(F::BaseField::ONE);

        let old_stack_ts_dest = (&phase0[Self::phase0_old_stack_ts_dest()]).try_into()?;
        UIntCmp::<TSUInt>::assert_lt(
            &mut circuit_builder,
            &mut rom_handler,
            &old_stack_ts_dest,
            &stack_ts,
            &phase0[Self::phase0_old_stack_ts_dest_lt()],
        )?;
        ram_handler.stack_pop(
            &mut circuit_builder,
            dest_stack_addr,
            old_stack_ts_dest.values(),
            dest_values,
        );

        // Pop the condition from stack.
        let cond_values = &phase0[Self::phase0_cond_values()];
        let old_stack_ts_cond = (&phase0[Self::phase0_old_stack_ts_cond()]).try_into()?;
        UIntCmp::<TSUInt>::assert_lt(
            &mut circuit_builder,
            &mut rom_handler,
            &old_stack_ts_cond,
            &stack_ts,
            &phase0[Self::phase0_old_stack_ts_cond_lt()],
        )?;

        ram_handler.stack_pop(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(2)),
            old_stack_ts_cond.values(),
            cond_values,
        );

        // Execution, cond_values_non_zero[i] = [cond_values[i] != 0]
        let cond_values_inv = &phase0[Self::phase0_cond_values_inv()];
        let mut cond_values_non_zero = Vec::new();
        for (val, wit) in izip!(cond_values, cond_values_inv) {
            cond_values_non_zero.push(rom_handler.non_zero(&mut circuit_builder, *val, *wit)?);
        }
        // cond_non_zero = [summation of cond_values_non_zero[i] != 0]
        let non_zero_or = circuit_builder.create_cell();
        cond_values_non_zero
            .iter()
            .for_each(|x| circuit_builder.add(non_zero_or, *x, F::BaseField::ONE));
        let cond_non_zero_or_inv = phase0[Self::phase0_cond_non_zero_or_inv().start];
        let cond_non_zero =
            rom_handler.non_zero(&mut circuit_builder, non_zero_or, cond_non_zero_or_inv)?;

        // If cond_non_zero, next_pc = dest, otherwise, pc = pc + 1
        let pc_add_1 = &phase0[Self::phase0_pc_add()];
        let pc_plus_1 = ROMHandler::add_pc_const(&mut circuit_builder, &pc, 1, pc_add_1)?;
        let pc_plus_1 = pc_plus_1.values();
        let next_pc = circuit_builder.create_cells(PCUInt::N_OPRAND_CELLS);
        for i in 0..PCUInt::N_OPRAND_CELLS {
            circuit_builder.select(next_pc[i], pc_plus_1[i], dest_values[i], cond_non_zero);
        }

        // State out
        ram_handler.state_out(
            &mut circuit_builder,
            &next_pc,
            stack_ts.values(), // Because there is no stack push.
            memory_ts,
            stack_top_expr.sub(F::BaseField::from(2)),
            clk_expr.add(F::BaseField::ONE),
        );

        // Bytecode check for (pc, jumpi)
        rom_handler.bytecode_with_pc_opcode(&mut circuit_builder, pc.values(), Self::OPCODE);

        // If cond_non_zero, next_opcode = JUMPDEST, otherwise, opcode = pc + 1 opcode
        let pc_plus_1_opcode = phase0[Self::phase0_pc_plus_1_opcode().start];
        let next_opcode = circuit_builder.create_cell();
        circuit_builder.sel_mixed(
            next_opcode,
            pc_plus_1_opcode.into(),
            MixedCell::Constant(F::BaseField::from(OpcodeType::JUMPDEST as u64)),
            cond_non_zero,
        );

        // Bytecode check for (next_pc, next_opcode)
        rom_handler.bytecode_with_pc_byte(&mut circuit_builder, &next_pc, next_opcode);

        let (ram_load_id, ram_store_id) = ram_handler.finalize(&mut circuit_builder);
        let rom_id = rom_handler.finalize(&mut circuit_builder);
        circuit_builder.configure();

        let outputs_wire_id = [ram_load_id, ram_store_id, rom_id];

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

    use crate::instructions::{ChipChallenges, Instruction, JumpiInstruction};
    use crate::test::{get_uint_params, test_opcode_circuit, u2vec};
    use goldilocks::Goldilocks;
    use simple_frontend::structs::CellId;
    use singer_utils::constants::RANGE_CHIP_BIT_WIDTH;
    use singer_utils::structs::TSUInt;

    impl JumpiInstruction {
        #[inline]
        fn phase0_idxes_map() -> BTreeMap<String, Range<CellId>> {
            let mut map = BTreeMap::new();
            map.insert("phase0_pc".to_string(), Self::phase0_pc());
            map.insert("phase0_stack_ts".to_string(), Self::phase0_stack_ts());
            map.insert("phase0_memory_ts".to_string(), Self::phase0_memory_ts());
            map.insert("phase0_stack_top".to_string(), Self::phase0_stack_top());
            map.insert("phase0_clk".to_string(), Self::phase0_clk());
            map.insert(
                "phase0_old_stack_ts_dest".to_string(),
                Self::phase0_old_stack_ts_dest(),
            );
            map.insert(
                "phase0_old_stack_ts_dest_lt".to_string(),
                Self::phase0_old_stack_ts_dest_lt(),
            );
            map.insert(
                "phase0_old_stack_ts_cond".to_string(),
                Self::phase0_old_stack_ts_cond(),
            );
            map.insert(
                "phase0_old_stack_ts_cond_lt".to_string(),
                Self::phase0_old_stack_ts_cond_lt(),
            );
            map.insert("phase0_dest_values".to_string(), Self::phase0_dest_values());
            map.insert("phase0_cond_values".to_string(), Self::phase0_cond_values());
            map.insert(
                "phase0_cond_values_inv".to_string(),
                Self::phase0_cond_values_inv(),
            );
            map.insert(
                "phase0_cond_non_zero_or_inv".to_string(),
                Self::phase0_cond_non_zero_or_inv(),
            );
            map.insert("phase0_pc_add".to_string(), Self::phase0_pc_add());
            map.insert(
                "phase0_pc_plus_1_opcode".to_string(),
                Self::phase0_pc_plus_1_opcode(),
            );

            map
        }
    }

    #[test]
    fn test_jumpi_construct_circuit() {
        let challenges = ChipChallenges::default();

        let phase0_idx_map = JumpiInstruction::phase0_idxes_map();
        let phase0_witness_size = JumpiInstruction::phase0_size();

        #[cfg(feature = "witness-count")]
        {
            println!("JUMPI {:?}", &phase0_idx_map);
            println!("JUMPI witness_size: {:?}", phase0_witness_size);
        }

        // initialize general test inputs associated with push1
        let inst_circuit = JumpiInstruction::construct_circuit(challenges).unwrap();

        #[cfg(feature = "test-dbg")]
        println!("{:?}", inst_circuit);

        let mut phase0_values_map = BTreeMap::<String, Vec<Goldilocks>>::new();
        phase0_values_map.insert("phase0_pc".to_string(), vec![Goldilocks::from(1u64)]);
        phase0_values_map.insert("phase0_stack_ts".to_string(), vec![Goldilocks::from(3u64)]);
        phase0_values_map.insert("phase0_memory_ts".to_string(), vec![Goldilocks::from(1u64)]);
        phase0_values_map.insert(
            "phase0_stack_top".to_string(),
            vec![Goldilocks::from(100u64)],
        );
        phase0_values_map.insert("phase0_clk".to_string(), vec![Goldilocks::from(1u64)]);
        phase0_values_map.insert(
            "phase0_old_stack_ts_dest".to_string(),
            vec![Goldilocks::from(2u64)],
        );
        let m: u64 = (1 << get_uint_params::<TSUInt>().1) - 1;
        let range_values = u2vec::<{ TSUInt::N_RANGE_CHECK_CELLS }, RANGE_CHIP_BIT_WIDTH>(m);
        phase0_values_map.insert(
            "phase0_old_stack_ts_dest_lt".to_string(),
            vec![
                Goldilocks::from(range_values[0]),
                Goldilocks::from(range_values[1]),
                Goldilocks::from(range_values[2]),
                Goldilocks::from(range_values[3]),
                Goldilocks::from(1u64), // borrow
            ],
        );
        phase0_values_map.insert(
            "phase0_old_stack_ts_cond".to_string(),
            vec![Goldilocks::from(1u64)],
        );
        let m: u64 = (1 << get_uint_params::<TSUInt>().1) - 2;
        let range_values = u2vec::<{ TSUInt::N_RANGE_CHECK_CELLS }, RANGE_CHIP_BIT_WIDTH>(m);
        phase0_values_map.insert(
            "phase0_old_stack_ts_cond_lt".to_string(),
            vec![
                Goldilocks::from(range_values[0]),
                Goldilocks::from(range_values[1]),
                Goldilocks::from(range_values[2]),
                Goldilocks::from(range_values[3]),
                Goldilocks::from(1u64), // borrow
            ],
        );
        phase0_values_map.insert(
            "phase0_dest_values".to_string(),
            vec![
                Goldilocks::from(7u64),
                Goldilocks::from(6u64),
                Goldilocks::from(5u64),
                Goldilocks::from(4u64),
                Goldilocks::from(3u64),
                Goldilocks::from(2u64),
                Goldilocks::from(1u64),
                Goldilocks::from(0u64),
            ],
        );
        phase0_values_map.insert(
            "phase0_cond_values".to_string(),
            vec![
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
            ], // when cond is zero, pc is increased by 1
        );
        phase0_values_map.insert(
            "phase0_cond_values_inv".to_string(),
            vec![
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
                Goldilocks::from(0u64),
            ], 
        );
        phase0_values_map.insert(
            "phase0_cond_non_zero_or_inv".to_string(),
            vec![Goldilocks::from(0u64)],
        );
        phase0_values_map.insert(
            "phase0_pc_add".to_string(),
            vec![], // carry is 0, may test carry using larger values in PCUInt
        );
        phase0_values_map.insert(
            "phase0_pc_plus_1_opcode".to_string(),
            vec![Goldilocks::from(2u64)],
        );

        println!("phase0_values_map {:?}", phase0_values_map);
        
        let circuit_witness_challenges = vec![
            Goldilocks::from(2),
            Goldilocks::from(2),
            Goldilocks::from(2),
        ];

        let _circuit_witness = test_opcode_circuit(
            &inst_circuit,
            &phase0_idx_map,
            phase0_witness_size,
            &phase0_values_map,
            circuit_witness_challenges,
        );
    }
}