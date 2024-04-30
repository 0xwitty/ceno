use ff::Field;
use gkr::structs::Circuit;
use goldilocks::SmallField;
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
pub struct SwapInstruction<const N: usize>;

impl<F: SmallField, const N: usize> InstructionGraph<F> for SwapInstruction<N> {
    type InstType = Self;
}

register_witness!(
    SwapInstruction<N>,
    phase0 {
        pc => PCUInt::N_OPRAND_CELLS,
        stack_ts => TSUInt::N_OPRAND_CELLS,
        memory_ts => TSUInt::N_OPRAND_CELLS,
        stack_top => 1,
        clk => 1,

        pc_add => UIntAddSub::<PCUInt>::N_NO_OVERFLOW_WITNESS_UNSAFE_CELLS,
        stack_ts_add => UIntAddSub::<TSUInt>::N_NO_OVERFLOW_WITNESS_CELLS,

        old_stack_ts_1 => TSUInt::N_OPRAND_CELLS,
        old_stack_ts_lt_1 => UIntCmp::<TSUInt>::N_WITNESS_CELLS,
        old_stack_ts_n_plus_1 => TSUInt::N_OPRAND_CELLS,
        old_stack_ts_lt_n_plus_1 => UIntCmp::<TSUInt>::N_WITNESS_CELLS,
        stack_values_1 => StackUInt::N_OPRAND_CELLS,
        stack_values_n_plus_1 => StackUInt::N_OPRAND_CELLS
    }
);

impl<F: SmallField, const N: usize> Instruction<F> for SwapInstruction<N> {
    const OPCODE: OpcodeType = match N {
        1 => OpcodeType::SWAP1,
        2 => OpcodeType::SWAP2,
        4 => OpcodeType::SWAP4,
        _ => unimplemented!(),
    };
    const NAME: &'static str = match N {
        1 => "SWAP1",
        2 => "SWAP2",
        4 => "SWAP4",
        _ => unimplemented!(),
    };
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

        let next_pc =
            ROMHandler::add_pc_const(&mut circuit_builder, &pc, 1, &phase0[Self::phase0_pc_add()])?;
        let next_stack_ts = rom_handler.add_ts_with_const(
            &mut circuit_builder,
            &stack_ts,
            1,
            &phase0[Self::phase0_stack_ts_add()],
        )?;

        ram_handler.state_out(
            &mut circuit_builder,
            next_pc.values(),
            next_stack_ts.values(),
            &memory_ts,
            stack_top_expr,
            clk_expr.add(F::BaseField::ONE),
        );

        // Check the range of stack_top - (N + 1) is within [0, 1 << STACK_TOP_BIT_WIDTH).
        rom_handler.range_check_stack_top(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(N as u64 + 1)),
        )?;

        // Pop rlc of stack[top - (N + 1)] from stack
        let old_stack_ts_n_plus_1 = (&phase0[Self::phase0_old_stack_ts_n_plus_1()]).try_into()?;
        UIntCmp::<TSUInt>::assert_lt(
            &mut circuit_builder,
            &mut rom_handler,
            &old_stack_ts_n_plus_1,
            &stack_ts,
            &phase0[Self::phase0_old_stack_ts_lt_n_plus_1()],
        )?;
        let stack_values_n_plus_1 = &phase0[Self::phase0_stack_values_n_plus_1()];
        ram_handler.stack_pop(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(N as u64 + 1)),
            old_stack_ts_n_plus_1.values(),
            stack_values_n_plus_1,
        );

        // Pop rlc of stack[top - 1] from stack
        let old_stack_ts_1 = (&phase0[Self::phase0_old_stack_ts_1()]).try_into()?;
        UIntCmp::<TSUInt>::assert_lt(
            &mut circuit_builder,
            &mut rom_handler,
            &old_stack_ts_1,
            &stack_ts,
            &phase0[Self::phase0_old_stack_ts_lt_1()],
        )?;
        let stack_values_1 = &phase0[Self::phase0_stack_values_1()];
        ram_handler.stack_pop(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::ONE),
            old_stack_ts_1.values(),
            stack_values_1,
        );

        // Push stack_1 to the stack at top - (N + 1)
        ram_handler.stack_push(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(N as u64 + 1)),
            stack_ts.values(),
            stack_values_1,
        );
        // Push stack_n_plus_1 to the stack at top - 1
        ram_handler.stack_push(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::ONE),
            stack_ts.values(),
            stack_values_n_plus_1,
        );

        // Bytecode check for (pc, SWAP{N}).
        rom_handler.bytecode_with_pc_opcode(
            &mut circuit_builder,
            pc.values(),
            <Self as Instruction<F>>::OPCODE,
        );

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
    use ark_std::test_rng;
    use core::ops::Range;
    use ff::Field;
    use gkr::structs::LayerWitness;
    use goldilocks::{Goldilocks, GoldilocksExt2, SmallField};
    use itertools::Itertools;
    use simple_frontend::structs::CellId;
    use singer_utils::constants::RANGE_CHIP_BIT_WIDTH;
    use singer_utils::structs::TSUInt;
    use std::collections::BTreeMap;
    use std::time::Instant;
    use transcript::Transcript;

    use crate::instructions::{
        ChipChallenges, Instruction, InstructionGraph, SingerCircuitBuilder, SwapInstruction,
    };
    use crate::scheme::GKRGraphProverState;
    use crate::test::{get_uint_params, test_opcode_circuit, u2vec};
    use crate::{CircuitWiresIn, SingerGraphBuilder, SingerParams};

    impl<const N: usize> SwapInstruction<N> {
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
                "phase0_stack_ts_add".to_string(),
                Self::phase0_stack_ts_add(),
            );
            map.insert(
                "phase0_old_stack_ts_1".to_string(),
                Self::phase0_old_stack_ts_1(),
            );
            map.insert(
                "phase0_old_stack_ts_lt_1".to_string(),
                Self::phase0_old_stack_ts_lt_1(),
            );
            map.insert(
                "phase0_old_stack_ts_n_plus_1".to_string(),
                Self::phase0_old_stack_ts_n_plus_1(),
            );
            map.insert(
                "phase0_old_stack_ts_lt_n_plus_1".to_string(),
                Self::phase0_old_stack_ts_lt_n_plus_1(),
            );
            map.insert(
                "phase0_stack_values_1".to_string(),
                Self::phase0_stack_values_1(),
            );
            map.insert(
                "phase0_stack_values_n_plus_1".to_string(),
                Self::phase0_stack_values_n_plus_1(),
            );

            map
        }
    }

    fn test_swap2_construct_circuit_helper<F: SmallField>() {
        let challenges = ChipChallenges::default();

        let phase0_idx_map = SwapInstruction::<2>::phase0_idxes_map();
        let phase0_witness_size = SwapInstruction::<2>::phase0_size();

        #[cfg(feature = "witness-count")]
        {
            println!("SWAP2 {:?}", &phase0_idx_map);
            println!("SWAP2 witness_size = {:?}", phase0_witness_size);
        }

        // initialize general test inputs associated with push1
        let inst_circuit = SwapInstruction::<2>::construct_circuit(challenges).unwrap();

        #[cfg(feature = "test-dbg")]
        println!("{:?}", inst_circuit);

        let mut phase0_values_map = BTreeMap::<String, Vec<F::BaseField>>::new();
        phase0_values_map.insert("phase0_pc".to_string(), vec![F::BaseField::from(1u64)]);
        phase0_values_map.insert(
            "phase0_stack_ts".to_string(),
            vec![F::BaseField::from(4u64)],
        );
        phase0_values_map.insert(
            "phase0_memory_ts".to_string(),
            vec![F::BaseField::from(1u64)],
        );
        phase0_values_map.insert(
            "phase0_stack_top".to_string(),
            vec![F::BaseField::from(100u64)],
        );
        phase0_values_map.insert("phase0_clk".to_string(), vec![F::BaseField::from(1u64)]);
        phase0_values_map.insert(
            "phase0_pc_add".to_string(),
            vec![], // carry is 0, may test carry using larger values in PCUInt
        );
        phase0_values_map.insert(
            "phase0_stack_ts_add".to_string(),
            vec![
                F::BaseField::from(5u64), // first TSUInt::N_RANGE_CHECK_CELLS = 1*(56/16) = 4 cells are range values, stack_ts + 1 = 4
                F::BaseField::from(0u64),
                F::BaseField::from(0u64),
                F::BaseField::from(0u64),
                // no place for carry
            ],
        );
        phase0_values_map.insert(
            "phase0_old_stack_ts_1".to_string(),
            vec![F::BaseField::from(3u64)],
        );
        let m: u64 = (1 << get_uint_params::<TSUInt>().1) - 1;
        let range_values = u2vec::<{ TSUInt::N_RANGE_CHECK_CELLS }, RANGE_CHIP_BIT_WIDTH>(m);
        phase0_values_map.insert(
            "phase0_old_stack_ts_lt_1".to_string(),
            vec![
                F::BaseField::from(range_values[0]),
                F::BaseField::from(range_values[1]),
                F::BaseField::from(range_values[2]),
                F::BaseField::from(range_values[3]),
                F::BaseField::from(1u64), // current length has no cells for borrow
            ],
        );
        phase0_values_map.insert(
            "phase0_old_stack_ts_n_plus_1".to_string(),
            vec![F::BaseField::from(1u64)],
        );
        let m: u64 = (1 << get_uint_params::<TSUInt>().1) - 3;
        let range_values = u2vec::<{ TSUInt::N_RANGE_CHECK_CELLS }, RANGE_CHIP_BIT_WIDTH>(m);
        phase0_values_map.insert(
            "phase0_old_stack_ts_lt_n_plus_1".to_string(),
            vec![
                F::BaseField::from(range_values[0]),
                F::BaseField::from(range_values[1]),
                F::BaseField::from(range_values[2]),
                F::BaseField::from(range_values[3]),
                F::BaseField::from(1u64), // current length has no cells for borrow
            ],
        );
        phase0_values_map.insert(
            "phase0_stack_values_1".to_string(),
            vec![
                F::BaseField::from(7u64),
                F::BaseField::from(6u64),
                F::BaseField::from(5u64),
                F::BaseField::from(4u64),
                F::BaseField::from(3u64),
                F::BaseField::from(2u64),
                F::BaseField::from(1u64),
                F::BaseField::from(0u64),
            ],
        );
        phase0_values_map.insert(
            "phase0_stack_values_n_plus_1".to_string(),
            vec![
                F::BaseField::from(0u64),
                F::BaseField::from(1u64),
                F::BaseField::from(2u64),
                F::BaseField::from(3u64),
                F::BaseField::from(4u64),
                F::BaseField::from(5u64),
                F::BaseField::from(6u64),
                F::BaseField::from(7u64),
            ],
        );

        let circuit_witness_challenges = vec![F::from(2), F::from(2), F::from(2)];

        let _circuit_witness = test_opcode_circuit(
            &inst_circuit,
            &phase0_idx_map,
            phase0_witness_size,
            &phase0_values_map,
            circuit_witness_challenges,
        );
    }

    #[test]
    fn test_swap2_construct_circuit() {
        test_swap2_construct_circuit_helper::<GoldilocksExt2>()
    }

    fn bench_swap_instruction_helper<F: SmallField, const N: usize>(instance_num_vars: usize) {
        let chip_challenges = ChipChallenges::default();
        let circuit_builder =
            SingerCircuitBuilder::<F>::new(chip_challenges).expect("circuit builder failed");
        let mut singer_builder = SingerGraphBuilder::<F>::new();

        let mut rng = test_rng();
        let size = SwapInstruction::<N>::phase0_size();
        let phase0: CircuitWiresIn<F::BaseField> = vec![LayerWitness {
            instances: (0..(1 << instance_num_vars))
                .map(|_| {
                    (0..size)
                        .map(|_| F::BaseField::random(&mut rng))
                        .collect_vec()
                })
                .collect_vec(),
        }];

        let real_challenges = vec![F::random(&mut rng), F::random(&mut rng)];

        let timer = Instant::now();

        let _ = SwapInstruction::<N>::construct_graph_and_witness(
            &mut singer_builder.graph_builder,
            &mut singer_builder.chip_builder,
            &circuit_builder.insts_circuits
                [<SwapInstruction<N> as Instruction<F>>::OPCODE as usize],
            vec![phase0],
            &real_challenges,
            1 << instance_num_vars,
            &SingerParams::default(),
        )
        .expect("gkr graph construction failed");

        let (graph, wit) = singer_builder.graph_builder.finalize_graph_and_witness();

        println!(
            "Swap{}Instruction::construct_graph_and_witness, instance_num_vars = {}, time = {}",
            N,
            instance_num_vars,
            timer.elapsed().as_secs_f64()
        );

        let point = vec![F::random(&mut rng), F::random(&mut rng)];
        let target_evals = graph.target_evals(&wit, &point);

        let mut prover_transcript = &mut Transcript::new(b"Singer");

        let timer = Instant::now();
        let _ = GKRGraphProverState::prove(&graph, &wit, &target_evals, &mut prover_transcript)
            .expect("prove failed");
        println!(
            "Swap{}Instruction::prove, instance_num_vars = {}, time = {}",
            N,
            instance_num_vars,
            timer.elapsed().as_secs_f64()
        );
    }

    #[test]
    fn bench_swap2_instruction() {
        bench_swap_instruction_helper::<GoldilocksExt2, 2>(10);
    }

    #[test]
    fn bench_swap4_instruction() {
        bench_swap_instruction_helper::<GoldilocksExt2, 4>(10);
    }
}
