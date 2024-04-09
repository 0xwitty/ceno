use ff::Field;
use gkr::structs::{Circuit, LayerWitness};
use gkr_graph::structs::{CircuitGraphBuilder, NodeOutputType, PredType};
use goldilocks::SmallField;
use paste::paste;
use revm_interpreter::Record;
use revm_primitives::U256;
use simple_frontend::structs::{CircuitBuilder, MixedCell};
use singer_utils::{
    chip_handler::{
        BytecodeChipOperations, GlobalStateChipOperations, OAMOperations, ROMOperations,
        RangeChipOperations, StackChipOperations,
    },
    chips::SingerChipBuilder,
    constants::OpcodeType,
    copy_carry_values_from_addends, copy_clock_from_record, copy_memory_ts_from_record,
    copy_operand_from_record, copy_operand_timestamp_from_record, copy_pc_from_record,
    copy_range_values_from_u256, copy_stack_top_from_record, copy_stack_ts_from_record,
    register_witness,
    structs::{PCUInt, RAMHandler, ROMHandler, StackUInt, TSUInt},
    uint::{u2fvec, UIntAddSub},
};
use std::{mem, sync::Arc};

use crate::{error::ZKVMError, utils::add_assign_each_cell, CircuitWiresIn, SingerParams};

use super::{ChipChallenges, InstCircuit, InstCircuitLayout, Instruction, InstructionGraph};

/// This circuit is to pop offset and public output size from stack.
pub struct ReturnInstruction;
/// This circuit is to load public output from memory, which is a data-parallel
/// circuit load one element in each sub-circuit.
pub struct ReturnPublicOutLoad;
/// This circuit is to load the remaining elmeents after the program execution
/// from memory, which is a data-parallel circuit load one element in each
/// sub-circuit.
pub struct ReturnRestMemLoad;
/// This circuit is to initialize the memory with 0 at the beginning. It can
/// only touches the used addresses.
pub struct ReturnRestMemStore;

impl<F: SmallField> InstructionGraph<F> for ReturnInstruction {
    type InstType = Self;

    fn construct_circuits(challenges: ChipChallenges) -> Result<Vec<InstCircuit<F>>, ZKVMError> {
        let circuits = vec![
            ReturnInstruction::construct_circuit(challenges)?,
            ReturnPublicOutLoad::construct_circuit(challenges)?,
            ReturnRestMemLoad::construct_circuit(challenges)?,
            ReturnRestMemStore::construct_circuit(challenges)?,
            ReturnRestStackPop::construct_circuit(challenges)?,
        ];
        Ok(circuits)
    }

    fn construct_graph_and_witness(
        graph_builder: &mut CircuitGraphBuilder<F>,
        chip_builder: &mut SingerChipBuilder<F>,
        inst_circuits: &[InstCircuit<F>],
        mut sources: Vec<CircuitWiresIn<F::BaseField>>,
        real_challenges: &[F],
        _: usize,
        params: &SingerParams,
    ) -> Result<Option<NodeOutputType>, ZKVMError> {
        // Add the instruction circuit to the graph.
        let inst_circuit = &inst_circuits[0];
        let n_witness_in = inst_circuit.circuit.n_witness_in;
        let inst_node_id = graph_builder.add_node_with_witness(
            stringify!(ReturnInstruction),
            &inst_circuit.circuit,
            vec![PredType::Source; n_witness_in],
            real_challenges.to_vec(),
            mem::take(&mut sources[0]),
            1,
        )?;
        chip_builder.construct_chip_check_graph_and_witness(
            graph_builder,
            inst_node_id,
            &inst_circuit.layout.chip_check_wire_id,
            real_challenges,
            1,
        )?;

        // Add the public output load circuit to the graph.
        let pub_out_load_circuit = &inst_circuits[1];
        let n_witness_in = pub_out_load_circuit.circuit.n_witness_in;
        let mut preds = vec![PredType::Source; n_witness_in];
        preds[pub_out_load_circuit.layout.pred_dup_wire_id.unwrap() as usize] =
            PredType::PredWireDup(NodeOutputType::WireOut(
                inst_node_id,
                inst_circuit.layout.succ_dup_wires_id[0],
            ));
        let pub_out_load_node_id = graph_builder.add_node_with_witness(
            stringify!(ReturnPublicOutLoad),
            &pub_out_load_circuit.circuit,
            preds,
            real_challenges.to_vec(),
            mem::take(&mut sources[1]),
            params.n_public_output_bytes,
        )?;
        chip_builder.construct_chip_check_graph_and_witness(
            graph_builder,
            pub_out_load_node_id,
            &pub_out_load_circuit.layout.chip_check_wire_id,
            real_challenges,
            params.n_public_output_bytes,
        )?;

        // Add the rest memory load circuit to the graph.
        let rest_mem_load_circuit = &inst_circuits[2];
        let n_witness_in = rest_mem_load_circuit.circuit.n_witness_in;
        let rest_mem_load_node_id = graph_builder.add_node_with_witness(
            stringify!(ReturnRestMemLoad),
            &rest_mem_load_circuit.circuit,
            vec![PredType::Source; n_witness_in],
            real_challenges.to_vec(),
            mem::take(&mut sources[2]),
            params.n_mem_finalize,
        )?;
        chip_builder.construct_chip_check_graph_and_witness(
            graph_builder,
            rest_mem_load_node_id,
            &rest_mem_load_circuit.layout.chip_check_wire_id,
            real_challenges,
            params.n_mem_finalize,
        )?;

        // Add the rest memory store circuit to the graph.
        let rest_mem_store_circuit = &inst_circuits[3];
        let n_witness_in = rest_mem_store_circuit.circuit.n_witness_in;
        let rest_mem_store_node_id = graph_builder.add_node_with_witness(
            stringify!(ReturnRestMemStore),
            &rest_mem_store_circuit.circuit,
            vec![PredType::Source; n_witness_in],
            real_challenges.to_vec(),
            mem::take(&mut sources[3]),
            params.n_mem_initialize,
        )?;
        chip_builder.construct_chip_check_graph_and_witness(
            graph_builder,
            rest_mem_store_node_id,
            &rest_mem_store_circuit.layout.chip_check_wire_id,
            real_challenges,
            params.n_mem_initialize,
        )?;

        // Add the rest stack pop circuit to the graph.
        let rest_stack_pop_circuit = &inst_circuits[4];
        let n_witness_in = rest_stack_pop_circuit.circuit.n_witness_in;
        let rest_stack_pop_node_id = graph_builder.add_node_with_witness(
            stringify!(ReturnRestStackPop),
            &rest_stack_pop_circuit.circuit,
            vec![PredType::Source; n_witness_in],
            real_challenges.to_vec(),
            mem::take(&mut sources[4]),
            params.n_stack_finalize,
        )?;
        chip_builder.construct_chip_check_graph_and_witness(
            graph_builder,
            rest_stack_pop_node_id,
            &rest_stack_pop_circuit.layout.chip_check_wire_id,
            real_challenges,
            params.n_stack_finalize,
        )?;

        Ok(inst_circuit
            .layout
            .target_wire_id
            .map(|target_wire_id| NodeOutputType::WireOut(inst_node_id, target_wire_id)))
    }

    fn construct_graph(
        graph_builder: &mut CircuitGraphBuilder<F>,
        chip_builder: &mut SingerChipBuilder<F>,
        inst_circuits: &[InstCircuit<F>],
        _real_n_instances: usize,
        params: &SingerParams,
    ) -> Result<Option<NodeOutputType>, ZKVMError> {
        // Add the instruction circuit to the graph.
        let inst_circuit = &inst_circuits[0];
        let n_witness_in = inst_circuit.circuit.n_witness_in;
        let inst_node_id = graph_builder.add_node(
            stringify!(ReturnInstruction),
            &inst_circuit.circuit,
            vec![PredType::Source; n_witness_in],
        )?;
        chip_builder.construct_chip_check_graph(
            graph_builder,
            inst_node_id,
            &inst_circuit.layout.chip_check_wire_id,
            1,
        )?;

        // Add the public output load circuit to the graph.
        let pub_out_load_circuit = &inst_circuits[1];
        let n_witness_in = pub_out_load_circuit.circuit.n_witness_in;
        let mut preds = vec![PredType::Source; n_witness_in];
        preds[pub_out_load_circuit.layout.pred_dup_wire_id.unwrap() as usize] =
            PredType::PredWireDup(NodeOutputType::WireOut(
                inst_node_id,
                inst_circuit.layout.succ_dup_wires_id[0],
            ));
        let pub_out_load_node_id = graph_builder.add_node(
            stringify!(ReturnPublicOutLoad),
            &pub_out_load_circuit.circuit,
            preds,
        )?;
        chip_builder.construct_chip_check_graph(
            graph_builder,
            pub_out_load_node_id,
            &pub_out_load_circuit.layout.chip_check_wire_id,
            params.n_public_output_bytes,
        )?;

        // Add the rest memory load circuit to the graph.
        let rest_mem_load_circuit = &inst_circuits[2];
        let n_witness_in = rest_mem_load_circuit.circuit.n_witness_in;
        let rest_mem_load_node_id = graph_builder.add_node(
            stringify!(ReturnRestMemLoad),
            &rest_mem_load_circuit.circuit,
            vec![PredType::Source; n_witness_in],
        )?;
        chip_builder.construct_chip_check_graph(
            graph_builder,
            rest_mem_load_node_id,
            &rest_mem_load_circuit.layout.chip_check_wire_id,
            params.n_mem_finalize,
        )?;

        // Add the rest memory store circuit to the graph.
        let rest_mem_store_circuit = &inst_circuits[3];
        let n_witness_in = rest_mem_store_circuit.circuit.n_witness_in;
        let rest_mem_store_node_id = graph_builder.add_node(
            stringify!(ReturnRestMemStore),
            &rest_mem_store_circuit.circuit,
            vec![PredType::Source; n_witness_in],
        )?;
        chip_builder.construct_chip_check_graph(
            graph_builder,
            rest_mem_store_node_id,
            &rest_mem_store_circuit.layout.chip_check_wire_id,
            params.n_mem_initialize,
        )?;

        // Add the rest stack pop circuit to the graph.
        let rest_stack_pop_circuit = &inst_circuits[4];
        let n_witness_in = rest_stack_pop_circuit.circuit.n_witness_in;
        let rest_stack_pop_node_id = graph_builder.add_node(
            stringify!(ReturnRestStackPop),
            &rest_stack_pop_circuit.circuit,
            vec![PredType::Source; n_witness_in],
        )?;
        chip_builder.construct_chip_check_graph(
            graph_builder,
            rest_stack_pop_node_id,
            &rest_stack_pop_circuit.layout.chip_check_wire_id,
            params.n_stack_finalize,
        )?;

        Ok(inst_circuit
            .layout
            .target_wire_id
            .map(|target_wire_id| NodeOutputType::WireOut(inst_node_id, target_wire_id)))
    }
}

register_witness!(
    ReturnInstruction,
    phase0 {
        pc => PCUInt::N_OPRAND_CELLS,
        stack_ts => TSUInt::N_OPRAND_CELLS,
        memory_ts => TSUInt::N_OPRAND_CELLS,
        stack_top => 1,
        clk => 1,

        old_stack_ts0 => TSUInt::N_OPRAND_CELLS,
        old_stack_ts1 => TSUInt::N_OPRAND_CELLS,

        offset => StackUInt::N_OPRAND_CELLS,
        mem_length => StackUInt::N_OPRAND_CELLS
    }
);

impl ReturnInstruction {
    const OPCODE: OpcodeType = OpcodeType::RETURN;
}

impl<F: SmallField> Instruction<F> for ReturnInstruction {
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
        ram_handler.state_in(
            &mut circuit_builder,
            pc.values(),
            stack_ts.values(),
            &memory_ts,
            stack_top,
            clk,
        );

        // Check the range of stack_top - 2 is within [0, 1 << STACK_TOP_BIT_WIDTH).
        rom_handler.range_check_stack_top(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(2)),
        )?;

        // Pop offset and mem_size from stack
        let old_stack_ts0 = StackUInt::try_from(&phase0[Self::phase0_old_stack_ts0()])?;
        let offset = StackUInt::try_from(&phase0[Self::phase0_offset()])?;
        ram_handler.stack_pop(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(1)),
            old_stack_ts0.values(),
            offset.values(),
        );

        let old_stack_ts1 = StackUInt::try_from(&phase0[Self::phase0_old_stack_ts1()])?;
        let length = StackUInt::try_from(&phase0[Self::phase0_mem_length()])?;
        ram_handler.stack_pop(
            &mut circuit_builder,
            stack_top_expr.sub(F::BaseField::from(2)),
            &old_stack_ts1.values(),
            length.values(),
        );

        // Bytecode check for (pc, ret)
        rom_handler.bytecode_with_pc_opcode(&mut circuit_builder, pc.values(), Self::OPCODE);

        let (ram_load_id, ram_store_id) = ram_handler.finalize(&mut circuit_builder);
        let rom_id = rom_handler.finalize(&mut circuit_builder);
        circuit_builder.configure();

        let outputs_wire_id = [ram_load_id, ram_store_id, rom_id];

        // Copy length to the target wire.
        let (target_wire_id, target) =
            circuit_builder.create_witness_out(StackUInt::N_OPRAND_CELLS);
        let length = length.values();
        for i in 1..length.len() {
            circuit_builder.assert_const(length[i], 0);
        }
        circuit_builder.add(target[0], length[0], F::BaseField::ONE);

        // Copy offset to wires of public output load circuit.
        let (pub_out_wire_id, pub_out) =
            circuit_builder.create_witness_out(ReturnPublicOutLoad::pred_size());
        let pub_out_offset = &pub_out[ReturnPublicOutLoad::pred_offset()];
        let offset = offset.values();
        add_assign_each_cell(&mut circuit_builder, pub_out_offset, offset);

        Ok(InstCircuit {
            circuit: Arc::new(Circuit::new(&circuit_builder)),
            layout: InstCircuitLayout {
                chip_check_wire_id: outputs_wire_id,
                phases_wire_id: vec![phase0_wire_id],
                target_wire_id: Some(target_wire_id),
                succ_dup_wires_id: vec![pub_out_wire_id],
                ..Default::default()
            },
        })
    }

    fn generate_wires_in(record: &Record) -> CircuitWiresIn<F> {
        let mut wire_values = vec![F::ZERO; Self::phase0_size()];
        copy_pc_from_record!(wire_values, record);
        copy_stack_ts_from_record!(wire_values, record);
        copy_memory_ts_from_record!(wire_values, record);
        copy_stack_top_from_record!(wire_values, record);
        copy_clock_from_record!(wire_values, record);
        copy_operand_timestamp_from_record!(wire_values, record, phase0_old_stack_ts0, 0, 0);
        copy_operand_timestamp_from_record!(wire_values, record, phase0_old_stack_ts1, 0, 1);

        copy_operand_from_record!(wire_values, record, phase0_offset, 0);
        copy_operand_from_record!(wire_values, record, phase0_mem_length, 0);

        vec![LayerWitness {
            instances: vec![wire_values],
        }]
    }
}

register_witness!(
    ReturnPublicOutLoad,
    pred {
        offset => StackUInt::N_OPRAND_CELLS
    },
    public_io {
        byte => 1
    },
    phase0 {
        old_memory_ts => TSUInt::N_OPRAND_CELLS,

        offset_add => UIntAddSub::<StackUInt>::N_WITNESS_CELLS
    }
);

impl<F: SmallField> Instruction<F> for ReturnPublicOutLoad {
    fn construct_circuit(challenges: ChipChallenges) -> Result<InstCircuit<F>, ZKVMError> {
        let mut circuit_builder = CircuitBuilder::new();
        let (pred_wire_id, pred) = circuit_builder.create_witness_in(Self::pred_size());
        let (phase0_wire_id, phase0) = circuit_builder.create_witness_in(Self::phase0_size());
        let mut ram_handler = RAMHandler::new(&challenges);
        let mut rom_handler = ROMHandler::new(&challenges);

        // Compute offset + counter
        let delta = circuit_builder.create_counter_in(0);
        let offset = StackUInt::try_from(&pred[Self::pred_offset()])?;
        let offset_add_delta_witness = &phase0[Self::phase0_offset_add()];
        let new_offset = UIntAddSub::<StackUInt>::add_small(
            &mut circuit_builder,
            &mut rom_handler,
            &offset,
            delta[0],
            offset_add_delta_witness,
        )?;

        // Load from memory
        let mem_byte = pred[Self::public_io_byte().start];
        let old_memory_ts = TSUInt::try_from(&phase0[Self::phase0_old_memory_ts()])?;
        ram_handler.oam_load(
            &mut circuit_builder,
            new_offset.values(),
            old_memory_ts.values(),
            &[mem_byte],
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
                pred_dup_wire_id: Some(pred_wire_id),
                ..Default::default()
            },
        })
    }

    fn generate_wires_in(record: &Record) -> CircuitWiresIn<F> {
        let offset = record.operands[0];
        let len = record.operands[1].as_limbs()[0] as usize;

        let mut public_io_values = vec![vec![F::ZERO; Self::public_io_size()]; len];
        let mut phase0_wire_values = vec![vec![F::ZERO; Self::phase0_size()]; len];
        for i in 0..len {
            public_io_values[i][..]
                .copy_from_slice(&[F::from(record.operands[i + 2].as_limbs()[0])]);
            phase0_wire_values[i][Self::phase0_old_memory_ts()]
                .copy_from_slice(&[F::from(record.operands_timestamps[i + 2])]);
            let delta = U256::from(i);
            copy_range_values_from_u256!(phase0_wire_values[i], phase0_offset_add, offset + delta);
            copy_carry_values_from_addends!(
                phase0_wire_values[i],
                phase0_offset_add,
                offset,
                delta
            );
        }

        vec![
            LayerWitness {
                instances: vec![Vec::new()],
            },
            LayerWitness {
                instances: public_io_values,
            },
            LayerWitness {
                instances: phase0_wire_values,
            },
        ]
    }
}

register_witness!(
    ReturnRestMemLoad,
    phase0 {
        mem_byte => 1,
        offset => StackUInt::N_OPRAND_CELLS,
        old_memory_ts => TSUInt::N_OPRAND_CELLS
    }
);

impl<F: SmallField> Instruction<F> for ReturnRestMemLoad {
    fn construct_circuit(challenges: ChipChallenges) -> Result<InstCircuit<F>, ZKVMError> {
        let mut circuit_builder = CircuitBuilder::new();
        let (phase0_wire_id, phase0) = circuit_builder.create_witness_in(Self::phase0_size());
        let mut ram_handler = RAMHandler::new(&challenges);

        // Load from memory
        let offset = &phase0[Self::phase0_offset()];
        let mem_byte = phase0[Self::phase0_mem_byte().start];
        let old_memory_ts = TSUInt::try_from(&phase0[Self::phase0_old_memory_ts()])?;
        ram_handler.oam_load(
            &mut circuit_builder,
            &offset,
            old_memory_ts.values(),
            &[mem_byte],
        );

        let (ram_load_id, ram_store_id) = ram_handler.finalize(&mut circuit_builder);
        circuit_builder.configure();

        let outputs_wire_id = [ram_load_id, ram_store_id, None];

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
        let mut wire_values = Vec::new();
        for i in 0..record.ret_info.rest_memory_loads.len() {
            let (offset, timestamp, value) = record.ret_info.rest_memory_loads[i];
            let mut wire_value = vec![F::ZERO; Self::phase0_size()];
            wire_value[Self::phase0_mem_byte()].copy_from_slice(&[F::from(value as u64)]);
            wire_value[Self::phase0_offset()]
                .copy_from_slice(StackUInt::uint_to_field_elems(offset).as_slice());
            wire_value[Self::phase0_old_memory_ts()]
                .copy_from_slice(TSUInt::uint_to_field_elems(timestamp).as_slice());
            wire_values.push(wire_value);
        }

        vec![LayerWitness {
            instances: wire_values,
        }]
    }
}

register_witness!(
    ReturnRestMemStore,
    phase0 {
        mem_byte => 1,
        offset => StackUInt::N_OPRAND_CELLS
    }
);

impl<F: SmallField> Instruction<F> for ReturnRestMemStore {
    fn construct_circuit(challenges: ChipChallenges) -> Result<InstCircuit<F>, ZKVMError> {
        let mut circuit_builder = CircuitBuilder::new();
        let (phase0_wire_id, phase0) = circuit_builder.create_witness_in(Self::phase0_size());
        let mut ram_handler = RAMHandler::new(&challenges);

        // Load from memory
        let offset = &phase0[Self::phase0_offset()];
        let mem_byte = phase0[Self::phase0_mem_byte().start];
        let memory_ts = circuit_builder.create_cells(StackUInt::N_OPRAND_CELLS);
        ram_handler.oam_store(&mut circuit_builder, offset, &memory_ts, &[mem_byte]);

        let (ram_load_id, ram_store_id) = ram_handler.finalize(&mut circuit_builder);
        circuit_builder.configure();

        let outputs_wire_id = [ram_load_id, ram_store_id, None];

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
        let mut wire_values = Vec::new();
        for i in 0..record.ret_info.rest_memory_store.len() {
            let (offset, value) = record.ret_info.rest_memory_store[i];
            let mut wire_value = vec![F::ZERO; Self::phase0_size()];
            // All memory addresses are initialized with zero when first
            // accessed.
            wire_value[Self::phase0_mem_byte()].copy_from_slice(&[F::from(value as u64)]);
            wire_value[Self::phase0_offset()]
                .copy_from_slice(StackUInt::uint_to_field_elems(offset).as_slice());
            wire_values.push(wire_value);
        }

        vec![LayerWitness {
            instances: wire_values,
        }]
    }
}

pub struct ReturnRestStackPop;

register_witness!(
    ReturnRestStackPop,
    phase0 {
        old_stack_ts => TSUInt::N_OPRAND_CELLS,
        stack_values => StackUInt::N_OPRAND_CELLS
    }
);

impl<F: SmallField> Instruction<F> for ReturnRestStackPop {
    fn construct_circuit(challenges: ChipChallenges) -> Result<InstCircuit<F>, ZKVMError> {
        let mut circuit_builder = CircuitBuilder::new();
        let (phase0_wire_id, phase0) = circuit_builder.create_witness_in(Self::phase0_size());
        let mut ram_handler = RAMHandler::new(&challenges);

        // Pop from stack
        let stack_top = circuit_builder.create_counter_in(0);
        let stack_values = &phase0[Self::phase0_stack_values()];

        let old_stack_ts = TSUInt::try_from(&phase0[Self::phase0_old_stack_ts()])?;
        ram_handler.stack_pop(
            &mut circuit_builder,
            stack_top[0].into(),
            old_stack_ts.values(),
            stack_values,
        );

        let (ram_load_id, ram_store_id) = ram_handler.finalize(&mut circuit_builder);
        circuit_builder.configure();

        let outputs_wire_id = [ram_load_id, ram_store_id, None];

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
        let mut wire_values = Vec::new();
        for i in 0..record.ret_info.rest_stack.len() {
            let (timestamp, value) = record.ret_info.rest_stack[i];
            let mut wire_value = vec![F::ZERO; Self::phase0_size()];
            // All memory addresses are initialized with zero when first
            // accessed.
            wire_value[Self::phase0_old_stack_ts()]
                .copy_from_slice(TSUInt::uint_to_field_elems(timestamp).as_slice());
            wire_value[Self::phase0_stack_values()]
                .copy_from_slice(StackUInt::u256_to_field_elems(value).as_slice());
            wire_values.push(wire_value);
        }

        vec![LayerWitness {
            instances: wire_values,
        }]
    }
}
