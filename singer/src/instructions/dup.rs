use ff::Field;
use gkr::structs::{Circuit, LayerWitness};
use goldilocks::SmallField;
use revm_interpreter::Record;

use super::InstructionGraph;
use crate::instructions::InstCircuitLayout;

use crate::{CircuitWiresIn, PrepareSingerWiresIn, SingerWiresIn};
use paste::paste;
use simple_frontend::structs::{CircuitBuilder, MixedCell};
use singer_utils::uint::u2fvec;
use singer_utils::{
    chip_handler::{
        BytecodeChipOperations, CalldataChipOperations, GlobalStateChipOperations, OAMOperations,
        ROMOperations, RangeChipOperations, StackChipOperations,
    },
    constants::OpcodeType,
    register_witness,
    structs::{PCUInt, RAMHandler, ROMHandler, StackUInt, TSUInt, UInt64},
    uint::{UIntAddSub, UIntCmp},
};
use singer_utils::{
    copy_carry_values_from_addends, copy_clock_from_record, copy_operand_from_record,
    copy_operand_timestamp_from_record, copy_pc_add_from_record, copy_pc_from_record,
    copy_range_values_from_u256, copy_stack_memory_ts_add_from_record, copy_stack_top_from_record,
    copy_stack_ts_add_from_record, copy_stack_ts_from_record, copy_stack_ts_lt_from_record,
};
use std::sync::Arc;

use crate::error::ZKVMError;

use super::{ChipChallenges, InstCircuit, Instruction};

pub struct DupInstruction<const N: usize>;

impl<F: SmallField, const N: usize> InstructionGraph<F> for DupInstruction<N> {
    type InstType = Self;
}

register_witness!(
    DupInstruction<N>,
    phase0 {
        pc => PCUInt::N_OPRAND_CELLS,
        stack_ts => TSUInt::N_OPRAND_CELLS,
        memory_ts => TSUInt::N_OPRAND_CELLS,
        stack_top => 1,
        clk => 1,

        pc_add => UIntAddSub::<PCUInt>::N_NO_OVERFLOW_WITNESS_UNSAFE_CELLS,
        stack_ts_add => UIntAddSub::<TSUInt>::N_NO_OVERFLOW_WITNESS_CELLS,

        stack_values => StackUInt::N_OPRAND_CELLS,
        old_stack_ts => TSUInt::N_OPRAND_CELLS,
        old_stack_ts_lt => UIntCmp::<TSUInt>::N_NO_OVERFLOW_WITNESS_CELLS
    }
);

impl<const N: usize> DupInstruction<N> {
    const OPCODE: OpcodeType = match N {
        1 => OpcodeType::DUP1,
        2 => OpcodeType::DUP2,
        _ => unimplemented!(),
    };
}

impl<F: SmallField, const N: usize> Instruction<F> for DupInstruction<N> {
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
            stack_top_expr.add(F::BaseField::from(1)),
            clk_expr.add(F::BaseField::ONE),
        );

        // Check the range of stack_top - N is within [0, 1 << STACK_TOP_BIT_WIDTH).
        rom_handler.range_check_stack_top(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(N as u64)),
        )?;

        // Pop rlc of stack[top - N] from stack
        let old_stack_ts = (&phase0[Self::phase0_old_stack_ts()]).try_into()?;
        UIntCmp::<TSUInt>::assert_lt(
            &mut circuit_builder,
            &mut rom_handler,
            &old_stack_ts,
            &stack_ts,
            &phase0[Self::phase0_old_stack_ts_lt()],
        )?;
        let stack_values = &phase0[Self::phase0_stack_values()];
        ram_handler.stack_pop(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(1)),
            old_stack_ts.values(),
            stack_values,
        );

        // Check the range of stack_top within [0, 1 << STACK_TOP_BIT_WIDTH).
        rom_handler.range_check_stack_top(&mut circuit_builder, stack_top.into())?;
        // Push stack_values twice to stack
        ram_handler.stack_push(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(1)),
            stack_ts.values(),
            stack_values,
        );
        ram_handler.stack_push(
            &mut circuit_builder,
            stack_top_expr,
            stack_ts.values(),
            stack_values,
        );

        // Bytecode check for (pc, DUP{N})
        rom_handler.bytecode_with_pc_opcode(&mut circuit_builder, pc.values(), Self::OPCODE);

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

    fn generate_wires_in(record: &Record) -> CircuitWiresIn<F> {
        let mut wire_values = vec![F::ZERO; Self::phase0_size()];
        copy_pc_from_record!(wire_values, record);
        copy_stack_ts_from_record!(wire_values, record);
        copy_stack_top_from_record!(wire_values, record);
        copy_clock_from_record!(wire_values, record);
        copy_pc_add_from_record!(wire_values, record);
        copy_stack_ts_add_from_record!(wire_values, record);
        copy_stack_ts_lt_from_record!(wire_values, record);

        vec![LayerWitness {
            instances: vec![wire_values],
        }]
    }
}
