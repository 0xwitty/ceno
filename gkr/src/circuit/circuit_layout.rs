use core::fmt;
use std::collections::HashMap;

use ark_std::iterable::Iterable;
use goldilocks::SmallField;
use itertools::Itertools;
use simple_frontend::structs::{
    CellId, CellType, ChallengeConst, CircuitBuilder, ConstantType, GateType, InType, LayerId,
    OutType,
};

use crate::{
    structs::{Circuit, Gate1In, Gate2In, Gate3In, GateCIn, Layer},
    utils::{ceil_log2, MatrixMLEColumnFirst, MatrixMLERowFirst},
};

impl<F: SmallField> Circuit<F> {
    /// Generate the circuit from circuit builder.
    pub fn new(circuit_builder: &CircuitBuilder<F>) -> Self {
        assert!(circuit_builder.n_layers.is_some());
        let n_layers = circuit_builder.n_layers.unwrap();

        // ==================================
        // Put cells into layers. Maintain two vectors:
        // - `layers_of_cell_id` stores all cell ids in each layer;
        // - `wire_ids_in_layer` stores the wire id of each cell in its layer.
        // ==================================
        let (layers_of_cell_id, wire_ids_in_layer) = {
            let mut layers_of_cell_id = vec![vec![]; n_layers as usize];
            let mut wire_ids_in_layer = vec![0; circuit_builder.cells.len()];
            for i in 0..circuit_builder.cells.len() {
                if let Some(layer) = circuit_builder.cells[i].layer {
                    wire_ids_in_layer[i] = layers_of_cell_id[layer as usize].len();
                    layers_of_cell_id[layer as usize].push(i);
                } else {
                    panic!("The layer of the cell is not specified.");
                }
            }
            // The layers are numbered from the output to the inputs.
            layers_of_cell_id.reverse();
            (layers_of_cell_id, wire_ids_in_layer)
        };

        let mut layers = (0..n_layers)
            .map(|i| Layer::<F> {
                add_consts: vec![],
                adds: vec![],
                mul2s: vec![],
                mul3s: vec![],
                assert_consts: vec![],
                copy_to: HashMap::new(),
                paste_from: HashMap::new(),
                num_vars: 0,
                max_previous_num_vars: 0,
                layer_id: i,
            })
            .collect_vec();

        // ==================================
        // From the input layer to the output layer, construct the gates. If a
        // gate has the input from multiple previous layers, then we need to
        // copy them to the current layer.
        // ==================================

        // Input layer if pasted from wires_in and constant.
        let (in_cell_ids, out_cell_ids) = {
            let mut in_cell_ids = HashMap::new();
            let mut out_cell_ids = vec![vec![]; circuit_builder.n_wires_out() as usize];
            for (id, cell) in circuit_builder.cells.iter().enumerate() {
                if let Some(cell_type) = cell.cell_type {
                    match cell_type {
                        CellType::In(in_type) => {
                            in_cell_ids.entry(in_type).or_insert(vec![]).push(id);
                        }
                        CellType::Out(OutType::Wire(wire_id)) => {
                            out_cell_ids[wire_id as usize].push(id);
                        }
                    }
                }
            }
            (in_cell_ids, out_cell_ids)
        };

        let mut input_paste_from_in = Vec::with_capacity(in_cell_ids.len());
        for (ty, in_cell_ids) in in_cell_ids.iter() {
            #[cfg(feature = "debug")]
            in_cell_ids.iter().enumerate().map(|(i, cell_id)| {
                // Each wire_in should be assigned with a consecutive
                // input layer segment. Then we can use a special
                // sumcheck protocol to prove it.
                assert!(
                    i == 0 || wire_ids_in_layer[*cell_id] == wire_ids_in_layer[wire_in[i - 1]] + 1
                );
            });
            input_paste_from_in.push((
                *ty,
                wire_ids_in_layer[in_cell_ids[0]],
                wire_ids_in_layer[in_cell_ids[in_cell_ids.len() - 1]] + 1,
            ));
        }

        // TODO: This is to avoid incorrect use of input paste_from. To be refined.
        for (ty, left, right) in input_paste_from_in.iter() {
            if let InType::Wire(id) = *ty {
                layers[n_layers as usize - 1]
                    .paste_from
                    .insert(id as LayerId, (*left..*right).collect_vec());
            }
        }

        let max_wires_in_num_vars = {
            let mut max_wires_in_num_vars = None;
            let max_wires_in_size = in_cell_ids
                .iter()
                .map(|(ty, vec)| {
                    if let InType::Constant(_) = *ty {
                        0
                    } else {
                        vec.len()
                    }
                })
                .max()
                .unwrap();
            if max_wires_in_size > 0 {
                max_wires_in_num_vars = Some(ceil_log2(max_wires_in_size) as usize);
            }
            max_wires_in_num_vars
        };

        // Compute gates and copy constraints of the other layers.
        for layer_id in (0..n_layers - 1).rev() {
            // current_subsets: old_layer_id -> (old_wire_id, new_wire_id)
            // It only stores the wires not in the current layer.
            let new_layer_id = layer_id + 1;
            let subsets = {
                let mut subsets = HashMap::new();
                let mut wire_id_assigner = layers_of_cell_id[new_layer_id as usize]
                    .len()
                    .next_power_of_two();
                let mut update_subset = |old_cell_id: CellId| {
                    let old_layer_id =
                        n_layers - 1 - circuit_builder.cells[old_cell_id].layer.unwrap();
                    #[cfg(debug_assertions)]
                    {
                        if old_layer_id == 0 {
                            println!(
                                "new_layer_id {:?}, old_layer_id {:?}, old_cell_id {:?}",
                                new_layer_id, old_layer_id, old_cell_id
                            );
                            println!(
                                "cells[old_cell_id].layer {:?}",
                                circuit_builder.cells[old_cell_id].layer.unwrap()
                            );
                        }
                    }
                    if old_layer_id == new_layer_id {
                        return;
                    }
                    subsets
                        .entry(old_layer_id)
                        .or_insert(HashMap::new())
                        .insert(wire_ids_in_layer[old_cell_id], wire_id_assigner);
                    wire_id_assigner += 1;
                };
                for cell_id in layers_of_cell_id[layer_id as usize].iter() {
                    #[cfg(debug_assertions)]
                    {
                        println!("layer_id {:?}, cell_id {:?}", layer_id, cell_id);
                    }
                    let cell = &circuit_builder.cells[*cell_id];
                    for gate in cell.gates.iter() {
                        match gate {
                            GateType::Add(in_0, _) => {
                                update_subset(*in_0);
                            }
                            GateType::Mul2(in_0, in_1, _) => {
                                update_subset(*in_0);
                                update_subset(*in_1);
                            }
                            GateType::Mul3(in_0, in_1, in_2, _) => {
                                update_subset(*in_0);
                                update_subset(*in_1);
                                update_subset(*in_2);
                            }
                            _ => {}
                        }
                    }
                }
                layers[new_layer_id as usize].num_vars = ceil_log2(wire_id_assigner) as usize;
                subsets
            };

            // Copy subsets from previous layers and put them into the current
            // layer.
            for (old_layer_id, old_wire_ids) in subsets.iter() {
                for (old_wire_id, new_wire_id) in old_wire_ids.iter() {
                    #[cfg(debug_assertions)]
                    {
                        println!("old_wire_id {:?}, new_wire_id {:?}, old_layer_id {:?}, new_layer_id {:?}", 
                                    old_wire_id, new_wire_id, *old_layer_id, new_layer_id);
                        assert!(
                            new_layer_id < *old_layer_id,
                            "layer paste_from err: need old_layer_id {:?} > new_layer_id {:?}",
                            *old_layer_id,
                            new_layer_id
                        );
                    }
                    layers[new_layer_id as usize]
                        .paste_from
                        .entry(*old_layer_id)
                        .or_insert(vec![])
                        .push(*new_wire_id);
                    layers[*old_layer_id as usize]
                        .copy_to
                        .entry(new_layer_id)
                        .or_insert(vec![])
                        .push(*old_wire_id);
                }
            }
            layers[new_layer_id as usize].max_previous_num_vars = layers[new_layer_id as usize]
                .max_previous_num_vars
                .max(ceil_log2(
                    layers[new_layer_id as usize]
                        .paste_from
                        .iter()
                        .map(|(_, old_wire_ids)| old_wire_ids.len())
                        .max()
                        .unwrap_or(1),
                ));
            layers[layer_id as usize].max_previous_num_vars =
                layers[new_layer_id as usize].num_vars;

            // Compute gates with new wire ids accordingly.
            let current_wire_id = |old_cell_id: CellId| -> CellId {
                let old_layer_id = n_layers - 1 - circuit_builder.cells[old_cell_id].layer.unwrap();
                let old_wire_id = wire_ids_in_layer[old_cell_id];
                if old_layer_id == new_layer_id {
                    return old_wire_id;
                }
                *subsets
                    .get(&old_layer_id)
                    .unwrap()
                    .get(&old_wire_id)
                    .unwrap()
            };
            for (i, cell_id) in layers_of_cell_id[layer_id as usize].iter().enumerate() {
                let cell = &circuit_builder.cells[*cell_id];
                if let Some(assert_const) = cell.assert_const {
                    layers[layer_id as usize].assert_consts.push(GateCIn {
                        idx_out: i,
                        constant: ConstantType::Field(assert_const),
                    });
                }
                for gate in cell.gates.iter() {
                    match gate {
                        GateType::AddC(c) => {
                            layers[layer_id as usize].add_consts.push(GateCIn {
                                idx_out: i,
                                constant: *c,
                            });
                        }
                        GateType::Add(in_0, scalar) => {
                            layers[layer_id as usize].adds.push(Gate1In {
                                idx_in: current_wire_id(*in_0),
                                idx_out: i,
                                scalar: *scalar,
                            });
                        }
                        GateType::Mul2(in_0, in_1, scalar) => {
                            layers[layer_id as usize].mul2s.push(Gate2In {
                                idx_in1: current_wire_id(*in_0),
                                idx_in2: current_wire_id(*in_1),
                                idx_out: i,
                                scalar: *scalar,
                            });
                        }
                        GateType::Mul3(in_0, in_1, in_2, scalar) => {
                            layers[layer_id as usize].mul3s.push(Gate3In {
                                idx_in1: current_wire_id(*in_0),
                                idx_in2: current_wire_id(*in_1),
                                idx_in3: current_wire_id(*in_2),
                                idx_out: i,
                                scalar: *scalar,
                            });
                        }
                    }
                }
            }
        }

        // Compute the copy_to from the output layer to the wires_out.
        layers[0].num_vars = ceil_log2(layers_of_cell_id[0].len()) as usize;

        let output_copy_to = out_cell_ids
            .iter()
            .map(|cell_ids| {
                cell_ids
                    .iter()
                    .map(|cell_id| wire_ids_in_layer[*cell_id])
                    .collect_vec()
            })
            .collect_vec();

        // TODO: This is to avoid incorrect use of output copy_to. To be refined.
        for (id, wire_out) in output_copy_to.iter().enumerate() {
            layers[0]
                .copy_to
                .insert(id as LayerId, wire_out.iter().map(|x| *x).collect_vec());
        }

        Self {
            layers,
            copy_to_wires_out: output_copy_to,
            n_wires_in: circuit_builder.n_wires_in(),
            paste_from_in: input_paste_from_in,
            max_wires_in_num_vars,
        }
    }

    pub(crate) fn generate_basefield_challenges(
        &self,
        challenges: &[F],
    ) -> HashMap<ChallengeConst, Vec<F::BaseField>> {
        let mut challenge_exps = HashMap::<ChallengeConst, F>::new();
        let mut update_const = |constant| match constant {
            ConstantType::Challenge(c, _) => {
                challenge_exps
                    .entry(c)
                    .or_insert(challenges[c.challenge as usize].pow(&[c.exp]));
            }
            ConstantType::ChallengeScaled(c, _, _) => {
                challenge_exps
                    .entry(c)
                    .or_insert(challenges[c.challenge as usize].pow(&[c.exp]));
            }
            _ => {}
        };
        self.layers.iter().for_each(|layer| {
            layer
                .add_consts
                .iter()
                .for_each(|gate| update_const(gate.constant));
            layer.adds.iter().for_each(|gate| update_const(gate.scalar));
            layer
                .mul2s
                .iter()
                .for_each(|gate| update_const(gate.scalar));
            layer
                .mul3s
                .iter()
                .for_each(|gate| update_const(gate.scalar));
        });
        challenge_exps
            .into_iter()
            .map(|(k, v)| (k, v.to_limbs()))
            .collect()
    }

    pub fn last_layer_ref(&self) -> &Layer<F> {
        self.layers.first().unwrap()
    }

    pub fn first_layer_ref(&self) -> &Layer<F> {
        self.layers.last().unwrap()
    }

    pub fn output_num_vars(&self) -> usize {
        self.last_layer_ref().num_vars
    }

    pub fn output_size(&self) -> usize {
        1 << self.last_layer_ref().num_vars
    }

    pub fn is_input_layer(&self, layer_id: LayerId) -> bool {
        layer_id as usize == self.layers.len() - 1
    }

    pub fn is_output_layer(&self, layer_id: LayerId) -> bool {
        layer_id == 0
    }
}

impl<F: SmallField> Layer<F> {
    pub fn size(&self) -> usize {
        1 << self.num_vars
    }

    pub fn num_vars(&self) -> usize {
        self.num_vars
    }

    pub fn max_previous_num_vars(&self) -> usize {
        self.max_previous_num_vars
    }

    pub fn max_previous_size(&self) -> usize {
        1 << self.max_previous_num_vars
    }

    pub fn paste_from_fix_variables_eq(
        &self,
        old_layer_id: LayerId,
        current_point_eq: &[F],
    ) -> Vec<F> {
        assert_eq!(current_point_eq.len(), self.size());
        self.paste_from
            .get(&old_layer_id)
            .unwrap()
            .as_slice()
            .fix_row_col_first(current_point_eq, self.max_previous_num_vars)
    }

    pub fn paste_from_eval_eq(
        &self,
        old_layer_id: LayerId,
        current_point_eq: &[F],
        subset_point_eq: &[F],
    ) -> F {
        assert_eq!(current_point_eq.len(), self.size());
        assert_eq!(subset_point_eq.len(), self.max_previous_size());
        self.paste_from
            .get(&old_layer_id)
            .unwrap()
            .as_slice()
            .eval_col_first(current_point_eq, subset_point_eq)
    }

    pub fn copy_to_fix_variables(&self, new_layer_id: LayerId, subset_point_eq: &[F]) -> Vec<F> {
        let old_wire_ids = self.copy_to.get(&new_layer_id).unwrap();
        old_wire_ids
            .as_slice()
            .fix_row_row_first(subset_point_eq, self.num_vars)
    }

    pub fn copy_to_eval_eq(
        &self,
        new_layer_id: LayerId,
        subset_point_eq: &[F],
        current_point_eq: &[F],
    ) -> F {
        self.copy_to
            .get(&new_layer_id)
            .unwrap()
            .as_slice()
            .eval_row_first(subset_point_eq, current_point_eq)
    }
}

impl<F: SmallField> fmt::Debug for Layer<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Layer {{")?;
        writeln!(f, "  layer_id: {}", self.layer_id)?;
        writeln!(f, "  num_vars: {}", self.num_vars)?;
        writeln!(f, "  max_previous_num_vars: {}", self.max_previous_num_vars)?;
        writeln!(f, "  adds: ")?;
        for add in self.adds.iter() {
            writeln!(f, "    {:?}", add)?;
        }
        writeln!(f, "  mul2s: ")?;
        for mul2 in self.mul2s.iter() {
            writeln!(f, "    {:?}", mul2)?;
        }
        writeln!(f, "  mul3s: ")?;
        for mul3 in self.mul3s.iter() {
            writeln!(f, "    {:?}", mul3)?;
        }
        writeln!(f, "  assert_consts: ")?;
        for assert_const in self.assert_consts.iter() {
            writeln!(f, "    {:?}", assert_const)?;
        }
        writeln!(f, "  copy_to: {:?}", self.copy_to)?;
        writeln!(f, "  paste_from: {:?}", self.paste_from)?;
        writeln!(f, "}}")
    }
}

impl<F: SmallField> fmt::Debug for Circuit<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Circuit {{")?;
        writeln!(f, "  output_copy_to: {:?}", self.copy_to_wires_out)?;
        writeln!(f, "  layers: ")?;
        for layer in self.layers.iter() {
            writeln!(f, "    {:?}", layer)?;
        }
        writeln!(f, "  n_wires_in: {}", self.n_wires_in)?;
        writeln!(f, "  paste_from_in: {:?}", self.paste_from_in)?;
        writeln!(
            f,
            "  max_wires_in_num_vars: {:?}",
            self.max_wires_in_num_vars
        )?;
        writeln!(f, "}}")
    }
}
