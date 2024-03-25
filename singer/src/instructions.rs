use num_traits::FromPrimitive;
use revm_interpreter::Record;
use std::{mem, sync::Arc};

use gkr::structs::Circuit;
use gkr_graph::structs::{CircuitGraphBuilder, NodeOutputType, PredType};
use goldilocks::SmallField;
use simple_frontend::structs::WitnessId;

use singer_utils::{chips::SingerChipBuilder, structs::ChipChallenges};
use strum_macros::EnumIter;

use crate::{error::ZKVMError, CircuitWiresIn, SingerParams};

use crate::{constants::OpcodeType, error::ZKVMError, CircuitWiresIn};

use crate::{chips::SingerChipBuilder, SingerParams};

use self::{
    add::AddInstruction, calldataload::CalldataloadInstruction, dup::DupInstruction,
    gt::GtInstruction, jump::JumpInstruction, jumpdest::JumpdestInstruction,
    jumpi::JumpiInstruction, mstore::MstoreInstruction, pop::PopInstruction, push::PushInstruction,
    ret::ReturnInstruction, swap::SwapInstruction,
};

// arithmetic
pub mod add;

// bitwise
pub mod gt;

// control
pub mod jump;
pub mod jumpdest;
pub mod jumpi;
pub mod ret;

// stack
pub mod dup;
pub mod pop;
pub mod push;
pub mod swap;

// memory
pub mod mstore;

// system
pub mod calldataload;

#[derive(Clone, Debug)]
pub struct SingerCircuitBuilder<F: SmallField> {
    /// Opcode circuits
    pub(crate) insts_circuits: [Vec<InstCircuit<F>>; 256],
    pub(crate) challenges: ChipChallenges,
}

impl<F: SmallField> SingerCircuitBuilder<F> {
    pub fn new(challenges: ChipChallenges) -> Result<Self, ZKVMError> {
        let mut insts_circuits = Vec::with_capacity(256);
        for opcode in 0..=255 {
            insts_circuits.push(construct_instruction_circuits(opcode, challenges)?);
        }
        let insts_circuits: [Vec<InstCircuit<F>>; 256] = insts_circuits
            .try_into()
            .map_err(|_| ZKVMError::CircuitError)?;
        Ok(Self {
            insts_circuits,
            challenges,
        })
    }
}

/// Construct instruction circuits and its extensions.
pub(crate) fn construct_instruction_circuits<F: SmallField>(
    opcode: u8,
    challenges: ChipChallenges,
) -> Result<Vec<InstCircuit<F>>, ZKVMError> {
    match OpcodeType::from_u8(opcode) {
        Some(OpcodeType::ADD) => AddInstruction::construct_circuits(challenges),
        Some(OpcodeType::GT) => GtInstruction::construct_circuits(challenges),
        Some(OpcodeType::CALLDATALOAD) => CalldataloadInstruction::construct_circuits(challenges),
        Some(OpcodeType::POP) => PopInstruction::construct_circuits(challenges),
        Some(OpcodeType::MSTORE) => MstoreInstruction::construct_circuits(challenges),
        Some(OpcodeType::JUMP) => JumpInstruction::construct_circuits(challenges),
        Some(OpcodeType::JUMPI) => JumpiInstruction::construct_circuits(challenges),
        Some(OpcodeType::JUMPDEST) => JumpdestInstruction::construct_circuits(challenges),
        Some(OpcodeType::PUSH1) => PushInstruction::<1>::construct_circuits(challenges),
        Some(OpcodeType::DUP1) => DupInstruction::<1>::construct_circuits(challenges),
        Some(OpcodeType::DUP2) => DupInstruction::<2>::construct_circuits(challenges),
        Some(OpcodeType::SWAP2) => SwapInstruction::<2>::construct_circuits(challenges),
        Some(OpcodeType::SWAP4) => SwapInstruction::<4>::construct_circuits(challenges),
        Some(OpcodeType::RETURN) => ReturnInstruction::construct_circuits(challenges),
        _ => unimplemented!(),
    }
}

pub(crate) fn construct_inst_graph_and_witness<F: SmallField>(
    opcode: u8,
    graph_builder: &mut CircuitGraphBuilder<F>,
    chip_builder: &mut SingerChipBuilder<F>,
    inst_circuits: &[InstCircuit<F>],
    sources: Vec<CircuitWiresIn<F::BaseField>>,
    real_challenges: &[F],
    real_n_instances: usize,
    params: &SingerParams,
) -> Result<Option<NodeOutputType>, ZKVMError> {
    let construct_circuit_graph = match OpcodeType::from_u8(opcode) {
        Some(OpcodeType::ADD) => AddInstruction::construct_circuit_graph,
        Some(OpcodeType::GT) => GtInstruction::construct_circuit_graph,
        Some(OpcodeType::CALLDATALOAD) => CalldataloadInstruction::construct_circuit_graph,
        Some(OpcodeType::POP) => PopInstruction::construct_circuit_graph,
        Some(OpcodeType::MSTORE) => MstoreInstruction::construct_circuit_graph,
        Some(OpcodeType::JUMP) => JumpInstruction::construct_circuit_graph,
        Some(OpcodeType::JUMPI) => JumpiInstruction::construct_circuit_graph,
        Some(OpcodeType::JUMPDEST) => JumpdestInstruction::construct_circuit_graph,
        Some(OpcodeType::PUSH1) => PushInstruction::<1>::construct_circuit_graph,
        Some(OpcodeType::DUP1) => DupInstruction::<1>::construct_circuit_graph,
        Some(OpcodeType::DUP2) => DupInstruction::<2>::construct_circuit_graph,
        Some(OpcodeType::SWAP2) => SwapInstruction::<2>::construct_circuit_graph,
        Some(OpcodeType::SWAP4) => SwapInstruction::<4>::construct_circuit_graph,
        Some(OpcodeType::RETURN) => ReturnInstruction::construct_circuit_graph,
        _ => unimplemented!(),
    };

    construct_circuit_graph(
        graph_builder,
        chip_builder,
        inst_circuits,
        sources,
        real_challenges,
        real_n_instances,
        params,
    )
}

pub(crate) fn construct_inst_graph<F: SmallField>(
    opcode: u8,
    graph_builder: &mut CircuitGraphBuilder<F>,
    chip_builder: &mut SingerChipBuilder<F>,
    inst_circuits: &[InstCircuit<F>],
    real_n_instances: usize,
    params: &SingerParams,
) -> Result<Option<NodeOutputType>, ZKVMError> {
    let construct_graph = match opcode {
        0x01 => AddInstruction::construct_graph,
        0x11 => GtInstruction::construct_graph,
        0x35 => CalldataloadInstruction::construct_graph,
        0x50 => PopInstruction::construct_graph,
        0x52 => MstoreInstruction::construct_graph,
        0x56 => JumpInstruction::construct_graph,
        0x57 => JumpiInstruction::construct_graph,
        0x5B => JumpdestInstruction::construct_graph,
        0x60 => PushInstruction::<1>::construct_graph,
        0x80 => DupInstruction::<1>::construct_graph,
        0x81 => DupInstruction::<2>::construct_graph,
        0x91 => SwapInstruction::<2>::construct_graph,
        0x93 => SwapInstruction::<4>::construct_graph,
        0xF3 => ReturnInstruction::construct_graph,
        _ => unimplemented!(),
    };

    construct_graph(
        graph_builder,
        chip_builder,
        inst_circuits,
        real_n_instances,
        params,
    )
}

#[derive(Clone, Copy, Debug, EnumIter)]
pub(crate) enum InstOutputType {
    RAMLoad,
    RAMStore,
    ROMInput,
}

#[derive(Clone, Debug)]
pub struct InstCircuit<F: SmallField> {
    pub(crate) circuit: Arc<Circuit<F>>,
    pub(crate) layout: InstCircuitLayout,
}

#[derive(Clone, Debug, Default)]
pub struct InstCircuitLayout {
    // Will be connected to the chips.
    pub(crate) chip_check_wire_id: [Option<(WitnessId, usize)>; 3],
    // Target. Especially for return the size of public output.
    pub(crate) target_wire_id: Option<WitnessId>,
    // Will be connected to the accessory circuits.
    pub(crate) succ_dup_wires_id: Vec<WitnessId>,
    pub(crate) succ_ooo_wires_id: Vec<WitnessId>,

    // Wires in index
    pub(crate) phases_wire_id: Vec<WitnessId>,
    // wire id fetched from pred circuit.
    pub(crate) pred_dup_wire_id: Option<WitnessId>,
    pub(crate) pred_ooo_wire_id: Option<WitnessId>,
}

pub(crate) trait Instruction<F: SmallField> {
    fn construct_circuit(challenges: ChipChallenges) -> Result<InstCircuit<F>, ZKVMError>;
    fn generate_wires_in(record: &Record) -> CircuitWiresIn<F>;
}

/// Construct the part of the circuit graph for an instruction.
pub(crate) trait InstructionGraph<F: SmallField> {
    type InstType: Instruction<F>;

    /// Construct instruction circuits and its extensions. Mostly there is no
    /// extensions.
    fn construct_circuits(challenges: ChipChallenges) -> Result<Vec<InstCircuit<F>>, ZKVMError> {
        let circuits = vec![Self::InstType::construct_circuit(challenges)?];
        Ok(circuits)
    }

    /// Add instruction circuits, its accessories and corresponding witnesses to
    /// the graph. Besides, Generate the tree-structured circuit to compute the
    /// product or fraction summation of the chip check wires.
    fn construct_graph_and_witness(
        graph_builder: &mut CircuitGraphBuilder<F>,
        chip_builder: &mut SingerChipBuilder<F>,
        inst_circuits: &[InstCircuit<F>],
        mut sources: Vec<CircuitWiresIn<F::BaseField>>,
        real_challenges: &[F],
        real_n_instances: usize,
        _: &SingerParams,
    ) -> Result<Option<NodeOutputType>, ZKVMError> {
        let inst_circuit = &inst_circuits[0];
        let inst_wires_in = mem::take(&mut sources[0]);
        let node_id = graph_builder.add_node_with_witness(
            stringify!(Self::InstType),
            &inst_circuits[0].circuit,
            vec![PredType::Source; inst_wires_in.len()],
            real_challenges.to_vec(),
            inst_wires_in,
            real_n_instances.next_power_of_two(),
        )?;

        chip_builder.construct_chip_check_graph_and_witness(
            graph_builder,
            node_id,
            &inst_circuit.layout.chip_check_wire_id,
            real_challenges,
            real_n_instances,
        )?;
        Ok(None)
    }

    /// Add instruction circuits, its accessories and corresponding witnesses to
    /// the graph. Besides, Generate the tree-structured circuit to compute the
    /// product or fraction summation of the chip check wires.
    fn construct_graph(
        graph_builder: &mut CircuitGraphBuilder<F>,
        chip_builder: &mut SingerChipBuilder<F>,
        inst_circuits: &[InstCircuit<F>],
        real_n_instances: usize,
        _: &SingerParams,
    ) -> Result<Option<NodeOutputType>, ZKVMError> {
        let inst_circuit = &inst_circuits[0];
        let node_id = graph_builder.add_node(
            stringify!(Self::InstType),
            &inst_circuits[0].circuit,
            vec![PredType::Source; inst_circuit.circuit.n_witness_in],
        )?;

        chip_builder.construct_chip_check_graph(
            graph_builder,
            node_id,
            &inst_circuit.layout.chip_check_wire_id,
            real_n_instances,
        )?;
        Ok(None)
    }
}
