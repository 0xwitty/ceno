use gkr::{
    structs::{Point, PointAndEval},
    utils::MultilinearExtensionFromVectors,
};
use goldilocks::SmallField;
use itertools::Itertools;

use crate::structs::{CircuitGraph, CircuitGraphWitness, NodeOutputType, TargetEvaluations};

impl<F: SmallField> CircuitGraph<F> {
    pub fn target_evals(
        &self,
        witness: &CircuitGraphWitness<F::BaseField>,
        point: &Point<F>,
    ) -> TargetEvaluations<F> {
        // println!("targets: {:?}, point: {:?}", self.targets, point);
        let target_evals = self
            .targets
            .iter()
            .map(|target| {
                let poly = match target {
                    NodeOutputType::OutputLayer(node_id) => witness.node_witnesses[*node_id]
                        .output_layer_witness_ref()
                        .instances
                        .as_slice()
                        .original_mle(),
                    NodeOutputType::WireOut(node_id, wit_id) => witness.node_witnesses[*node_id]
                        .witness_out_ref()[*wit_id as usize]
                        .instances
                        .as_slice()
                        .original_mle(),
                };
                // println!("target: {:?}, poly.num_vars: {:?}", target, poly.num_vars);
                let p = point[..poly.num_vars].to_vec();
                PointAndEval::new_from_ref(&p, &poly.evaluate(&p))
            })
            .collect_vec();
        TargetEvaluations(target_evals)
    }
}
